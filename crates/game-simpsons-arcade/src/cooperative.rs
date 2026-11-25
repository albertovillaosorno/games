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
//   - The two observed character-pair admission and locomotion rules.
// - Must-Not:
//   - Read reference media, render sprites or decide combat damage.
// - Allows:
//   - Inputs: Ready stations, held logical direction and movement bounds.
//   - Outputs: Pair pose phase and bounded common displacement.
//   - Side effects: Deterministic updates to the two participating stations.
// - Split-When:
//   - Verified pair-specific attacks require independent action strategies.
// - Merge-When:
//   - Pair motion is expressed by an existing title-local mechanism.
// - Summary:
//   - Optional opening-street cooperative locomotion.
// - Description:
//   - Maintains proximity admission, direction priority and independent station
//     health; a solo maneuver or hurt state breaks the pair.
// - Usage:
//   - Enable only the pair families supported by the host's local assets.
// - Defaults:
//   - Both pair families disabled; unchanged solo movement.

//! Provisional cooperative locomotion for two observed character pairs.

use super::{
    Cooperation, Facing, Maneuver, PairState, Player, Point, Recovery, Slot,
    StreetLayout,
};

const ADMISSION_TICKS: u8 = 38;
const HORIZONTAL_PROXIMITY: u16 = 32;
const DEPTH_PROXIMITY: u16 = 24;

/// Character combinations with observed shared locomotion.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Pair {
    /// Third and fourth character.
    BartLisa,
    /// First and second character.
    MargeHomer,
}

impl Pair {
    /// Pair families in cabinet order.
    pub const ALL: [Self; 2] = [Self::MargeHomer, Self::BartLisa];

    /// Identify the supported pair family for a character.
    #[must_use]
    pub const fn for_player(player: Player) -> Self {
        match player {
            Player::Marge | Player::Homer => Self::MargeHomer,
            Player::Bart | Player::Lisa => Self::BartLisa,
        }
    }

    /// Final opening-pose offset, held until movement begins.
    #[must_use]
    pub const fn idle_end(self) -> u8 {
        match self {
            Self::MargeHomer => 13,
            Self::BartLisa => 9,
        }
    }

    /// Characters in directional-priority order.
    #[must_use]
    pub const fn members(self) -> [Player; 2] {
        match self {
            Self::MargeHomer => [Player::Marge, Player::Homer],
            Self::BartLisa => [Player::Bart, Player::Lisa],
        }
    }

    /// Duration of the observed repeating locomotion sequence.
    #[must_use]
    pub const fn walk_duration(self) -> u8 {
        match self {
            Self::MargeHomer => 32,
            Self::BartLisa => 24,
        }
    }
}

/// Current presentation phase of a joined pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairMotion {
    /// Opening pose, clamped at the final still pose.
    Idle {
        /// Zero-based phase in the opening sequence.
        tick: u8,
    },
    /// Repeating locomotion pose, possibly held after stopping.
    Walk {
        /// Zero-based phase within the pair's walking cycle.
        tick: u8,
    },
}

/// Read-only shared pose of two stations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairPose {
    /// Direction of the combined sprite.
    pub facing: Facing,
    /// Current shared animation phase.
    pub motion: PairMotion,
    /// Provisional ground anchor of the first member.
    pub position: Point,
}

impl PairState {
    fn admit(&mut self, first: &mut Slot, second: &mut Slot) {
        if !self.enabled
            || !ready(first)
            || !ready(second)
            || first.direction() != (0, 0)
            || second.direction() != (0, 0)
            || !near(first.station.position, second.station.position)
        {
            self.idle_ticks = 0;
            return;
        }
        self.idle_ticks = self.idle_ticks.saturating_add(1);
        if self.idle_ticks < ADMISSION_TICKS {
            return;
        }
        self.motion = Some(PairMotion::Idle { tick: 0 });
        let facing = if first.station.position.x >= second.station.position.x {
            Facing::Left
        } else {
            Facing::Right
        };
        first.station.facing = facing;
        second.station.facing = facing;
    }

    fn advance(
        &mut self,
        pair: Pair,
        (first, second): (&mut Slot, &mut Slot),
        layout: StreetLayout,
    ) {
        self.invalidate(first, second);
        if let Some(motion) = self.motion {
            let direction = if first.direction() == (0, 0) {
                second.direction()
            } else {
                first.direction()
            };
            self.update_motion(pair, motion, direction);
            translate(first, second, direction, layout);
        } else {
            first.move_tick(layout);
            second.move_tick(layout);
            self.admit(first, second);
        }
    }

    fn invalidate(&mut self, first: &Slot, second: &Slot) {
        if !ready(first) || !ready(second) {
            self.reset();
        }
    }

    const fn pose(self, first: &Slot) -> Option<PairPose> {
        match self.motion {
            Some(motion) => Some(PairPose {
                facing: first.station.facing,
                motion,
                position: first.station.position,
            }),
            None => None,
        }
    }

    const fn reset(&mut self) {
        self.idle_ticks = 0;
        self.motion = None;
        self.walking = false;
    }

    fn update_motion(
        &mut self,
        pair: Pair,
        motion: PairMotion,
        direction: (i16, i16),
    ) {
        if direction == (0, 0) {
            self.motion = Some(match motion {
                PairMotion::Idle { tick } => PairMotion::Idle {
                    tick: tick.saturating_add(1).min(pair.idle_end()),
                },
                PairMotion::Walk { .. } if pair == Pair::BartLisa => {
                    PairMotion::Idle { tick: pair.idle_end() }
                },
                PairMotion::Walk { .. } => motion,
            });
            self.walking = false;
        } else {
            let tick = match motion {
                PairMotion::Walk { tick } if self.walking => tick
                    .saturating_add(1)
                    .checked_rem(pair.walk_duration())
                    .unwrap_or_default(),
                PairMotion::Idle { .. } | PairMotion::Walk { .. } => 0,
            };
            self.motion = Some(PairMotion::Walk { tick });
            self.walking = true;
        }
    }
}

impl Cooperation {
    pub(super) fn advance(
        &mut self,
        slots: &mut [Slot; 4],
        layout: StreetLayout,
    ) {
        let [marge, homer, bart, lisa] = slots;
        self.marge_homer
            .advance(Pair::MargeHomer, (marge, homer), layout);
        self.bart_lisa.advance(Pair::BartLisa, (bart, lisa), layout);
    }

    pub(super) const fn configure(enabled: [bool; 2]) -> Self {
        let [marge_homer, bart_lisa] = enabled;
        Self {
            bart_lisa: PairState {
                enabled: bart_lisa,
                idle_ticks: 0,
                motion: None,
                walking: false,
            },
            marge_homer: PairState {
                enabled: marge_homer,
                idle_ticks: 0,
                motion: None,
                walking: false,
            },
        }
    }

    pub(super) fn invalidate(&mut self, slots: &[Slot; 4]) {
        let [marge, homer, bart, lisa] = slots;
        self.marge_homer.invalidate(marge, homer);
        self.bart_lisa.invalidate(bart, lisa);
    }

    pub(super) const fn pose(
        self,
        pair: Pair,
        slots: &[Slot; 4],
    ) -> Option<PairPose> {
        let [marge, _, bart, _] = slots;
        match pair {
            Pair::MargeHomer => self.marge_homer.pose(marge),
            Pair::BartLisa => self.bart_lisa.pose(bart),
        }
    }

    pub(super) const fn reset(&mut self) {
        self.marge_homer.reset();
        self.bart_lisa.reset();
    }
}

fn bounded_step(
    step: i16,
    first: i16,
    second: i16,
    (min, max): (i16, i16),
) -> i16 {
    if step < 0 {
        step.max(min.saturating_sub(first.min(second)).min(0))
    } else {
        step.min(max.saturating_sub(first.max(second)).max(0))
    }
}

const fn near(first: Point, second: Point) -> bool {
    first.x.abs_diff(second.x) <= HORIZONTAL_PROXIMITY
        && first.y.abs_diff(second.y) <= DEPTH_PROXIMITY
}

fn ready(slot: &Slot) -> bool {
    slot.station.can_fight()
        && slot.station.maneuver == Maneuver::Idle
        && slot.action_recovery == Recovery::Ready
}

fn translate(
    first: &mut Slot,
    second: &mut Slot,
    (dx, dy): (i16, i16),
    layout: StreetLayout,
) {
    let step_x = bounded_step(
        dx,
        first.station.position.x,
        second.station.position.x,
        (layout.min.x, layout.max.x),
    );
    let step_y = bounded_step(
        dy,
        first.station.position.y,
        second.station.position.y,
        (layout.min.y, layout.max.y),
    );
    for slot in [first, second] {
        slot.station.position.x =
            slot.station.position.x.saturating_add(step_x);
        slot.station.position.y =
            slot.station.position.y.saturating_add(step_y);
        slot.station.walking = None;
        if dx != 0 {
            slot.station.facing = if dx < 0 {
                Facing::Left
            } else {
                Facing::Right
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use games::input::LogicalAction;
    use games::replay::InputStep as _;

    use super::*;
    use crate::{Action, Cabinet, Control, cabinet};

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

    fn join(state: &mut Cabinet, player: Player) {
        state.input_step(0, &[
            change(player, Control::Start, true),
            change(player, Control::Start, false),
        ]);
    }

    fn wait(state: &mut Cabinet, ticks: u64) {
        for tick in 0..ticks {
            state.input_step(tick, &[]);
        }
    }

    fn configured(layout: StreetLayout, enabled: [bool; 2]) -> Cabinet {
        let result = Cabinet::new(layout);
        assert!(result.is_ok());
        result
            .unwrap_or_else(|_error| cabinet())
            .with_cooperative_pairs(enabled)
    }

    fn joined() -> Cabinet {
        let mut state = configured(StreetLayout::DOWNTOWN, [true, true]);
        for player in Player::ALL {
            join(&mut state, player);
        }
        wait(&mut state, 38);
        state
    }

    #[test]
    fn admission_waits_for_neutral_ticks() {
        let mut state = configured(StreetLayout::DOWNTOWN, [true, false]);
        join(&mut state, Player::Marge);
        wait(&mut state, 40);
        assert_eq!(state.pair_pose(Pair::MargeHomer), None);
        join(&mut state, Player::Homer);
        wait(&mut state, 36);
        assert_eq!(state.pair_pose(Pair::MargeHomer), None);
        state.input_step(37, &[change(Player::Homer, Control::Right, true)]);
        state.input_step(38, &[change(Player::Homer, Control::Right, false)]);
        wait(&mut state, 36);
        assert_eq!(state.pair_pose(Pair::MargeHomer), None);
        wait(&mut state, 1);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::Idle { tick: 0 })
        );
        wait(&mut state, 40);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::Idle { tick: 13 })
        );
        join(&mut state, Player::Bart);
        join(&mut state, Player::Lisa);
        wait(&mut state, 40);
        assert_eq!(state.pair_pose(Pair::BartLisa), None);
    }

    #[test]
    fn only_nearby_members_join() {
        let layout = StreetLayout {
            spawn: [
                Point { x: 100, y: 190 },
                Point { x: 60, y: 190 },
                Point { x: 160, y: 148 },
                Point { x: 160, y: 180 },
            ],
            ..StreetLayout::DOWNTOWN
        };
        let mut state = configured(layout, [true, true]);
        for player in Player::ALL {
            join(&mut state, player);
        }
        wait(&mut state, 100);
        assert_eq!(state.pair_pose(Pair::MargeHomer), None);
        assert_eq!(state.pair_pose(Pair::BartLisa), None);
    }

    #[test]
    fn either_member_moves_and_first_member_resolves_conflicts() {
        let mut state = joined();
        let [marge, homer, bart, lisa] = state.stations();
        state.input_step(41, &[
            change(Player::Homer, Control::Right, true),
            change(Player::Lisa, Control::Down, true),
        ]);
        let moved = state.stations().map(|station| station.position);
        assert_eq!(moved, [
            Point {
                x: marge.position.x.saturating_add(2),
                ..marge.position
            },
            Point {
                x: homer.position.x.saturating_add(2),
                ..homer.position
            },
            Point {
                y: bart.position.y.saturating_add(2),
                ..bart.position
            },
            Point {
                y: lisa.position.y.saturating_add(2),
                ..lisa.position
            },
        ]);
        state.input_step(42, &[
            change(Player::Marge, Control::Up, true),
            change(Player::Bart, Control::Left, true),
        ]);
        let [moved_marge, moved_homer, moved_bart, moved_lisa] = moved;
        assert_eq!(state.stations().map(|station| station.position), [
            Point {
                y: moved_marge.y.saturating_sub(2),
                ..moved_marge
            },
            Point {
                y: moved_homer.y.saturating_sub(2),
                ..moved_homer
            },
            Point {
                x: moved_bart.x.saturating_sub(2),
                ..moved_bart
            },
            Point {
                x: moved_lisa.x.saturating_sub(2),
                ..moved_lisa
            },
        ]);
    }

    #[test]
    fn bounded_movement_preserves_pair_spacing_and_walk_cycle() {
        let mut state = joined();
        state.input_step(41, &[
            change(Player::Marge, Control::Left, true),
            change(Player::Bart, Control::Left, true),
        ]);
        wait(&mut state, 96);
        let [marge, homer, bart, lisa] = state.stations();
        assert_eq!(homer.position.x, StreetLayout::DOWNTOWN.min.x);
        assert_eq!(bart.position.x, StreetLayout::DOWNTOWN.min.x);
        assert_eq!(marge.position.x.abs_diff(homer.position.x), 22);
        assert_eq!(bart.position.x.abs_diff(lisa.position.x), 22);
        assert_eq!(
            Pair::ALL.map(|pair| state.pair_pose(pair).map(|pose| pose.motion)),
            [Some(PairMotion::Walk { tick: 0 }); 2]
        );
        wait(&mut state, 1);
        assert_eq!(state.stations(), [marge, homer, bart, lisa]);
        assert_eq!(
            Pair::ALL.map(|pair| state.pair_pose(pair).map(|pose| pose.motion)),
            [Some(PairMotion::Walk { tick: 1 }); 2]
        );
        state.input_step(139, &[
            change(Player::Marge, Control::Left, false),
            change(Player::Bart, Control::Left, false),
        ]);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::Walk { tick: 1 })
        );
        assert_eq!(
            state.pair_pose(Pair::BartLisa).map(|pose| pose.motion),
            Some(PairMotion::Idle { tick: 9 })
        );
    }

    #[test]
    fn solo_maneuver_and_damage_separate_pairs_independently() {
        let mut state = joined();
        state.input_step(41, &[change(Player::Homer, Control::Primary, true)]);
        assert_eq!(state.pair_pose(Pair::MargeHomer), None);
        assert!(state.pair_pose(Pair::BartLisa).is_some());
        let [_, homer, ..] = state.stations();
        assert!(matches!(homer.maneuver, Maneuver::Attack { tick: 0, .. }));
        let [_, _, _, lisa] = &mut state.slots;
        lisa.damage(3);
        state.input_step(42, &[]);
        assert_eq!(state.pair_pose(Pair::BartLisa), None);
    }
}
