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
//   - Portable authenticated LAN configuration and framed connection handles.
// - Must-Not:
//   - Open sockets, select transport implementations or interpret game input.
// - Allows:
//   - Inputs: Explicit private endpoints, credentials and session budgets.
//   - Outputs: Verified peer identities and bounded frame observations.
//   - Side effects: Delegated authenticated transport operations.
// - Split-When:
//   - Identity admission and host scheduling have independent lifetimes.
// - Merge-When:
//   - An existing transport contract owns the same bounded semantics.
// - Summary:
//   - Backend-independent capability for authenticated private sessions.
// - Description:
//   - Keeps transport handles and implementation-specific types private.
// - Usage:
//   - Request the capability after explicit user opt-in and enrolment.
// - Defaults:
//   - No network access, public endpoints or unauthenticated fallback.

//! Portable authenticated private LAN transport.

/// Local credentials and explicitly approved public trust material.
pub mod identity;
/// Bounded host-side remote input and reply scheduling.
pub mod pump;

use core::any::Any;
use core::net::SocketAddr;
use std::collections::BTreeSet;
use std::fmt::{self, Debug, Formatter};
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::time::Duration;

use games::network::{MessageSchema, SessionConfig};
#[cfg(feature = "secure-lan")]
use sha2::{Digest as _, Sha256};

use self::identity::IdentityPort;
use self::pump::PumpPort;
use super::security::lan::framing::SendAdmission;
pub use super::security::lan::framing::SendProgress;
use super::security::lan::host_binding::HostBindRequest;
use super::security::lan::{LanAdmissionError, LanLimits, admit_endpoint};

/// Whether the selected platform provides authenticated private transport.
pub const AVAILABLE: bool = <() as LanAvailability>::SUPPORTED;

/// Explicit transport limits inherited from an opted-in session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanConfig {
    limits: LanLimits,
}

impl LanConfig {
    pub(crate) const fn from_limits(limits: LanLimits) -> Self {
        Self { limits }
    }

    /// Admitted neutral wire limits.
    #[must_use]
    pub const fn limits(self) -> LanLimits {
        self.limits
    }

    /// Admit a transport configuration without accessing a clock or socket.
    ///
    /// # Errors
    /// Rejects disabled LAN, an unrepresentable frame budget or zero timeout.
    /// The backend verifies native deadline representability before I/O.
    pub fn new(session: SessionConfig, timeout: Duration) -> Result<Self> {
        Ok(Self::from_limits(LanLimits::new(session, timeout)?))
    }
}

impl From<LanAdmissionError> for Error {
    fn from(error: LanAdmissionError) -> Self {
        match error {
            LanAdmissionError::Disabled | LanAdmissionError::PublicEndpoint => {
                ErrorKind::PermissionDenied
            },
            LanAdmissionError::FrameBudget
            | LanAdmissionError::OutboundLength
            | LanAdmissionError::ZeroTimeout => ErrorKind::InvalidInput,
            LanAdmissionError::InboundLength => ErrorKind::InvalidData,
        }
        .into()
    }
}

/// SHA-256 identity of a verified leaf certificate.
///
/// A host must explicitly associate the fingerprint with a player slot;
/// certification alone is not an authorisation to join a specific session.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PeerFingerprint([u8; 32]);

impl PeerFingerprint {
    /// Restore an enrolled fingerprint from its canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derive the fingerprint of a certificate supplied for local enrolment.
    #[cfg(feature = "secure-lan")]
    #[must_use]
    pub fn from_der(certificate: &[u8]) -> Self {
        Self(Sha256::digest(certificate).into())
    }

    /// Return the canonical fingerprint bytes for host-side enrolment.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Private backend contract for one already authenticated framed stream.
pub(crate) trait FrameBackend: Send {
    fn has_pending_send(&self) -> bool;
    fn peer_fingerprint(&self) -> PeerFingerprint;
    fn poll_receive(&mut self) -> Result<Option<Vec<u8>>>;
    fn poll_send(&mut self) -> Result<SendProgress>;
    fn queue_send(&mut self, bytes: Vec<u8>) -> Result<()>;
    fn receive(&mut self) -> Result<Vec<u8>>;
    fn send(&mut self, bytes: &[u8]) -> Result<()>;
}

pub(crate) type FramePort = Box<dyn FrameBackend>;

/// Private platform selection and transport factory.
pub(crate) trait LanAvailability {
    const SUPPORTED: bool;
}

/// Private transport factory; concrete handles never leave the adapter.
pub(crate) trait LanFactory {
    fn bind<Message, Codec>(
        config: HostBindRequest,
        tls: ServerTls,
        codec: Codec,
    ) -> Result<PumpPort<Message>>
    where
        Message: Send + 'static,
        Codec: MessageSchema<Message = Message> + Send + 'static;
    fn client_identity(
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<IdentityPort>;
    fn connect(
        address: SocketAddr,
        config: LanConfig,
        tls: &ClientTls,
        name: &str,
    ) -> Result<FramePort>;
    fn host_identity(
        name: &str,
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<IdentityPort>;
    fn validate_root(root_der: &[u8]) -> Result<()>;
}

/// Shared opaque native configuration; no credential bytes are exposed.
pub(crate) type CredentialPort = Arc<dyn Any + Send + Sync>;

/// Client credentials and an explicitly approved host, privately configured.
pub struct ClientTls {
    approved: PeerFingerprint,
    pub(crate) backend: CredentialPort,
}

/// Host credentials and immutable approved client leaf fingerprints.
pub struct ServerTls {
    approved: Arc<BTreeSet<PeerFingerprint>>,
    pub(crate) backend: CredentialPort,
}

impl Clone for ClientTls {
    fn clone(&self) -> Self {
        Self {
            approved: self.approved,
            backend: Arc::clone(&self.backend),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.approved = source.approved;
        self.backend.clone_from(&source.backend);
    }
}

impl Clone for ServerTls {
    fn clone(&self) -> Self {
        Self {
            approved: Arc::clone(&self.approved),
            backend: Arc::clone(&self.backend),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.approved.clone_from(&source.approved);
        self.backend.clone_from(&source.backend);
    }
}

impl Debug for ClientTls {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientTls").finish_non_exhaustive()
    }
}

impl Debug for ServerTls {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerTls").finish_non_exhaustive()
    }
}

/// An encrypted framed connection with a verified peer certificate.
/// Blocking transfers belong on I/O workers, not the simulation thread.
pub struct SecureFrameStream {
    backend: Option<FramePort>,
    limits: LanLimits,
    peer: PeerFingerprint,
    send_admission: SendAdmission,
}

impl Debug for SecureFrameStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureFrameStream")
            .field("fingerprint", &self.peer_fingerprint())
            .finish_non_exhaustive()
    }
}

impl SecureFrameStream {
    fn admit_backend(
        backend: FramePort,
        config: LanConfig,
        approved: PeerFingerprint,
    ) -> Result<Self> {
        if backend.peer_fingerprint() != approved {
            return Err(ErrorKind::PermissionDenied.into());
        }
        let send_admission = SendAdmission::default();
        send_admission.check(backend.has_pending_send())?;
        Ok(Self {
            backend: Some(backend),
            limits: config.limits(),
            peer: approved,
            send_admission,
        })
    }

    fn admit_received(&mut self, bytes: Vec<u8>) -> Result<Vec<u8>> {
        let admitted = u32::try_from(bytes.len())
            .map_err(|_error| LanAdmissionError::InboundLength)
            .and_then(|length| self.limits.inbound_length(length));
        if let Err(error) = admitted {
            self.withdraw();
            return Err(error.into());
        }
        Ok(bytes)
    }

    fn admit_send_observation(
        &mut self,
        admit: impl FnOnce(&mut SendAdmission, bool) -> Result<()>,
    ) -> Result<()> {
        let observed = self
            .backend
            .as_ref()
            .ok_or(ErrorKind::NotConnected)?
            .has_pending_send();
        let result = admit(&mut self.send_admission, observed);
        if result.is_err() {
            self.withdraw();
        }
        result
    }

    fn backend_mut(&mut self) -> Result<&mut FramePort> {
        self.backend
            .as_mut()
            .ok_or_else(|| ErrorKind::NotConnected.into())
    }

    /// Connect to the explicitly approved host with one handshake deadline.
    ///
    /// # Errors
    /// Rejects invalid names, public endpoints, untrusted hosts, unavailable
    /// transport and I/O failures.
    pub fn connect(
        address: SocketAddr,
        config: LanConfig,
        tls: &ClientTls,
        name: &str,
    ) -> Result<Self> {
        admit_endpoint(address)?;
        let backend = <() as LanFactory>::connect(address, config, tls, name)?;
        Self::admit_backend(backend, config, tls.approved)
    }

    fn finish_transfer<Value>(
        &mut self,
        result: Result<Value>,
    ) -> Result<Value> {
        match result {
            Ok(value) => {
                self.verify_pending()?;
                Ok(value)
            },
            Err(error) => {
                self.withdraw();
                Err(error)
            },
        }
    }

    /// Whether an outgoing frame is queued or partially transmitted.
    #[must_use]
    pub const fn has_pending_send(&self) -> bool {
        self.send_admission.pending()
    }

    /// Get the authenticated peer's leaf certificate fingerprint.
    #[must_use]
    pub const fn peer_fingerprint(&self) -> PeerFingerprint {
        self.peer
    }

    /// Poll for a complete frame without waiting for network I/O.
    ///
    /// # Errors
    /// Reports a disconnected, malformed or expired transport.
    pub fn poll_receive(&mut self) -> Result<Option<Vec<u8>>> {
        self.verify_pending()?;
        let received = self.backend_mut()?.poll_receive();
        self.finish_transfer(received)?
            .map(|bytes| self.admit_received(bytes))
            .transpose()
    }

    /// Advance one queued frame without waiting for network I/O.
    ///
    /// # Errors
    /// Reports a disconnected or expired transport.
    pub fn poll_send(&mut self) -> Result<SendProgress> {
        self.verify_pending()?;
        let progress = match self.backend_mut()?.poll_send() {
            Ok(progress) => progress,
            Err(error) => {
                self.withdraw();
                return Err(error);
            },
        };
        self.admit_send_observation(|admission, observed| {
            admission.progressed(progress, observed)
        })?;
        Ok(progress)
    }

    /// Queue one complete payload without performing network I/O.
    ///
    /// # Errors
    /// Rejects oversized frames, a closed connection or a pending send.
    pub fn queue_send(&mut self, bytes: Vec<u8>) -> Result<()> {
        let _length = self.limits.outbound_length(bytes.len())?;
        self.verify_pending()?;
        self.send_admission.require_idle()?;
        match self.backend_mut()?.queue_send(bytes) {
            Ok(()) => self.admit_send_observation(SendAdmission::queued),
            Err(error) => {
                self.verify_pending()?;
                if error.kind() == ErrorKind::NotConnected {
                    self.withdraw();
                }
                Err(error)
            },
        }
    }

    /// Receive one complete frame within the configured deadline.
    ///
    /// # Errors
    /// Reports malformed, disconnected or expired transport.
    pub fn receive(&mut self) -> Result<Vec<u8>> {
        self.verify_pending()?;
        let received = self.backend_mut()?.receive();
        let bytes = self.finish_transfer(received)?;
        self.admit_received(bytes)
    }

    /// Send one complete frame within the configured deadline.
    ///
    /// # Errors
    /// Rejects oversized frames, pending sends or failed transport.
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        let _length = self.limits.outbound_length(bytes.len())?;
        self.verify_pending()?;
        self.send_admission.require_idle()?;
        let result = self.backend_mut()?.send(bytes);
        self.finish_transfer(result)
    }

    fn verify_pending(&mut self) -> Result<()> {
        self.admit_send_observation(|admission, observed| {
            admission.check(observed)
        })
    }

    fn withdraw(&mut self) {
        self.backend = None;
        self.send_admission = SendAdmission::default();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{Error, ErrorKind, Result};
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use games::network::SessionConfig;

    use super::identity::{LocalIdentity, PublicIdentity};
    use super::{
        AVAILABLE, FrameBackend, LanConfig, PeerFingerprint, SecureFrameStream,
        SendProgress,
    };

    struct ObservedFrame {
        dispatched: Arc<AtomicBool>,
        peer: PeerFingerprint,
        pending: bool,
        progress: VecDeque<(SendProgress, bool)>,
        queue_pending: bool,
        received: Vec<u8>,
    }

    impl ObservedFrame {
        fn admit(self, approved: PeerFingerprint) -> Result<SecureFrameStream> {
            let config = LanConfig::new(session(true), Duration::from_secs(1))?;
            SecureFrameStream::admit_backend(Box::new(self), config, approved)
        }

        fn new(
            peer: PeerFingerprint,
            received: Vec<u8>,
            dispatched: Arc<AtomicBool>,
        ) -> Self {
            Self {
                dispatched,
                peer,
                pending: false,
                progress: VecDeque::new(),
                queue_pending: true,
                received,
            }
        }
    }

    impl FrameBackend for ObservedFrame {
        fn has_pending_send(&self) -> bool {
            self.pending
        }

        fn peer_fingerprint(&self) -> PeerFingerprint {
            self.peer
        }

        fn poll_receive(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(Some(self.received.clone()))
        }

        fn poll_send(&mut self) -> Result<SendProgress> {
            let (progress, pending) =
                self.progress.pop_front().unwrap_or(if self.pending {
                    (SendProgress::Complete, false)
                } else {
                    (SendProgress::Idle, false)
                });
            self.pending = pending;
            Ok(progress)
        }

        fn queue_send(&mut self, _bytes: Vec<u8>) -> Result<()> {
            self.dispatched.store(true, Ordering::Release);
            self.pending = self.queue_pending;
            Ok(())
        }

        fn receive(&mut self) -> Result<Vec<u8>> {
            Ok(self.received.clone())
        }

        fn send(&mut self, _bytes: &[u8]) -> Result<()> {
            self.dispatched.store(true, Ordering::Release);
            Ok(())
        }
    }

    fn observed_stream(
        peer: PeerFingerprint,
        approved: PeerFingerprint,
        received: Vec<u8>,
        dispatched: Arc<AtomicBool>,
    ) -> Result<SecureFrameStream> {
        ObservedFrame::new(peer, received, dispatched).admit(approved)
    }

    #[test]
    fn mismatched_host_observation_never_exposes_a_stream() {
        let observed = observed_stream(
            PeerFingerprint::from_bytes([1; 32]),
            PeerFingerprint::from_bytes([2; 32]),
            vec![],
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(
            observed.err().map(|error| error.kind()),
            Some(ErrorKind::PermissionDenied),
        );
    }

    #[test]
    fn client_frame_budget_is_admitted_before_dispatch() -> Result<()> {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        let dispatched = Arc::new(AtomicBool::new(false));
        let mut stream =
            observed_stream(peer, peer, vec![], Arc::clone(&dispatched))?;
        require(stream.peer_fingerprint() == peer)?;
        require(
            stream.send(&[0; 65]).err().map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
        )?;
        require(
            stream
                .queue_send(vec![0; 65])
                .err()
                .map(|error| error.kind())
                == Some(ErrorKind::InvalidInput),
        )?;
        require(!dispatched.load(Ordering::Acquire))?;
        stream.send(&[0; 64])?;
        require(dispatched.load(Ordering::Acquire))?;
        dispatched.store(false, Ordering::Release);
        stream.queue_send(vec![])?;
        require(dispatched.load(Ordering::Acquire))?;
        Ok(())
    }

    #[test]
    fn queued_frames_reserve_capacity_until_admitted_completion() -> Result<()>
    {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        let dispatched = Arc::new(AtomicBool::new(false));
        let mut backend =
            ObservedFrame::new(peer, vec![], Arc::clone(&dispatched));
        backend.progress.push_back((SendProgress::Pending, true));
        let mut stream = backend.admit(peer)?;
        require(!stream.has_pending_send())?;
        stream.queue_send(vec![1])?;
        require(stream.has_pending_send())?;
        dispatched.store(false, Ordering::Release);
        require(stream.poll_receive()? == Some(vec![]))?;
        require(stream.poll_send()? == SendProgress::Pending)?;
        require(stream.has_pending_send())?;
        require(
            stream.queue_send(vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::AlreadyExists),
        )?;
        require(
            stream.send(&[]).err().map(|error| error.kind())
                == Some(ErrorKind::AlreadyExists),
        )?;
        require(!dispatched.load(Ordering::Acquire))?;
        require(stream.poll_send()? == SendProgress::Complete)?;
        require(!stream.has_pending_send())?;
        require(stream.poll_send()? == SendProgress::Idle)?;
        stream.send(&[])?;
        require(dispatched.load(Ordering::Acquire))
    }

    #[test]
    fn inconsistent_queue_receipts_withdraw_the_stream() -> Result<()> {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        let dispatched = Arc::new(AtomicBool::new(false));
        let mut initially_busy =
            ObservedFrame::new(peer, vec![], Arc::clone(&dispatched));
        initially_busy.pending = true;
        require(
            initially_busy.admit(peer).err().map(|error| error.kind())
                == Some(ErrorKind::InvalidData),
        )?;
        let mut backend =
            ObservedFrame::new(peer, vec![], Arc::clone(&dispatched));
        backend.queue_pending = false;
        let mut stream = backend.admit(peer)?;
        require(
            stream.queue_send(vec![]).err().map(|error| error.kind())
                == Some(ErrorKind::InvalidData),
        )?;
        dispatched.store(false, Ordering::Release);
        require(!stream.has_pending_send())?;
        require(
            stream.send(&[]).err().map(|error| error.kind())
                == Some(ErrorKind::NotConnected),
        )?;
        require(!dispatched.load(Ordering::Acquire))
    }

    #[test]
    fn inconsistent_send_progress_withdraws_the_stream() -> Result<()> {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        for (queued, progress, pending) in [
            (false, SendProgress::Pending, true),
            (false, SendProgress::Complete, false),
            (true, SendProgress::Idle, false),
            (true, SendProgress::Complete, true),
            (true, SendProgress::Pending, false),
        ] {
            let dispatched = Arc::new(AtomicBool::new(false));
            let mut backend = ObservedFrame::new(peer, vec![], dispatched);
            backend.progress.push_back((progress, pending));
            let mut stream = backend.admit(peer)?;
            if queued {
                stream.queue_send(vec![])?;
            }
            require(
                stream.poll_send().err().map(|error| error.kind())
                    == Some(ErrorKind::InvalidData),
            )?;
            require(!stream.has_pending_send())?;
            require(
                stream.poll_receive().err().map(|error| error.kind())
                    == Some(ErrorKind::NotConnected),
            )?;
        }
        Ok(())
    }

    #[test]
    fn oversized_inbound_observation_withdraws_the_stream() -> Result<()> {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        for poll in [false, true] {
            let dispatched = Arc::new(AtomicBool::new(false));
            let mut stream = observed_stream(
                peer,
                peer,
                vec![0; 65],
                Arc::clone(&dispatched),
            )?;
            let failure = if poll {
                stream.poll_receive().err()
            } else {
                stream.receive().err()
            };
            require(
                failure.map(|error| error.kind())
                    == Some(ErrorKind::InvalidData),
            )?;
            require(
                stream.send(&[]).err().map(|error| error.kind())
                    == Some(ErrorKind::NotConnected),
            )?;
            require(!dispatched.load(Ordering::Acquire))?;
        }
        Ok(())
    }

    #[test]
    fn inbound_observations_preserve_empty_and_exact_frames() -> Result<()> {
        let peer = PeerFingerprint::from_bytes([1; 32]);
        for received in [vec![], vec![5; 64]] {
            let mut stream = observed_stream(
                peer,
                peer,
                received.clone(),
                Arc::new(AtomicBool::new(false)),
            )?;
            require(stream.receive()? == received)?;
            require(stream.poll_receive()? == Some(received))?;
        }
        Ok(())
    }

    fn require(condition: bool) -> Result<()> {
        condition
            .then_some(())
            .ok_or_else(|| Error::other("stream observation mismatch"))
    }

    fn session(allow_lan: bool) -> SessionConfig {
        SessionConfig {
            allow_lan,
            max_message_bytes: NonZeroUsize::MIN.saturating_add(63),
            max_players: NonZeroU32::MIN.saturating_add(3),
        }
    }

    #[test]
    fn transport_admission_is_host_independent() -> Result<()> {
        let timeout = Duration::from_secs(2);
        let accepted = LanConfig::new(session(true), timeout)?;
        let limits = accepted.limits();
        if limits.timeout() != timeout
            || limits.max_bytes() != 64
            || limits.max_pending() != 4
        {
            return Err(Error::other("transport budgets changed"));
        }
        if LanConfig::new(session(false), timeout)
            .err()
            .map(|error| error.kind())
            != Some(ErrorKind::PermissionDenied)
            || LanConfig::new(session(true), Duration::ZERO)
                .err()
                .map(|error| error.kind())
                != Some(ErrorKind::InvalidInput)
        {
            return Err(Error::other("invalid transport admitted"));
        }
        let bytes = [17; 32];
        if PeerFingerprint::from_bytes(bytes).to_bytes() != bytes {
            return Err(Error::other("fingerprint bytes changed"));
        }
        Ok(())
    }

    #[test]
    fn unavailable_transport_does_not_admit_credentials() -> Result<()> {
        let fingerprint = PeerFingerprint::from_bytes([17; 32]);
        if PublicIdentity::from_der(&[], fingerprint)
            .err()
            .map(|error| error.kind())
            != Some(ErrorKind::InvalidInput)
            || LocalIdentity::client(&[1], &[1], &[])
                .err()
                .map(|error| error.kind())
                != Some(ErrorKind::InvalidInput)
        {
            return Err(Error::other("empty credentials admitted"));
        }
        if !AVAILABLE
            && (PublicIdentity::from_der(&[1], fingerprint)
                .err()
                .map(|error| error.kind())
                != Some(ErrorKind::Unsupported)
                || LocalIdentity::client(&[1], &[1], &[1])
                    .err()
                    .map(|error| error.kind())
                    != Some(ErrorKind::Unsupported)
                || LocalIdentity::host("games.test", &[1], &[1], &[1])
                    .err()
                    .map(|error| error.kind())
                    != Some(ErrorKind::Unsupported))
        {
            return Err(Error::other("missing backend not explicit"));
        }
        Ok(())
    }
}
