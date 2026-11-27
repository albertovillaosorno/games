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

// Baseline instruction entry is 953 reference frames after the terminal
// response, of which 128 are the separate wrestler defeat phase.
const VICTORY_TICKS: u16 = 825;
// The activity gate opens after the counter has exceeded 208.
const INSTRUCTION_MIN_TICKS: u16 = 210;
const INSTRUCTION_MAX_TICKS: u16 = 384;
// A 96-tick countdown bracketed by phase setup and pump entry.
const READY_TICKS: u16 = 98;
// Neutral timeout includes the packed-decimal countdown and entry.
const PUMP_TICKS: u16 = 492;
const RESULT_TICKS: u16 = 480;
const REST_TICKS: u8 = 16;
const INVERSE_BASE: u16 = 0xb000;
const MIN_WIDTH: u16 = 0x4000;
const MIN_HEIGHT: u16 = 0x2000;
const PUMP_STRENGTH: u32 = 0x1c00;
const LEAK_STRENGTH: u32 = 0x1800;

/// Maximum balloon inflation score.
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

/// Inverse balloon axes in fixed-point units, with a brief leak delay.
/// Axis selection is deterministic rather than emulating the board's RNG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Balloon {
    height: u16,
    rest: u8,
    turn: u8,
    width: u16,
}

impl Balloon {
    // The observed response is quadratic in the current inverse axis.
    // Two fixed-point products retain the intermediate truncation.
    fn adjust(axis: &mut u16, minimum: u16, growing: bool) -> bool {
        let inverse = u32::from(*axis);
        let squared = inverse.saturating_mul(inverse) >> 16u32;
        let strength = if growing {
            PUMP_STRENGTH
        } else {
            LEAK_STRENGTH
        };
        let delta = u16::try_from(squared.saturating_mul(strength) >> 16u32)
            .unwrap_or_default();
        let candidate = if growing {
            axis.saturating_sub(delta)
        } else {
            axis.saturating_add(delta)
        };
        let bounded = candidate.clamp(minimum, INVERSE_BASE);
        *axis = bounded;
        bounded == candidate
    }

    fn advance(&mut self, pressed: bool) {
        if pressed {
            self.rest = REST_TICKS;
        } else if self.rest != 0 {
            self.rest = self.rest.saturating_sub(1);
            return;
        }
        // Prefer the vertical axis in three out of four updates, without
        // coupling stations through an unrelated global random stream.
        let width_first = self.turn == 0;
        self.turn = self.turn.wrapping_add(1) & 3;
        if width_first {
            if !Self::adjust(&mut self.width, MIN_WIDTH, pressed) {
                let _bounded =
                    Self::adjust(&mut self.height, MIN_HEIGHT, pressed);
            }
        } else if !Self::adjust(&mut self.height, MIN_HEIGHT, pressed) {
            let _bounded = Self::adjust(&mut self.width, MIN_WIDTH, pressed);
        }
    }

    fn inflation(self) -> u8 {
        let [width, _] = self.width.to_be_bytes();
        let [height, _] = self.height.to_be_bytes();
        let total = u16::from(0u8.wrapping_sub(width))
            .saturating_add(u16::from(0u8.wrapping_sub(height)));
        u8::try_from(total.saturating_sub(160) >> 2).unwrap_or_default()
    }

    const fn new() -> Self {
        Self {
            height: INVERSE_BASE,
            rest: 0,
            turn: 0,
            width: INVERSE_BASE,
        }
    }
}

/// Read-only bonus state, in character rather than physical-seat order.
///
/// Axis scheduling and result duration are provisional; the bonus score
/// uses the observed inverse-axis bounds and response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bonus {
    balloons: [Balloon; 4],
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
        self.balloons.get(index).copied().map(Balloon::inflation)
    }

    pub(super) const fn new(participants: [bool; 4]) -> Self {
        Self {
            balloons: [Balloon::new(); 4],
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
        for ((admitted, balloon), pressed) in self
            .participants
            .iter()
            .zip(&mut self.balloons)
            .zip(presses)
        {
            if *admitted {
                balloon.advance(pressed);
            }
        }
        if self.tick >= PUMP_TICKS
            || self
                .balloons
                .iter()
                .any(|balloon| balloon.inflation() == BONUS_MAX_INFLATION)
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
    fn single_pump_retains_size_then_leaks_to_neutral() {
        let mut balloon = Balloon::new();
        balloon.advance(true);
        assert_eq!(balloon.width, 0xa2c4);
        assert_eq!(balloon.inflation(), 3);
        for _ in 0..REST_TICKS {
            balloon.advance(false);
            assert_eq!(balloon.inflation(), 3);
        }
        balloon.advance(false);
        assert_eq!(balloon.width, 0xac77);
        assert_eq!(balloon.inflation(), 1);
        balloon.advance(false);
        assert_eq!(balloon.inflation(), 0);
    }

    #[test]
    fn rapid_edges_end_early_but_sparse_edges_do_not() {
        let mut rapid = start_pump([true, true, false, false]);
        let mut sparse = rapid;
        for tick in 0..200u16 {
            rapid.advance([tick.is_multiple_of(2), false, false, false]);
            sparse.advance([tick.is_multiple_of(24), false, false, false]);
            if rapid.phase() == BonusPhase::Results {
                break;
            }
        }
        assert_eq!(rapid.phase(), BonusPhase::Results);
        assert_eq!(rapid.inflation(Player::Marge), Some(BONUS_MAX_INFLATION));
        assert_eq!(rapid.rank(Player::Marge), Some(1));
        assert_eq!(rapid.inflation(Player::Homer), Some(0));
        assert_eq!(sparse.phase(), BonusPhase::Pumping);
        assert!(matches!(sparse.inflation(Player::Marge), Some(0..=3)));
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
