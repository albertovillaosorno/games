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
//   - Bounded first-room enemy approach, alignment and target reservations.
// - Must-Not:
//   - Admit enemy attacks, decode source media or render sprites.
// - Allows:
//   - Inputs: Fixed-step station state and shared room projection.
//   - Outputs: Two persistent enemy positions, selected targets and body poses.
//   - Side effects: Deterministic in-memory progression only.
// - Split-When:
//   - Verified enemy attack or terrain policies need separate ownership.
// - Merge-When:
//   - Another encounter implements equivalent target reservation rules.
// - Summary:
//   - Model observed approach, lane alignment and animation phases.
// - Description:
//   - Track fractional motion, staggered steering and periodic target
//     balancing, stopping at the first unsupported transition.
// - Usage:
//   - Drive once per protected room tick, before station movement.
// - Defaults:
//   - No normal campaign admission or unverified enemy combat.

//! Bounded first-room enemy pursuit.

use games::animation::cycle_tick;

use crate::{Facing, Maneuver, Player, Point, Slot, nearest_fighting_station};

const UNIT: i32 = 256;
const APPROACH_SPEED: i32 = 320;
const ALIGNMENT_SPEED: i32 = 280;
const APPROACH_DISTANCE: u32 = 64;
const ATTACK_DISTANCE: u32 = 32;
const LANE_DISTANCE: u32 = 3;
const TARGET_INTERVAL: u8 = 16;
const STEERING_INTERVAL: u8 = 8;
const ANIMATION_INTERVAL: u8 = 6;
const ANIMATION_DURATION: u8 = 48;
const LEVEL_FACING_DISTANCE: u32 = 16;

/// Depth component of the sampled enemy body orientation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CemeteryEnemyDepth {
    /// Body faces toward decreasing world depth.
    Decreasing,
    /// Body faces toward increasing world depth.
    Increasing,
    /// Body faces along the horizontal axis.
    Level,
}

/// Latched body identity for one approach animation keyframe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryEnemyPose {
    /// Depth orientation selected when the animation frame advanced.
    pub depth: CemeteryEnemyDepth,
    /// Horizontal orientation at the same keyframe boundary.
    pub facing: Facing,
    /// One of eight six-tick approach keyframes.
    pub phase: u8,
}

impl CemeteryEnemyPose {
    const fn directed(dx: i32, dy: i32, phase: u8) -> Self {
        let depth = if dy.unsigned_abs() < LEVEL_FACING_DISTANCE {
            CemeteryEnemyDepth::Level
        } else if dy.is_negative() {
            CemeteryEnemyDepth::Decreasing
        } else {
            CemeteryEnemyDepth::Increasing
        };
        Self {
            depth,
            facing: if dx < 0 {
                Facing::Left
            } else {
                Facing::Right
            },
            phase,
        }
    }
}

/// One enemy's camera-relative position and currently selected station.
///
/// The bounded first-room approach does not include attack, response,
/// terrain changes or the subsequent encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryEnemyApproach {
    /// Latched approach art identity, absent before both bodies initialize.
    pub pose: Option<CemeteryEnemyPose>,
    /// Floored logical position at the sampled camera offset.
    pub position: Point,
    /// Linked station in cabinet order.
    pub target: Player,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PursuitMotion {
    AlignDepth,
    AlignHorizontal,
    LongRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pursuer {
    animation_tick: u8,
    motion: PursuitMotion,
    pose: Option<CemeteryEnemyPose>,
    position: (i32, i32),
    steering: u8,
    target: Player,
    velocity: (i32, i32),
}

impl Pursuer {
    fn advance_pose(&mut self, dx: i32, dy: i32) -> Option<()> {
        self.animation_tick = cycle_tick(
            u64::from(self.animation_tick).saturating_add(1),
            ANIMATION_DURATION,
        )?;
        if self.animation_tick.checked_rem(ANIMATION_INTERVAL) == Some(0) {
            self.pose = Some(CemeteryEnemyPose::directed(
                dx,
                dy,
                self.animation_tick.checked_div(ANIMATION_INTERVAL)?,
            ));
        }
        Some(())
    }

    const fn align(&mut self, dx: i32, dy: i32) -> Option<()> {
        // The next selector may choose an attack. Do not invent its
        // selection, or extrapolate right-facing/other depth families.
        if dx >= 0i32
            || dx.unsigned_abs() < ATTACK_DISTANCE
            || dy.unsigned_abs() >= LEVEL_FACING_DISTANCE
        {
            return None;
        }
        match self.motion {
            PursuitMotion::AlignDepth => {
                self.velocity =
                    (0i32, ALIGNMENT_SPEED.saturating_mul(dy.signum()));
                if dy.unsigned_abs() < LANE_DISTANCE {
                    self.motion = PursuitMotion::AlignHorizontal;
                }
            },
            PursuitMotion::AlignHorizontal => {
                self.velocity =
                    (ALIGNMENT_SPEED.saturating_mul(dx.signum()), 0i32);
                if dy.unsigned_abs() >= LANE_DISTANCE {
                    self.motion = PursuitMotion::AlignDepth;
                }
            },
            PursuitMotion::LongRange => return None,
        }
        Some(())
    }

    const fn begin_alignment(
        &mut self,
        dx: i32,
        dy: i32,
        enabled: bool,
    ) -> Option<()> {
        // Only the observed leftward, decreasing-depth admission is
        // registered. Preserve the preceding stride on entry.
        if !enabled
            || dx >= 0i32
            || dy >= 0i32
            || dy.unsigned_abs() >= LEVEL_FACING_DISTANCE
        {
            return None;
        }
        self.motion = PursuitMotion::AlignDepth;
        self.animation_tick = 0;
        self.pose = Some(CemeteryEnemyPose {
            depth: CemeteryEnemyDepth::Decreasing,
            facing: Facing::Left,
            phase: 0,
        });
        Some(())
    }

    fn long_range(&mut self, dx: i32, dy: i32) -> Option<()> {
        self.advance_pose(dx, dy)?;
        self.steering = self.steering.saturating_sub(1);
        if self.steering == 0 {
            self.velocity = approach_velocity(dx, dy)?;
            self.steering = STEERING_INTERVAL;
        }
        Some(())
    }

    const fn new(position: (i32, i32), steering: u8) -> Self {
        Self {
            animation_tick: 0,
            motion: PursuitMotion::LongRange,
            pose: None,
            position,
            steering,
            target: Player::Marge,
            velocity: (0, 0),
        }
    }

    fn point(self) -> Point {
        let (horizontal, depth) = self.position;
        Point {
            x: i16::try_from(horizontal.div_euclid(UNIT)).unwrap_or_default(),
            y: i16::try_from(depth.div_euclid(UNIT)).unwrap_or_default(),
        }
    }

    fn sample(self, depth_offset: i16) -> CemeteryEnemyApproach {
        let point = self.point();
        CemeteryEnemyApproach {
            pose: self.pose,
            position: Point {
                y: point.y.saturating_add(depth_offset),
                ..point
            },
            target: self.target,
        }
    }

    fn steer(&mut self, slots: &[Slot; 4], alignment: bool) -> Option<()> {
        let current = self.point();
        let target = slots.get(self.target.index())?.station.position;
        let dx = i32::from(target.x).saturating_sub(i32::from(current.x));
        let dy = i32::from(target.y).saturating_sub(i32::from(current.y));
        match self.motion {
            PursuitMotion::LongRange
                if dx.unsigned_abs() < APPROACH_DISTANCE =>
            {
                self.begin_alignment(dx, dy, alignment)?;
            },
            PursuitMotion::LongRange => self.long_range(dx, dy)?,
            PursuitMotion::AlignDepth | PursuitMotion::AlignHorizontal => {
                self.align(dx, dy)?;
                self.advance_pose(dx, dy)?;
            },
        }
        let (horizontal, depth) = self.position;
        let (vx, vy) = self.velocity;
        self.position =
            (horizontal.saturating_add(vx), depth.saturating_add(vy));
        Some(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PursuitPair {
    enemies: [Pursuer; 2],
    reservations: [u8; 4],
    target_timer: u8,
}

impl PursuitPair {
    // Until the first refresh both links refer to the first station,
    // without an assigned reservation. The refresh is sequential.
    pub(super) fn advance(
        &mut self,
        slots: &[Slot; 4],
        alignment: bool,
    ) -> Option<()> {
        if slots.iter().any(|slot| {
            !slot.station.can_fight() || slot.station.maneuver != Maneuver::Idle
        }) {
            return None;
        }
        self.target_timer = self.target_timer.saturating_sub(1);
        let refresh = self.target_timer == 0;
        if refresh {
            self.target_timer = TARGET_INTERVAL;
        }
        for enemy in &mut self.enemies {
            if refresh {
                Self::retarget(enemy, &mut self.reservations, slots)?;
            }
            enemy.steer(slots, alignment)?;
        }
        Some(())
    }

    pub(super) fn begin_animation(&mut self) {
        for enemy in &mut self.enemies {
            enemy.pose = Some(CemeteryEnemyPose {
                depth: CemeteryEnemyDepth::Increasing,
                facing: Facing::Left,
                phase: 0,
            });
        }
    }

    pub(super) const fn new() -> Self {
        Self {
            enemies: [
                Pursuer::new((98_304, 32_768), 1),
                Pursuer::new((100_352, 45_056), 2),
            ],
            reservations: [0; 4],
            target_timer: TARGET_INTERVAL.saturating_sub(1),
        }
    }

    fn retarget(
        enemy: &mut Pursuer,
        reservations: &mut [u8; 4],
        slots: &[Slot; 4],
    ) -> Option<()> {
        let previous_target = enemy.target;
        let previous = reservations.get_mut(enemy.target.index())?;
        *previous = previous.saturating_sub(1);
        let (nearest, _) = nearest_fighting_station(enemy.point(), slots)?;
        enemy.target = if *reservations.get(nearest.index())? == 0 {
            nearest
        } else {
            Player::ALL
                .into_iter()
                .zip(*reservations)
                .min_by_key(|&(_, count)| count)?
                .0
        };
        if enemy.motion != PursuitMotion::LongRange
            && enemy.target != previous_target
        {
            return None;
        }
        let selected = reservations.get_mut(enemy.target.index())?;
        *selected = selected.saturating_add(1);
        Some(())
    }

    pub(super) fn sample(
        self,
        depth_offset: i16,
    ) -> [CemeteryEnemyApproach; 2] {
        self.enemies.map(|enemy| enemy.sample(depth_offset))
    }
}

// For the verified horizontal band, a six-bit minor/major ratio chooses
// a quantized depth component. The main axis remains at full speed.
// Reduce a wide denominator before computing the ratio to retain the
// observed truncation near bin boundaries.
fn approach_velocity(dx: i32, dy: i32) -> Option<(i32, i32)> {
    let mut major = dx.unsigned_abs();
    let mut minor = dy.unsigned_abs().saturating_mul(64);
    while major > 255 {
        major >>= 1u32;
        minor >>= 1u32;
    }
    let ratio = minor.checked_div(major)?;
    let depth: i32 = match ratio {
        0 => 0,
        1..=5 => 16,
        6 => 32,
        7..=11 => 48,
        12..=14 => 64,
        _ => return None,
    };
    Some((
        APPROACH_SPEED.saturating_mul(dx.signum()),
        depth.saturating_mul(dy.signum()),
    ))
}
