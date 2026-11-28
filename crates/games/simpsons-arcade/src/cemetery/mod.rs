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

mod pursuit;

use games::animation::cycle_tick;
use pursuit::PursuitPair;
pub use pursuit::{
    CemeteryEnemyApproach, CemeteryEnemyDepth, CemeteryEnemyPose,
};

use super::{
    Facing, GROUND_ATTACK_RECOVERY_TICKS, Maneuver, PendingManeuver, Player,
    Point, Recovery, Slot, StreetLayout, WalkMotion, stage_card_departed,
};

const FIRST_BACKGROUND: u64 = 13;
const FIRST_STATION: u64 = 107;
const PLAYABLE_TICK: u64 = 237;
const FIRST_ENEMY_APPROACH: u64 = 238;
const FIRST_ENEMY_ANIMATION: u64 = 239;
const FIRST_ENEMY_MOVEMENT: u64 = 240;
const FIRST_WALK_TICK: u64 = 242;
const LAST_WALK_TICK: u64 = 357;
const FIRST_PROTECTED_BODY_SAMPLE: u64 = 240;
const LAST_PROTECTED_BODY_SAMPLE: u64 = 358;
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

/// Delayed ground-action art sample, including the recovery latch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryGroundAttack {
    /// Whether horizontal input selected the moving attack family.
    pub moving: bool,
    /// Relative sample from pre-action idle (0) through terminal art (18).
    pub tick: u8,
}

impl CemeteryGroundAttack {
    fn sample(slot: &Slot, previous: Option<Self>) -> Option<Self> {
        match slot.station.maneuver {
            Maneuver::Attack { moving, tick } => Some(Self { moving, tick }),
            Maneuver::Idle => {
                let Recovery::Ground(remaining) = slot.action_recovery else {
                    return None;
                };
                let tick =
                    GROUND_ATTACK_RECOVERY_TICKS.saturating_sub(remaining);
                previous
                    .filter(|sample| {
                        tick <= 18 && sample.tick.saturating_add(1) == tick
                    })
                    .map(|sample| Self { tick, ..sample })
            },
            Maneuver::AirAttack(_)
            | Maneuver::Hurt { .. }
            | Maneuver::Jump(_)
            | Maneuver::Restrained
            | Maneuver::Special { .. } => None,
        }
    }
}

/// One projected first-room station from the preceding simulation step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryRoomBody {
    /// Horizontal facing at the displayed sample.
    pub facing: Facing,
    /// Ground attack artwork phase, including its terminal recovery.
    pub ground_attack: Option<CemeteryGroundAttack>,
    /// Exact maneuver and phase at the displayed sample.
    pub maneuver: Maneuver,
    /// Camera-relative station record position.
    pub position: Point,
    /// Walking phase at the displayed sample.
    pub walking: Option<WalkMotion>,
}

impl CemeteryRoomBody {
    const fn idle(position: Point) -> Self {
        Self {
            facing: Facing::Right,
            ground_attack: None,
            maneuver: Maneuver::Idle,
            position,
            walking: None,
        }
    }
}

/// Bounded first-room actions and shared vertical projection.
///
/// World depth increases away from the viewer. The projected depth adds
/// the shared camera offset, so neutral stations retain their world
/// positions during a pan. This does not model subsequent room combat,
/// terrain sampling or the original camera descriptor scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryRoom {
    depth_offset: i16,
    enemy_alignment: bool,
    enemy_choice_seed: Option<u16>,
    presentation_bodies: [CemeteryRoomBody; 4],
    presentation_depth_offset: i16,
    presentation_enemies: Option<[CemeteryEnemyApproach; 2]>,
    pursuit: Option<PursuitPair>,
    walk_depth: [i16; 4],
}

impl Default for CemeteryRoom {
    fn default() -> Self {
        Self {
            depth_offset: 0,
            enemy_alignment: false,
            enemy_choice_seed: None,
            presentation_bodies: ROOM_LAYOUT.spawn.map(CemeteryRoomBody::idle),
            presentation_depth_offset: 0,
            presentation_enemies: None,
            pursuit: None,
            walk_depth: [0; 4],
        }
    }
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
        self.sample_presentation(slots);
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
        // Displayed tile state observes the preceding simulation sample.
        self.presentation_depth_offset = self.depth_offset;
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

    // The observed entrance activates two independently staggered actors.
    fn advance_pursuit(&mut self, tick: u64, slots: &[Slot; 4]) {
        if tick == FIRST_ENEMY_APPROACH {
            self.pursuit = Some(PursuitPair::new(self.enemy_choice_seed));
        }
        self.presentation_enemies =
            self.pursuit.map(|pair| pair.sample(self.depth_offset));
        if tick == FIRST_ENEMY_ANIMATION
            && let Some(pair) = &mut self.pursuit
            && pair.begin_animation().is_none()
        {
            self.pursuit = None;
        }
        if tick > LAST_WALK_TICK {
            self.pursuit = None;
        } else if tick >= FIRST_ENEMY_MOVEMENT
            && let Some(pair) = &mut self.pursuit
            && pair.advance(slots, self.enemy_alignment).is_none()
        {
            self.pursuit = None;
        }
    }

    /// Current shared depth offset, in logical units.
    #[must_use]
    pub const fn depth_offset(self) -> i16 {
        self.depth_offset
    }

    /// Previous station action, orientation and position in cabinet order.
    #[must_use]
    pub const fn presentation_bodies(self) -> [CemeteryRoomBody; 4] {
        self.presentation_bodies
    }

    /// Camera offset consumed by the displayed tile layers.
    ///
    /// Original tile-state sampling precedes the current room step.
    #[must_use]
    pub const fn presentation_depth_offset(self) -> i16 {
        self.presentation_depth_offset
    }

    /// Previous approach sample with the matching displayed camera offset.
    ///
    /// Returns no sample after the first unsupported encounter transition.
    #[must_use]
    pub const fn presentation_enemies(
        self,
    ) -> Option<[CemeteryEnemyApproach; 2]> {
        self.presentation_enemies
    }

    /// Projected station positions consumed by the displayed actor sample.
    ///
    /// The preceding simulation sample shares the displayed camera offset.
    /// Stations remain in cabinet order, even when movement is blocked.
    #[must_use]
    pub fn presentation_positions(self) -> [Point; 4] {
        self.presentation_bodies.map(|body| body.position)
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

    fn sample_presentation(&mut self, slots: &[Slot; 4]) {
        let preceding_room = *self;
        for (body, slot) in self.presentation_bodies.iter_mut().zip(slots) {
            *body = CemeteryRoomBody {
                facing: slot.station.facing,
                ground_attack: CemeteryGroundAttack::sample(
                    slot,
                    body.ground_attack,
                ),
                maneuver: slot.station.maneuver,
                position: preceding_room.project(slot.station.position),
                walking: slot.station.walking,
            };
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
        if !self.card
            && self.first == Player::Marge
            && self.tick >= FIRST_ENEMY_APPROACH
        {
            room.advance_pursuit(self.tick, slots);
        }
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

    // Opt in only when the close-range body families are available.
    pub(super) const fn enable_enemy_alignment(&mut self) {
        if let Some(room) = &mut self.room {
            room.enemy_alignment = true;
        }
    }

    // Only a caller with a verified original checkpoint may choose this
    // protected-strike diagnostic. The campaign never uses this seed.
    pub(super) const fn enable_enemy_strike(&mut self, seed: u16) {
        if let Some(room) = &mut self.room {
            room.enemy_alignment = true;
            room.enemy_choice_seed = Some(seed);
        }
    }

    pub(super) fn enable_room(&mut self) {
        if self.admitted.into_iter().all(|admitted| admitted) {
            self.room = Some(CemeteryRoom::default());
        }
    }

    /// Cumulative entrance effects after setup and admitted ranked descent.
    ///
    /// The count stays zero during the card and saturates after descent.
    /// Presentation can observe it across a fixed-step cue boundary.
    #[must_use]
    pub fn entrance_effect_cues(self) -> u8 {
        if self.card {
            return 0;
        }
        Player::ALL.into_iter().fold(
            u8::from(self.tick >= 26),
            |count, player| {
                count.saturating_add(u8::from(
                    self.rank_elapsed(player)
                        .is_some_and(|elapsed| elapsed >= 13i32),
                ))
            },
        )
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

    /// Body visibility in the observed protected room display samples.
    ///
    /// The original active body records receive zero object ordering
    /// bytes on alternate samples. Shadows have separate admission.
    /// Other rotations and the encounter boundary remain unverified.
    #[must_use]
    pub fn room_body_visibility(self) -> Option<bool> {
        if self.room().is_none()
            || self.first != Player::Marge
            || !(FIRST_PROTECTED_BODY_SAMPLE..=LAST_PROTECTED_BODY_SAMPLE)
                .contains(&self.tick)
        {
            return None;
        }
        Some(self.tick & 1 != 0)
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

    // Phase starts from the independent 119-frame moving-room observation.
    const BACKDROP_PHASE_STARTS: [(u64, u8); 14] = [
        (239, 3),
        (242, 2),
        (249, 1),
        (256, 0),
        (270, 1),
        (277, 2),
        (284, 3),
        (298, 2),
        (305, 1),
        (312, 0),
        (326, 1),
        (333, 2),
        (340, 3),
        (354, 2),
    ];

    // Independent preceding-position samples from the two routed controls.
    // Each row records both world X and projected depth, after the first
    // pursuer enters lane alignment while the second continues approaching.
    const HOMER_LISA_ALIGNMENT: [[i16; 4]; 12] = [
        [334, 162, 343, 207],
        [334, 160, 342, 207],
        [334, 159, 340, 207],
        [334, 158, 339, 207],
        [334, 157, 338, 207],
        [334, 156, 337, 207],
        [334, 155, 335, 207],
        [334, 154, 334, 207],
        [332, 154, 333, 207],
        [331, 154, 332, 207],
        [330, 154, 330, 207],
        [329, 154, 329, 207],
    ];
    const REVERSED_ALIGNMENT: [[i16; 4]; 26] = [
        [324, 161, 333, 199],
        [324, 160, 332, 199],
        [324, 159, 330, 199],
        [324, 158, 329, 199],
        [324, 157, 328, 198],
        [324, 156, 327, 198],
        [324, 154, 325, 198],
        [324, 153, 324, 198],
        [322, 153, 323, 197],
        [321, 153, 322, 197],
        [320, 153, 320, 197],
        [319, 153, 319, 197],
        [318, 153, 318, 196],
        [317, 153, 317, 196],
        [316, 153, 315, 196],
        [316, 154, 314, 196],
        [316, 156, 313, 195],
        [316, 157, 312, 195],
        [316, 158, 310, 195],
        [316, 159, 309, 195],
        [316, 160, 308, 194],
        [316, 161, 307, 194],
        [316, 162, 305, 194],
        [316, 163, 304, 194],
        [316, 164, 303, 194],
        [316, 165, 302, 194],
    ];

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

    fn room_positions(room: &CemeteryRoom, slots: &[Slot; 4]) -> [Point; 4] {
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

    // Movement intervals observed independently in two opposed-direction
    // controls. Both tracks have identical horizontal displacement up
    // to the first close-range selector change.
    fn observed_enemy_depth(tick: i16, homer_lisa: bool, first: bool) -> i16 {
        let mut units = if first {
            32_768i32
        } else {
            45_056i32
        };
        for sample in 240..=tick {
            let stride: i32 = match (first, homer_lisa, sample) {
                (true, _, 240..=255) | (true, true, 256..=263) => 48,
                (true, false, 256..=263) => 32,
                (true, false, 272..=279) | (false, false, 249..=256) => -16,
                (true, true, 272..=278) | (false, false, 257..=272) => -48,
                (true, false, 280..=286) => -32,
                (false, false, 273..=286) => -64,
                _ => 0,
            };
            units = units.saturating_add(stride);
        }
        let offset = downward_sample(tick.saturating_sub(241).max(0)).0;
        i16::try_from(units.div_euclid(256))
            .unwrap_or_default()
            .saturating_add(offset)
    }

    fn observed_enemy_approach(
        tick: i16,
        homer_lisa: bool,
        first: bool,
    ) -> CemeteryEnemyApproach {
        let (start, delay): (i16, i16) = if first {
            (384, 239)
        } else {
            (392, 240)
        };
        let steps = tick.saturating_sub(delay).max(0);
        let x = start.saturating_sub(
            steps
                .saturating_mul(5)
                .saturating_add(3)
                .checked_div(4)
                .unwrap_or_default(),
        );
        let target = if !first || tick < 254 {
            Player::Marge
        } else if homer_lisa {
            Player::Lisa
        } else {
            Player::Bart
        };
        CemeteryEnemyApproach {
            pose: observed_enemy_pose(tick, homer_lisa, first),
            position: Point {
                x,
                y: observed_enemy_depth(tick, homer_lisa, first),
            },
            target,
        }
    }

    fn observed_enemy_pose(
        tick: i16,
        homer_lisa: bool,
        first: bool,
    ) -> Option<CemeteryEnemyPose> {
        let phase = match tick {
            239..=244 => 0,
            245..=250 => 1,
            251..=256 => 2,
            257..=262 => 3,
            263..=268 => 4,
            269..=274 => 5,
            275..=280 => 6,
            281..=286 => 7,
            _ => return None,
        };
        let depth = match (first, homer_lisa, phase) {
            (_, _, 0) | (true, _, 1..=2) => CemeteryEnemyDepth::Increasing,
            (false, false, 2..=7) => CemeteryEnemyDepth::Decreasing,
            _ => CemeteryEnemyDepth::Level,
        };
        Some(CemeteryEnemyPose::Approach {
            depth,
            facing: Facing::Left,
            phase,
        })
    }

    fn assert_enemy_approach_trace(homer_lisa: bool) {
        let mut entrance = arrival(Player::Marge).at(237);
        entrance.enable_room();
        let mut slots = room_slots();
        let [marge, homer, bart, lisa] = &mut slots;
        let down = Control::Down.mask();
        let left = down | Control::Left.mask();
        let right = down | Control::Right.mask();
        if homer_lisa {
            homer.held = left;
            lisa.held = right;
        } else {
            marge.held = left;
            homer.held = right;
            bart.held = right;
            lisa.held = left;
        }
        let terminal = if homer_lisa {
            280i16
        } else {
            288i16
        };
        for tick in 238i16..=terminal {
            entrance.advance(false);
            let _advanced = entrance
                .advance_room(&mut slots, [PendingManeuver::default(); 4]);
            let enemies =
                entrance.room().and_then(CemeteryRoom::presentation_enemies);
            if tick == terminal {
                assert_eq!(enemies, None);
                break;
            }
            let prior = tick.saturating_sub(1);
            assert_eq!(
                enemies,
                Some([
                    observed_enemy_approach(prior, homer_lisa, true),
                    observed_enemy_approach(prior, homer_lisa, false),
                ]),
                "control {homer_lisa}, source tick {prior}",
            );
        }
    }

    fn alignment_controls(homer_lisa: bool, tick: u64) -> [u8; 4] {
        if tick >= 299 {
            let up = Control::Up.mask();
            let left = up | Control::Left.mask();
            let right = up | Control::Right.mask();
            return [right, left, left, right];
        }
        if tick >= 292 {
            return [0; 4];
        }
        let down = Control::Down.mask();
        let left = down | Control::Left.mask();
        let right = down | Control::Right.mask();
        if homer_lisa {
            [0, left, 0, right]
        } else {
            [left, right, right, left]
        }
    }

    fn observed_alignment_sample(
        homer_lisa: bool,
        index: usize,
        [x1, y1, x2, y2]: [i16; 4],
    ) -> [CemeteryEnemyApproach; 2] {
        let phase = u8::try_from(index.checked_div(6).unwrap_or_default())
            .unwrap_or_default();
        let first_depth = if index < 6 {
            CemeteryEnemyDepth::Decreasing
        } else {
            CemeteryEnemyDepth::Level
        };
        let second_phase = if homer_lisa {
            match index {
                0..=1 => 6,
                2..=7 => 7,
                _ => 0,
            }
        } else {
            phase
        };
        let second_depth = if homer_lisa {
            CemeteryEnemyDepth::Level
        } else {
            CemeteryEnemyDepth::Decreasing
        };
        [
            CemeteryEnemyApproach {
                pose: Some(CemeteryEnemyPose::Approach {
                    depth: first_depth,
                    facing: Facing::Left,
                    phase,
                }),
                position: Point { x: x1, y: y1 },
                target: if homer_lisa {
                    Player::Lisa
                } else {
                    Player::Bart
                },
            },
            CemeteryEnemyApproach {
                pose: Some(CemeteryEnemyPose::Approach {
                    depth: second_depth,
                    facing: Facing::Left,
                    phase: second_phase,
                }),
                position: Point { x: x2, y: y2 },
                target: Player::Marge,
            },
        ]
    }

    fn assert_enemy_alignment_trace(homer_lisa: bool) {
        let mut entrance = arrival(Player::Marge).at(237);
        entrance.enable_room();
        entrance.enable_enemy_alignment();
        let mut slots = room_slots();
        let start = if homer_lisa {
            280u64
        } else {
            288u64
        };
        let trace: &[[i16; 4]] = if homer_lisa {
            &HOMER_LISA_ALIGNMENT
        } else {
            &REVERSED_ALIGNMENT
        };
        let end = start
            .saturating_add(u64::try_from(trace.len()).unwrap_or_default());
        for tick in 238..=end {
            for (slot, controls) in
                slots.iter_mut().zip(alignment_controls(homer_lisa, tick))
            {
                slot.held = controls;
            }
            entrance.advance(false);
            let _advanced = entrance
                .advance_room(&mut slots, [PendingManeuver::default(); 4]);
            let enemies =
                entrance.room().and_then(CemeteryRoom::presentation_enemies);
            if tick == end {
                assert_eq!(enemies, None, "unsupported boundary {tick}");
                break;
            }
            if tick < start {
                continue;
            }
            let index =
                usize::try_from(tick.saturating_sub(start)).unwrap_or_default();
            let expected = trace.get(index).copied().map(|coordinates| {
                observed_alignment_sample(homer_lisa, index, coordinates)
            });
            assert_eq!(enemies, expected, "{tick}");
        }
    }

    fn strike_controls(route: u8, tick: u64) -> [u8; 4] {
        if tick < 292 {
            return alignment_controls(true, tick);
        }
        if route == 1 && (299..317).contains(&tick) {
            return [0, 0, 0, Control::Right.mask()];
        }
        if route == 2 && tick >= 299 {
            let up = Control::Up.mask();
            return [
                0,
                up | Control::Right.mask(),
                0,
                up | Control::Left.mask(),
            ];
        }
        [0; 4]
    }

    fn expected_first_strike_pose(tick: u64) -> CemeteryEnemyPose {
        match tick {
            325..=326 => CemeteryEnemyPose::Approach {
                depth: CemeteryEnemyDepth::Decreasing,
                facing: Facing::Left,
                phase: 0,
            },
            292 => CemeteryEnemyPose::Approach {
                depth: CemeteryEnemyDepth::Level,
                facing: Facing::Left,
                phase: 2,
            },
            _ => CemeteryEnemyPose::Strike {
                facing: Facing::Left,
                phase: match if tick >= 327 {
                    tick.saturating_sub(34)
                } else {
                    tick
                } {
                    293..=295 => 0,
                    296..=298 => 1,
                    299..=301 => 2,
                    302..=304 => 3,
                    305..=308 => 4,
                    309..=313 => 5,
                    314..=319 => 6,
                    _ => 7,
                },
            },
        }
    }

    fn expected_strike(route: u8, tick: u64) -> [CemeteryEnemyApproach; 2] {
        let pan = if route == 2 {
            i16::try_from(tick.saturating_sub(319).saturating_mul(2))
                .unwrap_or_default()
        } else {
            0
        };
        let second_phase =
            u8::try_from(tick.saturating_sub(240).div_euclid(6).rem_euclid(8))
                .unwrap_or_default();
        let second_x = 392i16.saturating_sub(
            i16::try_from(
                tick.min(354)
                    .saturating_sub(241)
                    .saturating_mul(5)
                    .saturating_add(3)
                    .div_euclid(4),
            )
            .unwrap_or_default(),
        );
        [
            CemeteryEnemyApproach {
                pose: Some(expected_first_strike_pose(tick)),
                position: Point {
                    x: 329,
                    y: 154i16.saturating_sub(pan),
                },
                target: Player::Lisa,
            },
            CemeteryEnemyApproach {
                pose: Some(if tick >= 356 {
                    CemeteryEnemyPose::Strike {
                        facing: Facing::Left,
                        phase: 7,
                    }
                } else {
                    CemeteryEnemyPose::Approach {
                        depth: CemeteryEnemyDepth::Level,
                        facing: Facing::Left,
                        phase: second_phase,
                    }
                }),
                position: Point {
                    x: second_x,
                    y: 207i16.saturating_sub(pan),
                },
                target: if tick >= 351 {
                    Player::Bart
                } else {
                    Player::Marge
                },
            },
        ]
    }

    #[test]
    fn room_protected_strike_matches_three_independent_controls() {
        for route in 0u8..3 {
            verify_strike_route(route);
        }
    }

    fn verify_strike_route(route: u8) {
        let mut entrance = arrival(Player::Marge).at(237);
        entrance.enable_room();
        // Original source word at the actors' shared admission.
        entrance.enable_enemy_strike(0x1207);
        let mut slots = room_slots();
        let terminal = if route == 2 {
            325
        } else {
            359
        };
        for tick in 238..=terminal {
            let controls = strike_controls(route, tick);
            for (slot, held) in slots.iter_mut().zip(controls) {
                slot.held = held;
            }
            entrance.advance(false);
            let _advanced = entrance
                .advance_room(&mut slots, [PendingManeuver::default(); 4]);
            let enemies =
                entrance.room().and_then(CemeteryRoom::presentation_enemies);
            if tick < 292 {
                continue;
            }
            if tick == terminal {
                assert_eq!(enemies, None, "{route} {tick}");
                break;
            }
            assert_eq!(
                enemies,
                Some(expected_strike(route, tick)),
                "{route} {tick}"
            );
        }
    }

    #[test]
    fn room_enemy_alignment_matches_opposed_two_station_control() {
        assert_enemy_alignment_trace(true);
    }

    #[test]
    fn room_enemy_alignment_matches_released_four_station_control() {
        assert_enemy_alignment_trace(false);
    }

    #[test]
    fn room_enemy_approach_matches_reversed_four_station_control() {
        assert_enemy_approach_trace(false);
    }

    #[test]
    fn room_enemy_approach_matches_opposed_two_station_control() {
        assert_enemy_approach_trace(true);
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
            assert_eq!(room_positions(&room, &slots), expected, "tick {tick}");
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
            assert_eq!(
                room.presentation_depth_offset(),
                downward_sample(tick.saturating_sub(1)).0,
            );
            assert_eq!(room_positions(&room, &slots), [
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
            assert_eq!(
                room.presentation_depth_offset(),
                upward_sample(tick.saturating_sub(1)).0,
            );
            assert_eq!(room_positions(&room, &slots), [
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
            assert_depths(&room, &slots, all, depth);
        }
        hold(&mut slots, [None; 4]);
        for _ in 0u8..21 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            assert_eq!(room.depth_offset(), 31);
            assert_depths(&room, &slots, all, 153);
        }
        hold(&mut slots, controls(Control::Up));
        for tick in 1i16..=38 {
            assert!(room.advance(&mut slots, [PendingManeuver::default(); 4]));
            let (offset, depth) = upward_sample(tick);
            assert_eq!(room.depth_offset(), offset, "tick {tick}");
            assert_depths(&room, &slots, all, depth);
        }
    }

    fn assert_depths(
        room: &CemeteryRoom,
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

    fn observed_backdrop_offset(tick: u64) -> i16 {
        let prior = i16::try_from(tick.saturating_sub(1)).unwrap_or_default();
        match prior {
            ..=241 => 0,
            242..=281 => downward_sample(prior.saturating_sub(241)).0,
            282..=289 => 31,
            290..=329 => upward_sample(prior.saturating_sub(289)).0,
            _ => 15,
        }
    }

    // The independently aligned actor records precede display by one tick.
    fn observed_actor_positions(tick: u64) -> [Point; 4] {
        let prior = i16::try_from(tick.saturating_sub(1)).unwrap_or_default();
        let (marge, bart, depth) = match prior {
            ..=241 => (116, 172, 176),
            242..=281 => {
                let step = prior.saturating_sub(241);
                let stride = step.saturating_mul(2);
                (
                    116i16.saturating_add(stride),
                    172i16.saturating_sub(stride).max(112),
                    downward_sample(step).1,
                )
            },
            282..=289 => (196, 112, 153),
            290..=329 => {
                let step = prior.saturating_sub(289);
                let stride = step.saturating_mul(2);
                (
                    196i16.saturating_sub(stride),
                    112i16.saturating_add(stride),
                    upward_sample(step).1,
                )
            },
            _ => (116, 192, 191),
        };
        let idle = 176i16.saturating_add(observed_backdrop_offset(tick));
        [
            Point { x: marge, y: depth },
            Point { x: 144, y: idle },
            Point { x: bart, y: depth },
            Point { x: 200, y: idle },
        ]
    }

    fn apply_diagonal_schedule(slots: &mut [Slot; 4], tick: u64) {
        let [marge, _, bart, _] = slots;
        (marge.held, bart.held) = match tick {
            242..=281 => (
                Control::Down.mask() | Control::Right.mask(),
                Control::Down.mask() | Control::Left.mask(),
            ),
            290..=329 => (
                Control::Up.mask() | Control::Left.mask(),
                Control::Up.mask() | Control::Right.mask(),
            ),
            _ => (0, 0),
        };
    }

    #[test]
    fn protected_room_body_visibility_matches_observed_transfer_samples() {
        let mut state = arrival(Player::Marge).at(239);
        assert_eq!(state.room_body_visibility(), None);
        state.enable_room();
        assert_eq!(state.room_body_visibility(), None);
        for tick in 240..=358 {
            state.advance(false);
            let visible = tick & 1 == 1;
            assert_eq!(state.tick(), tick);
            assert_eq!(state.room_body_visibility(), Some(visible));
        }
        state.advance(false);
        assert_eq!(state.room_body_visibility(), None);
        let mut other = arrival(Player::Homer).at(240);
        other.enable_room();
        assert_eq!(other.room_body_visibility(), None);
    }

    fn observed_diagonal_walking(prior: u64) -> Option<WalkMotion> {
        match prior {
            242..=281 => Some(WalkMotion {
                away: false,
                tick: prior.saturating_sub(242),
            }),
            290..=329 => Some(WalkMotion {
                away: true,
                tick: prior.saturating_sub(290),
            }),
            _ => None,
        }
    }

    fn assert_diagonal_body_sample(room: &CemeteryRoom, tick: u64) {
        let [marge, homer, bart, lisa] = room.presentation_bodies();
        let prior = tick.saturating_sub(1);
        let walking = observed_diagonal_walking(prior);
        assert_eq!(marge.walking, walking, "Marge at {tick}");
        assert_eq!(bart.walking, walking, "Bart at {tick}");
        assert_eq!(homer.walking, None, "Homer at {tick}");
        assert_eq!(lisa.walking, None, "Lisa at {tick}");
        assert_eq!(
            marge.facing,
            if prior < 290 {
                Facing::Right
            } else {
                Facing::Left
            },
            "Marge at {tick}",
        );
        assert_eq!(
            bart.facing,
            if (242..290).contains(&prior) {
                Facing::Left
            } else {
                Facing::Right
            },
            "Bart at {tick}",
        );
        assert!(
            room.presentation_bodies()
                .iter()
                .all(|body| body.maneuver == Maneuver::Idle)
        );
    }

    #[test]
    fn room_body_samples_keep_the_previous_action_and_orientation() {
        let mut state = arrival(Player::Marge).at(238);
        state.enable_room();
        let mut slots = room_slots();
        for tick in 239u64..=357 {
            apply_diagonal_schedule(&mut slots, tick);
            state.advance(false);
            let _advanced =
                state.advance_room(&mut slots, [PendingManeuver::default(); 4]);
            assert!(state.room().is_some());
            let room = state.room().unwrap_or_default();
            assert_diagonal_body_sample(&room, tick);
        }
    }

    #[test]
    fn moving_room_matches_all_aligned_presentation_samples() {
        let mut state = arrival(Player::Marge).at(238);
        state.enable_room();
        let mut slots = room_slots();
        for tick in 239..=357 {
            apply_diagonal_schedule(&mut slots, tick);
            state.advance(false);
            let advanced =
                state.advance_room(&mut slots, [PendingManeuver::default(); 4]);
            assert_eq!(advanced, tick >= FIRST_WALK_TICK);
            let expected_phase = BACKDROP_PHASE_STARTS
                .into_iter()
                .rev()
                .find_map(|(start, phase)| (tick >= start).then_some(phase));
            assert_eq!(state.background_phase(), expected_phase, "tick {tick}");
            assert_eq!(
                state.room().map(CemeteryRoom::presentation_depth_offset),
                Some(observed_backdrop_offset(tick)),
                "tick {tick}",
            );
            assert_eq!(
                state.room().map(CemeteryRoom::presentation_positions),
                Some(observed_actor_positions(tick)),
                "tick {tick}",
            );
        }
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

    fn assert_entrance_effect_cues(admitted: [bool; 4], first: Player) {
        let mut entrance = CemeteryEntrance::new(admitted, first);
        for _ in 0u16..192 {
            entrance.advance(false);
            assert_eq!(entrance.entrance_effect_cues(), 0);
        }
        for tick in 0..=260u64 {
            let expected = Player::ALL
                .into_iter()
                .filter(|player| {
                    admitted.get(player.index()).copied() == Some(true)
                        && tick
                            >= 120u64.saturating_add(16u64.saturating_mul(
                                u64::from(entrance.rank(*player)),
                            ))
                })
                .count()
                .saturating_add(usize::from(tick >= 26));
            assert_eq!(usize::from(entrance.entrance_effect_cues()), expected);
            entrance.advance(false);
        }
    }

    #[test]
    fn entrance_effects_follow_visible_ranked_descent_and_not_card_time() {
        for first in Player::ALL {
            for bits in 0u8..16 {
                let admitted = [1, 2, 4, 8].map(|bit| bits & bit != 0);
                assert_entrance_effect_cues(admitted, first);
            }
        }
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
        initial: &CemeteryEntrance,
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
                assert_landing_holds(&initial, player, holds);
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
