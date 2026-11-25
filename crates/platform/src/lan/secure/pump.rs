// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Bounded fair scheduling of native inbound host operations.
// - Must-Not:
//   - Authenticate identities, decode title messages or simulate games.
// - Allows:
//   - Inputs: Explicit host configuration and nonblocking caller polls.
//   - Outputs: Verified lifecycle and schema-checked remote input events.
//   - Side effects: Owns a listener and an existing host I/O worker.
// - Split-When:
//   - Outbound state publication requires an independent scheduling policy.
// - Merge-When:
//   - Another shared host scheduler provides the same worker composition.
// - Summary:
//   - Poll independent remote peers without blocking the caller.
// - Description:
//   - Each pending handshake or active peer has one round-robin task.
// - Usage:
//   - Poll with a finite work budget and deliver each returned event.
// - Defaults:
//   - Explicit private-address binding and pre-approved identities only.

//! Fair caller-driven admission and inbound progress on the host worker.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use games::network::{MessageSchema, SessionConfig, SessionError};

use super::host::{HostError, HostSession};
use super::worker::{
    HostCompletion, HostOperation, HostOutcome, HostRequest, HostResult,
    HostWorker, OperationError, RejectionReason,
};
use super::{
    LanConfig, PeerFingerprint, PendingConnection, SecureLanListener, ServerTls,
};

/// One verified remote lifecycle or schema-checked input event.
#[derive(Debug)]
pub enum RemoteEvent<Message> {
    /// A host-enrolled peer joined or reclaimed its reserved identity.
    Connected(u64),
    /// A transfer failure closed the peer's channel and reserved its identity.
    Disconnected {
        /// Host-approved peer identity.
        peer: u64,
        /// Transport failure that ended the active connection.
        reason: ErrorKind,
    },
    /// An authenticated peer supplied a valid application message.
    Message {
        /// Host-approved peer identity, independent of the frame payload.
        peer: u64,
        /// Decoded application message.
        value: Message,
    },
}

/// Result of one nonblocking scheduler poll.
#[derive(Debug)]
pub enum PumpProgress<Message> {
    /// The caller owns this event and must deliver it before the next poll.
    Event(RemoteEvent<Message>),
    /// Nothing could progress immediately; a worker may still be busy.
    Idle,
    /// Internal work progressed without a deliverable application event.
    Progress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PollKind {
    Authenticate,
    Disconnect(u64),
    Receive(u64, Option<Instant>),
}

#[derive(Debug)]
struct PollTask {
    kind: PollKind,
    operation: HostOperation,
}

impl PollTask {
    const fn authenticate(pending: Box<PendingConnection>) -> Self {
        Self {
            kind: PollKind::Authenticate,
            operation: HostOperation::PollAuthenticate(pending),
        }
    }

    const fn disconnect(peer: u64) -> Self {
        Self {
            kind: PollKind::Disconnect(peer),
            operation: HostOperation::Disconnect(peer),
        }
    }

    const fn receive(peer: u64, deadline: Option<Instant>) -> Self {
        Self {
            kind: PollKind::Receive(peer, deadline),
            operation: HostOperation::PollReceive(peer),
        }
    }
}

/// Explicit endpoint, transport limits and pre-approved player identities.
#[derive(Clone, Copy, Debug)]
pub struct HostInputConfig<'roster> {
    /// Private interface and port chosen by the host.
    pub address: SocketAddr,
    /// Approved certificate fingerprints mapped to host player identities.
    pub enrolments: &'roster [(PeerFingerprint, u64)],
    /// Optional maximum time without a valid application frame.
    pub idle_timeout: Option<Duration>,
    /// Host-owned local player identities.
    pub local_peers: &'roster [u64],
    /// Opted-in roster and frame limits.
    pub session: SessionConfig,
    /// Deadline for each handshake or in-progress transfer.
    pub timeout: Duration,
}

/// Nonblocking host input scheduler over the existing authenticated worker.
///
/// Pending sockets are bounded by the listener permit count. Active
/// channels are bounded by the roster. Each socket or channel has exactly
/// one ready or in-flight poll task; completed tasks return to the tail.
/// The worker queues are sized for both populations together.
///
/// Polling does not open outbound sockets, schedule gameplay or buffer
/// delivered events. Callers must cap their own polls per update and stop
/// on backpressure rather than discarding a lifecycle or input event.
/// This inbound scheduler does not publish game state to clients.
#[derive(Debug)]
pub struct HostInputPump<Message> {
    idle_timeout: Option<Duration>,
    in_flight: BTreeMap<u64, PollKind>,
    listener: SecureLanListener,
    next_request: u64,
    ready: VecDeque<PollTask>,
    worker: HostWorker<Message>,
}

impl<Message: Send + 'static> HostInputPump<Message> {
    fn accept_one(&mut self) -> io::Result<bool> {
        match self.listener.try_accept() {
            Ok(Some(pending)) => {
                self.ready
                    .push_back(PollTask::authenticate(Box::new(pending)));
                Ok(true)
            },
            Ok(None) => Ok(false),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::PermissionDenied
                        | ErrorKind::Interrupted
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::ConnectionReset
                        | ErrorKind::TimedOut
                ) =>
            {
                Ok(true)
            },
            Err(error) => Err(error),
        }
    }

    /// Bind one explicitly selected private interface and start its worker.
    ///
    /// Local and approved remote identities reserve distinct roster capacity.
    /// The same session limits govern transport, enrolment and queue sizing.
    /// # Errors
    /// Rejects disabled LAN, empty or excess enrolments, invalid identities,
    /// unrepresentable limits, or listener and worker creation failures.
    pub fn bind<Codec>(
        config: HostInputConfig<'_>,
        tls: ServerTls,
        codec: Codec,
    ) -> io::Result<Self>
    where
        Codec: MessageSchema<Message = Message> + Send + 'static,
    {
        let HostInputConfig {
            address,
            session: limits,
            timeout,
            enrolments,
            idle_timeout,
            local_peers,
        } = config;
        let settings = LanConfig::new(limits, timeout)?;
        if idle_timeout.is_some_and(|limit| {
            limit.is_zero() || Instant::now().checked_add(limit).is_none()
        }) {
            return Err(ErrorKind::InvalidInput.into());
        }
        let players = NonZeroUsize::from(limits.max_players).get();
        if enrolments.is_empty()
            || enrolments.len().saturating_add(local_peers.len()) > players
        {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut session = HostSession::new(limits, enrolments)
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        for &peer in local_peers {
            session
                .join_local(peer)
                .map_err(|error| io::Error::other(format!("{error:?}")))?;
        }
        let capacity = NonZeroUsize::new(players.saturating_mul(2))
            .ok_or(ErrorKind::InvalidInput)?;
        let listener = SecureLanListener::bind(address, settings, tls)?;
        let worker = HostWorker::spawn(session, codec, capacity)?;
        Ok(Self {
            idle_timeout,
            in_flight: BTreeMap::new(),
            listener,
            next_request: 0,
            ready: VecDeque::new(),
            worker,
        })
    }

    fn complete(
        &mut self,
        completion: HostCompletion<Message>,
    ) -> io::Result<PumpProgress<Message>> {
        let kind = self
            .in_flight
            .remove(&completion.id)
            .ok_or(ErrorKind::InvalidData)?;
        match kind {
            PollKind::Authenticate => self.complete_auth(completion.result),
            PollKind::Disconnect(peer) => match completion.result {
                Ok(HostOutcome::Applied) => {
                    Ok(PumpProgress::Event(RemoteEvent::Disconnected {
                        peer,
                        reason: ErrorKind::TimedOut,
                    }))
                },
                _ => Err(ErrorKind::InvalidData.into()),
            },
            PollKind::Receive(peer, deadline) => {
                self.complete_receive(peer, deadline, completion.result)
            },
        }
    }

    fn complete_auth(
        &mut self,
        result: HostResult<Message>,
    ) -> io::Result<PumpProgress<Message>> {
        match result {
            Ok(HostOutcome::Admitted(peer)) => {
                self.ready
                    .push_back(PollTask::receive(peer, self.deadline()?));
                Ok(PumpProgress::Event(RemoteEvent::Connected(peer)))
            },
            Ok(HostOutcome::Authenticating(pending)) => {
                self.ready.push_back(PollTask::authenticate(pending));
                Ok(PumpProgress::Progress)
            },
            Err(
                OperationError::Authentication(_)
                | OperationError::Host(
                    HostError::Unenrolled
                    | HostError::Session(
                        SessionError::AlreadyConnected
                        | SessionError::NoCapacity,
                    ),
                ),
            ) => Ok(PumpProgress::Progress),
            _ => Err(ErrorKind::InvalidData.into()),
        }
    }

    fn complete_receive(
        &mut self,
        peer: u64,
        deadline: Option<Instant>,
        result: HostResult<Message>,
    ) -> io::Result<PumpProgress<Message>> {
        let progress = match result {
            Ok(HostOutcome::Message { peer: actual, value })
                if actual == peer =>
            {
                PumpProgress::Event(RemoteEvent::Message { peer, value })
            },
            Ok(HostOutcome::Pending)
            | Err(OperationError::Host(HostError::Session(
                SessionError::MalformedMessage,
            ))) => PumpProgress::Progress,
            Err(OperationError::Host(HostError::Transport(reason))) => {
                return Ok(PumpProgress::Event(RemoteEvent::Disconnected {
                    peer,
                    reason,
                }));
            },
            _ => return Err(ErrorKind::InvalidData.into()),
        };
        let next = if matches!(progress, PumpProgress::Event(_)) {
            self.deadline()?
        } else {
            deadline
        };
        self.ready.push_back(PollTask::receive(peer, next));
        Ok(progress)
    }

    fn deadline(&self) -> io::Result<Option<Instant>> {
        self.idle_timeout
            .map(|timeout| {
                Instant::now()
                    .checked_add(timeout)
                    .ok_or_else(|| ErrorKind::InvalidInput.into())
            })
            .transpose()
    }

    /// Return the bound address, including an assigned ephemeral port.
    ///
    /// # Errors
    /// Propagates socket inspection failures.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Advance at most one completion, or accept and submit one poll.
    ///
    /// Authentication failures and malformed application frames do not
    /// produce a gameplay event or stop unrelated peers. A transfer failure
    /// produces one explicit disconnect; verified rejoin is a new connection.
    /// # Errors
    /// Reports listener, worker or internal correlation failures.
    pub fn poll(&mut self) -> io::Result<PumpProgress<Message>> {
        if let Some(completion) = self.worker.poll()? {
            return self.complete(completion);
        }
        let accepted = self.accept_one()?;
        let submitted = self.submit_one()?;
        if accepted || submitted {
            Ok(PumpProgress::Progress)
        } else {
            Ok(PumpProgress::Idle)
        }
    }

    /// Close the listener, cancel queued work and join the I/O worker.
    ///
    /// # Errors
    /// Reports worker shutdown failure.
    pub fn shutdown(self) -> io::Result<()> {
        self.worker.shutdown()
    }

    fn submit_one(&mut self) -> io::Result<bool> {
        let Some(task) = self.ready.pop_front() else {
            return Ok(false);
        };
        let id = self.next_request;
        let next = id.checked_add(1).ok_or(ErrorKind::InvalidData)?;
        let PollTask { kind, operation } = match task.kind {
            PollKind::Receive(peer, Some(deadline))
                if Instant::now() >= deadline =>
            {
                PollTask::disconnect(peer)
            },
            PollKind::Authenticate
            | PollKind::Disconnect(_)
            | PollKind::Receive(..) => task,
        };
        let request = HostRequest { id, operation };
        match self.worker.try_submit(request) {
            Ok(()) => {
                let _prior = self.in_flight.insert(id, kind);
                self.next_request = next;
                Ok(true)
            },
            Err(rejected) if rejected.reason == RejectionReason::Full => {
                self.ready.push_front(PollTask {
                    kind,
                    operation: rejected.request.operation,
                });
                Ok(false)
            },
            Err(_rejected) => Err(ErrorKind::BrokenPipe.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::num::NonZeroU8;
    use std::thread;

    use super::super::super::tests::{
        LOOPBACK, TIMEOUT, TestCodec, config, poll_accept, require,
        session_config,
    };
    use super::super::tests::{
        authority, client_credentials, credentials, send_unframed,
    };
    use super::super::{ClientTls, SecureFrameStream, ServerName};
    use super::*;

    fn next_event(pump: &mut HostInputPump<u8>) -> io::Result<RemoteEvent<u8>> {
        poll_accept(|| match pump.poll()? {
            PumpProgress::Event(event) => Ok(Some(event)),
            PumpProgress::Idle | PumpProgress::Progress => Ok(None),
        })
    }

    fn connect(
        pump: &mut HostInputPump<u8>,
        client: &ClientTls,
        peer: u64,
    ) -> io::Result<SecureFrameStream> {
        let address = pump.local_addr()?;
        let settings = config()?;
        let name =
            ServerName::try_from("games.test").map_err(io::Error::other)?;
        thread::scope(|scope| {
            let remote = scope.spawn(|| {
                SecureFrameStream::connect(address, settings, client, name)
            });
            require(
                matches!(next_event(pump)?, RemoteEvent::Connected(actual)
                    if actual == peer),
                "verified admission delivered",
            )?;
            remote
                .join()
                .map_err(|_error| io::Error::other("client panicked"))?
        })
    }

    fn expect_message(
        pump: &mut HostInputPump<u8>,
        peer: u64,
        expected: u8,
    ) -> io::Result<()> {
        let event = next_event(pump)?;
        let valid = match event {
            RemoteEvent::Message { peer: actual, value } => {
                actual == peer && value == expected
            },
            RemoteEvent::Connected(_) | RemoteEvent::Disconnected { .. } => {
                false
            },
        };
        require(valid, "validated peer message")
    }

    fn partial_frame(
        pump: &mut HostInputPump<u8>,
        first: &mut SecureFrameStream,
        second: &mut SecureFrameStream,
    ) -> io::Result<()> {
        send_unframed(first, &[0, 0])?;
        require(
            !matches!(pump.poll()?, PumpProgress::Event(_)),
            "partial frame cannot become input",
        )?;
        second.send(b"M\x06")?;
        expect_message(pump, 2, 6)?;
        send_unframed(first, &[0, 2, b'M', 5])?;
        expect_message(pump, 1, 5)
    }

    fn malformed_frame(
        pump: &mut HostInputPump<u8>,
        first: &mut SecureFrameStream,
        second: &mut SecureFrameStream,
    ) -> io::Result<()> {
        first.send(b"bad")?;
        second.send(b"M\x07")?;
        expect_message(pump, 2, 7)?;
        first.send(b"M\x08")?;
        expect_message(pump, 1, 8)
    }

    fn disconnect_rejoin(
        pump: &mut HostInputPump<u8>,
        first: SecureFrameStream,
        first_credentials: &ClientTls,
        second: &mut SecureFrameStream,
    ) -> io::Result<()> {
        drop(first);
        require(
            matches!(next_event(pump)?, RemoteEvent::Disconnected {
                peer: 1,
                reason: ErrorKind::UnexpectedEof,
            }),
            "loss reserves and publishes the same peer identity",
        )?;
        second.send(b"M\x09")?;
        expect_message(pump, 2, 9)?;
        let mut rejoined = connect(pump, first_credentials, 1)?;
        rejoined.send(b"M\x0a")?;
        expect_message(pump, 1, 10)
    }

    #[test]
    fn scheduler_drives_independent_authenticated_peers() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let (second_keys, second_id) =
            client_credentials(&issuer, &issuer, keys.server_id)?;
        let mut session = session_config(true);
        session.max_players =
            NonZeroU8::new(3).ok_or(ErrorKind::InvalidInput)?;
        let mut pump = HostInputPump::bind(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &[(keys.client_id, 1), (second_id, 2)],
                idle_timeout: None,
                local_peers: &[0],
                session,
                timeout: TIMEOUT,
            },
            keys.server,
            TestCodec,
        )?;
        // A quiet unauthenticated socket must not monopolize the worker.
        let silent = TcpStream::connect(pump.local_addr()?)?;
        require(
            matches!(pump.poll()?, PumpProgress::Progress),
            "silent socket admitted only as pending",
        )?;
        let mut first = connect(&mut pump, &keys.client, 1)?;
        let mut second = connect(&mut pump, &second_keys, 2)?;
        partial_frame(&mut pump, &mut first, &mut second)?;
        malformed_frame(&mut pump, &mut first, &mut second)?;
        disconnect_rejoin(&mut pump, first, &keys.client, &mut second)?;
        drop(silent);
        pump.shutdown()
    }

    #[test]
    fn scheduler_requires_explicit_lan_and_available_roster_capacity()
    -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let approved = [(keys.client_id, 1)];
        let disabled = HostInputPump::bind(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &approved,
                idle_timeout: None,
                local_peers: &[0],
                session: session_config(false),
                timeout: TIMEOUT,
            },
            keys.server.clone(),
            TestCodec,
        );
        require(
            disabled.err().map(|error| error.kind())
                == Some(ErrorKind::PermissionDenied),
            "LAN cannot bind without opt-in",
        )?;
        let full = HostInputPump::bind(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &approved,
                idle_timeout: None,
                local_peers: &[0, 2],
                session: session_config(true),
                timeout: TIMEOUT,
            },
            keys.server.clone(),
            TestCodec,
        );
        require(
            full.err().map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
            "approved remote capacity cannot be consumed by local seats",
        )?;
        let anonymous = HostInputPump::bind(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &[],
                idle_timeout: None,
                local_peers: &[0],
                session: session_config(true),
                timeout: TIMEOUT,
            },
            keys.server,
            TestCodec,
        );
        require(
            anonymous.err().map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
            "at least one explicit remote approval is required",
        )
    }

    #[test]
    fn malformed_frames_cannot_renew_an_idle_lease() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let mut pump = HostInputPump::bind(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &[(keys.client_id, 1)],
                idle_timeout: Some(Duration::from_millis(500)),
                local_peers: &[0],
                session: session_config(true),
                timeout: TIMEOUT,
            },
            keys.server,
            TestCodec,
        )?;
        let mut client = connect(&mut pump, &keys.client, 1)?;
        let mut next_send = Instant::now();
        let event = poll_accept(|| {
            if let PumpProgress::Event(event) = pump.poll()? {
                return Ok(Some(event));
            }
            if Instant::now() < next_send {
                return Ok(None);
            }
            next_send = Instant::now()
                .checked_add(Duration::from_millis(25))
                .ok_or(ErrorKind::TimedOut)?;
            if client.send(b"bad").is_err() {
                // The worker can close the socket before its event
                // reaches this caller. Still require lease expiry.
                return next_event(&mut pump).map(Some);
            }
            Ok(None)
        })?;
        require(
            matches!(event, RemoteEvent::Disconnected {
                peer: 1,
                reason: ErrorKind::TimedOut,
            }),
            "malformed frames neither publish input nor renew the lease",
        )?;
        pump.shutdown()
    }
}
