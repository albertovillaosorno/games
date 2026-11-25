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
//   - Host-approved TLS identity binding and authenticated session channels.
// - Must-Not:
//   - Generate credentials or implement game-specific message schemas.
// - Allows:
//   - Inputs: Local roster settings, approved fingerprints and TLS streams.
//   - Outputs: Validated game messages and connection lifecycle failures.
//   - Side effects: Bounded frame I/O and in-memory channel ownership.
// - Split-When:
//   - A platform worker scheduler needs a separate concurrency boundary.
// - Merge-When:
//   - Shared host composition provides identical verified admission.
// - Summary:
//   - Bind verified LAN certificates to stable local player identities.
// - Description:
//   - Composes TLS framing with the shared transport-neutral roster.
// - Usage:
//   - On an I/O worker, admit authenticated streams and decode their frames.
// - Defaults:
//   - No implicit enrolment, anonymous peers or central accounts.

use std::collections::BTreeMap;
use std::io;

use games::network::{
    MessageSchema, PeerKind, Session, SessionConfig, SessionError,
};
use rustls::Connection;

use super::{PeerFingerprint, SecureFrameStream};

/// Host admission, session policy or transport failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostError {
    /// A fingerprint or player identity is already enrolled.
    DuplicateEnrollment,
    /// The stream is closed, outbound or uses a different frame budget.
    InvalidTransport,
    /// A local controller cannot claim a LAN-enrolled identity.
    LocalIdentityReserved,
    /// The shared roster or message policy rejected the operation.
    Session(SessionError),
    /// Enrolments exceed the configured maximum player count.
    TooManyEnrollments,
    /// A bounded socket transfer failed and the LAN slot was reserved.
    Transport(io::ErrorKind),
    /// The verified certificate has no host-approved player identity.
    Unenrolled,
}

impl From<SessionError> for HostError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

/// A host-owned roster with one authenticated channel per active LAN player.
///
/// Admission uses only the verified certificate, never a client-supplied ID.
/// This synchronous adapter belongs on an I/O worker, not the simulation loop.
#[derive(Debug)]
pub struct HostSession {
    channels: BTreeMap<u64, SecureFrameStream>,
    enrolled: BTreeMap<PeerFingerprint, u64>,
    frame_budget: usize,
    roster: Session,
}

impl HostSession {
    /// Admit an inbound authenticated stream or reclaim its reserved slot.
    ///
    /// # Errors
    /// Rejects unenrolled or incompatible streams and roster policy failures.
    pub fn admit(
        &mut self,
        stream: SecureFrameStream,
    ) -> Result<u64, HostError> {
        let inbound = stream.channel.stream.as_ref().is_some_and(|tls| {
            matches!(&tls.connection, Connection::Server(_))
        });
        if !inbound
            || u32::try_from(self.frame_budget)
                != Ok(stream.channel.config.max_bytes)
        {
            return Err(HostError::InvalidTransport);
        }
        let peer = *self
            .enrolled
            .get(&stream.peer_fingerprint())
            .ok_or(HostError::Unenrolled)?;
        match self.roster.reconnect(peer, PeerKind::Lan) {
            Err(SessionError::UnknownPeer) => {
                self.roster.join(peer, PeerKind::Lan)?;
            },
            result => result?,
        }
        let _previous = self.channels.insert(peer, stream);
        Ok(peer)
    }

    /// Disconnect a controller or LAN player and reserve its existing slot.
    ///
    /// # Errors
    /// Rejects identities without an active roster entry.
    pub fn disconnect(&mut self, peer: u64) -> Result<(), HostError> {
        self.roster.disconnect(peer)?;
        drop(self.channels.remove(&peer));
        Ok(())
    }

    fn failed_transfer(&mut self, peer: u64, error: &io::Error) -> HostError {
        match self.disconnect(peer) {
            Ok(()) => HostError::Transport(error.kind()),
            Err(invariant) => invariant,
        }
    }

    /// Admit a directly connected local controller.
    ///
    /// # Errors
    /// Rejects LAN-enrolled identities, duplicates and full sessions.
    pub fn join_local(&mut self, peer: u64) -> Result<(), HostError> {
        if self.enrolled.values().any(|identity| *identity == peer) {
            return Err(HostError::LocalIdentityReserved);
        }
        self.roster.join(peer, PeerKind::Local)?;
        Ok(())
    }

    /// Construct a host from an explicit, bounded local enrolment list.
    ///
    /// # Errors
    /// Rejects enrolments when LAN is disabled, duplicates and excess entries.
    pub fn new(
        config: SessionConfig,
        enrolments: &[(PeerFingerprint, u64)],
    ) -> Result<Self, HostError> {
        if !config.allow_lan && !enrolments.is_empty() {
            return Err(SessionError::LanDisabled.into());
        }
        if enrolments.len() > usize::from(config.max_players.get()) {
            return Err(HostError::TooManyEnrollments);
        }
        let mut enrolled = BTreeMap::new();
        for &(fingerprint, peer) in enrolments {
            if enrolled.contains_key(&fingerprint)
                || enrolled.values().any(|identity| *identity == peer)
            {
                return Err(HostError::DuplicateEnrollment);
            }
            let _previous = enrolled.insert(fingerprint, peer);
        }
        Ok(Self {
            channels: BTreeMap::new(),
            enrolled,
            frame_budget: config.max_message_bytes.get(),
            roster: Session::new(config),
        })
    }

    /// Receive and schema-check one frame for an authenticated LAN identity.
    ///
    /// # Errors
    /// Rejects inactive channels and invalid messages. A transport failure
    /// closes the channel and reserves the player's slot for verified rejoin.
    pub fn receive<Codec>(
        &mut self,
        peer: u64,
        codec: &Codec,
    ) -> Result<Codec::Message, HostError>
    where
        Codec: MessageSchema,
    {
        let stream = self
            .channels
            .get_mut(&peer)
            .ok_or(SessionError::UnknownPeer)?;
        let bytes = stream
            .receive()
            .map_err(|error| self.failed_transfer(peer, &error))?;
        Ok(self.roster.decode(peer, &bytes, codec)?)
    }

    /// Rejoin a reserved local controller slot.
    ///
    /// # Errors
    /// Rejects unknown or differently originated reserved identities.
    pub fn reconnect_local(&mut self, peer: u64) -> Result<(), HostError> {
        self.roster.reconnect(peer, PeerKind::Local)?;
        Ok(())
    }

    /// Release an active or reserved slot without changing its enrolment.
    ///
    /// # Errors
    /// Rejects identities absent from the roster.
    pub fn remove(&mut self, peer: u64) -> Result<(), HostError> {
        self.roster.remove(peer)?;
        drop(self.channels.remove(&peer));
        Ok(())
    }

    /// Revoke a previously approved certificate and release its player slot.
    ///
    /// The certificate may still pass TLS chain validation, but this host
    /// will no longer admit it. Revocation also drops any active channel
    /// and releases a disconnected slot reserved for that identity.
    ///
    /// # Errors
    /// Rejects unknown fingerprints and unexpected roster failures.
    pub fn revoke_enrollment(
        &mut self,
        fingerprint: &PeerFingerprint,
    ) -> Result<(), HostError> {
        let peer = *self
            .enrolled
            .get(fingerprint)
            .ok_or(HostError::Unenrolled)?;
        match self.remove(peer) {
            Ok(()) | Err(HostError::Session(SessionError::UnknownPeer)) => {},
            Err(error) => return Err(error),
        }
        let _revoked = self.enrolled.remove(fingerprint);
        Ok(())
    }

    /// Send a bounded application frame to an active LAN player.
    ///
    /// # Errors
    /// Rejects missing channels and oversized messages. A transport failure
    /// closes the channel and reserves the identity.
    pub fn send(&mut self, peer: u64, bytes: &[u8]) -> Result<(), HostError> {
        self.roster.accept(peer, bytes)?;
        self.channels
            .get_mut(&peer)
            .ok_or(SessionError::UnknownPeer)?
            .send(bytes)
            .map_err(|error| self.failed_transfer(peer, &error))
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::num::NonZeroUsize;

    use games::network::SessionError;

    use super::super::super::tests::{
        LOOPBACK, TestCodec, config, require, session_config,
    };
    use super::super::tests::{
        authority, client_credentials, credentials, pair,
    };
    use super::super::{PeerFingerprint, SecureLanListener};
    use super::{HostError, HostSession};

    #[test]
    fn enrolment_is_explicit_unique_and_bounded() {
        let first = PeerFingerprint::from_bytes([1; 32]);
        let second = PeerFingerprint::from_bytes([2; 32]);
        assert!(matches!(
            HostSession::new(session_config(false), &[(first, 1)]),
            Err(HostError::Session(SessionError::LanDisabled))
        ));
        assert!(matches!(
            HostSession::new(session_config(true), &[(first, 1), (first, 2)]),
            Err(HostError::DuplicateEnrollment)
        ));
        assert!(matches!(
            HostSession::new(session_config(true), &[(first, 1), (second, 1)]),
            Err(HostError::DuplicateEnrollment)
        ));
        assert!(matches!(
            HostSession::new(session_config(true), &[
                (first, 1),
                (second, 2),
                (first, 3)
            ],),
            Err(HostError::TooManyEnrollments)
        ));
    }

    fn host_ok<Value>(result: Result<Value, HostError>) -> io::Result<Value> {
        result.map_err(|_error| io::Error::other("host rejected operation"))
    }

    #[test]
    fn only_enrolled_inbound_peers_can_occupy_slots() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let (outsider, _identity) =
            client_credentials(&issuer, &issuer, keys.server_id)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let mut host = host_ok(HostSession::new(session_config(true), &[(
            keys.client_id,
            2,
        )]))?;
        host_ok(host.join_local(1))?;
        require(
            host.join_local(2) == Err(HostError::LocalIdentityReserved),
            "local cannot claim enrolled identity",
        )?;
        let (unapproved, unapproved_client) =
            pair(&listener, settings, &outsider, "games.test")?;
        require(
            host.admit(unapproved?) == Err(HostError::Unenrolled),
            "trusted CA alone is not enrolment",
        )?;
        drop(unapproved_client?);

        let (incoming, outgoing) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(host_ok(host.admit(incoming?))? == 2, "approved peer")?;
        let _remote = outgoing?;
        let (duplicate, extra) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(
            host.admit(duplicate?)
                == Err(HostError::Session(SessionError::AlreadyConnected)),
            "duplicate connection cannot replace active channel",
        )?;
        require(
            host.admit(extra?) == Err(HostError::InvalidTransport),
            "outbound stream is not a host admission",
        )
    }

    #[test]
    fn host_decodes_frames_and_reserves_slot() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let mut host = host_ok(HostSession::new(session_config(true), &[(
            keys.client_id,
            2,
        )]))?;
        host_ok(host.join_local(1))?;
        let (incoming, outgoing) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(host_ok(host.admit(incoming?))? == 2, "approved peer")?;
        let mut remote = outgoing?;
        remote.send(b"bad")?;
        require(
            host.receive(2, &TestCodec)
                == Err(HostError::Session(SessionError::MalformedMessage)),
            "malformed frame cannot reach simulation",
        )?;
        remote.send(b"M\x04")?;
        require(host_ok(host.receive(2, &TestCodec))? == 4, "valid frame")?;
        host_ok(host.send(2, b"reply"))?;
        require(remote.receive()? == b"reply", "host reply")?;
        require(
            host.send(2, b"123456789")
                == Err(HostError::Session(SessionError::MessageTooLarge)),
            "oversized outgoing frame",
        )?;
        drop(remote);
        require(
            matches!(host.receive(2, &TestCodec), Err(HostError::Transport(_))),
            "transport failure disconnects peer",
        )?;
        require(
            host.join_local(3)
                == Err(HostError::Session(SessionError::NoCapacity)),
            "disconnected slot is reserved",
        )?;
        let (replacement, replacement_client) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(host_ok(host.admit(replacement?))? == 2, "same identity")?;
        let mut rejoined = replacement_client?;
        rejoined.send(b"M\x07")?;
        require(host_ok(host.receive(2, &TestCodec))? == 7, "rejoined frame")?;
        host_ok(host.remove(2))?;
        host_ok(host.join_local(3))?;
        host_ok(host.disconnect(1))?;
        host_ok(host.reconnect_local(1))
    }

    #[test]
    fn revocation_closes_channel_and_rejects_rejoin() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let mut host = host_ok(HostSession::new(session_config(true), &[(
            keys.client_id,
            2,
        )]))?;
        let (incoming, outgoing) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(host_ok(host.admit(incoming?))? == 2, "approved peer")?;
        let _remote = outgoing?;
        host_ok(host.revoke_enrollment(&keys.client_id))?;
        require(
            host.send(2, b"revoked")
                == Err(HostError::Session(SessionError::UnknownPeer)),
            "revoked channel is absent",
        )?;
        host_ok(host.join_local(2))?;
        let (rejoin, client) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(
            host.admit(rejoin?) == Err(HostError::Unenrolled),
            "revoked peer cannot rejoin",
        )?;
        drop(client?);
        require(
            host.revoke_enrollment(&keys.client_id)
                == Err(HostError::Unenrolled),
            "already revoked identity is unknown",
        )
    }

    #[test]
    fn revocation_releases_reserved_and_unused_identities() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let unused = PeerFingerprint::from_bytes([7; 32]);
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let mut host = host_ok(HostSession::new(session_config(true), &[
            (keys.client_id, 2),
            (unused, 1),
        ]))?;
        let (incoming, outgoing) =
            pair(&listener, settings, &keys.client, "games.test")?;
        require(host_ok(host.admit(incoming?))? == 2, "approved peer")?;
        drop(outgoing?);
        host_ok(host.disconnect(2))?;
        host_ok(host.revoke_enrollment(&keys.client_id))?;
        host_ok(host.revoke_enrollment(&unused))?;
        host_ok(host.join_local(2))?;
        host_ok(host.join_local(1))
    }

    #[test]
    fn offline_host_uses_only_local_roster() -> io::Result<()> {
        let mut host = host_ok(HostSession::new(session_config(false), &[]))?;
        host_ok(host.join_local(1))?;
        host_ok(host.join_local(2))?;
        require(
            host.receive(1, &TestCodec)
                == Err(HostError::Session(SessionError::UnknownPeer)),
            "local controllers do not have network channels",
        )?;
        host_ok(host.disconnect(1))?;
        host_ok(host.reconnect_local(1))?;
        host_ok(host.remove(2))?;
        host_ok(host.join_local(3))
    }

    #[test]
    fn mismatched_frame_budgets_are_rejected() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let mut limits = session_config(true);
        limits.max_message_bytes = NonZeroUsize::MIN;
        let mut host =
            host_ok(HostSession::new(limits, &[(keys.client_id, 2)]))?;
        let (incoming, outgoing) =
            pair(&listener, settings, &keys.client, "games.test")?;
        let _remote = outgoing?;
        require(
            host.admit(incoming?) == Err(HostError::InvalidTransport),
            "transport allocation limit must match host policy",
        )
    }
}
