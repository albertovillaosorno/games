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
//   - Bounded fair scheduling of native host input and reply operations.
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

//! Fair caller-driven admission, input and replies on the host worker.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::time::Duration;

use games::network::{MessageSchema, SessionError};

use super::host::{HostError, HostSession};
use super::worker::{
    HostCompletion, HostOperation, HostOutcome, HostRequest, HostResult,
    HostWorker, OperationError, RejectionReason, capacity_for_players,
};
use super::{PendingConnection, SecureLanListener, SendProgress, ServerTls};
use crate::ir::lan::pump::{PumpBackend, PumpProgress, RemoteEvent};
use crate::ir::security::lan::host::HostBindRequest;
use crate::software::linux::performance::now;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PollKind {
    Authenticate,
    // The flag records whether gameplay was notified of admission.
    Disconnect(u64, bool),
    Receive(u64, Option<Duration>),
    Reply(u64, Option<Duration>),
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

    const fn disconnect(peer: u64, announced: bool) -> Self {
        Self {
            kind: PollKind::Disconnect(peer, announced),
            operation: HostOperation::Disconnect(peer),
        }
    }

    const fn receive(peer: u64, deadline: Option<Duration>) -> Self {
        Self {
            kind: PollKind::Receive(peer, deadline),
            operation: HostOperation::PollReceive(peer),
        }
    }

    const fn reply(peer: u64, deadline: Option<Duration>) -> Self {
        Self {
            kind: PollKind::Reply(peer, deadline),
            operation: HostOperation::PollSend(peer),
        }
    }
}

/// Nonblocking host input and reply scheduler over the authenticated worker.
///
/// Pending sockets are bounded by the listener permit count. Active
/// channels are bounded by the roster. Each socket or channel has exactly
/// one ready or in-flight poll task; completed tasks return to the tail.
/// The worker queues are sized for both populations together.
///
/// Polling does not open outbound sockets, schedule gameplay or buffer
/// delivered events. Callers must cap their own polls per update and stop
/// on backpressure rather than discarding a lifecycle or input event.
/// One bounded reply can replace a ready receive task; it cannot grow the
/// queue or renew an inbound lease. It does not publish world state.
#[derive(Debug)]
pub(super) struct HostInputPump<Message> {
    frame_budget: usize,
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
    pub(super) fn bind<Codec>(
        request: HostBindRequest,
        tls: ServerTls,
        codec: Codec,
    ) -> io::Result<Self>
    where
        Codec: MessageSchema<Message = Message> + Send + 'static,
    {
        let idle_timeout = request.idle_timeout();
        if idle_timeout.is_some_and(|limit| now().checked_add(limit).is_none())
        {
            return Err(ErrorKind::InvalidInput.into());
        }
        let address = request.address();
        let settings = request.transport();
        let roster = request.into_roster();
        let limits = roster.config();
        let capacity = capacity_for_players(limits.max_players)?;
        let session = HostSession::from_roster(roster);
        let listener = SecureLanListener::bind(address, settings, tls)?;
        let worker = HostWorker::spawn(session, codec, capacity)?;
        Ok(Self {
            frame_budget: limits.max_message_bytes.get(),
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
        observed_at: Duration,
    ) -> io::Result<PumpProgress<Message>> {
        let kind = self
            .in_flight
            .remove(&completion.id)
            .ok_or(ErrorKind::InvalidData)?;
        match kind {
            PollKind::Authenticate => self.complete_auth(
                completion.completed_at,
                observed_at,
                completion.result,
            ),
            PollKind::Disconnect(peer, announced) => match completion.result {
                Ok(HostOutcome::Applied) if announced => {
                    Ok(PumpProgress::Event(RemoteEvent::Disconnected {
                        peer,
                        reason: ErrorKind::TimedOut,
                    }))
                },
                Ok(HostOutcome::Applied) => Ok(PumpProgress::Progress),
                _ => Err(ErrorKind::InvalidData.into()),
            },
            PollKind::Receive(peer, deadline) => {
                self.complete_receive((peer, deadline), completion, observed_at)
            },
            PollKind::Reply(peer, deadline) => {
                self.complete_reply(peer, deadline, &completion.result)
            },
        }
    }

    fn complete_auth(
        &mut self,
        completed_at: Duration,
        observed_at: Duration,
        result: HostResult<Message>,
    ) -> io::Result<PumpProgress<Message>> {
        match result {
            Ok(HostOutcome::Admitted(peer)) => {
                let deadline = self.deadline(completed_at)?;
                if deadline.is_some_and(|limit| observed_at >= limit) {
                    self.ready.push_back(PollTask::disconnect(peer, false));
                    return Ok(PumpProgress::Progress);
                }
                self.ready.push_back(PollTask::receive(peer, deadline));
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
        lease: (u64, Option<Duration>),
        completion: HostCompletion<Message>,
        observed_at: Duration,
    ) -> io::Result<PumpProgress<Message>> {
        let (peer, deadline) = lease;
        let completed_at = completion.completed_at;
        let progress = match completion.result {
            Ok(HostOutcome::Message { peer: actual, value })
                if actual == peer =>
            {
                PumpProgress::Event(RemoteEvent::Message {
                    completed_at,
                    peer,
                    value,
                })
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
            let renewed = self.deadline(completed_at)?;
            // Worker completion, not delayed caller polling, renews a lease.
            // Do not deliver a control whose old or renewed lease expired.
            if deadline.is_some_and(|limit| completed_at >= limit)
                || renewed.is_some_and(|limit| observed_at >= limit)
            {
                self.ready.push_back(PollTask::disconnect(peer, true));
                return Ok(PumpProgress::Progress);
            }
            renewed
        } else {
            deadline
        };
        self.ready.push_back(PollTask::receive(peer, next));
        Ok(progress)
    }

    fn complete_reply(
        &mut self,
        peer: u64,
        deadline: Option<Duration>,
        result: &HostResult<Message>,
    ) -> io::Result<PumpProgress<Message>> {
        match result {
            Ok(
                HostOutcome::Applied
                | HostOutcome::SendState(SendProgress::Pending),
            ) => {
                self.ready.push_back(PollTask::reply(peer, deadline));
            },
            Ok(HostOutcome::SendState(SendProgress::Complete)) => {
                self.ready.push_back(PollTask::receive(peer, deadline));
            },
            Err(OperationError::Host(HostError::Transport(reason))) => {
                return Ok(PumpProgress::Event(RemoteEvent::Disconnected {
                    peer,
                    reason: *reason,
                }));
            },
            _ => return Err(ErrorKind::InvalidData.into()),
        }
        Ok(PumpProgress::Progress)
    }

    fn deadline(&self, since: Duration) -> io::Result<Option<Duration>> {
        self.idle_timeout
            .map(|timeout| {
                since
                    .checked_add(timeout)
                    .ok_or_else(|| ErrorKind::InvalidInput.into())
            })
            .transpose()
    }

    fn submit_one(&mut self) -> io::Result<bool> {
        let Some(task) = self.ready.pop_front() else {
            return Ok(false);
        };
        let id = self.next_request;
        let next = id.checked_add(1).ok_or(ErrorKind::InvalidData)?;
        let PollTask { kind, operation } = match task.kind {
            PollKind::Receive(peer, Some(deadline))
            | PollKind::Reply(peer, Some(deadline))
                if now() >= deadline =>
            {
                PollTask::disconnect(peer, true)
            },
            PollKind::Authenticate
            | PollKind::Disconnect(..)
            | PollKind::Receive(..)
            | PollKind::Reply(..) => task,
        };
        let lease = match kind {
            PollKind::Receive(peer, deadline)
            | PollKind::Reply(peer, deadline) => deadline.map(|at| (peer, at)),
            PollKind::Authenticate | PollKind::Disconnect(..) => None,
        };
        let request = HostRequest { id, operation };
        match self.worker.try_submit_leased(request, lease) {
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

impl<Message: Send + 'static> PumpBackend<Message> for HostInputPump<Message> {
    /// Return the bound address, including an assigned ephemeral port.
    ///
    /// # Errors
    /// Propagates socket inspection failures.
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Advance at most one completion, or accept and submit one poll.
    ///
    /// Authentication failures and malformed application frames do not
    /// produce a gameplay event or stop unrelated peers. A transfer failure
    /// produces one explicit disconnect; verified rejoin is a new connection.
    /// Leases use worker completion time, rejecting stale admissions and
    /// decoded input when a caller resumes polling after an extended stall.
    /// # Errors
    /// Reports listener, worker or internal correlation failures.
    fn poll(&mut self) -> io::Result<PumpProgress<Message>> {
        if let Some(completion) = self.worker.poll()? {
            return self.complete(completion, now());
        }
        let accepted = self.accept_one()?;
        let submitted = self.submit_one()?;
        if accepted || submitted {
            Ok(PumpProgress::Progress)
        } else {
            Ok(PumpProgress::Idle)
        }
    }

    /// Schedule a bounded reply before the next receive for this peer.
    ///
    /// Call before polling again after receiving the peer event. The same
    /// round-robin slot handles send progress and resumes receive on success.
    ///
    /// # Errors
    /// Rejects oversized replies and unavailable or already busy peers.
    fn queue_reply(&mut self, peer: u64, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.len() > self.frame_budget {
            return Err(ErrorKind::InvalidInput.into());
        }
        for task in &mut self.ready {
            if let PollKind::Receive(actual, deadline) = task.kind
                && actual == peer
            {
                *task = PollTask {
                    kind: PollKind::Reply(peer, deadline),
                    operation: HostOperation::QueueSend { peer, bytes },
                };
                return Ok(());
            }
        }
        Err(ErrorKind::WouldBlock.into())
    }

    /// Close the listener, cancel queued work and join the I/O worker.
    ///
    /// # Errors
    /// Reports worker shutdown failure.
    fn shutdown(self: Box<Self>) -> io::Result<()> {
        self.worker.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::num::NonZeroU32;
    use std::thread;

    use super::super::super::tests::{
        LOOPBACK, TIMEOUT, TestCodec, config, poll_accept, require,
        session_config,
    };
    use super::super::tests::{
        authority, client_credentials, credentials, send_unframed,
    };
    use super::super::{ClientTls, SecureFrameStream};
    use super::*;
    use crate::ir::lan::FrameBackend as _;
    use crate::ir::lan::pump::HostInputConfig;

    struct TestPeers {
        first: ClientTls,
        pump: HostInputPump<u8>,
        second: ClientTls,
    }

    struct UnobservedAdmission {
        completion: HostCompletion<u8>,
        stream: SecureFrameStream,
    }

    fn bind_test(
        config: HostInputConfig<'_>,
        tls: ServerTls,
        codec: TestCodec,
    ) -> io::Result<HostInputPump<u8>> {
        let approved = config
            .enrolments
            .iter()
            .map(|&(fingerprint, _peer)| fingerprint)
            .collect();
        HostInputPump::bind(
            HostBindRequest::new(config, &approved)?,
            tls,
            codec,
        )
    }

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
        let name = "games.test";
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
            RemoteEvent::Message { peer: actual, value, .. } => {
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

    fn two_peers() -> io::Result<TestPeers> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let (second_keys, second_id) =
            client_credentials(&issuer, &issuer, keys.server_id)?;
        let mut session = session_config(true);
        session.max_players =
            NonZeroU32::new(3).ok_or(ErrorKind::InvalidInput)?;
        let pump = bind_test(
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
        Ok(TestPeers {
            first: keys.client,
            pump,
            second: second_keys,
        })
    }

    #[test]
    fn scheduler_drives_independent_authenticated_peers() -> io::Result<()> {
        let TestPeers {
            mut pump,
            first: first_keys,
            second: second_keys,
        } = two_peers()?;
        // A quiet unauthenticated socket must not monopolize the worker.
        let silent = TcpStream::connect(pump.local_addr()?)?;
        require(
            matches!(pump.poll()?, PumpProgress::Progress),
            "silent socket admitted only as pending",
        )?;
        let mut first = connect(&mut pump, &first_keys, 1)?;
        let mut second = connect(&mut pump, &second_keys, 2)?;
        partial_frame(&mut pump, &mut first, &mut second)?;
        malformed_frame(&mut pump, &mut first, &mut second)?;
        disconnect_rejoin(&mut pump, first, &first_keys, &mut second)?;
        drop(silent);
        Box::new(pump).shutdown()
    }

    #[test]
    fn bounded_reply_preserves_peer_order_and_capacity() -> io::Result<()> {
        let TestPeers {
            mut pump,
            first: first_keys,
            second: second_keys,
        } = two_peers()?;
        let mut first = connect(&mut pump, &first_keys, 1)?;
        let mut second = connect(&mut pump, &second_keys, 2)?;
        first.send(b"M\x05")?;
        expect_message(&mut pump, 1, 5)?;
        require(
            pump.queue_reply(1, vec![0; pump.frame_budget.saturating_add(1)])
                .err()
                .map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
            "oversized reply rejected without consuming the receive slot",
        )?;
        pump.queue_reply(1, b"ACK".to_vec())?;
        require(
            pump.queue_reply(1, b"ACK".to_vec())
                .err()
                .map(|error| error.kind())
                == Some(ErrorKind::WouldBlock)
                && pump
                    .queue_reply(0, b"ACK".to_vec())
                    .err()
                    .map(|error| error.kind())
                    == Some(ErrorKind::WouldBlock),
            "one reply per active remote, not a local or busy seat",
        )?;
        second.send(b"M\x06")?;
        expect_message(&mut pump, 2, 6)?;
        let reply = poll_accept(|| {
            require(
                !matches!(pump.poll()?, PumpProgress::Event(_)),
                "reply is not application input",
            )?;
            first.poll_receive()
        })?;
        require(
            reply == b"ACK" && second.poll_receive()?.is_none(),
            "reply reaches only the intended peer",
        )?;
        first.send(b"M\x07")?;
        expect_message(&mut pump, 1, 7)?;
        Box::new(pump).shutdown()
    }

    #[test]
    fn scheduler_requires_explicit_lan_and_available_roster_capacity()
    -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let approved = [(keys.client_id, 1)];
        let disabled = bind_test(
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
        let full = bind_test(
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
        let anonymous = bind_test(
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
    fn scheduler_admits_a_wider_budget_and_rejects_excess_before_bind()
    -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let approved = [(keys.client_id, 1)];
        let mut limits = session_config(true);
        limits.max_players = NonZeroU32::MIN.saturating_add(255);
        let config = HostInputConfig {
            address: LOOPBACK,
            enrolments: &approved,
            idle_timeout: None,
            local_peers: &[0],
            session: limits,
            timeout: TIMEOUT,
        };
        let pump = bind_test(config, keys.server.clone(), TestCodec)?;
        let mut excess = config;
        excess.address = pump.local_addr()?;
        excess.session.max_players = NonZeroU32::MIN.saturating_add(512);
        let rejected = bind_test(excess, keys.server, TestCodec);
        require(
            rejected.err().map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
            "queue capacity is rejected before rebinding the occupied port",
        )?;
        Box::new(pump).shutdown()
    }

    #[test]
    fn malformed_frames_cannot_renew_an_idle_lease() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let mut pump = bind_test(
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
        let mut next_send = now();
        let event = poll_accept(|| {
            if let PumpProgress::Event(event) = pump.poll()? {
                return Ok(Some(event));
            }
            if now() < next_send {
                return Ok(None);
            }
            next_send = now()
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
        Box::new(pump).shutdown()
    }

    fn poll_admission_completion(
        pump: &mut HostInputPump<u8>,
    ) -> io::Result<Option<HostCompletion<u8>>> {
        let Some(completion) = pump.worker.poll()? else {
            let _accepted = pump.accept_one()?;
            let _submitted = pump.submit_one()?;
            return Ok(None);
        };
        if matches!(completion.result, Ok(HostOutcome::Admitted(1))) {
            return Ok(Some(completion));
        }
        require(
            !matches!(
                pump.complete(completion, now())?,
                PumpProgress::Event(_)
            ),
            "no peer announced before the target admission",
        )?;
        Ok(None)
    }

    fn admission_completion(
        pump: &mut HostInputPump<u8>,
        client: &ClientTls,
    ) -> io::Result<UnobservedAdmission> {
        let address = pump.local_addr()?;
        let settings = config()?;
        let name = "games.test";
        thread::scope(|scope| {
            let remote = scope.spawn(|| {
                SecureFrameStream::connect(address, settings, client, name)
            });
            let completion = poll_accept(|| poll_admission_completion(pump))?;
            let stream = remote
                .join()
                .map_err(|_error| io::Error::other("client panicked"))??;
            Ok(UnobservedAdmission { completion, stream })
        })
    }

    #[test]
    fn expired_admission_does_not_publish_a_phantom_peer() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let idle = Duration::from_secs(30);
        let mut pump = bind_test(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &[(keys.client_id, 1)],
                idle_timeout: Some(idle),
                local_peers: &[0],
                session: session_config(true),
                timeout: TIMEOUT,
            },
            keys.server,
            TestCodec,
        )?;
        let UnobservedAdmission {
            stream: mut stale,
            completion,
        } = admission_completion(&mut pump, &keys.client)?;
        stale.send(b"M\x05")?;
        let observed_at = completion
            .completed_at
            .checked_add(idle)
            .ok_or(ErrorKind::InvalidInput)?;
        require(
            matches!(
                pump.complete(completion, observed_at)?,
                PumpProgress::Progress
            ),
            "admission observed at lease expiry must stay unpublished",
        )?;
        require(pump.submit_one()?, "schedule expired admission cleanup")?;
        let disconnected = poll_accept(|| pump.worker.poll())?;
        require(
            matches!(
                pump.complete(disconnected, observed_at)?,
                PumpProgress::Progress
            ),
            "cleanup cannot publish disconnect for an unpublished peer",
        )?;
        drop(stale);

        // The authenticated roster must reserve, not strand, this seat.
        let mut rejoined = connect(&mut pump, &keys.client, 1)?;
        rejoined.send(b"M\x06")?;
        expect_message(&mut pump, 1, 6)?;
        Box::new(pump).shutdown()
    }

    fn message_completion(
        pump: &mut HostInputPump<u8>,
    ) -> io::Result<HostCompletion<u8>> {
        poll_accept(|| {
            let Some(completion) = pump.worker.poll()? else {
                let _submitted = pump.submit_one()?;
                return Ok(None);
            };
            if matches!(
                &completion.result,
                Ok(HostOutcome::Message { peer: 1, value: 5 })
            ) {
                return Ok(Some(completion));
            }
            require(
                !matches!(
                    pump.complete(completion, now())?,
                    PumpProgress::Event(_)
                ),
                "only the intended message may complete",
            )?;
            Ok(None)
        })
    }

    fn pending_deadline(
        pump: &HostInputPump<u8>,
        completion: &HostCompletion<u8>,
    ) -> io::Result<Duration> {
        match pump.in_flight.get(&completion.id) {
            Some(PollKind::Receive(1, Some(deadline))) => Ok(*deadline),
            _ => Err(ErrorKind::InvalidData.into()),
        }
    }

    fn expect_expired(
        pump: &mut HostInputPump<u8>,
        completion: HostCompletion<u8>,
        observed_at: Duration,
    ) -> io::Result<()> {
        require(
            matches!(
                pump.complete(completion, observed_at)?,
                PumpProgress::Progress
            ),
            "expired completion cannot deliver application input",
        )?;
        require(
            matches!(next_event(pump)?, RemoteEvent::Disconnected {
                peer: 1,
                reason: ErrorKind::TimedOut,
            }),
            "stale completion causes a real authenticated disconnect",
        )
    }

    #[test]
    fn completed_messages_do_not_revive_expired_leases() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let idle = Duration::from_secs(30);
        let mut pump = bind_test(
            HostInputConfig {
                address: LOOPBACK,
                enrolments: &[(keys.client_id, 1)],
                idle_timeout: Some(idle),
                local_peers: &[0],
                session: session_config(true),
                timeout: TIMEOUT,
            },
            keys.server,
            TestCodec,
        )?;
        let mut client = connect(&mut pump, &keys.client, 1)?;

        // Use real authenticated completions, with controlled observation
        // times rather than waiting through a long host stall.
        client.send(b"M\x05")?;
        let completion = message_completion(&mut pump)?;
        let deadline = pending_deadline(&pump, &completion)?;
        let decoded_at = completion.completed_at;
        require(
            matches!(
                pump.complete(completion, deadline)?,
                PumpProgress::Event(RemoteEvent::Message {
                    peer: 1, value: 5, completed_at,
                }) if completed_at == decoded_at
            ),
            "timely decode may be observed after the previous deadline",
        )?;

        client.send(b"M\x05")?;
        let queued = message_completion(&mut pump)?;
        let observed_at = queued
            .completed_at
            .checked_add(idle)
            .ok_or(ErrorKind::InvalidInput)?;
        expect_expired(&mut pump, queued, observed_at)?;
        drop(client);

        let mut rejoined = connect(&mut pump, &keys.client, 1)?;
        rejoined.send(b"M\x05")?;
        let mut late = message_completion(&mut pump)?;
        let rejoined_deadline = pending_deadline(&pump, &late)?;
        // Model a worker operation that completed exactly at expiry.
        late.completed_at = rejoined_deadline;
        expect_expired(&mut pump, late, rejoined_deadline)?;
        Box::new(pump).shutdown()
    }
}
