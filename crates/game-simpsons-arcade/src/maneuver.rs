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
    release_tick: Option<u8>,
    tick: u8,
    velocity: i16,
}

impl JumpMotion {
    // A follow-up at input offset four sees three completed airborne ticks.
    pub(super) const fn accepts_attack(self) -> bool {
        self.tick >= 3
    }

    const fn advance(mut self, held: bool) -> Option<Self> {
        if !held {
            self.release();
        }
        let tick = self.tick.saturating_add(1);
        // Early release increases deceleration only during ascent.
        // The common minimum lift preserves the observed short tap.
        let gravity = if self.released_at(tick)
            && tick >= MIN_LIFT_TICKS
            && self.velocity > 0
        {
            3
        } else {
            1
        };
        let velocity = self.velocity.saturating_sub(gravity);
        let height = self.height.saturating_add(velocity);
        if height < QUARTERS_PER_PIXEL {
            None
        } else {
            Some(Self {
                height,
                release_tick: self.release_tick,
                tick,
                velocity,
            })
        }
    }

    fn elevation(self) -> i16 {
        self.height.checked_div(QUARTERS_PER_PIXEL).unwrap_or(0)
    }

    // The observed sustained ascent retains its opening pose through
    // the first eleven displayed ticks.
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    pub(super) const fn opening_pose(self) -> bool {
        self.tick < 11 && !self.released_at(self.tick)
    }

    // Bound selection to the existing short-jump catalogue.
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    pub(super) fn pose_tick(self) -> u8 {
        self.tick.min(SHORT_JUMP_TICKS.saturating_sub(1))
    }

    // Retain the middle pose only through the sustained hold,
    // ending at release or the observed apex transition.
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    pub(super) const fn raised_pose(self) -> bool {
        self.tick >= 11 && self.tick < 31 && !self.released_at(self.tick)
    }

    const fn release(&mut self) {
        if self.release_tick.is_none() {
            // Input precedes the next fixed update. The first observable
            // release transition is one displayed tick after that update.
            self.release_tick = Some(self.tick.saturating_add(2));
        }
    }

    const fn released_at(self, tick: u8) -> bool {
        matches!(self.release_tick, Some(release) if tick >= release)
    }

    const fn start() -> Self {
        Self {
            height: 29,
            release_tick: None,
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

/// A forward airborne attack, with height and velocity in sixteenths
/// of a pixel. The forward rate is selected at the attack edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AirAttackMotion {
    fast: bool,
    height: i16,
    hit: bool,
    tick: u8,
    velocity: i16,
}

impl AirAttackMotion {
    const fn advance(mut self) -> Option<Self> {
        let gravity = if self.fast {
            4
        } else {
            3
        };
        self.velocity = self.velocity.saturating_sub(gravity);
        self.height = self.height.saturating_add(self.velocity);
        self.tick = self.tick.saturating_add(1);
        if self.height < 16 {
            None
        } else {
            Some(self)
        }
    }

    fn elevation(self) -> i16 {
        self.height.checked_div(16).unwrap_or(0)
    }

    const fn start(jump: JumpMotion) -> Self {
        let fast = jump.height >= 64 * QUARTERS_PER_PIXEL;
        let velocity = if fast {
            -22
        } else {
            34
        };
        Self {
            fast,
            height: jump.height.saturating_mul(4).saturating_add(velocity),
            hit: false,
            tick: 0,
            velocity,
        }
    }

    pub(super) const fn step(self) -> i16 {
        if self.fast {
            4
        } else {
            2
        }
    }

    /// Number of displayed ticks since the attack began.
    #[must_use]
    pub const fn tick(self) -> u8 {
        self.tick
    }
}

/// Current player maneuver.
///
/// Cooperative combinations, exact original pivots and hitboxes remain open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Maneuver {
    /// Station is performing a forward airborne attack.
    AirAttack(AirAttackMotion),
    /// Station is attacking.
    Attack {
        /// Zero-based displayed pose tick.
        tick: u8,
    },
    /// Station is on the ground and free to walk.
    Idle,
    /// Station is airborne.
    Jump(JumpMotion),
    /// Station is performing a stationary simultaneous-button attack.
    Special {
        /// Whether the provisional encounter strike has been consumed.
        hit: bool,
        /// Zero-based displayed maneuver tick.
        tick: u8,
    },
}

impl Maneuver {
    pub(super) const fn advance(self, player: Player, jump_held: bool) -> Self {
        match self {
            Self::AirAttack(motion) => match motion.advance() {
                Some(next) => Self::AirAttack(next),
                None => Self::Idle,
            },
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
            Self::Special { hit, tick } => {
                let next = tick.saturating_add(1);
                if next < self.pose_duration(player) {
                    Self::Special { hit, tick: next }
                } else {
                    Self::Idle
                }
            },
        }
    }

    pub(super) const fn air_attack(jump: JumpMotion) -> Self {
        Self::AirAttack(AirAttackMotion::start(jump))
    }

    pub(super) const fn airborne(self) -> bool {
        matches!(
            self,
            Self::Jump(_) | Self::AirAttack(_) | Self::Special { .. }
        )
    }

    pub(super) const fn attack_cue(self, player: Player) -> bool {
        let onset = match player {
            Player::Marge | Player::Bart => 4,
            Player::Homer | Player::Lisa => 5,
        };
        matches!(self, Self::Attack { tick } if tick == onset)
    }

    // The encounter is still provisional. Admit one aerial hit per flight.
    pub(super) fn consume_air_strike(&mut self) -> bool {
        if let Self::AirAttack(motion) = self
            && !motion.hit
            && motion.elevation() <= 64
        {
            motion.hit = true;
            return true;
        }
        if let Self::Special { hit, .. } = self
            && !*hit
        {
            *hit = true;
            return true;
        }
        false
    }

    /// Approximate elevation above the ground anchor, in logical pixels.
    ///
    /// Short and held jumps share a deterministic fixed-point trajectory.
    /// Actor pivots and original physics constants remain approximate.
    #[must_use]
    pub fn elevation(self) -> i16 {
        match self {
            Self::Jump(motion) => motion.elevation(),
            Self::AirAttack(motion) => motion.elevation(),
            Self::Attack { .. } | Self::Idle => 0,
            Self::Special { tick, .. } => {
                let elapsed = i16::from(tick).saturating_add(1);
                elapsed
                    .saturating_mul(31i16.saturating_sub(elapsed))
                    .saturating_mul(3)
                    .checked_div(16)
                    .unwrap_or(0)
            },
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
            Self::AirAttack(_) => 60,
            Self::Special { .. } => 30,
        }
    }

    pub(super) const fn release_jump(&mut self) {
        if let Self::Jump(motion) = self {
            motion.release();
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

    fn air_trace(takeoff_ticks: u8) -> (Vec<i16>, i16) {
        let mut state = Maneuver::jump();
        for _ in 1..takeoff_ticks {
            state = state.advance(Player::Marge, true);
        }
        let Maneuver::Jump(jump) = state else {
            return (Vec::new(), 0);
        };
        state = Maneuver::air_attack(jump);
        let mut heights = Vec::new();
        let mut distance = 0i16;
        for _ in 0u8..60 {
            let Maneuver::AirAttack(motion) = state else {
                break;
            };
            heights.push(state.elevation());
            distance = distance.saturating_add(motion.step());
            state = state.advance(Player::Marge, true);
        }
        assert_eq!(state, Maneuver::Idle);
        (heights, distance)
    }

    #[test]
    fn air_attack_matches_observed_height_and_displacement() {
        let early = [
            35, 37, 39, 41, 42, 43, 44, 45, 46, 46, 46, 46, 46, 46, 45, 45, 44,
            43, 42, 40, 39, 37, 35, 33, 30, 28, 25, 22, 19, 15, 12, 8, 4,
        ];
        let late = [
            81, 79, 77, 75, 73, 70, 67, 64, 61, 57, 53, 49, 45, 40, 35, 30, 25,
            19, 13, 7, 1,
        ];
        let apex = [
            107, 105, 103, 101, 99, 96, 93, 90, 87, 83, 79, 75, 71, 66, 61, 56,
            51, 45, 39, 33, 27, 20, 13, 6,
        ];
        assert_eq!(air_trace(5), (early.to_vec(), 66));
        assert_eq!(air_trace(15), (late.to_vec(), 84));
        assert_eq!(air_trace(30), (apex.to_vec(), 96));
        assert_eq!(air_trace(10).0.first(), Some(&63));
        assert_eq!(air_trace(11).0.first(), Some(&64));
    }

    #[test]
    fn stationary_special_matches_observed_arc() {
        let expected = [
            5, 10, 15, 20, 24, 28, 31, 34, 37, 39, 41, 42, 43, 44, 45, 45, 44,
            43, 42, 41, 39, 37, 34, 31, 28, 24, 20, 15, 10, 5,
        ];
        for player in Player::ALL {
            let mut state = Maneuver::Special { hit: false, tick: 0 };
            for height in expected {
                assert_eq!(state.elevation(), height);
                assert!(state.airborne());
                state = state.advance(player, true);
            }
            assert_eq!(state, Maneuver::Idle);
        }
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
            assert_eq!(medium.len(), 51);
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
    fn release_matches_observed_intermediate_hold_heights() {
        // Lower sprite-cell edge relative to the stationary shadow.
        let short = [
            7, 14, 21, 27, 33, 39, 45, 49, 53, 56, 58, 60, 60, 60, 60, 60, 59,
            58, 57, 55, 53, 51, 49, 47, 44, 41, 38, 34, 30, 26, 22, 18, 13, 8,
            3,
        ];
        let medium = [
            7, 14, 21, 27, 33, 39, 45, 51, 56, 61, 66, 70, 74, 78, 82, 86, 88,
            90, 92, 92, 92, 91, 91, 90, 88, 87, 85, 83, 81, 78, 76, 73, 69, 66,
            62, 58, 54, 49, 45, 40, 34, 29, 23, 17, 11, 4,
        ];
        assert_eq!(trajectory(Player::Marge, 5), short);
        assert_eq!(trajectory(Player::Marge, 15), medium);
        for (hold, duration, peak) in [
            (5, 35, 60),
            (10, 40, 78),
            (15, 46, 92),
            (20, 51, 102),
            (25, 54, 107),
            (30, 58, 108),
            (60, 58, 108),
        ] {
            let heights = trajectory(Player::Marge, hold);
            assert_eq!(heights.len(), duration);
            assert_eq!(heights.iter().max(), Some(&peak));
        }
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
