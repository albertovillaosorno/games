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
//   - Title-specific attack and jump timing and provisional elevation.
// - Must-Not:
//   - Render, interpret original media or resolve enemy collisions.
// - Allows:
//   - Inputs: Character, held jump control and fixed-step maneuver.
//   - Outputs: Next maneuver and approximate vertical displacement.
//   - Side effects: None.
// - Split-When:
//   - Hit detection or damage requires independent state.
// - Merge-When:
//   - Another title-local module duplicates the maneuver timeline.
// - Summary:
//   - Isolated attacks and variable-height jump trajectories.
// - Description:
//   - Uses observed action timings and provisional fixed-point lift.
// - Usage:
//   - Advance once per tick before admitting new button edges.
// - Defaults:
//   - Idle; no height or automatic repetitions.

//! Fixed-step initial attack and jump timelines.

use super::Player;

const QUARTERS_PER_PIXEL: i16 = 4;
const SHORT_JUMP_TICKS: u8 = 31;
const MIN_LIFT_TICKS: u8 = 4;

/// A fixed-step airborne trajectory, independent of rendering and ground
/// movement. Heights and vertical speeds use quarter-pixel units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JumpMotion {
    height: i16,
    released: bool,
    tick: u8,
    velocity: i16,
}

impl JumpMotion {
    const fn advance(self, held: bool) -> Option<Self> {
        let tick = self.tick.saturating_add(1);
        let released = self.released || !held;
        // Early release increases deceleration only during ascent.
        // The common minimum lift preserves the observed short tap.
        let gravity = if released && tick >= MIN_LIFT_TICKS && self.velocity > 0
        {
            3
        } else {
            1
        };
        let velocity = self.velocity.saturating_sub(gravity);
        let height = self.height.saturating_add(velocity);
        if height <= 0 {
            None
        } else {
            Some(Self {
                height,
                released,
                tick,
                velocity,
            })
        }
    }

    fn elevation(self) -> i16 {
        self.height.checked_div(QUARTERS_PER_PIXEL).unwrap_or(0)
    }

    // The existing short-jump catalogue is still the only admitted art.
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    pub(super) fn pose_tick(self) -> u8 {
        self.tick.min(SHORT_JUMP_TICKS.saturating_sub(1))
    }

    const fn start() -> Self {
        Self {
            height: 29,
            released: false,
            tick: 0,
            velocity: 29,
        }
    }

    /// Number of elapsed simulation ticks since takeoff.
    #[must_use]
    pub const fn tick(self) -> u8 {
        self.tick
    }
}

/// Current isolated player maneuver.
///
/// Attack combinations, exact original pivots and hitboxes are not modeled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Maneuver {
    /// Station is attacking.
    Attack {
        /// Zero-based displayed pose tick.
        tick: u8,
    },
    /// Station is on the ground and free to walk.
    Idle,
    /// Station is airborne.
    Jump(JumpMotion),
}

impl Maneuver {
    pub(super) const fn advance(self, player: Player, jump_held: bool) -> Self {
        match self {
            Self::Attack { tick } => {
                let duration = self.pose_duration(player);
                let next = tick.saturating_add(1);
                if next < duration {
                    Self::Attack { tick: next }
                } else {
                    Self::Idle
                }
            },
            Self::Idle => Self::Idle,
            Self::Jump(motion) => match motion.advance(jump_held) {
                Some(next) => Self::Jump(next),
                None => Self::Idle,
            },
        }
    }

    pub(super) const fn attack_cue(self, player: Player) -> bool {
        let onset = match player {
            Player::Marge | Player::Bart => 4,
            Player::Homer | Player::Lisa => 5,
        };
        matches!(self, Self::Attack { tick } if tick == onset)
    }

    /// Approximate elevation above the ground anchor, in logical pixels.
    ///
    /// Short and held jumps share a deterministic fixed-point trajectory.
    /// Actor pivots and original physics constants remain approximate.
    #[must_use]
    pub fn elevation(self) -> i16 {
        match self {
            Self::Jump(motion) => motion.elevation(),
            Self::Attack { .. } | Self::Idle => 0,
        }
    }

    pub(super) const fn jump() -> Self {
        Self::Jump(JumpMotion::start())
    }

    // The existing catalogue contains short-tap jump art only.
    pub(super) const fn pose_duration(self, player: Player) -> u8 {
        match self {
            Self::Attack { .. } if matches!(player, Player::Bart) => 18,
            Self::Attack { .. } => 17,
            Self::Idle => 0,
            Self::Jump(_) => SHORT_JUMP_TICKS,
        }
    }

    pub(super) const fn release_jump(&mut self) {
        if let Self::Jump(motion) = self {
            motion.released = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trajectory(player: Player, hold: u8) -> Vec<i16> {
        let mut state = Maneuver::jump();
        let mut heights = Vec::new();
        for tick in 1..=60 {
            if state == Maneuver::Idle {
                return heights;
            }
            heights.push(state.elevation());
            state = state.advance(player, tick < hold);
        }
        assert_eq!(state, Maneuver::Idle);
        heights
    }

    #[test]
    fn isolated_attack_durations_remain_station_specific() {
        for player in Player::ALL {
            let attack_ticks = if player == Player::Bart {
                18u8
            } else {
                17
            };
            let mut state = Maneuver::Attack { tick: 0 };
            for _ in 0..attack_ticks {
                assert_ne!(state, Maneuver::Idle);
                state = state.advance(player, false);
            }
            assert_eq!(state, Maneuver::Idle);
        }
    }

    #[test]
    fn jump_duration_and_peak_increase_with_hold() {
        for player in Player::ALL {
            let tap = trajectory(player, 3);
            let medium = trajectory(player, 20);
            let long = trajectory(player, 60);
            assert_eq!(tap.len(), 31);
            assert_eq!(medium.len(), 49);
            assert_eq!(long.len(), 58);
            assert!(tap.iter().max() < medium.iter().max());
            assert!(medium.iter().max() < long.iter().max());
            assert_eq!(tap.first(), Some(&7));
        }
    }

    #[test]
    fn jump_elevation_is_bounded_and_continuous() {
        let long = trajectory(Player::Marge, 60);
        assert!(long.iter().all(|height| (1..=110).contains(height)));
        assert!(long.windows(2).all(|pair| {
            if let [first, second] = pair {
                first.abs_diff(*second) <= 8
            } else {
                false
            }
        }));
        assert_eq!(Maneuver::Idle.elevation(), 0);
    }

    #[test]
    fn repeated_press_cannot_restore_released_lift() {
        let mut released = Maneuver::jump();
        for _ in 0u8..10 {
            released = released.advance(Player::Marge, true);
        }
        released.release_jump();
        let mut repeated = released;
        for _ in 0u8..58 {
            released = released.advance(Player::Marge, false);
            repeated = repeated.advance(Player::Marge, true);
            assert_eq!(released, repeated);
        }
        assert_eq!(released, Maneuver::Idle);
    }
}
