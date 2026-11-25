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
    Cooperation, Facing, Maneuver, PairCommand, PairState, PairSupport,
    PendingManeuver, Player, Point, Recovery, Slot, StreetLayout,
};

const ADMISSION_TICKS: u8 = 38;
const HORIZONTAL_PROXIMITY: u16 = 32;
const DEPTH_PROXIMITY: u16 = 24;
// Device-observed elevation relative to the combined idle pose.
const BART_LISA_ELEVATIONS: [i16; 29] = [
    7, 14, 21, 28, 35, 40, 45, 50, 53, 56, 58, 59, 60, 59, 58, 57, 54, 51, 47,
    44, 40, 35, 31, 26, 21, 16, 10, 5, 0,
];
const MARGE_HOMER_ELEVATIONS: [i16; 29] = [
    10, 17, 28, 35, 38, 43, 52, 57, 55, 58, 65, 66, 60, 59, 61, 60, 55, 52, 53,
    50, 43, 38, 38, 33, 23, 18, 17, 12, 0,
];
/// Fixed duration of the observed paired secondary actions.
pub const BART_LISA_JUMP_TICKS: u8 = 29;

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
    /// Marge/Homer ground pose following a combined vertical action.
    AfterJump {
        /// Zero-based post-landing locomotion phase.
        tick: u8,
    },
    /// Opening pose, clamped at the final still pose.
    Idle {
        /// Zero-based phase in the opening sequence.
        tick: u8,
    },
    /// Combined vertical action, including its landing pose.
    Jump {
        /// Zero-based action phase.
        tick: u8,
    },
    /// One-frame grounded Bart/Lisa Primary response.
    Primary,
    /// Repeating locomotion pose, possibly held after stopping.
    Walk {
        /// Zero-based phase within the pair's walking cycle.
        tick: u8,
    },
}

impl PairMotion {
    /// Vertical offset from the provisional shared ground point.
    #[must_use]
    pub fn elevation(self, pair: Pair) -> i16 {
        match self {
            Self::Jump { tick } => match pair {
                Pair::BartLisa => &BART_LISA_ELEVATIONS,
                Pair::MargeHomer => &MARGE_HOMER_ELEVATIONS,
            }
            .get(usize::from(tick))
            .copied()
            .unwrap_or_default(),
            Self::AfterJump { .. }
            | Self::Idle { .. }
            | Self::Primary
            | Self::Walk { .. } => 0,
        }
    }
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
        if self.support == PairSupport::Disabled
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
        (pair, command): (Pair, PairCommand),
        (first, second): (&mut Slot, &mut Slot),
        layout: StreetLayout,
    ) {
        self.invalidate(first, second);
        if let Some(motion) = self.motion {
            let direction =
                self.direction(first, second, command == PairCommand::Jump);
            match command {
                PairCommand::Jump => {
                    self.motion = Some(PairMotion::Jump { tick: 0 });
                    self.walking = false;
                },
                PairCommand::Primary if direction == (0, 0) => {
                    self.motion = Some(PairMotion::Primary);
                    self.walking = false;
                },
                PairCommand::Primary | PairCommand::None => {
                    self.update_motion(pair, motion, direction);
                },
            }
            translate(first, second, direction, layout);
        } else {
            first.move_tick(layout);
            second.move_tick(layout);
            self.admit(first, second);
        }
    }

    const fn advance_jump(&mut self, pair: Pair, tick: u8) {
        let next = tick.saturating_add(1);
        self.motion = Some(if next >= BART_LISA_JUMP_TICKS {
            self.second_controls = true;
            match pair {
                Pair::MargeHomer => PairMotion::AfterJump { tick: 0 },
                Pair::BartLisa => PairMotion::Idle { tick: pair.idle_end() },
            }
        } else {
            PairMotion::Jump { tick: next }
        });
    }

    fn direction(self, first: &Slot, second: &Slot, jump: bool) -> (i16, i16) {
        let airborne =
            jump || matches!(self.motion, Some(PairMotion::Jump { .. }));
        if (self.second_controls && !airborne) || first.direction() == (0, 0) {
            second.direction()
        } else {
            first.direction()
        }
    }

    fn filter_actions(
        self,
        pair: Pair,
        pending: [&mut PendingManeuver; 2],
    ) -> PairCommand {
        if self.support != PairSupport::Jump || self.motion.is_none() {
            return PairCommand::None;
        }
        let actions = pending.each_ref().map(|action| action.candidate);
        let airborne = matches!(self.motion, Some(PairMotion::Jump { .. }));
        let jump = actions
            .into_iter()
            .any(|action| matches!(action, Some(Maneuver::Jump(_))));
        if !airborne && !jump && pair == Pair::MargeHomer {
            return PairCommand::None;
        }
        for action in pending {
            *action = PendingManeuver::default();
        }
        if airborne {
            PairCommand::None
        } else if jump {
            PairCommand::Jump
        } else if actions
            .into_iter()
            .any(|action| matches!(action, Some(Maneuver::Attack { .. })))
        {
            PairCommand::Primary
        } else {
            PairCommand::None
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
        self.second_controls = false;
        self.walking = false;
    }

    fn update_motion(
        &mut self,
        pair: Pair,
        motion: PairMotion,
        direction: (i16, i16),
    ) {
        if let PairMotion::Jump { tick } = motion {
            self.advance_jump(pair, tick);
            return;
        }
        if direction == (0, 0) {
            self.motion = Some(match motion {
                PairMotion::Idle { tick } => PairMotion::Idle {
                    tick: tick.saturating_add(1).min(pair.idle_end()),
                },
                PairMotion::Walk { .. } | PairMotion::Primary
                    if pair == Pair::BartLisa =>
                {
                    PairMotion::Idle { tick: pair.idle_end() }
                },
                PairMotion::AfterJump { .. }
                | PairMotion::Walk { .. }
                | PairMotion::Jump { .. }
                | PairMotion::Primary => motion,
            });
            self.walking = false;
        } else if pair == Pair::MargeHomer && self.second_controls {
            let tick = match motion {
                PairMotion::AfterJump { tick } if self.walking => {
                    tick.saturating_add(1) & 15
                },
                PairMotion::AfterJump { tick } => {
                    ((tick & !1).saturating_add(2)) & 15
                },
                PairMotion::Idle { .. }
                | PairMotion::Jump { .. }
                | PairMotion::Primary
                | PairMotion::Walk { .. } => 2,
            };
            self.motion = Some(PairMotion::AfterJump { tick });
            self.walking = true;
        } else {
            let tick = match motion {
                PairMotion::Walk { tick } if self.walking => tick
                    .saturating_add(1)
                    .checked_rem(pair.walk_duration())
                    .unwrap_or_default(),
                PairMotion::AfterJump { .. }
                | PairMotion::Idle { .. }
                | PairMotion::Jump { .. }
                | PairMotion::Primary
                | PairMotion::Walk { .. } => 0,
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
        pair_commands: [PairCommand; 2],
    ) {
        let [marge, homer, bart, lisa] = slots;
        let [marge_homer, bart_lisa] = pair_commands;
        self.marge_homer.advance(
            (Pair::MargeHomer, marge_homer),
            (marge, homer),
            layout,
        );
        self.bart_lisa.advance(
            (Pair::BartLisa, bart_lisa),
            (bart, lisa),
            layout,
        );
    }

    pub(super) const fn configure(enabled: [bool; 2]) -> Self {
        let [marge_homer, bart_lisa] = enabled;
        Self {
            bart_lisa: PairState {
                idle_ticks: 0,
                motion: None,
                second_controls: false,
                support: if bart_lisa {
                    PairSupport::Locomotion
                } else {
                    PairSupport::Disabled
                },
                walking: false,
            },
            marge_homer: PairState {
                idle_ticks: 0,
                motion: None,
                second_controls: false,
                support: if marge_homer {
                    PairSupport::Locomotion
                } else {
                    PairSupport::Disabled
                },
                walking: false,
            },
        }
    }

    pub(super) fn filter_actions(
        &self,
        pending: &mut [PendingManeuver; 4],
    ) -> [PairCommand; 2] {
        let [marge, homer, bart, lisa] = pending;
        [
            self.marge_homer
                .filter_actions(Pair::MargeHomer, [marge, homer]),
            self.bart_lisa.filter_actions(Pair::BartLisa, [bart, lisa]),
        ]
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
    fn combined_jump_has_observed_phase_and_landing() {
        let mut state = joined().with_bart_lisa_jump(true);
        let [_, _, bart, lisa] = state.stations();
        state.input_step(41, &[
            change(Player::Bart, Control::Secondary, true),
            change(Player::Bart, Control::Secondary, false),
        ]);
        let phases = [7, 60, 0];
        for (tick, elevation) in [(0, 0), (12, 1), (28, 2)] {
            let advance = match tick {
                0 => 0,
                12 => 12,
                _ => 16,
            };
            wait(&mut state, advance);
            assert_eq!(
                state.pair_pose(Pair::BartLisa).map(|pose| pose.motion),
                Some(PairMotion::Jump { tick })
            );
            assert_eq!(
                state
                    .pair_pose(Pair::BartLisa)
                    .map(|pose| pose.motion.elevation(Pair::BartLisa)),
                phases.get(elevation).copied()
            );
        }
        wait(&mut state, 1);
        assert_eq!(
            state.pair_pose(Pair::BartLisa).map(|pose| pose.motion),
            Some(PairMotion::Idle { tick: 9 })
        );
        let [_, _, landed_bart, landed_lisa] = state.stations();
        assert_eq!([landed_bart, landed_lisa], [bart, lisa]);
    }

    #[test]
    fn combined_jump_discards_pair_button_edges_without_buffering() {
        for player in Pair::BartLisa.members() {
            for offset in [0, 10, 27, 28] {
                check_airborne_buttons(player, offset);
            }
        }
    }

    fn check_airborne_buttons(player: Player, offset: u64) {
        let mut baseline = joined().with_bart_lisa_jump(true);
        baseline.input_step(41, &[
            change(Player::Lisa, Control::Secondary, true),
            change(Player::Lisa, Control::Secondary, false),
        ]);
        wait(&mut baseline, offset);
        let mut state = baseline;
        let mut chord = baseline;
        let other = || change(Player::Marge, Control::Primary, true);
        baseline.input_step(42, &[other()]);
        state.input_step(42, &[
            other(),
            change(player, Control::Primary, true),
            change(player, Control::Primary, false),
        ]);
        chord.input_step(42, &[
            other(),
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
            change(player, Control::Primary, false),
            change(player, Control::Secondary, false),
        ]);
        assert_eq!(state, baseline);
        assert_eq!(chord, baseline);
        wait(&mut baseline, 40);
        wait(&mut state, 40);
        wait(&mut chord, 40);
        assert_eq!(state, baseline);
        assert_eq!(chord, baseline);
        assert!(state.pair_pose(Pair::BartLisa).is_some());
    }

    #[test]
    fn marge_homer_combined_jump_retains_pair_and_transfers_control() {
        for player in Pair::MargeHomer.members() {
            check_marge_homer_jump(player);
        }
    }

    fn check_marge_homer_jump(player: Player) {
        let mut state = joined().with_pair_jump(Pair::MargeHomer, true);
        let stations = state.stations();
        state.input_step(41, &[
            change(player, Control::Secondary, true),
            change(player, Control::Secondary, false),
        ]);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::Jump { tick: 0 })
        );
        wait(&mut state, 11);
        assert_eq!(
            state
                .pair_pose(Pair::MargeHomer)
                .map(|pose| pose.motion.elevation(Pair::MargeHomer)),
            Some(66)
        );
        let mut control = state;
        state.input_step(53, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Marge, Control::Primary, false),
            change(Player::Homer, Control::Secondary, true),
            change(Player::Homer, Control::Secondary, false),
        ]);
        control.input_step(53, &[]);
        assert_eq!(state, control);
        wait(&mut state, 17);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::AfterJump { tick: 0 })
        );
        assert_eq!(state.stations(), stations);
        check_marge_homer_landing(state);
    }

    fn check_marge_homer_landing(mut state: Cabinet) {
        let stations = state.stations();
        state.input_step(71, &[change(Player::Marge, Control::Left, true)]);
        assert_eq!(state.stations(), stations);
        state.input_step(72, &[change(Player::Homer, Control::Right, true)]);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::AfterJump { tick: 2 })
        );
        assert_ne!(state.stations(), stations);
        wait(&mut state, 1);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::AfterJump { tick: 3 })
        );
        state.input_step(74, &[change(Player::Homer, Control::Right, false)]);
        let stopped = state.stations();
        wait(&mut state, 20);
        assert_eq!(state.stations(), stopped);
        assert_eq!(
            state.pair_pose(Pair::MargeHomer).map(|pose| pose.motion),
            Some(PairMotion::AfterJump { tick: 3 })
        );
    }

    #[test]
    fn combined_primary_preserves_stationary_pair_for_both_members() {
        for player in Pair::BartLisa.members() {
            check_grounded_primary(player);
        }
    }

    fn check_grounded_primary(player: Player) {
        let mut state = joined().with_bart_lisa_jump(true);
        wait(&mut state, 12);
        let stations = state.stations();
        let idle = state.pair_pose(Pair::BartLisa);
        state.input_step(52, &[change(player, Control::Primary, true)]);
        assert_eq!(
            state.pair_pose(Pair::BartLisa).map(|pose| pose.motion),
            Some(PairMotion::Primary)
        );
        assert_eq!(state.stations(), stations);
        wait(&mut state, 1);
        assert_eq!(state.pair_pose(Pair::BartLisa), idle);
        wait(&mut state, 30);
        assert_eq!(state.pair_pose(Pair::BartLisa), idle);
        assert_eq!(state.stations(), stations);
        state.input_step(84, &[
            change(player, Control::Primary, false),
            change(player, Control::Primary, true),
        ]);
        assert_eq!(
            state.pair_pose(Pair::BartLisa).map(|pose| pose.motion),
            Some(PairMotion::Primary)
        );
    }

    #[test]
    fn combined_primary_does_not_interrupt_walking() {
        let mut baseline = joined().with_bart_lisa_jump(true);
        baseline.input_step(41, &[change(Player::Lisa, Control::Right, true)]);
        wait(&mut baseline, 8);
        for player in Pair::BartLisa.members() {
            let mut state = baseline;
            let mut control = baseline;
            state.input_step(50, &[
                change(player, Control::Primary, true),
                change(player, Control::Primary, false),
            ]);
            control.input_step(50, &[]);
            assert_eq!(state, control);
            wait(&mut state, 24);
            wait(&mut control, 24);
            assert_eq!(state, control);
        }
    }

    #[test]
    fn combined_chords_match_secondary_with_or_without_movement() {
        for player in Pair::BartLisa.members() {
            check_combined_chord(player, false);
            check_combined_chord(player, true);
        }
    }

    fn check_combined_chord(player: Player, moving: bool) {
        let mut baseline = joined().with_bart_lisa_jump(true);
        if moving {
            baseline.input_step(41, &[change(
                Player::Lisa,
                Control::Right,
                true,
            )]);
        }
        let mut chord = baseline;
        baseline.input_step(42, &[
            change(player, Control::Secondary, true),
            change(player, Control::Secondary, false),
        ]);
        chord.input_step(42, &[
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
            change(player, Control::Primary, false),
            change(player, Control::Secondary, false),
        ]);
        assert_eq!(chord, baseline);
        wait(&mut chord, 30);
        wait(&mut baseline, 30);
        assert_eq!(chord, baseline);
    }

    #[test]
    fn combined_jump_changes_grounded_direction_priority() {
        let mut state = joined().with_bart_lisa_jump(true);
        let [_, _, before, _] = state.stations();
        state.input_step(41, &[
            change(Player::Lisa, Control::Secondary, true),
            change(Player::Lisa, Control::Secondary, false),
            change(Player::Bart, Control::Right, true),
        ]);
        let [_, _, moving, _] = state.stations();
        assert_eq!(moving.position.x, before.position.x.saturating_add(2));
        wait(&mut state, 29);
        let [_, _, landed, _] = state.stations();
        wait(&mut state, 3);
        let [_, _, still, _] = state.stations();
        assert_eq!(landed.position, still.position);
        state.input_step(75, &[change(Player::Lisa, Control::Left, true)]);
        let [_, _, moved, _] = state.stations();
        assert_eq!(moved.position.x, still.position.x.saturating_sub(2));
    }

    #[test]
    fn legacy_pair_catalogue_keeps_solo_jump() {
        let mut state = joined();
        state.input_step(41, &[
            change(Player::Lisa, Control::Secondary, true),
            change(Player::Lisa, Control::Secondary, false),
        ]);
        assert_eq!(state.pair_pose(Pair::BartLisa), None);
        let [_, _, _, lisa] = state.stations();
        assert!(lisa.maneuver.airborne());
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
