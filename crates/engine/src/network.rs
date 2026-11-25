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
#[path = "../../../tests/engine/network.rs"]
mod tests;
