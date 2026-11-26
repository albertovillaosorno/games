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
//   - Portable host input and lifecycle observations and reply requests.
// - Must-Not:
//   - Schedule game simulation, open sockets or expose worker internals.
// - Allows:
//   - Inputs: Explicit endpoint, roster, schema and finite polling requests.
//   - Outputs: Ordered authenticated lifecycle and schema-checked messages.
//   - Side effects: Delegated bounded host input and replies.
// - Split-When:
//   - A new scheduling requirement cannot use the shared host contract.
// - Merge-When:
//   - An existing authenticated host capability owns the same observations.
// - Summary:
//   - Caller-driven authenticated host input without native worker handles.
// - Description:
//   - Preserves event ownership, admission deadlines and reply backpressure.
// - Usage:
//   - Deliver each event before polling again and bound polls per update.
// - Defaults:
//   - No discovery, implicit enrolment or unlimited buffering.

//! Portable authenticated host input and reply scheduling.

use core::net::SocketAddr;
use std::fmt::{self, Debug, Formatter};
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

use games::network::{MessageSchema, SessionConfig, SessionError};

use super::{LanFactory, PeerFingerprint, ServerTls};
use crate::ir::security::lan::host_binding::{
    HostAdmissionError, HostBindRequest, HostObservationError, PumpAdmission,
};

/// Explicit endpoint, transport limits and pre-approved player identities.
#[derive(Clone, Copy, Debug)]
pub struct HostInputConfig<'roster> {
    /// Private interface and port chosen by the host.
    pub address: SocketAddr,
    /// Approved certificate fingerprints mapped to host player identities.
    pub enrolments: &'roster [(PeerFingerprint, u64)],
    /// Optional maximum time between completed valid application frames.
    /// Caller stalls and completions observed after expiry do not renew it.
    pub idle_timeout: Option<Duration>,
    /// Host-owned local player identities.
    pub local_peers: &'roster [u64],
    /// Opted-in roster and frame limits.
    pub session: SessionConfig,
    /// Deadline for each handshake or in-progress transfer.
    pub timeout: Duration,
}

impl From<HostAdmissionError> for Error {
    fn from(error: HostAdmissionError) -> Self {
        match error {
            HostAdmissionError::Lan(reason) => reason.into(),
            HostAdmissionError::Session(SessionError::LanDisabled)
            | HostAdmissionError::UnapprovedIdentity => {
                ErrorKind::PermissionDenied.into()
            },
            HostAdmissionError::DuplicateEnrollment
            | HostAdmissionError::EmptyEnrollment
            | HostAdmissionError::LocalIdentityReserved
            | HostAdmissionError::Session(_)
            | HostAdmissionError::TooManyEnrollments => {
                ErrorKind::InvalidInput.into()
            },
        }
    }
}

impl From<HostObservationError> for Error {
    fn from(error: HostObservationError) -> Self {
        match error {
            HostObservationError::Lan(reason) => reason.into(),
            HostObservationError::UnavailablePeer => {
                ErrorKind::WouldBlock.into()
            },
            HostObservationError::UnexpectedAddress
            | HostObservationError::UnexpectedEvent => {
                ErrorKind::InvalidData.into()
            },
        }
    }
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
        /// Host monotonic time when the worker completed valid decoding.
        completed_at: Duration,
        /// Host-approved peer identity, independent of the frame payload.
        peer: u64,
        /// Decoded application message.
        value: Message,
    },
}

/// Private backend contract; no listener, thread or native handle escapes.
pub(crate) trait PumpBackend<Message>: Send {
    fn local_addr(&self) -> Result<SocketAddr>;
    fn poll(&mut self) -> Result<PumpProgress<Message>>;
    fn queue_reply(&mut self, peer: u64, bytes: Vec<u8>) -> Result<()>;
    fn shutdown(self: Box<Self>) -> Result<()>;
}

pub(crate) type PumpPort<Message> = Box<dyn PumpBackend<Message>>;

/// Nonblocking host input and reply scheduler over authenticated transport.
///
/// Polling does not open outbound sockets, schedule gameplay or buffer
/// delivered events. Callers must cap their own polls per update.
pub struct HostInputPump<Message: Send + 'static> {
    admission: PumpAdmission,
    backend: PumpPort<Message>,
}

impl<Message: Send + 'static> Debug for HostInputPump<Message> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostInputPump").finish_non_exhaustive()
    }
}

impl<Message: Send + 'static> HostInputPump<Message> {
    fn admit_backend(
        mut admission: PumpAdmission,
        backend: PumpPort<Message>,
    ) -> Result<Self> {
        admission.admit_address(backend.local_addr()?)?;
        Ok(Self { admission, backend })
    }

    /// Bind an explicitly selected private interface and start its worker.
    /// Local and approved remote identities reserve distinct roster capacity.
    ///
    /// # Errors
    /// Rejects disabled LAN, unapproved, duplicate or excessive enrolments,
    /// invalid identities, limits, or listener and worker creation failures.
    pub fn bind<Codec>(
        config: HostInputConfig<'_>,
        tls: ServerTls,
        codec: Codec,
    ) -> Result<Self>
    where
        Codec: MessageSchema<Message = Message> + Send + 'static,
    {
        let request = HostBindRequest::new(config, &tls.approved)?;
        let admission = PumpAdmission::new(&request);
        let backend = <() as LanFactory>::bind(request, tls, codec)?;
        Self::admit_backend(admission, backend)
    }

    /// Return the bound address, including an assigned ephemeral port.
    ///
    /// # Errors
    /// Propagates transport inspection failures.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        let observed = self.backend.local_addr()?;
        Ok(self.admission.address(observed)?)
    }

    /// Advance at most one completion, or accept and submit one poll.
    /// Authentication failures and malformed application frames do not
    /// produce a gameplay event or stop unrelated peers.
    ///
    /// # Errors
    /// Reports transport, worker or internal correlation failures.
    pub fn poll(&mut self) -> Result<PumpProgress<Message>> {
        self.admission.close_reply_window();
        let progress = self.backend.poll()?;
        if let PumpProgress::Event(event) = &progress {
            self.admission.admit_event(event)?;
        }
        Ok(progress)
    }

    /// Schedule one bounded reply to the most recent connected peer or
    /// message event, before polling again.
    ///
    /// # Errors
    /// Rejects oversized replies, absent reply opportunities and backend
    /// failures. A rejected reply does not consume the opportunity.
    pub fn queue_reply(&mut self, peer: u64, bytes: Vec<u8>) -> Result<()> {
        self.admission.admit_reply(peer, bytes.len())?;
        self.backend.queue_reply(peer, bytes)?;
        self.admission.close_reply_window();
        Ok(())
    }

    /// Close the listener, cancel queued work and join the I/O worker.
    ///
    /// # Errors
    /// Reports worker shutdown failure.
    pub fn shutdown(self) -> Result<()> {
        self.backend.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};
    use std::collections::{BTreeSet, VecDeque};
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Codec;

    impl MessageSchema for Codec {
        type Message = ();

        fn decode(&self, _bytes: &[u8]) -> Option<Self::Message> {
            Some(())
        }
    }

    struct ObservedPump {
        address: SocketAddr,
        events: VecDeque<RemoteEvent<()>>,
        reject_next_reply: bool,
        replies: Arc<AtomicUsize>,
    }

    impl PumpBackend<()> for ObservedPump {
        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.address)
        }

        fn poll(&mut self) -> Result<PumpProgress<()>> {
            Ok(self
                .events
                .pop_front()
                .map_or(PumpProgress::Idle, PumpProgress::Event))
        }

        fn queue_reply(&mut self, _peer: u64, _bytes: Vec<u8>) -> Result<()> {
            let _previous = self.replies.fetch_add(1, Ordering::Relaxed);
            if self.reject_next_reply {
                self.reject_next_reply = false;
                return Err(ErrorKind::WouldBlock.into());
            }
            Ok(())
        }

        fn shutdown(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    fn admission(address: SocketAddr) -> Result<PumpAdmission> {
        let fingerprint = PeerFingerprint::from_bytes([1; 32]);
        let config = HostInputConfig {
            address,
            enrolments: &[(fingerprint, 1)],
            idle_timeout: None,
            local_peers: &[0],
            session: SessionConfig {
                allow_lan: true,
                max_players: NonZeroU32::MIN.saturating_add(1),
                max_message_bytes: NonZeroUsize::MIN,
            },
            timeout: Duration::from_secs(1),
        };
        let request =
            HostBindRequest::new(config, &BTreeSet::from([fingerprint]))?;
        Ok(PumpAdmission::new(&request))
    }

    fn loopback(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    fn observed_pump(
        address: SocketAddr,
        events: Vec<RemoteEvent<()>>,
        replies: Arc<AtomicUsize>,
    ) -> Result<HostInputPump<()>> {
        HostInputPump::admit_backend(
            admission(loopback(0))?,
            Box::new(ObservedPump {
                address,
                events: events.into(),
                reject_next_reply: false,
                replies,
            }),
        )
    }

    fn message(peer: u64, millis: u64) -> RemoteEvent<()> {
        RemoteEvent::Message {
            completed_at: Duration::from_millis(millis),
            peer,
            value: (),
        }
    }

    fn require(condition: bool) -> Result<()> {
        condition
            .then_some(())
            .ok_or_else(|| Error::other("host observation mismatch"))
    }

    #[test]
    fn bind_receipt_preserves_requested_interface_and_port() -> Result<()> {
        for address in [
            loopback(0),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 42),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 42),
        ] {
            require(
                observed_pump(address, vec![], Arc::new(AtomicUsize::new(0)))
                    .err()
                    .map(|error| error.kind())
                    == Some(ErrorKind::InvalidData),
            )?;
        }
        let pump =
            observed_pump(loopback(42), vec![], Arc::new(AtomicUsize::new(0)))?;
        require(pump.local_addr()? == loopback(42))?;
        let mut fixed = admission(loopback(42))?;
        require(fixed.admit_address(loopback(43)).is_err())?;
        require(fixed.admit_address(loopback(42)).is_ok())
    }

    #[test]
    fn remote_lifecycle_rejects_unenrolled_and_out_of_order_events()
    -> Result<()> {
        let mut pump = observed_pump(
            loopback(42),
            vec![
                RemoteEvent::Connected(0),
                message(1, 1),
                RemoteEvent::Connected(1),
                RemoteEvent::Connected(1),
                message(1, 3),
                message(1, 2),
                RemoteEvent::Disconnected {
                    peer: 1,
                    reason: ErrorKind::ConnectionReset,
                },
                message(1, 4),
                RemoteEvent::Connected(1),
                message(1, 3),
            ],
            Arc::new(AtomicUsize::new(0)),
        )?;
        for admitted in [
            false, false, true, false, true, false, true, false, true, true,
        ] {
            if admitted {
                require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
            } else {
                require(
                    pump.poll().err().map(|error| error.kind())
                        == Some(ErrorKind::InvalidData),
                )?;
            }
        }
        require(matches!(pump.poll()?, PumpProgress::Idle))
    }

    #[test]
    fn only_active_remote_replies_within_budget_dispatch() -> Result<()> {
        let replies = Arc::new(AtomicUsize::new(0));
        let mut pump = observed_pump(
            loopback(42),
            vec![RemoteEvent::Connected(1), RemoteEvent::Disconnected {
                peer: 1,
                reason: ErrorKind::ConnectionReset,
            }],
            Arc::clone(&replies),
        )?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        for (peer, bytes, kind) in [
            (0, vec![], ErrorKind::WouldBlock),
            (1, vec![0, 1], ErrorKind::InvalidInput),
        ] {
            require(
                pump.queue_reply(peer, bytes)
                    .err()
                    .map(|error| error.kind())
                    == Some(kind),
            )?;
        }
        require(replies.load(Ordering::Relaxed) == 0)?;
        pump.queue_reply(1, vec![0])?;
        require(replies.load(Ordering::Relaxed) == 1)?;
        require(
            pump.queue_reply(1, vec![0]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(replies.load(Ordering::Relaxed) == 1)?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(replies.load(Ordering::Relaxed) == 1)
    }

    #[test]
    fn reply_opportunities_expire_on_any_subsequent_poll() -> Result<()> {
        let replies = Arc::new(AtomicUsize::new(0));
        let mut pump = observed_pump(
            loopback(42),
            vec![RemoteEvent::Connected(1), message(0, 0), message(1, 1)],
            Arc::clone(&replies),
        )?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        require(
            pump.poll().err().map(|error| error.kind())
                == Some(ErrorKind::InvalidData),
        )?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        require(matches!(pump.poll()?, PumpProgress::Idle))?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(replies.load(Ordering::Relaxed) == 0)
    }

    #[test]
    fn rejected_backend_reply_can_retry_the_same_opportunity() -> Result<()> {
        let replies = Arc::new(AtomicUsize::new(0));
        let mut pump = HostInputPump::admit_backend(
            admission(loopback(0))?,
            Box::new(ObservedPump {
                address: loopback(42),
                events: VecDeque::from([
                    RemoteEvent::Connected(1),
                    message(1, 1),
                ]),
                reject_next_reply: true,
                replies: Arc::clone(&replies),
            }),
        )?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        pump.queue_reply(1, vec![])?;
        require(replies.load(Ordering::Relaxed) == 2)?;
        require(
            pump.queue_reply(1, vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::WouldBlock),
        )?;
        require(matches!(pump.poll()?, PumpProgress::Event(_)))?;
        pump.queue_reply(1, vec![])?;
        require(replies.load(Ordering::Relaxed) == 3)
    }

    #[test]
    fn invalid_host_requests_are_rejected_before_backend_dispatch() {
        let tls = ServerTls {
            approved: Arc::new(BTreeSet::from([PeerFingerprint::from_bytes(
                [1; 32],
            )])),
            backend: Arc::new(()),
        };
        let mut config = HostInputConfig {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            enrolments: &[(PeerFingerprint::from_bytes([1; 32]), 1)],
            idle_timeout: None,
            local_peers: &[0],
            session: SessionConfig::local(
                NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
                NonZeroUsize::MIN,
            ),
            timeout: Duration::from_secs(1),
        };
        assert_eq!(
            HostInputPump::bind(config, tls.clone(), Codec)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::PermissionDenied),
        );
        config.session.allow_lan = true;
        config.local_peers = &[1];
        assert_eq!(
            HostInputPump::bind(config, tls.clone(), Codec)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidInput),
        );
        config.local_peers = &[0];
        let unapproved = [(PeerFingerprint::from_bytes([2; 32]), 1)];
        config.enrolments = &unapproved;
        assert_eq!(
            HostInputPump::bind(config, tls, Codec)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::PermissionDenied),
        );
    }
}
