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
//   - Inputs: Character and current fixed-step maneuver.
//   - Outputs: Next maneuver and approximate vertical displacement.
//   - Side effects: None.
// - Split-When:
//   - Hit detection or damage requires independent state.
// - Merge-When:
//   - Another title-local module duplicates the maneuver timeline.
// - Summary:
//   - Initial isolated attack and jump timelines.
// - Description:
//   - Uses observed pose durations, with provisional jump elevation.
// - Usage:
//   - Advance once per tick before admitting new button edges.
// - Defaults:
//   - Idle; no height or automatic repetitions.

//! Fixed-step initial attack and jump timelines.

use super::Player;

/// Current isolated player maneuver.
///
/// Attack hitboxes, combinations, interruptions and held-jump variants
/// are not yet modeled.
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
    Jump {
        /// Zero-based displayed pose tick.
        tick: u8,
    },
}

impl Maneuver {
    pub(super) const fn advance(self, player: Player) -> Self {
        match self {
            Self::Attack { tick } => {
                let duration = if matches!(player, Player::Bart) {
                    18
                } else {
                    17
                };
                let next = tick.saturating_add(1);
                if next < duration {
                    Self::Attack { tick: next }
                } else {
                    Self::Idle
                }
            },
            Self::Idle => Self::Idle,
            Self::Jump { tick } => {
                let next = tick.saturating_add(1);
                if next < 31 {
                    Self::Jump { tick: next }
                } else {
                    Self::Idle
                }
            },
        }
    }

    /// Approximate elevation above the ground anchor, in logical pixels.
    ///
    /// The isolated jump lasts 31 displayed ticks. This curve approximates
    /// the observed rise and fall without claiming exact actor pivots.
    #[must_use]
    pub fn elevation(self) -> i16 {
        // Within a pose, the original object moves upward then downward.
        // The two poses have different anchors, so the transition and the
        // landing baseline are estimates, not decoded physics constants.
        const HEIGHTS: [i16; 31] = [
            7, 14, 21, 27, 32, 37, 41, 45, 47, 49, 51, 51, 51, 50, 50, 49, 47,
            46, 44, 42, 40, 37, 35, 32, 28, 25, 21, 17, 13, 8, 4,
        ];
        if let Self::Jump { tick } = self {
            HEIGHTS.get(usize::from(tick)).copied().unwrap_or(0)
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_maneuvers_finish_at_observed_tick_counts() {
        for player in Player::ALL {
            let attack_ticks = if player == Player::Bart {
                18
            } else {
                17
            };
            assert_duration(player, Maneuver::Attack { tick: 0 }, attack_ticks);
            assert_duration(player, Maneuver::Jump { tick: 0 }, 31);
        }
    }

    fn assert_duration(player: Player, initial: Maneuver, duration: u8) {
        let mut state = initial;
        for _ in 0..duration {
            assert_ne!(state, Maneuver::Idle);
            state = state.advance(player);
        }
        assert_eq!(state, Maneuver::Idle);
    }

    #[test]
    fn elevation_rises_and_falls_without_changing_ground() {
        let heights: Vec<_> = (0..31)
            .map(|tick| Maneuver::Jump { tick }.elevation())
            .collect();
        assert_eq!(heights.first(), Some(&7));
        assert_eq!(heights.iter().max(), Some(&51));
        assert_eq!(heights.last(), Some(&4));
        assert!(heights.windows(2).all(|pair| {
            if let [first, second] = pair {
                first.abs_diff(*second) <= 7
            } else {
                false
            }
        }));
        assert_eq!(Maneuver::Idle.elevation(), 0);
    }
}
