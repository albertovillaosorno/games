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
//   - Transport-neutral local and self-hosted session access policies.
// - Must-Not:
//   - Open network sockets, authenticate identities or require cloud login.
// - Allows:
//   - Inputs: Verified peer IDs, connection types and bounded data frames.
//   - Outputs: Session membership, reconnect state and validation failures.
//   - Side effects: Tracks volatile session membership in memory.
// - Split-When:
//   - A platform transport or secure identity provider needs an adapter.
// - Merge-When:
//   - Another shared host/join policy covers the same invariants.
// - Summary:
//   - Explicit LAN opt-in and bounded self-hosted sessions.
// - Description:
//   - Maintains trusted peer membership independently of network I/O.
// - Usage:
//   - Use only after a transport adapter verifies a peer identity.
// - Defaults:
//   - Local controllers only and no external service dependencies.
//
//! In-memory session contracts for offline and self-hosted multiplayer.

use std::collections::BTreeMap;
use std::num::{NonZeroU8, NonZeroUsize};

/// The trusted transport origin of a member of a self-hosted session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerKind {
    /// A LAN connection whose identity was verified by a transport adapter.
    Lan,
    /// A controller directly connected to the host device.
    Local,
}

/// Observable failures when the host rejects a session operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// A supplied identity already has an active connection.
    AlreadyConnected,
    /// The host does not accept LAN connections without explicit opt-in.
    LanDisabled,
    /// The content failed a caller-supplied message schema.
    MalformedMessage,
    /// The peer message exceeds the configured byte budget.
    MessageTooLarge,
    /// The session has no available player slots.
    NoCapacity,
    /// The identity is not currently connected or reserved for rejoin.
    UnknownPeer,
    /// Rejoining with a different transport origin is not permitted.
    WrongOrigin,
}

/// Host settings with finite budgets and local-only default access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    /// Permit LAN clients only when explicitly enabled by the host.
    pub allow_lan: bool,
    /// Maximum frame payload in bytes.
    pub max_message_bytes: NonZeroUsize,
    /// Maximum local and LAN players combined.
    pub max_players: NonZeroU8,
}

impl SessionConfig {
    /// Configure a local-only session without a network or account service.
    #[must_use]
    pub const fn local(
        max_players: NonZeroU8,
        max_message_bytes: NonZeroUsize,
    ) -> Self {
        Self {
            allow_lan: false,
            max_message_bytes,
            max_players,
        }
    }
}

/// A game-specific, non-executing decoder for bounded peer input.
pub trait MessageSchema {
    /// The validated value passed on to the game simulation.
    type Message;

    /// Reject malformed frames without executing client-supplied code.
    fn decode(&self, bytes: &[u8]) -> Option<Self::Message>;
}

/// Session roster; peer authentication must be owned by the host transport.
#[derive(Debug)]
pub struct Session {
    active: BTreeMap<u64, PeerKind>,
    config: SessionConfig,
    reserved: BTreeMap<u64, PeerKind>,
}

impl Session {
    /// Validate an untrusted frame against an active, verified identity.
    ///
    /// # Errors
    ///
    /// Rejects unknown peers and frames exceeding the configured budget.
    pub fn accept(
        &self,
        peer: u64,
        message: &[u8],
    ) -> Result<(), SessionError> {
        if !self.active.contains_key(&peer) {
            return Err(SessionError::UnknownPeer);
        }
        if message.len() > self.config.max_message_bytes.get() {
            return Err(SessionError::MessageTooLarge);
        }
        Ok(())
    }

    /// Apply a title-specific schema after checking identity and size.
    ///
    /// # Errors
    ///
    /// Rejects unknown, oversized and malformed messages.
    pub fn decode<Codec>(
        &self,
        peer: u64,
        bytes: &[u8],
        codec: &Codec,
    ) -> Result<Codec::Message, SessionError>
    where
        Codec: MessageSchema,
    {
        self.accept(peer, bytes)?;
        codec.decode(bytes).ok_or(SessionError::MalformedMessage)
    }

    /// Mark an authenticated member as disconnected but reserve their slot.
    ///
    /// # Errors
    ///
    /// Fails if no active peer has the trusted identity.
    pub fn disconnect(&mut self, peer: u64) -> Result<(), SessionError> {
        let kind =
            self.active.remove(&peer).ok_or(SessionError::UnknownPeer)?;
        let _old = self.reserved.insert(peer, kind);
        Ok(())
    }

    /// Join with an identity and origin verified by the transport adapter.
    ///
    /// # Errors
    ///
    /// Rejects duplicate IDs, disabled LAN and exhausted player slots.
    pub fn join(
        &mut self,
        peer: u64,
        kind: PeerKind,
    ) -> Result<(), SessionError> {
        self.origin_allowed(kind)?;
        if self.active.contains_key(&peer) || self.reserved.contains_key(&peer)
        {
            return Err(SessionError::AlreadyConnected);
        }
        let used = self.active.len().saturating_add(self.reserved.len());
        if used >= usize::from(self.config.max_players.get()) {
            return Err(SessionError::NoCapacity);
        }
        let _old = self.active.insert(peer, kind);
        Ok(())
    }

    /// Create a session without connecting to any external account service.
    #[must_use]
    pub const fn new(config: SessionConfig) -> Self {
        Self {
            active: BTreeMap::new(),
            config,
            reserved: BTreeMap::new(),
        }
    }

    fn origin_allowed(&self, kind: PeerKind) -> Result<(), SessionError> {
        if kind == PeerKind::Lan && !self.config.allow_lan {
            return Err(SessionError::LanDisabled);
        }
        Ok(())
    }

    /// Rejoin after a verified transport reconnect on the same origin.
    ///
    /// # Errors
    ///
    /// Rejects unknown peers, changed origins and LAN without host opt-in.
    pub fn reconnect(
        &mut self,
        peer: u64,
        kind: PeerKind,
    ) -> Result<(), SessionError> {
        self.origin_allowed(kind)?;
        let former =
            self.reserved.get(&peer).ok_or(SessionError::UnknownPeer)?;
        if *former != kind {
            return Err(SessionError::WrongOrigin);
        }
        let _reservation = self.reserved.remove(&peer);
        let _previous = self.active.insert(peer, kind);
        Ok(())
    }

    /// Permanently release a disconnected player's reserved slot.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no active or reserved identity.
    pub fn remove(&mut self, peer: u64) -> Result<(), SessionError> {
        let existing = self
            .active
            .remove(&peer)
            .or_else(|| self.reserved.remove(&peer));
        existing.ok_or(SessionError::UnknownPeer).map(|_value| ())
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroUsize};

    use super::{
        MessageSchema, PeerKind, Session, SessionConfig, SessionError,
    };

    #[derive(Debug)]
    struct SmallActionCodec;

    impl MessageSchema for SmallActionCodec {
        type Message = u8;

        fn decode(&self, bytes: &[u8]) -> Option<Self::Message> {
            match bytes {
                [b'M', value] if *value <= 10 => Some(*value),
                _ => None,
            }
        }
    }
    fn config(lan: bool) -> SessionConfig {
        SessionConfig {
            allow_lan: lan,
            max_message_bytes: NonZeroUsize::new(8)
                .unwrap_or(NonZeroUsize::MIN),
            max_players: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
        }
    }

    #[test]
    fn local_controllers_do_not_require_accounts_or_lan() {
        let mut session = Session::new(config(false));
        assert_eq!(session.join(1, PeerKind::Local), Ok(()));
        assert_eq!(session.join(2, PeerKind::Local), Ok(()));
        assert_eq!(session.accept(1, b"move"), Ok(()));
        assert_eq!(
            session.join(3, PeerKind::Local),
            Err(SessionError::NoCapacity)
        );
        assert_eq!(
            session.join(1, PeerKind::Local),
            Err(SessionError::AlreadyConnected)
        );
    }

    #[test]
    fn lan_is_explicitly_opted_in_and_payloads_are_bounded() {
        let mut local = Session::new(config(false));
        assert_eq!(
            local.join(10, PeerKind::Lan),
            Err(SessionError::LanDisabled)
        );
        let mut lan = Session::new(config(true));
        assert_eq!(lan.join(10, PeerKind::Lan), Ok(()));
        assert_eq!(lan.accept(10, b"12345678"), Ok(()));
        assert_eq!(
            lan.accept(10, b"123456789"),
            Err(SessionError::MessageTooLarge)
        );
        assert_eq!(lan.accept(19, b"move"), Err(SessionError::UnknownPeer));
    }

    #[test]
    fn disconnect_rejoin_preserves_reserved_slot_and_transport_origin() {
        let mut session = Session::new(config(true));
        assert_eq!(session.join(10, PeerKind::Lan), Ok(()));
        assert_eq!(session.join(11, PeerKind::Local), Ok(()));
        assert_eq!(session.disconnect(10), Ok(()));
        assert_eq!(
            session.accept(10, b"hello"),
            Err(SessionError::UnknownPeer)
        );
        assert_eq!(
            session.join(12, PeerKind::Lan),
            Err(SessionError::NoCapacity)
        );
        assert_eq!(
            session.reconnect(10, PeerKind::Local),
            Err(SessionError::WrongOrigin)
        );
    }

    #[test]
    fn reconnect_reclaims_only_reserved_identity() {
        let mut session = Session::new(config(true));
        assert_eq!(session.join(10, PeerKind::Lan), Ok(()));
        assert_eq!(session.disconnect(10), Ok(()));
        assert_eq!(
            session.reconnect(20, PeerKind::Lan),
            Err(SessionError::UnknownPeer)
        );
        assert_eq!(session.reconnect(10, PeerKind::Lan), Ok(()));
        assert_eq!(session.accept(10, b"hello"), Ok(()));
        assert_eq!(session.remove(10), Ok(()));
        assert_eq!(session.join(12, PeerKind::Lan), Ok(()));
    }

    #[test]
    fn malformed_remote_actions_never_enter_gameplay() {
        let mut session = Session::new(config(true));
        assert_eq!(session.join(9, PeerKind::Lan), Ok(()));
        assert_eq!(session.decode(9, b"M\x04", &SmallActionCodec), Ok(4));
        assert_eq!(
            session.decode(9, b"M\xff", &SmallActionCodec),
            Err(SessionError::MalformedMessage)
        );
        assert_eq!(
            session.decode(9, b"bad", &SmallActionCodec),
            Err(SessionError::MalformedMessage)
        );
        assert_eq!(
            session.decode(8, b"M\x04", &SmallActionCodec),
            Err(SessionError::UnknownPeer)
        );
    }
}
