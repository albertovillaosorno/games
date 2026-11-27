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
//   - Title-specific fixed-step second-stage arrival and landing motion.
// - Must-Not:
//   - Decode original media, admit stations or render sprites.
// - Allows:
//   - Inputs: Elapsed simulation ticks and character identity.
//   - Outputs: Visibility, scripted positions, height and steering timing.
//   - Side effects: Deterministic in-memory progression.
// - Split-When:
//   - Another stage needs an independent arrival policy.
// - Merge-When:
//   - A shared scene mechanism replaces the title's arrival timing.
// - Summary:
//   - Bridge the stage card and playable second-stage encounter.
// - Description:
//   - Model the observed descent and subsequent airborne landing.
// - Usage:
//   - Advance once per unpaused playing tick.
// - Defaults:
//   - No entrance outside second-stage admission.

//! Deterministic second-stage arrival and landing.

use super::{Player, Point};

const FIRST_ACTOR: u16 = 128;
const DESCENT_STEPS: u16 = 100;
const SCRIPT_END: u16 = 229;
const FIRST_BOUNCE: u16 = 230;
const LAST_LANDING: u16 = 295;
const UNITS_PER_PIXEL: i32 = 256;
const INITIAL_HEIGHT: i32 = 192;
const GROUND: i16 = 192;

/// Elapsed fixed-step second-stage arrival.
///
/// During the scripted interval, players cannot move the actors. The
/// following landing permits the sampled alternating directional ticks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StageEntrance {
    tick: u16,
}

impl StageEntrance {
    pub(super) const fn advance(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    fn bounce_height(self, player: Player) -> i32 {
        let steps = i32::from(
            self.tick
                .saturating_sub(FIRST_BOUNCE)
                .checked_div(2)
                .unwrap_or_default()
                .saturating_add(1),
        );
        let initial = Self { tick: SCRIPT_END }.descent_height(player);
        let rising = steps.min(8i32);
        let falling = steps.saturating_sub(rising);
        initial
            .saturating_add(1344i32.saturating_mul(rising))
            .saturating_sub(
                96i32
                    .saturating_mul(rising)
                    .saturating_mul(rising.saturating_sub(1)),
            )
            .saturating_sub(
                32i32
                    .saturating_mul(falling)
                    .saturating_mul(falling.saturating_add(1)),
            )
    }

    pub(super) const fn complete(self) -> bool {
        self.tick >= LAST_LANDING
    }

    fn descent_height(self, player: Player) -> i32 {
        let steps =
            i32::from(self.tick.saturating_sub(FIRST_ACTOR).min(DESCENT_STEPS));
        initial_height(player)
            .saturating_sub(640i32.saturating_mul(steps))
            .saturating_add(
                3i32.saturating_mul(steps)
                    .saturating_mul(steps.saturating_sub(1)),
            )
    }

    /// Visible height above the ground, in logical pixels.
    ///
    /// Returns no height before the character's scripted admission.
    #[must_use]
    pub fn elevation(self, player: Player) -> Option<i16> {
        if !self.visible(player) {
            return None;
        }
        let height: i32 = if self.complete() {
            0
        } else if self.tick <= SCRIPT_END {
            self.descent_height(player)
        } else {
            self.bounce_height(player)
        };
        i16::try_from(height.max(0).checked_div(UNITS_PER_PIXEL)?).ok()
    }

    pub(super) fn scripted_position(self, player: Player) -> Option<Point> {
        if !self.visible(player) || self.tick > SCRIPT_END {
            return None;
        }
        let start = match player {
            Player::Marge => 32i16,
            Player::Homer => 64,
            Player::Bart => 96,
            Player::Lisa => 128,
        };
        let steps = i16::try_from(
            self.tick.saturating_sub(FIRST_ACTOR).min(DESCENT_STEPS),
        )
        .ok()?;
        Some(Point {
            x: start.saturating_add(steps),
            y: GROUND,
        })
    }

    pub(super) const fn steering_tick(self) -> bool {
        self.tick > FIRST_BOUNCE && self.tick.is_multiple_of(2)
    }

    /// Elapsed arrival ticks, including the pre-admission interval.
    #[must_use]
    pub const fn tick(self) -> u16 {
        self.tick
    }

    const fn visible(self, player: Player) -> bool {
        self.tick > FIRST_ACTOR
            || (self.tick == FIRST_ACTOR && matches!(player, Player::Marge))
    }
}

const fn initial_height(player: Player) -> i32 {
    let offset: i32 = match player {
        Player::Marge => 0,
        Player::Homer => -8,
        Player::Bart => 8,
        Player::Lisa => -16,
    };
    INITIAL_HEIGHT
        .saturating_add(offset)
        .saturating_mul(UNITS_PER_PIXEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descent_admits_four_stations_and_preserves_relative_offsets() {
        let before = StageEntrance { tick: 127 };
        assert_eq!(
            Player::ALL.map(|player| before.elevation(player)),
            [None; 4]
        );
        let first = StageEntrance { tick: 128 };
        assert_eq!(Player::ALL.map(|player| first.elevation(player)), [
            Some(192),
            None,
            None,
            None
        ]);
        let visible = StageEntrance { tick: 129 };
        assert_eq!(Player::ALL.map(|player| visible.elevation(player)), [
            Some(189),
            Some(181),
            Some(197),
            Some(173)
        ]);
        let end = StageEntrance { tick: 229 };
        assert_eq!(Player::ALL.map(|player| end.elevation(player)), [
            Some(58),
            Some(50),
            Some(66),
            Some(42)
        ]);
        assert_eq!(Player::ALL.map(|player| end.scripted_position(player)), [
            Some(Point { x: 132, y: 192 }),
            Some(Point { x: 164, y: 192 }),
            Some(Point { x: 196, y: 192 }),
            Some(Point { x: 228, y: 192 }),
        ]);
        assert!(!end.steering_tick());
    }

    #[test]
    fn bounce_holds_alternate_frames_and_reaches_apex() {
        let first = StageEntrance { tick: 230 };
        assert_eq!(first.elevation(Player::Marge), Some(63));
        assert!(!first.steering_tick());
        let held = StageEntrance { tick: 231 };
        assert_eq!(held.elevation(Player::Marge), Some(63));
        assert!(!held.steering_tick());
        let steer = StageEntrance { tick: 232 };
        assert_eq!(steer.elevation(Player::Marge), Some(67));
        assert!(steer.steering_tick());
        let apex = StageEntrance { tick: 244 };
        assert_eq!(apex.elevation(Player::Marge), Some(79));
    }

    #[test]
    fn players_land_in_character_order() {
        let lisa = StageEntrance { tick: 288 };
        assert_eq!(lisa.elevation(Player::Lisa), Some(0));
        assert_eq!(lisa.elevation(Player::Marge), Some(15));
        let homer = StageEntrance { tick: 292 };
        assert_eq!(homer.elevation(Player::Homer), Some(0));
        assert_eq!(homer.elevation(Player::Marge), Some(4));
        let marge = StageEntrance { tick: 294 };
        assert_eq!(marge.elevation(Player::Marge), Some(0));
        assert_eq!(marge.elevation(Player::Bart), Some(5));
        let bart = StageEntrance { tick: 295 };
        assert!(bart.complete());
        assert_eq!(
            Player::ALL.map(|player| bart.elevation(player)),
            [Some(0); 4]
        );
    }
}
