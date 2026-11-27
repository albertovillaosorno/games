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
//   - Optional fixed-step first-stage victory and balloon bonus rules.
// - Must-Not:
//   - Decode original media, draw sprites or admit the subsequent stage.
// - Allows:
//   - Inputs: Admitted character set and routed button edges.
//   - Outputs: Phase, independent inflation and tied competition ranks.
//   - Side effects: Deterministic in-memory state only.
// - Split-When:
//   - Another distinct bonus game needs its own rules.
// - Merge-When:
//   - Shared bonus rules replace this title-specific strategy.
// - Summary:
//   - Give the cleared wrestler arena a playable bonus continuation.
// - Description:
//   - Timing and inflation tuning remain provisional.
// - Usage:
//   - Enable only alongside a complete local bonus art selection.
// - Defaults:
//   - No bonus without explicit opt-in; no next-stage simulation.

//! Experimental balloon pumping bonus.

use super::Player;

const VICTORY_TICKS: u16 = 900;
const INSTRUCTION_MIN_TICKS: u16 = 120;
const INSTRUCTION_MAX_TICKS: u16 = 400;
const READY_TICKS: u16 = 160;
const PUMP_TICKS: u16 = 420;
const RESULT_TICKS: u16 = 480;
const DECAY_TICKS: u16 = 12;
const INFLATION_PER_PRESS: u8 = 2;

/// Maximum provisional balloon inflation.
pub const BONUS_MAX_INFLATION: u8 = 64;

/// Fixed-step phase of the optional first bonus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BonusPhase {
    /// Results remain available until the session is restarted.
    Complete,
    /// Explain the button-pumping interaction.
    Instructions,
    /// Admit independent pump button edges.
    Pumping,
    /// Countdown before the pumping interval.
    Ready,
    /// Show final balloon sizes and tied ranks.
    Results,
    /// Hold the cleared street before the bonus.
    Victory,
}

/// Read-only bonus state, in character rather than physical-seat order.
///
/// Inflation and duration are independently tuned from sparse original
/// observations; exact score, animation and next-stage rules remain open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bonus {
    inflation: [u8; 4],
    participants: [bool; 4],
    phase: BonusPhase,
    tick: u16,
}

impl Bonus {
    pub(super) fn advance(&mut self, presses: [bool; 4]) {
        if self.phase == BonusPhase::Complete {
            return;
        }
        self.tick = self.tick.saturating_add(1);
        let any_edge = self
            .participants
            .iter()
            .zip(presses)
            .any(|(admitted, pressed)| *admitted && pressed);
        let next = match self.phase {
            BonusPhase::Victory if self.tick >= VICTORY_TICKS => {
                BonusPhase::Instructions
            },
            BonusPhase::Instructions
                if self.tick >= INSTRUCTION_MAX_TICKS
                    || (self.tick >= INSTRUCTION_MIN_TICKS && any_edge) =>
            {
                BonusPhase::Ready
            },
            BonusPhase::Ready if self.tick >= READY_TICKS => {
                BonusPhase::Pumping
            },
            BonusPhase::Pumping => self.pump(presses),
            BonusPhase::Results if self.tick >= RESULT_TICKS => {
                BonusPhase::Complete
            },
            BonusPhase::Complete
            | BonusPhase::Instructions
            | BonusPhase::Ready
            | BonusPhase::Results
            | BonusPhase::Victory => self.phase,
        };
        if next != self.phase {
            self.phase = next;
            self.tick = 0;
        }
    }

    /// Inflation for an admitted character; absent for other characters.
    #[must_use]
    pub fn inflation(self, player: Player) -> Option<u8> {
        let index = player.index();
        let _admitted = self
            .participants
            .get(index)
            .copied()
            .filter(|admitted| *admitted)?;
        self.inflation.get(index).copied()
    }

    pub(super) const fn new(participants: [bool; 4]) -> Self {
        Self {
            inflation: [0; 4],
            participants,
            phase: BonusPhase::Victory,
            tick: 0,
        }
    }

    /// The current victory or bonus phase.
    #[must_use]
    pub const fn phase(self) -> BonusPhase {
        self.phase
    }

    fn pump(&mut self, presses: [bool; 4]) -> BonusPhase {
        let decay = self.tick.is_multiple_of(DECAY_TICKS);
        for ((admitted, size), pressed) in self
            .participants
            .iter()
            .zip(&mut self.inflation)
            .zip(presses)
        {
            if *admitted {
                *size = size
                    .saturating_sub(u8::from(decay))
                    .saturating_add(
                        u8::from(pressed).saturating_mul(INFLATION_PER_PRESS),
                    )
                    .min(BONUS_MAX_INFLATION);
            }
        }
        if self.tick >= PUMP_TICKS
            || self.inflation.contains(&BONUS_MAX_INFLATION)
        {
            BonusPhase::Results
        } else {
            BonusPhase::Pumping
        }
    }

    /// Competition rank for an admitted character after pumping ends.
    ///
    /// Equal inflation receives equal rank, including an all-zero tie.
    #[must_use]
    pub fn rank(self, player: Player) -> Option<u8> {
        if !matches!(self.phase, BonusPhase::Results | BonusPhase::Complete) {
            return None;
        }
        let value = self.inflation(player)?;
        Some(
            Player::ALL
                .into_iter()
                .filter_map(|other| self.inflation(other))
                .fold(1u8, |rank, other| {
                    rank.saturating_add(u8::from(other > value))
                }),
        )
    }

    /// Fixed playing ticks in the current phase.
    #[must_use]
    pub const fn tick(self) -> u16 {
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_pump(participants: [bool; 4]) -> Bonus {
        let mut bonus = Bonus::new(participants);
        for _ in 0..VICTORY_TICKS {
            bonus.advance([true; 4]);
        }
        assert_eq!(bonus.phase(), BonusPhase::Instructions);
        for _ in 0..INSTRUCTION_MIN_TICKS {
            bonus.advance([true; 4]);
        }
        assert_eq!(bonus.phase(), BonusPhase::Ready);
        for _ in 0..READY_TICKS {
            bonus.advance([true; 4]);
        }
        assert_eq!(bonus.phase(), BonusPhase::Pumping);
        assert_eq!(bonus.inflation(Player::Marge), Some(0));
        bonus
    }

    #[test]
    fn idle_timeout_preserves_zero_and_tied_ranks() {
        let mut bonus = start_pump([true, true, false, true]);
        for _ in 0..PUMP_TICKS {
            bonus.advance([false, false, true, false]);
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_eq!(bonus.inflation(Player::Bart), None);
        assert_eq!(bonus.rank(Player::Marge), Some(1));
        assert_eq!(bonus.rank(Player::Lisa), Some(1));
        for _ in 0..RESULT_TICKS {
            bonus.advance([true; 4]);
        }
        assert_eq!(bonus.phase(), BonusPhase::Complete);
        let result = bonus;
        bonus.advance([true; 4]);
        assert_eq!(bonus, result);
    }

    #[test]
    fn simultaneous_pumping_admits_independent_ties_and_early_finish() {
        let mut bonus = start_pump([true; 4]);
        for tick in 0..PUMP_TICKS {
            bonus.advance([true, false, true, tick.is_multiple_of(2)]);
            if bonus.phase() == BonusPhase::Results {
                break;
            }
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_eq!(bonus.inflation(Player::Marge), Some(BONUS_MAX_INFLATION));
        assert_eq!(bonus.rank(Player::Marge), Some(1));
        assert_eq!(bonus.rank(Player::Bart), Some(1));
        assert_eq!(bonus.rank(Player::Lisa), Some(3));
        assert_eq!(bonus.rank(Player::Homer), Some(4));
    }

    #[test]
    fn instruction_timeout_cannot_be_skipped_by_absent_station() {
        let mut bonus = Bonus::new([true, false, false, false]);
        for _ in 0..VICTORY_TICKS {
            bonus.advance([false; 4]);
        }
        for _ in 1..INSTRUCTION_MAX_TICKS {
            bonus.advance([false, true, false, false]);
            assert_eq!(bonus.phase(), BonusPhase::Instructions);
        }
        bonus.advance([false; 4]);
        assert_eq!(bonus.phase(), BonusPhase::Ready);
    }
}
