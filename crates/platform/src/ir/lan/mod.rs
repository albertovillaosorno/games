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
use std::fmt::{self, Debug, Formatter};
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::time::Duration;

use games::network::{MessageSchema, SessionConfig};
#[cfg(feature = "secure-lan")]
use sha2::{Digest as _, Sha256};

use self::identity::IdentityPort;
use self::pump::PumpPort;
pub use super::security::lan::framing::SendProgress;
use super::security::lan::host::HostBindRequest;
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
pub struct ClientTls(pub(crate) CredentialPort);

/// Host credentials and explicitly supplied trusted client roots.
pub struct ServerTls(pub(crate) CredentialPort);

impl Clone for ClientTls {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    fn clone_from(&mut self, source: &Self) {
        self.0.clone_from(&source.0);
    }
}

impl Clone for ServerTls {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    fn clone_from(&mut self, source: &Self) {
        self.0.clone_from(&source.0);
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
    backend: FramePort,
}

impl Debug for SecureFrameStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureFrameStream")
            .field("fingerprint", &self.peer_fingerprint())
            .finish_non_exhaustive()
    }
}

impl SecureFrameStream {
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
        Ok(Self { backend })
    }

    /// Whether an outgoing frame is queued or partially transmitted.
    #[must_use]
    pub fn has_pending_send(&self) -> bool {
        self.backend.has_pending_send()
    }

    /// Get the authenticated peer's leaf certificate fingerprint.
    #[must_use]
    pub fn peer_fingerprint(&self) -> PeerFingerprint {
        self.backend.peer_fingerprint()
    }

    /// Poll for a complete frame without waiting for network I/O.
    ///
    /// # Errors
    /// Reports a disconnected, malformed or expired transport.
    pub fn poll_receive(&mut self) -> Result<Option<Vec<u8>>> {
        self.backend.poll_receive()
    }

    /// Advance one queued frame without waiting for network I/O.
    ///
    /// # Errors
    /// Reports a disconnected or expired transport.
    pub fn poll_send(&mut self) -> Result<SendProgress> {
        self.backend.poll_send()
    }

    /// Queue one complete payload without performing network I/O.
    ///
    /// # Errors
    /// Rejects oversized frames, a closed connection or a pending send.
    pub fn queue_send(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.backend.queue_send(bytes)
    }

    /// Receive one complete frame within the configured deadline.
    ///
    /// # Errors
    /// Reports malformed, disconnected or expired transport.
    pub fn receive(&mut self) -> Result<Vec<u8>> {
        self.backend.receive()
    }

    /// Send one complete frame within the configured deadline.
    ///
    /// # Errors
    /// Rejects oversized frames, pending sends or failed transport.
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.backend.send(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind, Result};
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::network::SessionConfig;

    use super::identity::{LocalIdentity, PublicIdentity};
    use super::{AVAILABLE, LanConfig, PeerFingerprint};

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
