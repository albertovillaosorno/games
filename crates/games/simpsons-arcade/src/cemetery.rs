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
//   - Fixed-step third-stage card, cemetery entrance and body timing.
// - Must-Not:
//   - Decode original media, choose a random seed or render sprites.
// - Allows:
//   - Inputs: Admitted stations, selected fall order and fresh action edges.
//   - Outputs: Scene phase, rank, fixed-point height and body pose.
//   - Side effects: Deterministic in-memory progression only.
// - Split-When:
//   - Cemetery gameplay needs a distinct encounter policy.
// - Merge-When:
//   - A shared scene transition subsumes the title's entrance rules.
// - Summary:
//   - Reproduce the observed ranked entrance without assuming RNG cadence.
// - Description:
//   - Four-station motion is verified; subset admission is provisional.
// - Usage:
//   - Advance only on unpaused playing ticks.
// - Defaults:
//   - No entrance until explicitly selected.

//! Observed third-stage arrival with an explicit fall order.

use games::animation::cycle_tick;

use super::{Player, Point, stage_card_departed};

const FIRST_BACKGROUND: u64 = 13;
const FIRST_STATION: u64 = 107;
const PLAYABLE_TICK: u64 = 237;
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
    tick: u64,
}

impl CemeteryEntrance {
    /// Number of observed grounded presentation families per character.
    pub const LANDING_POSE_COUNT: usize = 8;

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
        let x = match player {
            Player::Marge => 116,
            Player::Homer => 144,
            Player::Bart => 172,
            Player::Lisa => 200,
        };
        Some(Point { x, y: 176 })
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
