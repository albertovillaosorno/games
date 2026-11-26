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
use crate::ir::security::lan::host::{HostAdmissionError, HostBindRequest};

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
///
/// Polling does not open outbound sockets, schedule gameplay or buffer
/// delivered events. Callers must cap their own polls per update.
pub struct HostInputPump<Message: Send + 'static> {
    backend: PumpPort<Message>,
}

impl<Message: Send + 'static> Debug for HostInputPump<Message> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostInputPump").finish_non_exhaustive()
    }
}

impl<Message: Send + 'static> HostInputPump<Message> {
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
        let backend = <() as LanFactory>::bind(request, tls, codec)?;
        Ok(Self { backend })
    }

    /// Return the bound address, including an assigned ephemeral port.
    ///
    /// # Errors
    /// Propagates transport inspection failures.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.backend.local_addr()
    }

    /// Advance at most one completion, or accept and submit one poll.
    /// Authentication failures and malformed application frames do not
    /// produce a gameplay event or stop unrelated peers.
    ///
    /// # Errors
    /// Reports transport, worker or internal correlation failures.
    pub fn poll(&mut self) -> Result<PumpProgress<Message>> {
        self.backend.poll()
    }

    /// Schedule a bounded reply before the next receive for this peer.
    ///
    /// # Errors
    /// Rejects oversized replies and unavailable or already busy peers.
    pub fn queue_reply(&mut self, peer: u64, bytes: Vec<u8>) -> Result<()> {
        self.backend.queue_reply(peer, bytes)
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
    use std::collections::BTreeSet;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::Arc;

    use super::*;

    struct Codec;

    impl MessageSchema for Codec {
        type Message = ();

        fn decode(&self, _bytes: &[u8]) -> Option<Self::Message> {
            Some(())
        }
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
