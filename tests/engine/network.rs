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
//   - Synthetic host/join, LAN policy and disconnect/rejoin tests.
// - Must-Not:
//   - Imply real transport encryption or LAN connectivity was tested.
// - Allows:
//   - Inputs: Trusted synthetic identities, frames and session settings.
//   - Outputs: Public admission, capacity and reconnect assertions.
//   - Side effects: None beyond volatile in-memory session state.
// - Split-When:
//   - Platform network transport obtains its own integration fixtures.
// - Merge-When:
//   - Shared self-hosted session tests cover these contracts.
// - Summary:
//   - Offline-first session policy tests.
// - Description:
//   - Verifies local default, opt-in LAN, bounded frames and rejoin.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No sockets, external accounts or original game media.
//

use std::num::{NonZeroU8, NonZeroUsize};

use super::{MessageSchema, PeerKind, Session, SessionConfig, SessionError};

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
        max_message_bytes: NonZeroUsize::new(8).unwrap_or(NonZeroUsize::MIN),
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
    assert_eq!(session.accept(10, b"hello"), Err(SessionError::UnknownPeer));
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
