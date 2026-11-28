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
//   - Fixed-step automatic bonus input and linked departure motion.
// - Must-Not:
//   - Own human roster admission, ranking or presentation.
// - Allows:
//   - Inputs: Frozen roster, scan start and elapsed fixed playing ticks.
//   - Outputs: Pump edges and read-only departure motion.
//   - Side effects: In-memory counters only.
// - Split-When:
//   - Another title needs a different automatic competitor strategy.
// - Merge-When:
//   - A shared strategy can replace these title-specific timing rules.
// - Summary:
//   - Preserve independent automatic pumping and ranked retirement.
// - Description:
//   - The initial scan rotation is a provisional native policy.
// - Usage:
//   - Enable only for an explicitly selected opponent art family.
// - Defaults:
//   - No automatic competitors without opt-in.

//! Automatic bonus competitor timing.

use super::BonusDeparture;

pub(super) const EXIT_TICKS: u16 = 209;
const RELEASE_TICK: u16 = 84;
const LAND_TICK: u16 = 111;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Pump {
    reload: u8,
    remaining: u8,
}

impl Pump {
    pub(super) const fn advance(&mut self) -> bool {
        if self.remaining == 0 {
            self.remaining = self.reload;
            true
        } else {
            self.remaining = self.remaining.saturating_sub(1);
            false
        }
    }

    pub(super) const fn pressed(self) -> bool {
        self.remaining == self.reload
    }

    pub(super) fn roster(human: [bool; 4], start: usize) -> [Option<Self>; 4] {
        let mut pumps = [None; 4];
        let mut first = true;
        for offset in 0..human.len() {
            let index = start.wrapping_add(offset).rem_euclid(human.len());
            if human.get(index) != Some(&false) {
                continue;
            }
            let reload = if first {
                5
            } else {
                5u8.saturating_add(
                    u8::try_from(offset).unwrap_or_default().saturating_mul(2),
                )
            };
            if let Some(pump) = pumps.get_mut(index) {
                *pump = Some(Self { reload, remaining: 0 });
                first = false;
            }
        }
        pumps
    }
}

/// Read-only motion of an automatic competitor after its ranked delay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutomaticDeparture {
    /// Released body descends while the balloon bursts.
    Falling {
        /// Elapsed ticks since release, including the landing hold.
        age: u16,
        /// Body height above the landing point in 1/256 pixels.
        elevation: u32,
    },
    /// Landed body remains until the whole competition ends.
    Landed {
        /// Elapsed ticks since release.
        age: u16,
    },
    /// Shared balloon and body ascent before the opponent releases.
    Rising {
        /// Balloon displacement in 1/256 pixels.
        balloon: u32,
        /// Body displacement in 1/256 pixels.
        body: u32,
    },
}

impl AutomaticDeparture {
    pub(super) fn at(elapsed: u16) -> Self {
        if elapsed < RELEASE_TICK {
            let (balloon, body) = BonusDeparture::displacement(elapsed);
            return Self::Rising { balloon, body };
        }
        let age = elapsed.saturating_sub(RELEASE_TICK);
        if elapsed >= LAND_TICK {
            return Self::Landed { age };
        }
        let steps = i32::from(age);
        let elevation = 19_440i32
            .saturating_add(steps.saturating_mul(568))
            .saturating_sub(
                48i32
                    .saturating_mul(steps)
                    .saturating_mul(steps.saturating_sub(1)),
            );
        Self::Falling {
            age,
            elevation: u32::try_from(elevation).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::array::from_fn;

    use super::*;

    #[test]
    fn every_roster_and_rotation_preserves_independent_pump_periods() {
        for mask in 0..16u8 {
            let human = from_fn(|index| mask & (1 << index) != 0);
            for start in 0..4 {
                verify_roster(human, start);
            }
        }
    }

    fn verify_roster(human: [bool; 4], start: usize) {
        let pumps = Pump::roster(human, start);
        let mut first = true;
        for offset in 0..4usize {
            let index = start.saturating_add(offset).rem_euclid(4);
            let entry = pumps.get(index).copied().flatten();
            if human.get(index) == Some(&true) {
                assert_eq!(entry, None);
                continue;
            }
            let period = if first {
                6
            } else {
                6usize.saturating_add(offset.saturating_mul(2))
            };
            first = false;
            assert!(entry.is_some());
            if let Some(pump) = entry {
                verify_pulses(pump, period);
            }
        }
    }

    fn verify_pulses(mut pump: Pump, period: usize) {
        for tick in 0..120usize {
            let pulse = tick.is_multiple_of(period);
            assert_eq!(pump.advance(), pulse);
            assert_eq!(pump.pressed(), pulse);
        }
    }

    #[test]
    fn descent_matches_integrated_velocity_and_retains_landing() {
        let mut height = 19_440i32;
        let mut speed = 568i32;
        for elapsed in RELEASE_TICK..LAND_TICK {
            assert_eq!(
                AutomaticDeparture::at(elapsed),
                AutomaticDeparture::Falling {
                    age: elapsed.saturating_sub(RELEASE_TICK),
                    elevation: u32::try_from(height).unwrap_or_default(),
                }
            );
            height = height.saturating_add(speed);
            speed = speed.saturating_sub(96);
        }
        for elapsed in LAND_TICK..=EXIT_TICKS {
            assert_eq!(
                AutomaticDeparture::at(elapsed),
                AutomaticDeparture::Landed {
                    age: elapsed.saturating_sub(RELEASE_TICK),
                }
            );
        }
    }
}
