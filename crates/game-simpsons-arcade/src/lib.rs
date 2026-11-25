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
//   - Title-specific four-player entry and initial street movement rules.
// - Must-Not:
//   - Access operating-system devices, render or interpret original ROMs.
// - Allows:
//   - Inputs: Ordered logical controls at a fixed simulation tick.
//   - Outputs: Cabinet phase, active slots, positions and facing.
//   - Side effects: Deterministic in-memory state transitions.
// - Split-When:
//   - Independent title combat or stage progression rules are implemented.
// - Merge-When:
//   - Another title module duplicates player entry or held-control state.
// - Summary:
//   - Initial four-player entry and street movement prototype.
// - Description:
//   - Models coin, start and confirmation before fixed-step movement.
// - Usage:
//   - Drive with the shared InputRouter and StepClock or replay adapter.
// - Defaults:
//   - All players absent; no implicit credits or platform activity.

//! Initial cabinet entry and street movement for the pinned four-player set.

mod maneuver;

#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Native prototype host for locally imported art.
pub mod native;
use std::time::Duration;

use games::input::LogicalAction;
use games::replay::InputStep;
pub use maneuver::Maneuver;

/// Fixed reference screen period: 384 by 264 clocks at 6 MHz.
pub const TICK_INTERVAL: Duration = Duration::from_micros(16_896);

/// Fixed cabinet positions for the original four-player edition.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Player {
    /// Third station.
    Bart,
    /// Second station.
    Homer,
    /// Fourth station.
    Lisa,
    /// First station.
    Marge,
}

impl Player {
    /// Stations in cabinet order.
    pub const ALL: [Self; 4] =
        [Self::Marge, Self::Homer, Self::Bart, Self::Lisa];

    /// Index in a complete four-station array.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Marge => 0,
            Self::Homer => 1,
            Self::Bart => 2,
            Self::Lisa => 3,
        }
    }
}

/// Public digital controls of one cabinet station.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Control {
    /// Virtual coin; no monetary transaction.
    Coin,
    /// Down on the eight-way stick.
    Down,
    /// Left on the eight-way stick.
    Left,
    /// First button; confirms entry or starts an attack in the street.
    Primary,
    /// Right on the eight-way stick.
    Right,
    /// Second button; starts a jump in the street.
    Secondary,
    /// Claim a station after inserting a virtual coin.
    Start,
    /// Up on the eight-way stick.
    Up,
}

impl Control {
    const fn mask(self) -> u8 {
        match self {
            Self::Coin => 1,
            Self::Start => 2,
            Self::Primary => 4,
            Self::Secondary => 8,
            Self::Left => 16,
            Self::Right => 32,
            Self::Up => 64,
            Self::Down => 128,
        }
    }
}

/// A logical action routed to exactly one cabinet station.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Action {
    /// Digital control being pressed or released.
    pub control: Control,
    /// Cabinet station receiving the input.
    pub player: Player,
}

/// The three implemented cabinet phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Waiting for a first virtual coin.
    Attract,
    /// Four-player entry; Start claims a station and Primary confirms.
    Selection,
    /// Initial street movement; the intervening intro is not yet modelled.
    Street,
}

/// A station's current admission state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Participation {
    /// Not playing.
    Absent,
    /// Station is active in the street.
    Playing,
    /// Station claimed in the selection screen.
    Selected,
}

/// Integer screen-space foot position in the first street prototype.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    /// Horizontal location.
    pub x: i16,
    /// Vertical location.
    pub y: i16,
}

/// Horizontal character facing, retained during vertical movement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Facing {
    /// Facing toward the left.
    Left,
    /// Facing toward the right.
    Right,
}

/// Prototype walkable area and four initial foot positions.
///
/// Isolated original-machine observations support the two-pixel rate
/// during some unobstructed movement intervals for all four stations.
/// Boundaries, spawn offsets, diagonal motion and collisions remain
/// provisional until further independent comparisons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreetLayout {
    /// Maximum foot position.
    pub max: Point,
    /// Minimum foot position.
    pub min: Point,
    /// Spawn positions in cabinet order.
    pub spawn: [Point; 4],
}

impl StreetLayout {
    /// Initial downtown screen prototype, before camera and collision rules.
    pub const DOWNTOWN: Self = Self {
        max: Point { x: 288, y: 208 },
        min: Point { x: 32, y: 146 },
        spawn: [
            Point { x: 110, y: 190 },
            Point { x: 88, y: 190 },
            Point { x: 132, y: 178 },
            Point { x: 154, y: 178 },
        ],
    };

    fn contains(self, point: Point) -> bool {
        (self.min.x..=self.max.x).contains(&point.x)
            && (self.min.y..=self.max.y).contains(&point.y)
    }
}

/// Invalid prototype movement configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// A spawn point is outside the walkable area.
    SpawnOutside,
}

/// A read-only view of an individual station.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Station {
    /// Remaining virtual credits.
    pub credits: u8,
    /// Current sprite facing.
    pub facing: Facing,
    /// Current attack or jump, independent of rendering.
    pub maneuver: Maneuver,
    /// Whether the station has joined.
    pub participation: Participation,
    /// Current foot position.
    pub position: Point,
    /// Consecutive displaced ticks, zero-based; None when stationary.
    pub walking_ticks: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Slot {
    held: u8,
    station: Station,
}

impl Slot {
    fn action(&mut self, control: Control, down: bool, phase: Phase) -> bool {
        let mask = control.mask();
        let was_held = self.held & mask != 0;
        if down {
            self.held |= mask;
        } else {
            self.held &= !mask;
        }
        if !down || was_held {
            return false;
        }
        match control {
            Control::Coin => {
                self.station.credits = self.station.credits.saturating_add(1);
            },
            Control::Start => self.start(phase),
            Control::Primary => {
                if phase == Phase::Selection {
                    return self.station.participation
                        == Participation::Selected;
                }
                if phase == Phase::Street {
                    self.begin_maneuver(Maneuver::Attack { tick: 0 });
                }
            },
            Control::Secondary => {
                if phase == Phase::Street {
                    self.begin_maneuver(Maneuver::Jump { tick: 0 });
                }
            },
            Control::Down | Control::Left | Control::Right | Control::Up => {},
        }
        false
    }

    fn begin_maneuver(&mut self, maneuver: Maneuver) {
        if self.station.participation == Participation::Playing
            && self.station.maneuver == Maneuver::Idle
        {
            self.station.maneuver = maneuver;
            self.station.walking_ticks = None;
        }
    }

    const fn held(self, control: Control) -> bool {
        self.held & control.mask() != 0
    }

    fn move_tick(&mut self, layout: StreetLayout) {
        if self.station.participation != Participation::Playing {
            return;
        }
        let dx = axis(self.held(Control::Left), self.held(Control::Right));
        let dy = axis(self.held(Control::Up), self.held(Control::Down));
        if dx < 0 {
            self.station.facing = Facing::Left;
        } else if dx > 0 {
            self.station.facing = Facing::Right;
        }
        if matches!(self.station.maneuver, Maneuver::Attack { .. }) {
            return;
        }
        let previous = self.station.position;
        let position = &mut self.station.position;
        position.x = position
            .x
            .saturating_add(dx)
            .clamp(layout.min.x, layout.max.x);
        position.y = position
            .y
            .saturating_add(dy)
            .clamp(layout.min.y, layout.max.y);
        self.station.walking_ticks = if *position == previous
            || self.station.maneuver != Maneuver::Idle
        {
            None
        } else {
            Some(
                self.station
                    .walking_ticks
                    .map_or(0, |elapsed| elapsed.saturating_add(1)),
            )
        };
    }

    const fn new(position: Point) -> Self {
        Self {
            held: 0,
            station: Station {
                credits: 0,
                facing: Facing::Right,
                maneuver: Maneuver::Idle,
                participation: Participation::Absent,
                position,
                walking_ticks: None,
            },
        }
    }

    fn start(&mut self, phase: Phase) {
        if self.station.credits == 0
            || self.station.participation != Participation::Absent
        {
            return;
        }
        self.station.credits = self.station.credits.saturating_sub(1);
        self.station.participation = if phase == Phase::Street {
            Participation::Playing
        } else {
            Participation::Selected
        };
    }
}

/// Deterministic first-street title state, independent of the host platform.
///
/// The prototype omits the original intro, camera, collision, enemy
/// combat, scoring and continuing. Maneuver timings are provisional.
/// It is not a gameplay parity claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cabinet {
    layout: StreetLayout,
    phase: Phase,
    slots: [Slot; 4],
}

impl Cabinet {
    fn activate_selected(&mut self) {
        self.phase = Phase::Street;
        for selected in &mut self.slots {
            if selected.station.participation == Participation::Selected {
                selected.station.participation = Participation::Playing;
            }
        }
    }

    fn apply(&mut self, change: &LogicalAction<Action>) {
        let Some(slot) = self.slots.get_mut(change.action.player.index())
        else {
            return;
        };
        let confirm =
            slot.action(change.action.control, change.down, self.phase);
        if self.phase == Phase::Attract && slot.station.credits > 0 {
            self.phase = Phase::Selection;
        }
        if confirm {
            self.activate_selected();
        }
    }

    /// Construct a cabinet with explicit spawn and movement boundaries.
    ///
    /// # Errors
    /// Rejects invalid bounds or a spawn outside them.
    pub fn new(layout: StreetLayout) -> Result<Self, LayoutError> {
        if !layout.spawn.iter().all(|point| layout.contains(*point)) {
            return Err(LayoutError::SpawnOutside);
        }
        Ok(Self {
            layout,
            phase: Phase::Attract,
            slots: layout.spawn.map(Slot::new),
        })
    }

    /// Current cabinet phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Inspect all four stations in cabinet order.
    #[must_use]
    pub fn stations(&self) -> [Station; 4] {
        self.slots.map(|slot| slot.station)
    }
}

impl InputStep for Cabinet {
    type Input = LogicalAction<Action>;

    fn input_step(&mut self, _tick: u64, actions: &[Self::Input]) {
        for (player, slot) in Player::ALL.into_iter().zip(&mut self.slots) {
            slot.station.maneuver = slot.station.maneuver.advance(player);
        }
        for action in actions {
            self.apply(action);
        }
        if self.phase == Phase::Street {
            for slot in &mut self.slots {
                slot.move_tick(self.layout);
            }
        }
    }
}

const fn axis(negative: bool, positive: bool) -> i16 {
    match (negative, positive) {
        (true, false) => -2,
        (false, true) => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use games::replay::{InputScript, Scripted};
    use games::timing::StepClock;

    use super::*;

    fn cabinet() -> Cabinet {
        let result = Cabinet::new(StreetLayout::DOWNTOWN);
        assert!(result.is_ok());
        // A valid constant layout cannot fail; keep fixture construction
        // independent from any mutable global or host input.
        result.unwrap_or_else(|_error| Cabinet {
            layout: StreetLayout::DOWNTOWN,
            phase: Phase::Attract,
            slots: StreetLayout::DOWNTOWN.spawn.map(Slot::new),
        })
    }

    fn change(
        player: Player,
        control: Control,
        down: bool,
    ) -> LogicalAction<Action> {
        LogicalAction {
            action: Action { control, player },
            down,
        }
    }

    fn pulse(player: Player, control: Control) -> [LogicalAction<Action>; 2] {
        [
            change(player, control, true),
            change(player, control, false),
        ]
    }

    #[test]
    fn coin_start_and_confirmation_are_distinct() {
        let mut state = cabinet();
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        assert_eq!(state.phase(), Phase::Attract);
        state.input_step(1, &pulse(Player::Marge, Control::Coin));
        assert_eq!(state.phase(), Phase::Selection);
        assert_eq!(state.stations().first().map(|slot| slot.credits), Some(1));
        state.input_step(2, &pulse(Player::Marge, Control::Start));
        assert_eq!(state.phase(), Phase::Selection);
        assert_eq!(
            state.stations().first().map(|slot| slot.participation),
            Some(Participation::Selected)
        );
        state.input_step(3, &pulse(Player::Homer, Control::Primary));
        assert_eq!(state.phase(), Phase::Selection);
        state.input_step(4, &pulse(Player::Marge, Control::Primary));
        assert_eq!(state.phase(), Phase::Street);
        assert_eq!(
            state.stations().first().map(|slot| slot.participation),
            Some(Participation::Playing)
        );
    }

    fn four_playing() -> Cabinet {
        let mut state = cabinet();
        for player in Player::ALL {
            state.input_step(0, &pulse(player, Control::Coin));
            state.input_step(1, &pulse(player, Control::Start));
        }
        state.input_step(2, &pulse(Player::Marge, Control::Primary));
        state
    }

    #[test]
    fn four_stations_move_independently_and_clamp_to_the_layout() {
        let mut state = four_playing();
        let original = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Left, true),
            change(Player::Homer, Control::Right, true),
            change(Player::Bart, Control::Up, true),
            change(Player::Lisa, Control::Down, true),
        ]);
        for tick in 4..8 {
            state.input_step(tick, &[]);
        }
        let [marge, homer, bart, lisa] = state.stations();
        let [old_marge, old_homer, old_bart, old_lisa] = original;
        assert_eq!(marge.position.x, old_marge.position.x - 10);
        assert_eq!(homer.position.x, old_homer.position.x + 10);
        assert_eq!(bart.position.y, old_bart.position.y - 10);
        assert_eq!(lisa.position.y, old_lisa.position.y + 10);
        assert_eq!(marge.facing, Facing::Left);
        state.input_step(8, &[change(Player::Marge, Control::Right, true)]);
        assert_eq!(
            state.stations().first().map(|slot| slot.position),
            Some(marge.position)
        );
        assert_clamped(state);
    }

    fn assert_clamped(mut state: Cabinet) {
        for tick in 9..300 {
            state.input_step(tick, &[]);
        }
        let [_, right, upper, lower] = state.stations();
        assert_eq!(right.position.x, StreetLayout::DOWNTOWN.max.x);
        assert_eq!(upper.position.y, StreetLayout::DOWNTOWN.min.y);
        assert_eq!(lower.position.y, StreetLayout::DOWNTOWN.max.y);
    }

    #[test]
    fn held_coin_does_not_repeat_and_late_join_has_own_credit() {
        let mut state = cabinet();
        let coin = change(Player::Marge, Control::Coin, true);
        state.input_step(0, &[coin.clone(), coin]);
        state.input_step(1, &pulse(Player::Marge, Control::Start));
        state.input_step(2, &pulse(Player::Marge, Control::Primary));
        state.input_step(3, &pulse(Player::Marge, Control::Start));
        state.input_step(4, &pulse(Player::Homer, Control::Start));
        let [marge, homer, ..] = state.stations();
        assert_eq!(marge.credits, 0);
        assert_eq!(homer.participation, Participation::Absent);
        state.input_step(5, &pulse(Player::Homer, Control::Coin));
        state.input_step(6, &pulse(Player::Homer, Control::Start));
        let [_, joined, ..] = state.stations();
        assert_eq!(joined.participation, Participation::Playing);
    }

    #[test]
    fn walking_ticks_track_actual_displacement_and_restart() {
        let mut state = four_playing();
        state.input_step(3, &[change(Player::Marge, Control::Right, true)]);
        let [started, _, _, _] = state.stations();
        assert_eq!(started.walking_ticks, Some(0));
        state.input_step(4, &[]);
        let [moving, _, _, _] = state.stations();
        assert_eq!(moving.walking_ticks, Some(1));
        state.input_step(5, &[change(Player::Marge, Control::Left, true)]);
        let [opposed, _, _, _] = state.stations();
        assert_eq!(opposed.walking_ticks, None);
        state.input_step(6, &[change(Player::Marge, Control::Right, false)]);
        let [restarted, _, _, _] = state.stations();
        assert_eq!(restarted.walking_ticks, Some(0));
        for tick in 7..300 {
            state.input_step(tick, &[]);
        }
        let [blocked, _, _, _] = state.stations();
        assert_eq!(blocked.position.x, StreetLayout::DOWNTOWN.min.x);
        assert_eq!(blocked.walking_ticks, None);
    }

    #[test]
    fn isolated_actions_respect_station_and_ground_state() {
        let mut state = four_playing();
        let [before, homer, ..] = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Marge, Control::Right, true),
            change(Player::Homer, Control::Secondary, true),
        ]);
        let [attacking, jumping, ..] = state.stations();
        assert_eq!(attacking.maneuver, Maneuver::Attack { tick: 0 });
        assert_eq!(attacking.position, before.position);
        assert_eq!(jumping.maneuver, Maneuver::Jump { tick: 0 });
        assert_eq!(jumping.position, homer.position);
        state.input_step(4, &pulse(Player::Marge, Control::Secondary));
        for tick in 5..20 {
            state.input_step(tick, &[]);
        }
        assert_maneuvers_at_end(&state, before.position);
        state.input_step(20, &[]);
        let [walking, ..] = state.stations();
        assert_eq!(walking.maneuver, Maneuver::Idle);
        assert_eq!(walking.position.x, before.position.x + 2);
        assert_eq!(walking.walking_ticks, Some(0));
    }

    fn assert_maneuvers_at_end(state: &Cabinet, position: Point) {
        let [attack_end, still_airborne, ..] = state.stations();
        assert_eq!(attack_end.position, position);
        assert_eq!(attack_end.maneuver, Maneuver::Attack { tick: 16 });
        assert_eq!(still_airborne.maneuver, Maneuver::Jump { tick: 16 });
    }

    #[test]
    fn jump_requires_new_press_after_landing() {
        let mut state = four_playing();
        let [_, homer, ..] = state.stations();
        state.input_step(3, &[change(Player::Homer, Control::Secondary, true)]);
        for tick in 4..35 {
            state.input_step(tick, &[]);
        }
        let [_, landed, ..] = state.stations();
        assert_eq!(landed.maneuver, Maneuver::Idle);
        assert_eq!(landed.position, homer.position);
        state.input_step(35, &[change(
            Player::Homer,
            Control::Secondary,
            true,
        )]);
        let [_, held, ..] = state.stations();
        assert_eq!(held.maneuver, Maneuver::Idle);
        state.input_step(36, &[change(
            Player::Homer,
            Control::Secondary,
            false,
        )]);
        state.input_step(37, &pulse(Player::Homer, Control::Secondary));
        let [_, restarted, ..] = state.stations();
        assert_eq!(restarted.maneuver, Maneuver::Jump { tick: 0 });
    }

    #[test]
    fn replay_is_independent_of_render_frame_partition() {
        let mut script = InputScript::new();
        for action in pulse(Player::Marge, Control::Coin) {
            script.record(0, action);
        }
        for action in pulse(Player::Marge, Control::Start) {
            script.record(1, action);
        }
        for action in pulse(Player::Marge, Control::Primary) {
            script.record(2, action);
        }
        script.record(3, change(Player::Marge, Control::Right, true));
        script.record(8, change(Player::Marge, Control::Right, false));
        for action in pulse(Player::Marge, Control::Secondary) {
            script.record(9, action);
        }
        let clocks = StepClock::new(TICK_INTERVAL, NonZeroU32::MAX);
        assert!(clocks.is_ok());
        let Ok(mut slow_clock) = clocks else {
            return;
        };
        let mut fast_clock = slow_clock;
        let mut slow = Scripted::new(cabinet(), script.clone());
        let mut fast = Scripted::new(cabinet(), script);
        assert!(
            slow_clock
                .advance(TICK_INTERVAL.saturating_mul(12), &mut slow)
                .is_ok()
        );
        for _ in 0u8..12 {
            assert!(fast_clock.advance(TICK_INTERVAL, &mut fast).is_ok());
        }
        assert_eq!(slow.state(), fast.state());
    }

    #[test]
    fn invalid_spawn_is_rejected() {
        let layout = StreetLayout {
            max: Point { x: 1, y: 1 },
            ..StreetLayout::DOWNTOWN
        };
        assert_eq!(Cabinet::new(layout), Err(LayoutError::SpawnOutside));
    }
}
