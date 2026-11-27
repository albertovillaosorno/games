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
//   - Fixed-step third-stage entrance, body timing and bounded room actions.
// - Must-Not:
//   - Decode original media, choose a random seed or render sprites.
// - Allows:
//   - Inputs: Admitted stations, fall order and logical station inputs.
//   - Outputs: Scene phase, body pose, positions and shared depth offset.
//   - Side effects: Deterministic in-memory progression only.
// - Split-When:
//   - Cemetery gameplay needs a distinct encounter policy.
// - Merge-When:
//   - A shared scene transition subsumes the title's entrance rules.
// - Summary:
//   - Reproduce the observed ranked entrance without assuming RNG cadence.
// - Description:
//   - Four-station motion and isolated actions are verified in a bounded
//     window.
// - Usage:
//   - Advance only on unpaused playing ticks.
// - Defaults:
//   - No entrance until explicitly selected.

//! Observed third-stage arrival with an explicit fall order.

use games::animation::cycle_tick;

use super::{
    Maneuver, PendingManeuver, Player, Point, Slot, StreetLayout,
    stage_card_departed,
};

const FIRST_BACKGROUND: u64 = 13;
const FIRST_STATION: u64 = 107;
const PLAYABLE_TICK: u64 = 237;
const FIRST_WALK_TICK: u64 = 242;
const LAST_WALK_TICK: u64 = 357;
const ROOM_LAYOUT: StreetLayout = StreetLayout {
    max: Point { x: 320, y: 176 },
    min: Point { x: 112, y: 122 },
    spawn: [
        Point { x: 116, y: 176 },
        Point { x: 144, y: 176 },
        Point { x: 172, y: 176 },
        Point { x: 200, y: 176 },
    ],
};
const DESCENT_TICKS: u64 = 130;
const FALL_RANK_DELAY: i32 = 16;
const UNITS_PER_PIXEL: u32 = 256;
const LANDING_POSE_COUNT: usize = CemeteryEntrance::LANDING_POSE_COUNT;

const MARGE_LANDING: [u8; LANDING_POSE_COUNT] = [0, 4, 9, 15, 23, 31, 38, 44];
const HOMER_LANDING: [u8; LANDING_POSE_COUNT] = [0, 5, 11, 17, 23, 29, 36, 44];
const BART_LANDING: [u8; LANDING_POSE_COUNT] = [0, 5, 10, 16, 23, 30, 37, 44];
const LISA_LANDING: [u8; LANDING_POSE_COUNT] = [0, 5, 10, 15, 23, 32, 39, 45];

/// Entrance progress, excluding any unimplemented subsequent encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CemeteryPhase {
    /// Await a fresh action edge or title-card timeout.
    Card,
    /// Stations are present and falling or waiting on the ground.
    Descent,
    /// All four rank trajectories have reached the input boundary.
    Landed,
    /// The station records are not yet initialized.
    Setup,
}

/// Six independently verified sparse airborne body families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CemeteryPose {
    /// Bart's airborne body.
    Bart,
    /// Homer's airborne body.
    Homer,
    /// Lisa's twelve-tick family.
    LisaA,
    /// Lisa's eight-tick family.
    LisaB,
    /// Marge's alternate six-tick family.
    MargeA,
    /// Marge's initial six-tick family.
    MargeB,
}

/// Airborne body sample after the observed presentation delay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryBody {
    /// Height above the ground, floored to logical pixels.
    pub elevation: i16,
    /// Selected sparse body family.
    pub pose: CemeteryPose,
    /// Ground position in logical coordinates.
    pub position: Point,
}

/// Bounded first-room actions and shared vertical projection.
///
/// World depth increases away from the viewer. The projected depth adds
/// the shared camera offset, so neutral stations retain their world
/// positions during a pan. This does not model subsequent room combat,
/// terrain sampling or the original camera descriptor scheduler.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CemeteryRoom {
    depth_offset: i16,
    walk_depth: [i16; 4],
}

impl CemeteryRoom {
    fn advance(
        &mut self,
        slots: &mut [Slot; 4],
        pending: [PendingManeuver; 4],
    ) -> bool {
        // Only the four-station, pre-encounter interval is established.
        if !slots.iter().all(|slot| slot.station.can_fight()) {
            return false;
        }
        for ((player, slot), queued) in
            Player::ALL.into_iter().zip(&mut *slots).zip(pending)
        {
            slot.advance_maneuver(player, queued);
        }
        let projected = slots.map(|slot| self.project(slot.station.position));
        let motion = slots.map(|slot| {
            let (dx, dy) = slot.direction();
            (dx, dy.saturating_neg())
        });
        let toward = projected.iter().all(|point| point.y < 320);
        let away = projected.iter().all(|point| point.y > 168);
        let depth_motion = slots
            .map(|slot| slot.depth_step(slot.direction().1.saturating_neg()));
        let request = projected.into_iter().zip(depth_motion).fold(
            0i16,
            |total, (point, dy)| {
                if (dy < 0 && point.y < 168 && toward)
                    || (dy > 0 && point.y > 192 && away)
                {
                    total.saturating_sub(dy)
                } else {
                    total
                }
            },
        );
        // A party of four can request a pan, not four times its speed.
        self.depth_offset = self
            .depth_offset
            .saturating_add(request.clamp(-2, 2))
            .clamp(0, 31);
        for ((slot, direction), previous) in
            slots.iter_mut().zip(motion).zip(&mut self.walk_depth)
        {
            slot.move_with_direction(ROOM_LAYOUT, direction);
            if matches!(slot.station.maneuver, Maneuver::Attack { tick: 0, .. })
            {
                // The admission tick retains the preceding depth stride,
                // without issuing another camera request.
                slot.station.position.y = slot
                    .station
                    .position
                    .y
                    .saturating_add(*previous)
                    .clamp(ROOM_LAYOUT.min.y, ROOM_LAYOUT.max.y);
            }
            *previous = if slot.station.walking.is_some() {
                direction.1
            } else {
                0
            };
        }
        true
    }

    /// Current shared depth offset, in logical units.
    #[must_use]
    pub const fn depth_offset(self) -> i16 {
        self.depth_offset
    }

    /// Project a room world position into camera-relative record space.
    ///
    /// This is not the final sprite registration or display transform.
    #[must_use]
    pub const fn project(self, point: Point) -> Point {
        Point {
            y: point.y.saturating_add(self.depth_offset),
            ..point
        }
    }
}

/// Deterministic cemetery arrival clock with explicitly selected fall order.
///
/// Selecting the first rank is deliberately separate from the unverified
/// original recurrence invocation cadence. The four-station rank rotation
/// and motion are independent of the rendering rate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryEntrance {
    admitted: [bool; 4],
    card: bool,
    first: Player,
    room: Option<CemeteryRoom>,
    tick: u64,
}

impl CemeteryEntrance {
    /// Number of observed grounded presentation families per character.
    pub const LANDING_POSE_COUNT: usize = 8;

    pub(super) fn accepts_room_actions(self) -> bool {
        self.at(self.tick.saturating_add(1)).room_walkable()
    }

    /// Advance once with a fresh button edge from an admitted station.
    pub fn advance(&mut self, active_edge: bool) {
        self.tick = self.tick.saturating_add(1);
        if self.card
            && stage_card_departed(
                u16::try_from(self.tick).unwrap_or(u16::MAX),
                active_edge,
            )
        {
            self.card = false;
            self.tick = 0;
        }
    }

    pub(super) fn advance_room(
        &mut self,
        slots: &mut [Slot; 4],
        pending: [PendingManeuver; 4],
    ) -> bool {
        let walking = self.room_walkable();
        let Some(room) = &mut self.room else {
            return false;
        };
        if walking {
            return room.advance(slots, pending);
        }
        if !self.card && self.tick == LAST_WALK_TICK.saturating_add(1) {
            for slot in slots {
                slot.station.walking = None;
            }
        }
        false
    }

    const fn at(self, tick: u64) -> Self {
        Self { tick, ..self }
    }

    /// Visible cemetery background family, independent of station motion.
    #[must_use]
    pub fn background_phase(self) -> Option<u8> {
        if self.card || self.tick < FIRST_BACKGROUND {
            return None;
        }
        let elapsed = self.tick.saturating_sub(FIRST_BACKGROUND);
        if elapsed < 5 {
            return Some(3);
        }
        let cycle = cycle_tick(elapsed.saturating_sub(5), 56u8)?;
        Some(match cycle {
            0..=6 | 35..=41 => 2,
            7..=13 | 28..=34 => 1,
            14..=27 => 0,
            _ => 3,
        })
    }

    /// Delayed airborne sample for the selected station.
    ///
    /// Original display composition consumes the preceding object sample.
    /// Landing variants are not included in the sparse airborne selection.
    #[must_use]
    pub fn body(self, player: Player) -> Option<CemeteryBody> {
        if self.card || self.tick < FIRST_STATION + 2 {
            return None;
        }
        let source = self.at(self.tick.saturating_sub(1));
        if source.rank_elapsed(player)? > 80i32 {
            return None;
        }
        let pose_tick = self.tick.saturating_sub(FIRST_STATION + 2);
        let pose = match player {
            Player::Bart => CemeteryPose::Bart,
            Player::Homer => CemeteryPose::Homer,
            Player::Lisa => {
                if cycle_tick(pose_tick, 20u8)? < 8 {
                    CemeteryPose::LisaB
                } else {
                    CemeteryPose::LisaA
                }
            },
            Player::Marge => {
                if cycle_tick(pose_tick, 12u8)? < 6 {
                    CemeteryPose::MargeB
                } else {
                    CemeteryPose::MargeA
                }
            },
        };
        Some(CemeteryBody {
            elevation: i16::try_from(
                source.height(player)?.checked_div(UNITS_PER_PIXEL)?,
            )
            .ok()?,
            pose,
            position: source.position(player)?,
        })
    }

    pub(super) fn enable_room(&mut self) {
        if self.admitted.into_iter().all(|admitted| admitted) {
            self.room = Some(CemeteryRoom::default());
        }
    }

    /// Current elevation in 1/256 logical pixels.
    ///
    /// An absent or not-yet-initialized station has no elevation.
    #[must_use]
    pub fn height(self, player: Player) -> Option<u32> {
        let elapsed = self.rank_elapsed(player)?;
        let height = match elapsed {
            ..=11 => 58112i32.saturating_sub(2048i32.saturating_mul(elapsed)),
            12..=16 => 34816i32.saturating_sub(
                1536i32.saturating_mul(elapsed.saturating_sub(12)),
            ),
            17..=20 => 28672i32.saturating_sub(
                640i32.saturating_mul(elapsed.saturating_sub(16)),
            ),
            21..=57 => 26112i32,
            58..=79 => 26112i32.saturating_sub(
                48i32
                    .saturating_mul(elapsed.saturating_sub(57))
                    .saturating_mul(elapsed.saturating_sub(56)),
            ),
            _ => 0i32,
        };
        u32::try_from(height).ok()
    }

    /// Grounded presentation family after the preceding airborne sample.
    ///
    /// The preview holds the final entrance sample at the input boundary;
    /// later gameplay has not been implemented.
    #[must_use]
    pub fn landing_pose(self, player: Player) -> Option<usize> {
        let source = self.at(self.tick.min(PLAYABLE_TICK).saturating_sub(1));
        let elapsed =
            u8::try_from(source.rank_elapsed(player)?.saturating_sub(81))
                .ok()?;
        let track = match player {
            Player::Marge => MARGE_LANDING,
            Player::Homer => HOMER_LANDING,
            Player::Bart => BART_LANDING,
            Player::Lisa => LISA_LANDING,
        };
        track
            .partition_point(|tick| *tick <= elapsed)
            .checked_sub(1)
    }

    /// Begin the title card with a selected first-landing station.
    ///
    /// The admitted mask omits absent stations from the preview without
    /// changing the four observed rank trajectories.
    #[must_use]
    pub const fn new(admitted: [bool; 4], first: Player) -> Self {
        Self {
            admitted,
            card: true,
            first,
            room: None,
            tick: 0,
        }
    }

    /// The current scene phase, independent of the presentation rate.
    #[must_use]
    pub const fn phase(self) -> CemeteryPhase {
        if self.card {
            CemeteryPhase::Card
        } else if self.tick < FIRST_STATION {
            CemeteryPhase::Setup
        } else if self.tick < PLAYABLE_TICK {
            CemeteryPhase::Descent
        } else {
            CemeteryPhase::Landed
        }
    }

    /// Initialized foot position, if this station is admitted.
    #[must_use]
    pub fn position(self, player: Player) -> Option<Point> {
        if self.card
            || self.tick < FIRST_STATION
            || !self.admitted.get(player.index()).copied()?
        {
            return None;
        }
        ROOM_LAYOUT.spawn.get(player.index()).copied()
    }

    /// Rank among the four observed trajectories, independent of admission.
    #[must_use]
    pub const fn rank(self, player: Player) -> u8 {
        base_rank(player)
            .saturating_add(4)
            .saturating_sub(base_rank(self.first))
            & 3
    }

    fn rank_elapsed(self, player: Player) -> Option<i32> {
        let _position = self.position(player)?;
        let elapsed = i32::try_from(
            self.tick.saturating_sub(FIRST_STATION).min(DESCENT_TICKS),
        )
        .ok()?;
        Some(elapsed.saturating_sub(
            i32::from(self.rank(player)).saturating_mul(FALL_RANK_DELAY),
        ))
    }

    /// Opted-in first-room projection after the ranked landing.
    ///
    /// The final offset remains inspectable after the verified motion
    /// interval ends. The ordinary rendered entrance has no room state.
    #[must_use]
    pub const fn room(self) -> Option<CemeteryRoom> {
        if matches!(self.phase(), CemeteryPhase::Landed) {
            self.room
        } else {
            None
        }
    }

    /// Whether the bounded room accepts motion on the current tick.
    #[must_use]
    pub fn room_walkable(self) -> bool {
        !self.card
            && self.room.is_some()
            && (FIRST_WALK_TICK..=LAST_WALK_TICK).contains(&self.tick)
    }

    /// Elapsed card ticks, or ticks since title departure.
    #[must_use]
    pub const fn tick(self) -> u64 {
        self.tick
    }
}

const fn base_rank(player: Player) -> u8 {
    match player {
        Player::Marge => 0,
        Player::Homer => 3,
        Player::Bart => 1,
        Player::Lisa => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::super::Control;
    use super::*;

    fn arrival(first: Player) -> CemeteryEntrance {
        let mut state = CemeteryEntrance::new([true; 4], first);
        for _ in 0u64..192 {
            state.advance(false);
        }
        assert_eq!(state.phase(), CemeteryPhase::Setup);
        assert_eq!(state.tick(), 0);
        state
    }

    fn room_slots() -> [Slot; 4] {
        let mut slots = ROOM_LAYOUT.spawn.map(Slot::new);
        for slot in &mut slots {
            slot.start(0, false);
        }
        slots
    }

    fn hold(slots: &mut [Slot; 4], controls: [Option<Control>; 4]) {
        for (slot, control) in slots.iter_mut().zip(controls) {
            slot.held = control.map_or(0, Control::mask);
        }
    }

    fn room_positions(room: CemeteryRoom, slots: &[Slot; 4]) -> [Point; 4] {
        slots.map(|slot| room.project(slot.station.position))
    }

    // Independent piecewise description of the recorded projected samples.
    fn downward_sample(tick: i16) -> (i16, i16) {
        let offset = tick.saturating_sub(5).max(0).saturating_mul(2).min(31);
        let depth = match tick {
            0..=5 => 176i16.saturating_sub(tick.saturating_mul(2)),
            6..=20 => 166,
            21..=26 => {
                165i16.saturating_sub(tick.saturating_sub(21).saturating_mul(2))
            },
            _ => 153,
        };
        (offset, depth)
    }

    #[test]
    fn room_opposed_cardinal_trace_matches_observed_positions() {
        let mut room = CemeteryRoom::default();
        let mut slots = room_slots();
        hold(&mut slots, [
            Some(Control::Up),
            Some(Control::Down),
            Some(Control::Left),
            Some(Control::Right),
        ]);
        for tick in 1i16..=116 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = downward_sample(tick);
            let idle = 176i16.saturating_add(offset);
            let expected = [
                Point { x: 116, y: idle },
                Point { x: 144, y: depth },
                Point {
                    x: 172i16.saturating_sub(tick.saturating_mul(2)).max(112),
                    y: idle,
                },
                Point {
                    x: 200i16.saturating_add(tick.saturating_mul(2)).min(320),
                    y: idle,
                },
            ];
            assert_eq!(room.depth_offset(), offset, "tick {tick}");
            assert_eq!(room_positions(room, &slots), expected, "tick {tick}");
        }
    }

    #[test]
    fn room_diagonal_reversal_retains_both_cardinal_components() {
        let mut room = CemeteryRoom::default();
        let mut slots = room_slots();
        let [marge, _, bart, _] = &mut slots;
        marge.held = Control::Down.mask() | Control::Right.mask();
        bart.held = Control::Down.mask() | Control::Left.mask();
        for tick in 1i16..=40 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = downward_sample(tick);
            let idle = 176i16.saturating_add(offset);
            assert_eq!(room.depth_offset(), offset);
            assert_eq!(room_positions(room, &slots), [
                Point {
                    x: 116i16.saturating_add(tick.saturating_mul(2)),
                    y: depth,
                },
                Point { x: 144, y: idle },
                Point {
                    x: 172i16.saturating_sub(tick.saturating_mul(2)).max(112),
                    y: depth,
                },
                Point { x: 200, y: idle },
            ]);
        }
        assert_diagonal_return(room, slots);
    }

    fn upward_sample(tick: i16) -> (i16, i16) {
        let offset = 31i16
            .saturating_sub(tick.saturating_sub(20).max(0).saturating_mul(2))
            .max(15);
        let depth = if tick <= 20 {
            153i16.saturating_add(tick.saturating_mul(2))
        } else if tick < 28 {
            193
        } else {
            191
        };
        (offset, depth)
    }

    fn assert_diagonal_return(mut room: CemeteryRoom, mut slots: [Slot; 4]) {
        hold(&mut slots, [None; 4]);
        for _ in 0u8..8 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
        }
        let [marge, _, bart, _] = &mut slots;
        marge.held = Control::Up.mask() | Control::Left.mask();
        bart.held = Control::Up.mask() | Control::Right.mask();
        for tick in 1i16..=40 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = upward_sample(tick);
            assert_eq!(room.depth_offset(), offset);
            assert_eq!(room_positions(room, &slots), [
                Point {
                    x: 196i16.saturating_sub(tick.saturating_mul(2)),
                    y: depth,
                },
                Point {
                    x: 144,
                    y: 176i16.saturating_add(offset)
                },
                Point {
                    x: 112i16.saturating_add(tick.saturating_mul(2)),
                    y: depth,
                },
                Point {
                    x: 200,
                    y: 176i16.saturating_add(offset)
                },
            ]);
        }
    }

    fn assert_reversal(all: bool) {
        let mut room = CemeteryRoom::default();
        let mut slots = room_slots();
        let controls = |control| {
            if all {
                [Some(control); 4]
            } else {
                [Some(control), None, None, None]
            }
        };
        hold(&mut slots, controls(Control::Down));
        for tick in 1i16..=27 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = downward_sample(tick);
            assert_eq!(room.depth_offset(), offset);
            assert_depths(room, &slots, all, depth);
        }
        hold(&mut slots, [None; 4]);
        for _ in 0u8..21 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            assert_eq!(room.depth_offset(), 31);
            assert_depths(room, &slots, all, 153);
        }
        hold(&mut slots, controls(Control::Up));
        for tick in 1i16..=38 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = upward_sample(tick);
            assert_eq!(room.depth_offset(), offset, "tick {tick}");
            assert_depths(room, &slots, all, depth);
        }
    }

    fn assert_depths(
        room: CemeteryRoom,
        slots: &[Slot; 4],
        all: bool,
        depth: i16,
    ) {
        let idle = 176i16.saturating_add(room.depth_offset());
        let expected = if all {
            [depth; 4]
        } else {
            [depth, idle, idle, idle]
        };
        assert_eq!(room_positions(room, slots).map(|point| point.y), expected);
        if !all {
            let [_, homer, bart, lisa] = slots;
            for slot in [homer, bart, lisa] {
                assert_eq!(slot.station.position.y, 176);
            }
        }
    }

    #[test]
    fn room_reversal_keeps_idle_stations_fixed_in_world_space() {
        assert_reversal(false);
    }

    #[test]
    fn room_party_camera_does_not_multiply_scroll_speed() {
        assert_reversal(true);
    }

    #[test]
    fn title_requires_a_fresh_admitted_edge_or_timeout() {
        let mut state = CemeteryEntrance::new([true; 4], Player::Marge);
        for tick in 1..=64 {
            state.advance(true);
            assert_eq!(state.tick(), tick);
            assert_eq!(state.phase(), CemeteryPhase::Card);
        }
        let mut timeout = state;
        state.advance(true);
        assert_eq!(state.phase(), CemeteryPhase::Setup);
        assert_eq!(state.tick(), 0);
        for _ in 65u64..192 {
            timeout.advance(false);
            assert_eq!(timeout.phase(), CemeteryPhase::Card);
        }
        timeout.advance(false);
        assert_eq!(timeout, state);
    }

    #[test]
    fn four_rank_rotations_preserve_positions_and_offsets() {
        let expected = [[0, 3, 1, 2], [1, 0, 2, 3], [3, 2, 0, 1], [2, 1, 3, 0]];
        for (first, ranks) in Player::ALL.into_iter().zip(expected) {
            let initial = arrival(first);
            assert_eq!(Player::ALL.map(|player| initial.rank(player)), ranks);
            let state = initial.at(FIRST_STATION);
            assert_eq!(
                Player::ALL.map(|player| state.position(player)),
                [116, 144, 172, 200].map(|x| Some(Point { x, y: 176 })),
            );
            for (player, rank) in Player::ALL.into_iter().zip(ranks) {
                assert_eq!(
                    state.height(player),
                    Some(58112u32.saturating_add(
                        32768u32.saturating_mul(u32::from(rank))
                    )),
                );
                let ground = FIRST_STATION
                    .saturating_add(80)
                    .saturating_add(16u64.saturating_mul(u64::from(rank)));
                assert!(
                    state.at(ground.saturating_sub(1)).height(player) > Some(0)
                );
                assert_eq!(state.at(ground).height(player), Some(0));
            }
        }
    }

    #[test]
    fn height_boundaries_follow_the_observed_fixed_point_descent() {
        let state = arrival(Player::Marge);
        let samples = [
            (0, 58112),
            (11, 35584),
            (12, 34816),
            (16, 28672),
            (20, 26112),
            (57, 26112),
            (58, 26016),
            (79, 1824),
            (80, 0),
        ];
        for (elapsed, height) in samples {
            assert_eq!(
                state
                    .at(FIRST_STATION.saturating_add(elapsed))
                    .height(Player::Marge),
                Some(height),
            );
        }
        assert_eq!(state.at(236).phase(), CemeteryPhase::Descent);
        assert_eq!(state.at(237).phase(), CemeteryPhase::Landed);
        assert_eq!(state.at(237).height(Player::Homer), Some(0));
    }

    #[test]
    fn background_repeats_after_the_initial_short_hold() {
        let state = arrival(Player::Marge);
        assert_eq!(state.at(12).background_phase(), None);
        for tick in 13..18 {
            assert_eq!(state.at(tick).background_phase(), Some(3));
        }
        for (offset, phase) in [2, 1, 0, 0, 1, 2, 3, 3].into_iter().enumerate()
        {
            let start = 18u64.saturating_add(
                7u64.saturating_mul(u64::try_from(offset).unwrap_or_default()),
            );
            for tick in start..start.saturating_add(7) {
                assert_eq!(state.at(tick).background_phase(), Some(phase));
                assert_eq!(
                    state.at(tick.saturating_add(56)).background_phase(),
                    Some(phase),
                );
            }
        }
    }

    #[test]
    fn sparse_body_uses_delayed_sample_and_character_cadence() {
        let state = arrival(Player::Homer);
        assert_eq!(state.at(108).body(Player::Marge), None);
        let sample = |tick, player| state.at(tick).body(player);
        for (tick, player, expected) in [
            (109, Player::Marge, CemeteryPose::MargeB),
            (114, Player::Marge, CemeteryPose::MargeB),
            (115, Player::Marge, CemeteryPose::MargeA),
            (121, Player::Marge, CemeteryPose::MargeB),
            (116, Player::Lisa, CemeteryPose::LisaB),
            (117, Player::Lisa, CemeteryPose::LisaA),
            (129, Player::Lisa, CemeteryPose::LisaB),
        ] {
            assert_eq!(
                sample(tick, player).map(|body| body.pose),
                Some(expected)
            );
        }
        assert_eq!(
            sample(109, Player::Homer).map(|body| body.elevation),
            Some(219)
        );
        assert_eq!(
            sample(188, Player::Homer).map(|body| body.elevation),
            Some(0)
        );
        assert_eq!(sample(189, Player::Homer), None);
    }

    fn assert_landing_holds(
        initial: CemeteryEntrance,
        player: Player,
        holds: [u64; LANDING_POSE_COUNT],
    ) {
        let landing = 189u64.saturating_add(
            16u64.saturating_mul(u64::from(initial.rank(player))),
        );
        let before = initial.at(landing.saturating_sub(1));
        assert!(before.body(player).is_some());
        assert_eq!(before.landing_pose(player), None);
        for tick in landing..=PLAYABLE_TICK {
            let state = initial.at(tick);
            let expected = holds
                .iter()
                .take_while(|hold| **hold <= tick.saturating_sub(landing))
                .count()
                .checked_sub(1);
            assert_eq!(state.body(player), None);
            assert_eq!(state.landing_pose(player), expected);
        }
        assert_eq!(
            initial.at(1000).landing_pose(player),
            initial.at(PLAYABLE_TICK).landing_pose(player),
        );
    }

    #[test]
    fn landing_pose_holds_follow_all_four_rank_rotations() {
        let keys = [
            [0u64, 4, 9, 15, 23, 31, 38, 44],
            [0, 5, 11, 17, 23, 29, 36, 44],
            [0, 5, 10, 16, 23, 30, 37, 44],
            [0, 5, 10, 15, 23, 32, 39, 45],
        ];
        for first in Player::ALL {
            let initial = arrival(first);
            for (player, holds) in Player::ALL.into_iter().zip(keys) {
                assert_landing_holds(initial, player, holds);
            }
        }
    }

    #[test]
    fn absent_stations_do_not_acquire_preview_bodies() {
        let mut state =
            CemeteryEntrance::new([false, true, false, true], Player::Marge);
        for _ in 0u64..192 {
            state.advance(false);
        }
        let sampled = state.at(125);
        for player in [Player::Marge, Player::Bart] {
            assert_eq!(sampled.position(player), None);
            assert_eq!(sampled.height(player), None);
            assert_eq!(sampled.body(player), None);
            assert_eq!(state.at(237).landing_pose(player), None);
        }
        assert!(sampled.body(Player::Homer).is_some());
        assert!(sampled.body(Player::Lisa).is_some());
    }
}
