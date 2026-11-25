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
//   - Bounded title controls and host-assigned remote seat input.
// - Must-Not:
//   - Authenticate clients, open sockets or implement a second simulation.
// - Allows:
//   - Inputs: Host-confirmed lifecycle events and decoded remote controls.
//   - Outputs: The same deterministic cabinet state as local play.
//   - Side effects: Tracks connected seats and applies logical transitions.
// - Split-When:
//   - A distinct client presentation or state replication protocol exists.
// - Merge-When:
//   - Shared remote input routing covers these exact title semantics.
// - Summary:
//   - Host-authoritative cabinet input with safe disconnect releases.
// - Description:
//   - Remote peers can play, but only local seats control host menus.
// - Usage:
//   - Route verified lifecycle and messages through the fixed-step queue.
// - Defaults:
//   - No remote seat is connected until the trusted host admits it.

//! Transport-independent remote control admission for hosted free play.

use games::input::LogicalAction;
use games::network::MessageSchema;
use games::replay::InputStep;

use super::{Action, Cabinet, Control, Maneuver, Phase, Player};

/// Exact length of a version-two remote control message.
pub const CONTROL_BYTES: usize = 3;
const VERSION: u8 = 2;

/// Exact host-to-controller liveness acknowledgement, never a control.
pub const HOST_ACK: [u8; CONTROL_BYTES] = [VERSION, 0, 1];

// These are the seven ordinary station controls. Pause is host-only.
const REMOTE_CONTROLS: [Control; 7] = [
    Control::Start,
    Control::Primary,
    Control::Secondary,
    Control::Left,
    Control::Right,
    Control::Up,
    Control::Down,
];

/// One versioned, seat-independent remote digital transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteControl {
    control: Control,
    down: bool,
}

impl RemoteControl {
    /// Serialize one exact frame; no identity or tick is supplied by peers.
    #[must_use]
    pub fn encode(self) -> [u8; CONTROL_BYTES] {
        [VERSION, self.control.mask(), u8::from(self.down)]
    }

    /// Construct an ordinary station control, excluding host-only Pause.
    #[must_use]
    pub const fn new(control: Control, down: bool) -> Option<Self> {
        if matches!(control, Control::Pause) {
            None
        } else {
            Some(Self { control, down })
        }
    }
}

/// A validated controller message or liveness-only heartbeat.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteMessage {
    /// A station control transition.
    Control(RemoteControl),
    /// Refresh the transport lease without affecting simulation.
    Heartbeat,
}

impl RemoteMessage {
    /// Encode a version-two message.
    #[must_use]
    pub fn encode(self) -> [u8; CONTROL_BYTES] {
        match self {
            Self::Control(value) => value.encode(),
            Self::Heartbeat => [VERSION, 0, 0],
        }
    }
}

/// Strict version-two codec for an authenticated peer's station input.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteCodec;

impl MessageSchema for RemoteCodec {
    type Message = RemoteMessage;

    fn decode(&self, bytes: &[u8]) -> Option<Self::Message> {
        let [VERSION, code, state] = *bytes else {
            return None;
        };
        if code == 0 && state == 0 {
            return Some(RemoteMessage::Heartbeat);
        }
        let down = match state {
            0 => false,
            1 => true,
            _ => return None,
        };
        let control = REMOTE_CONTROLS
            .into_iter()
            .find(|candidate| candidate.mask() == code)?;
        RemoteControl::new(control, down).map(RemoteMessage::Control)
    }
}

/// One ordered event from the trusted host, not directly from the wire.
///
/// Remote identities are assigned locally as the numeric input seat index
/// (0–3). An identity can never select a seat in its message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostedInput {
    /// A verified, host-approved remote identity joined or rejoined.
    Connected(u64),
    /// A remote identity disconnected, was removed or was revoked.
    Disconnected(u64),
    /// An action from a directly attached local input source.
    Local(LogicalAction<Action>),
    /// A remote transition decoded by the host's authenticated roster.
    Remote {
        /// The host-assigned identity, never parsed from the frame.
        peer: u64,
        /// The validated remote control transition.
        value: RemoteControl,
    },
}

impl From<LogicalAction<Action>> for HostedInput {
    fn from(action: LogicalAction<Action>) -> Self {
        Self::Local(action)
    }
}

/// Cabinet input adapter for a host with explicitly reserved remote seats.
///
/// The reservation is fixed for the session, including disconnections.
/// Local actions targeting remote seats are ignored. Host menus only
/// respond to local seats. Remote presses require an active playing phase;
/// disconnect and host suspension release held remote controls. The caller
/// must submit authenticated lifecycle events in order and must not drop
/// release events on queue rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostedCabinet {
    cabinet: Cabinet,
    connected: [bool; 4],
    reserved: [bool; 4],
}

impl HostedCabinet {
    fn apply(
        &mut self,
        event: &HostedInput,
        pending: &mut [Option<Maneuver>; 4],
    ) {
        match event {
            HostedInput::Connected(peer) => {
                if let Some(seat) = self.remote_seat(*peer)
                    && let Some(connected) =
                        self.connected.get_mut(seat.index())
                {
                    *connected = true;
                }
            },
            HostedInput::Disconnected(peer) => {
                if let Some(seat) = self.remote_seat(*peer)
                    && let Some(connected) =
                        self.connected.get_mut(seat.index())
                    && *connected
                {
                    *connected = false;
                    self.release(seat, pending);
                }
            },
            HostedInput::Local(change) => {
                let seat = change.action.player;
                if self.reserved.get(seat.index()) == Some(&false) {
                    self.cabinet.apply(change, pending);
                }
            },
            HostedInput::Remote { peer, value } => {
                if let Some(seat) = self.remote_seat(*peer)
                    && self.connected.get(seat.index()) == Some(&true)
                    && (!value.down || self.cabinet.phase() == Phase::Playing)
                {
                    self.cabinet.apply(
                        &LogicalAction {
                            action: Action {
                                control: value.control,
                                player: seat,
                            },
                            down: value.down,
                        },
                        pending,
                    );
                }
            },
        }
    }

    /// Verified remote connections after the most recent fixed update.
    ///
    /// Local and unreserved seats are always false.
    #[must_use]
    pub const fn connected_remotes(&self) -> [bool; 4] {
        self.connected
    }

    /// Construct a hosted cabinet with fixed remote seat reservations.
    ///
    /// Reserve at most three seats, leaving one local seat for host menus.
    /// The supplied cabinet must be idle in its main menu, with no holds.
    #[must_use]
    pub fn new(cabinet: &Cabinet, reserved: [bool; 4]) -> Option<Self> {
        if reserved.into_iter().all(|remote| remote)
            || cabinet.phase() != Phase::MainMenu
            || cabinet.slots.iter().any(|slot| slot.held != 0)
        {
            return None;
        }
        Some(Self {
            cabinet: *cabinet,
            connected: [false; 4],
            reserved,
        })
    }

    fn release(&mut self, seat: Player, pending: &mut [Option<Maneuver>; 4]) {
        for control in REMOTE_CONTROLS {
            self.cabinet.apply(
                &LogicalAction {
                    action: Action { control, player: seat },
                    down: false,
                },
                pending,
            );
        }
    }

    fn release_remote(&mut self, pending: &mut [Option<Maneuver>; 4]) {
        for (seat, connected) in Player::ALL.into_iter().zip(self.connected) {
            if connected {
                self.release(seat, pending);
            }
        }
    }

    fn remote_seat(&self, peer: u64) -> Option<Player> {
        let index = usize::try_from(peer).ok()?;
        if self.reserved.get(index) != Some(&true) {
            return None;
        }
        Player::ALL.get(index).copied()
    }

    /// Inspect the existing cabinet after the most recent fixed update.
    #[must_use]
    pub const fn state(&self) -> &Cabinet {
        &self.cabinet
    }
}

impl InputStep for HostedCabinet {
    type Input = HostedInput;

    fn input_step(&mut self, _tick: u64, actions: &[Self::Input]) {
        let mut pending = [None; 4];
        for action in actions {
            let playing = self.cabinet.phase() == Phase::Playing;
            self.apply(action, &mut pending);
            if playing && self.cabinet.phase() != Phase::Playing {
                self.release_remote(&mut pending);
            }
        }
        let playing = self.cabinet.phase() == Phase::Playing;
        self.cabinet.finish_tick(pending);
        if playing && self.cabinet.phase() != Phase::Playing {
            self.release_remote(&mut pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroUsize};
    use std::slice;

    use games::network::{PeerKind, Session, SessionConfig, SessionError};

    use super::*;
    use crate::{Participation, StreetLayout};

    fn local(seat: Player, control: Control, down: bool) -> HostedInput {
        HostedInput::Local(LogicalAction {
            action: Action { control, player: seat },
            down,
        })
    }

    fn remote(peer: u64, control: Control, down: bool) -> HostedInput {
        assert_ne!(control, Control::Pause);
        HostedInput::Remote {
            peer,
            value: RemoteControl { control, down },
        }
    }

    fn hosted() -> Result<HostedCabinet, &'static str> {
        let cabinet = Cabinet::new(StreetLayout::DOWNTOWN)
            .map_err(|_error| "valid street")?;
        HostedCabinet::new(&cabinet, [false, true, true, false])
            .ok_or("valid reservations")
    }

    fn check_wire_code(code: u8, control: Option<Control>) {
        for down in [false, true] {
            let encoded = [2, code, u8::from(down)];
            let expected = if code == 0 && !down {
                Some(RemoteMessage::Heartbeat)
            } else {
                control
                    .and_then(|value| RemoteControl::new(value, down))
                    .map(RemoteMessage::Control)
            };
            let decoded = RemoteCodec.decode(&encoded);
            assert_eq!(decoded, expected);
            assert_eq!(
                decoded.map(RemoteMessage::encode),
                expected.map(|_value| encoded)
            );
        }
    }

    #[test]
    fn unreserved_host_is_identical_to_offline_cabinet()
    -> Result<(), &'static str> {
        let mut offline = Cabinet::new(StreetLayout::DOWNTOWN)
            .map_err(|_error| "valid street")?;
        let mut host = HostedCabinet::new(&offline, [false; 4])
            .ok_or("valid local cabinet")?;
        for (tick, (control, down)) in (0u64..).zip([
            (Control::Start, true),
            (Control::Start, false),
            (Control::Right, true),
            (Control::Primary, true),
            (Control::Pause, true),
            (Control::Right, false),
            (Control::Pause, false),
            (Control::Start, true),
            (Control::Secondary, true),
            (Control::Start, false),
        ]) {
            let change = LogicalAction {
                action: Action {
                    control,
                    player: Player::Marge,
                },
                down,
            };
            offline.input_step(tick, slice::from_ref(&change));
            host.input_step(tick, &[change.into()]);
            if *host.state() != offline {
                return Err("offline cabinet diverged");
            }
        }
        Ok(())
    }

    #[test]
    fn codec_admits_only_exact_versioned_control_edges() {
        for code in u8::MIN..=u8::MAX {
            let control = match code {
                2 => Some(Control::Start),
                4 => Some(Control::Primary),
                8 => Some(Control::Secondary),
                16 => Some(Control::Left),
                32 => Some(Control::Right),
                64 => Some(Control::Up),
                128 => Some(Control::Down),
                _ => None,
            };
            check_wire_code(code, control);
        }
        assert_eq!(RemoteControl::new(Control::Pause, true), None);
    }

    #[test]
    fn codec_rejects_wrong_length_version_and_state() {
        let codec = RemoteCodec;
        assert_eq!(codec.decode(&[]), None);
        assert_eq!(codec.decode(&[2, 2]), None);
        assert_eq!(codec.decode(&[2, 2, 1, 0]), None);
        assert_eq!(codec.decode(&[1, 2, 1]), None);
        assert_eq!(HOST_ACK, [2, 0, 1]);
        assert_eq!(codec.decode(&HOST_ACK), None);
        for invalid in 2..=u8::MAX {
            assert_eq!(codec.decode(&[2, 2, invalid]), None);
        }
    }

    fn roster() -> Result<Session, &'static str> {
        let config = SessionConfig {
            allow_lan: true,
            max_message_bytes: NonZeroUsize::new(CONTROL_BYTES)
                .ok_or("frame size")?,
            max_players: NonZeroU8::new(4).ok_or("four seats")?,
        };
        let mut roster = Session::new(config);
        roster
            .join(0, PeerKind::Local)
            .map_err(|_error| "local seat")?;
        for peer in [1, 2] {
            roster
                .join(peer, PeerKind::Lan)
                .map_err(|_error| "LAN seat")?;
        }
        Ok(roster)
    }

    fn check_roster(roster: &mut Session) {
        let start = [2, 2, 1];
        assert_eq!(
            roster.decode(3, &start, &RemoteCodec),
            Err(SessionError::UnknownPeer)
        );
        assert_eq!(
            roster.decode(1, &[2, 1, 1], &RemoteCodec),
            Err(SessionError::MalformedMessage)
        );
        assert_eq!(
            roster.decode(1, &[2, 2, 1, 0], &RemoteCodec),
            Err(SessionError::MessageTooLarge)
        );
        assert_eq!(roster.disconnect(1), Ok(()));
        assert_eq!(
            roster.decode(1, &start, &RemoteCodec),
            Err(SessionError::UnknownPeer)
        );
        assert_eq!(roster.reconnect(1, PeerKind::Lan), Ok(()));
        assert_eq!(
            roster.decode(1, &start, &RemoteCodec).ok(),
            RemoteControl::new(Control::Start, true)
                .map(RemoteMessage::Control)
        );
    }

    #[test]
    fn roster_applies_title_codec_after_identity_and_budget_checks()
    -> Result<(), &'static str> {
        check_roster(&mut roster()?);
        Ok(())
    }

    fn join_players(host: &mut HostedCabinet, message: RemoteControl) {
        host.input_step(0, &[
            HostedInput::Connected(1),
            HostedInput::Connected(2),
            HostedInput::Connected(3),
            HostedInput::Connected(u64::MAX),
            local(Player::Marge, Control::Start, true),
            HostedInput::Remote { peer: 1, value: message },
            remote(2, Control::Start, true),
            remote(3, Control::Start, true),
            remote(u64::MAX, Control::Start, true),
        ]);
        let [marge, homer, bart, lisa] = host.state().stations();
        assert_eq!(marge.participation, Participation::Playing);
        assert_eq!(homer.participation, Participation::Playing);
        assert_eq!(bart.participation, Participation::Playing);
        assert_eq!(lisa.participation, Participation::Absent);
    }

    fn move_players(host: &mut HostedCabinet) {
        let [marge, homer, bart, lisa] = host.state().stations();
        host.input_step(1, &[
            local(Player::Marge, Control::Right, true),
            local(Player::Homer, Control::Left, true),
            remote(1, Control::Right, true),
            remote(2, Control::Up, true),
        ]);
        let [moved_marge, moved_homer, moved_bart, unchanged] =
            host.state().stations();
        assert!(moved_marge.position.x > marge.position.x);
        assert!(moved_homer.position.x > homer.position.x);
        assert!(moved_bart.position.y < bart.position.y);
        assert_eq!(unchanged, lisa);
    }

    fn rejoin_player(host: &mut HostedCabinet) {
        let [moved_marge, moved_homer, moved_bart, _] = host.state().stations();
        host.input_step(2, &[
            HostedInput::Disconnected(1),
            remote(1, Control::Right, true),
            local(Player::Homer, Control::Right, true),
            HostedInput::Connected(2),
        ]);
        let [moving_marge, stopped_homer, moving_bart, _] =
            host.state().stations();
        assert!(moving_marge.position.x > moved_marge.position.x);
        assert_eq!(stopped_homer.position, moved_homer.position);
        assert!(moving_bart.position.y < moved_bart.position.y);
        host.input_step(3, &[HostedInput::Connected(1)]);
        let [_, rejoined, continuing, _] = host.state().stations();
        assert_eq!(rejoined.position, stopped_homer.position);
        assert!(continuing.position.y < moving_bart.position.y);
        host.input_step(4, &[remote(1, Control::Left, true)]);
        let [_, reversed, _, _] = host.state().stations();
        assert!(reversed.position.x < rejoined.position.x);
    }

    #[test]
    fn hosted_play_keeps_authenticated_seats_independent()
    -> Result<(), &'static str> {
        let mut host = hosted()?;
        let roster = roster()?;
        let message = roster
            .decode(1, &[2, 2, 1], &RemoteCodec)
            .map_err(|_error| "decode approved start")?;
        let RemoteMessage::Control(control) = message else {
            return Err("unexpected heartbeat");
        };
        join_players(&mut host, control);
        move_players(&mut host);
        rejoin_player(&mut host);
        Ok(())
    }

    fn check_host_menu(host: &mut HostedCabinet) {
        let original = host.state().session();
        host.input_step(0, &[
            HostedInput::Connected(1),
            remote(1, Control::Down, true),
            remote(1, Control::Primary, true),
            remote(1, Control::Start, true),
            local(Player::Homer, Control::Start, true),
        ]);
        assert_eq!(host.state().session(), original);
        host.input_step(1, &[
            local(Player::Marge, Control::Start, true),
            remote(1, Control::Start, true),
            remote(1, Control::Right, true),
        ]);
        let [_, moving, ..] = host.state().stations();
        assert_eq!(moving.participation, Participation::Playing);
    }

    fn check_pause(host: &mut HostedCabinet) {
        let [_, moving, ..] = host.state().stations();
        host.input_step(2, &[
            local(Player::Marge, Control::Pause, true),
            remote(1, Control::Down, true),
            remote(1, Control::Primary, true),
            remote(1, Control::Start, true),
        ]);
        let paused = host.state().session();
        assert_eq!(paused.phase(), Phase::Paused);
        host.input_step(3, &[
            remote(1, Control::Start, false),
            remote(1, Control::Start, true),
            remote(1, Control::Up, true),
            remote(1, Control::Left, true),
        ]);
        assert_eq!(host.state().session(), paused);
        host.input_step(4, &[
            local(Player::Marge, Control::Pause, false),
            local(Player::Marge, Control::Pause, true),
        ]);
        let [_, stopped, ..] = host.state().stations();
        assert_eq!(host.state().phase(), Phase::Playing);
        assert_eq!(stopped.position, moving.position);
        host.input_step(5, &[remote(1, Control::Right, true)]);
        let [_, resumed, ..] = host.state().stations();
        assert!(resumed.position.x > stopped.position.x);
    }

    #[test]
    fn remote_input_cannot_control_host_menus_or_resume_stale_holds()
    -> Result<(), &'static str> {
        let mut host = hosted()?;
        check_host_menu(&mut host);
        check_pause(&mut host);
        if HostedCabinet::new(host.state(), [false; 4]).is_some() {
            return Err("active cabinet accepted");
        }
        Ok(())
    }

    #[test]
    fn presentation_observes_only_verified_remote_connections()
    -> Result<(), &'static str> {
        let mut host = hosted()?;
        if host.connected_remotes() != [false; 4] {
            return Err("unconnected seats reported online");
        }
        host.input_step(0, &[
            HostedInput::Connected(0),
            HostedInput::Connected(1),
            HostedInput::Connected(2),
            HostedInput::Connected(3),
            HostedInput::Connected(u64::MAX),
        ]);
        if host.connected_remotes() != [false, true, true, false] {
            return Err("incorrect remote admission state");
        }
        host.input_step(1, &[
            HostedInput::Disconnected(1),
            remote(1, Control::Start, true),
            HostedInput::Connected(2),
        ]);
        if host.connected_remotes() != [false, false, true, false] {
            return Err("incorrect remote disconnect state");
        }
        host.input_step(2, &[HostedInput::Connected(1)]);
        if host.connected_remotes() != [false, true, true, false] {
            return Err("incorrect remote admission state");
        }
        if host.state().phase() != Phase::MainMenu {
            return Err("remote status affected host menu");
        }
        Ok(())
    }

    #[test]
    fn hosted_configuration_requires_a_local_seat() -> Result<(), &'static str>
    {
        let host = hosted()?;
        if HostedCabinet::new(host.state(), [true; 4]).is_some() {
            return Err("no local seat accepted");
        }
        Ok(())
    }
}
