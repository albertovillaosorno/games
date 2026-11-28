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
//   - Bounded first-room enemy approach and target reservations.
// - Must-Not:
//   - Admit close-range combat, decode source media or render sprites.
// - Allows:
//   - Inputs: Fixed-step station state and shared room projection.
//   - Outputs: Two persistent enemy positions and selected targets.
//   - Side effects: Deterministic in-memory progression only.
// - Split-When:
//   - Verified enemy attack or terrain policies need separate ownership.
// - Merge-When:
//   - Another encounter implements equivalent target reservation rules.
// - Summary:
//   - Independently model the observed long-range approach phase.
// - Description:
//   - Track fractional motion, staggered steering and periodic target
//     balancing, stopping at the first unsupported transition.
// - Usage:
//   - Drive once per protected room tick, before station movement.
// - Defaults:
//   - No normal campaign admission or unverified close-range behaviour.

//! Bounded first-room enemy pursuit.

use crate::{Maneuver, Player, Point, Slot, nearest_fighting_station};

const UNIT: i32 = 256;
const APPROACH_SPEED: i32 = 320;
const TARGET_INTERVAL: u8 = 16;
const STEERING_INTERVAL: u8 = 8;

/// One enemy's camera-relative position and currently selected station.
///
/// The bounded first-room approach does not include attack, response,
/// terrain changes or the subsequent encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryEnemyApproach {
    /// Floored logical position at the sampled camera offset.
    pub position: Point,
    /// Linked station in cabinet order.
    pub target: Player,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pursuer {
    position: (i32, i32),
    steering: u8,
    target: Player,
    velocity: (i32, i32),
}

impl Pursuer {
    const fn new(position: (i32, i32), steering: u8) -> Self {
        Self {
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
            position: Point {
                y: point.y.saturating_add(depth_offset),
                ..point
            },
            target: self.target,
        }
    }

    fn steer(&mut self, slots: &[Slot; 4]) -> Option<()> {
        let current = self.point();
        let target = slots.get(self.target.index())?.station.position;
        let dx = i32::from(target.x).saturating_sub(i32::from(current.x));
        let dy = i32::from(target.y).saturating_sub(i32::from(current.y));
        // The independent captures establish long-range horizontal motion.
        // A different direction band or near-range action is not admitted.
        if dx.unsigned_abs() < 64 {
            return None;
        }
        self.steering = self.steering.saturating_sub(1);
        if self.steering == 0 {
            self.velocity = approach_velocity(dx, dy)?;
            self.steering = STEERING_INTERVAL;
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
    pub(super) fn advance(&mut self, slots: &[Slot; 4]) -> Option<()> {
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
            enemy.steer(slots)?;
        }
        Some(())
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
