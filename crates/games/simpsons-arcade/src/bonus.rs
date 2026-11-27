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
//   - Outputs: Phase, independent inflation, tied ranks and result timing.
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
const RESULT_GROUP_TICKS: u16 = 14;
const RESULT_REVEAL_HOLD: u16 = 60;
const RESULT_COUNT_TICKS: u16 = 4;
const RESULT_INFLATION_TARGET: u16 = 192;
const RESULT_EXIT_STAGGER: u16 = 15;
const RESULT_EXIT_TICKS: u16 = 160;
const DEPARTURE_HOLD_TICKS: u16 = 32;
const DEPARTURE_SPEED: u32 = 160;
const DEPARTURE_ACCELERATION: u32 = 8;
const STAGE_CARD_MIN_TICKS: u16 = 65;
const STAGE_CARD_MAX_TICKS: u16 = 192;
// Separate measured setup phase; the entrance starts with its own tick 0.
const STAGE_SETUP_TICKS: u16 = 11;
const STAGE_SETUP_BLANK: u16 = 2;
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
    /// Results remain available when no subsequent stage is selected.
    Complete,
    /// Explain the button-pumping interaction.
    Instructions,
    /// Admit independent pump button edges.
    Pumping,
    /// Countdown before the pumping interval.
    Ready,
    /// Reveal ranked scores, inflate balloons and depart.
    Results,
    /// Await the next stage through a bounded, skippable title card.
    StageCard,
    /// Finish the noninteractive scene setup after the title card.
    StageSetup,
    /// Hold the cleared street before the bonus.
    Victory,
}

/// Read-only motion of a ranked participant during the bonus departure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BonusDeparture {
    /// The balloon and character have retired; the pump stays on the ground.
    Departed,
    /// The character is attached to the balloon independently of the pump.
    Rising {
        /// Balloon displacement above its starting height, in 1/256 pixels.
        balloon: u32,
        /// Character displacement above its starting height, in 1/256 pixels.
        character: u32,
    },
}

impl BonusDeparture {
    fn at(elapsed: u16) -> Self {
        if elapsed >= RESULT_EXIT_TICKS - 1 {
            return Self::Departed;
        }
        let (balloon, character) = Self::displacement(elapsed);
        Self::Rising { balloon, character }
    }

    fn displacement(elapsed: u16) -> (u32, u32) {
        // The balloon advances at constant speed before the linked character
        // begins moving. Both accelerate together after the hold.
        let constant_ticks = u32::from(elapsed.saturating_sub(1));
        let acceleration_ticks =
            u32::from(elapsed.saturating_sub(DEPARTURE_HOLD_TICKS + 1));
        let acceleration = acceleration_ticks
            .saturating_mul(acceleration_ticks.saturating_add(1))
            .saturating_mul(DEPARTURE_ACCELERATION >> 1u32);
        let balloon = constant_ticks
            .saturating_mul(DEPARTURE_SPEED)
            .saturating_add(acceleration);
        let character = balloon.saturating_sub(
            u32::from(DEPARTURE_HOLD_TICKS - 1).saturating_mul(DEPARTURE_SPEED),
        );
        (balloon, character)
    }
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
        u8::try_from(self.inflation_units() >> 2).unwrap_or_default()
    }

    fn inflation_units(self) -> u16 {
        let [width, _] = self.width.to_be_bytes();
        let [height, _] = self.height.to_be_bytes();
        let total = u16::from(0u8.wrapping_sub(width))
            .saturating_add(u16::from(0u8.wrapping_sub(height)));
        total.saturating_sub(160)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResultStage {
    Exit,
    Inflate,
    Reveal,
}

/// Competition scores are frozen before the presentation inflates balloons.
/// Each distinct score gets its own reveal, followed by parallel count-up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResultSequence {
    scores: [Option<u8>; 4],
    stage: ResultStage,
    stage_tick: u16,
}

impl ResultSequence {
    fn advance(&mut self, balloons: &mut [Balloon; 4], tick: u16) -> bool {
        match self.stage {
            ResultStage::Reveal => {
                if tick >= self.countup_end() {
                    self.stage = ResultStage::Inflate;
                }
                false
            },
            ResultStage::Inflate => {
                self.inflate(balloons);
                false
            },
            ResultStage::Exit => {
                self.stage_tick = self.stage_tick.saturating_add(1);
                self.scores
                    .into_iter()
                    .flatten()
                    .all(|score| self.stage_tick >= self.exit_ticks(score))
            },
        }
    }

    fn countup_end(self) -> u16 {
        let maximum = self.scores.into_iter().flatten().max().unwrap_or(0);
        self.countup_start().saturating_add(
            u16::from(maximum)
                .saturating_add(1)
                .saturating_mul(RESULT_COUNT_TICKS),
        )
    }

    fn countup_start(self) -> u16 {
        u16::from(self.distinct_scores(None))
            .saturating_mul(RESULT_GROUP_TICKS)
            .saturating_add(RESULT_REVEAL_HOLD)
            .saturating_add(1)
    }

    fn departure(self, score: u8) -> Option<BonusDeparture> {
        if self.stage != ResultStage::Exit {
            return None;
        }
        self.stage_tick
            .checked_sub(self.exit_start(score))
            .map(BonusDeparture::at)
    }

    fn displayed(self, score: u8, tick: u16) -> Option<(u8, u8)> {
        let rank = self.rank(score);
        if self.stage != ResultStage::Reveal {
            return Some((rank, score));
        }
        let reveal = u16::from(self.group(score).saturating_sub(1))
            .saturating_mul(RESULT_GROUP_TICKS)
            .saturating_add(1);
        if tick < reveal {
            return None;
        }
        let count = tick
            .saturating_sub(self.countup_start())
            .checked_div(RESULT_COUNT_TICKS)
            .unwrap_or_default();
        Some((rank, u8::try_from(count.min(u16::from(score))).ok()?))
    }

    fn distinct_scores(self, above: Option<u8>) -> u8 {
        let bits = self
            .scores
            .into_iter()
            .flatten()
            .filter(|score| above.is_none_or(|limit| *score > limit))
            .fold(0u128, |bits, score| {
                bits | 1u128.checked_shl(u32::from(score)).unwrap_or_default()
            });
        u8::try_from(bits.count_ones()).unwrap_or_default()
    }

    fn exit_start(self, score: u8) -> u16 {
        u16::from(self.rank(score)).saturating_mul(RESULT_EXIT_STAGGER)
    }

    fn exit_ticks(self, score: u8) -> u16 {
        self.exit_start(score).saturating_add(RESULT_EXIT_TICKS)
    }

    fn group(self, score: u8) -> u8 {
        self.distinct_scores(Some(score)).saturating_add(1)
    }

    fn inflate(&mut self, balloons: &mut [Balloon; 4]) {
        let mut pending = false;
        for (score, balloon) in self.scores.into_iter().zip(balloons) {
            if score.is_some()
                && balloon.inflation_units() <= RESULT_INFLATION_TARGET
            {
                pending = true;
                balloon.advance(self.stage_tick.is_multiple_of(2));
            }
        }
        if pending {
            self.stage_tick = self.stage_tick.saturating_add(1);
        } else {
            self.stage = ResultStage::Exit;
            self.stage_tick = 0;
        }
    }

    fn new(balloons: [Balloon; 4], participants: [bool; 4]) -> Self {
        let mut scores = [None; 4];
        for ((score, balloon), admitted) in
            scores.iter_mut().zip(balloons).zip(participants)
        {
            if admitted {
                *score = Some(balloon.inflation());
            }
        }
        Self {
            scores,
            stage: ResultStage::Reveal,
            stage_tick: 0,
        }
    }

    fn rank(self, score: u8) -> u8 {
        if score == 0 {
            4
        } else {
            self.group(score)
        }
    }

    fn score(self, player: Player) -> Option<u8> {
        self.scores.get(player.index()).copied().flatten()
    }

    fn visible(self, score: u8) -> bool {
        self.stage != ResultStage::Exit
            || self.stage_tick < self.exit_ticks(score)
    }
}

/// Read-only bonus state, in character rather than physical-seat order.
///
/// Axis scheduling remains an independent native choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bonus {
    balloons: [Balloon; 4],
    participants: [bool; 4],
    phase: BonusPhase,
    result: Option<ResultSequence>,
    tick: u16,
}

impl Bonus {
    // Live action edges advance instructions; individual competition
    // edges affect pumping only through the ready-entry snapshot.
    pub(super) fn advance(&mut self, presses: [bool; 4], active_edge: bool) {
        if matches!(
            self.phase,
            BonusPhase::Complete
                | BonusPhase::StageCard
                | BonusPhase::StageSetup
        ) {
            return;
        }
        self.tick = self.tick.saturating_add(1);
        let next = match self.phase {
            BonusPhase::Victory if self.tick >= VICTORY_TICKS => {
                BonusPhase::Instructions
            },
            BonusPhase::Instructions
                if self.tick >= INSTRUCTION_MAX_TICKS
                    || (self.tick >= INSTRUCTION_MIN_TICKS && active_edge) =>
            {
                BonusPhase::Ready
            },
            BonusPhase::Ready if self.tick >= READY_TICKS => {
                BonusPhase::Pumping
            },
            BonusPhase::Pumping => self.pump(presses),
            BonusPhase::Results => self.advance_results(),
            BonusPhase::Complete
            | BonusPhase::Instructions
            | BonusPhase::Ready
            | BonusPhase::StageCard
            | BonusPhase::StageSetup
            | BonusPhase::Victory => self.phase,
        };
        if next != self.phase {
            self.phase = next;
            self.tick = 0;
        }
    }

    fn advance_results(&mut self) -> BonusPhase {
        if let Some(result) = &mut self.result
            && result.advance(&mut self.balloons, self.tick)
        {
            BonusPhase::Complete
        } else {
            BonusPhase::Results
        }
    }

    /// Advance using a fresh action edge from a live campaign station.
    pub(super) fn advance_stage_card(&mut self, active_edge: bool) -> bool {
        if self.phase == BonusPhase::Complete {
            self.phase = BonusPhase::StageCard;
            self.tick = 0;
            return false;
        }
        self.tick = self.tick.saturating_add(1);
        if self.phase == BonusPhase::StageSetup {
            return self.tick >= STAGE_SETUP_TICKS;
        }
        if self.tick >= STAGE_CARD_MAX_TICKS
            || (self.tick >= STAGE_CARD_MIN_TICKS && active_edge)
        {
            self.phase = BonusPhase::StageSetup;
            self.tick = 0;
        }
        false
    }

    /// The displayed pumping countdown in centiseconds, if pumping.
    ///
    /// The original display advances by two hundredths each tick and
    /// carries to the next second on the tick reaching zero hundredths.
    #[must_use]
    pub fn countdown(self) -> Option<u16> {
        if self.phase != BonusPhase::Pumping {
            return None;
        }
        if self.tick == 0 {
            return Some(1000);
        }
        let elapsed = self.tick.saturating_sub(1);
        let seconds = 9u16.saturating_sub(elapsed.checked_div(49)?);
        let hundredths =
            98u16.saturating_sub(elapsed.checked_rem(49)?.saturating_mul(2));
        Some(if self.tick >= 491 {
            0
        } else {
            seconds.saturating_mul(100).saturating_add(hundredths)
        })
    }

    /// Departure motion once this admitted station's ranked delay expires.
    #[must_use]
    pub fn departure(self, player: Player) -> Option<BonusDeparture> {
        let result = self.result?;
        result.departure(result.score(player)?)
    }

    /// Revealed rank and animated point count during the results sequence.
    #[must_use]
    pub fn displayed_result(self, player: Player) -> Option<(u8, u8)> {
        let result = self.result?;
        result.displayed(result.score(player)?, self.tick)
    }

    /// Inflation for an admitted character, frozen when pumping finishes.
    #[must_use]
    pub fn inflation(self, player: Player) -> Option<u8> {
        if let Some(result) = self.result {
            return result.score(player);
        }
        self.visual_inflation(player)
    }

    pub(super) const fn new(participants: [bool; 4]) -> Self {
        Self {
            balloons: [Balloon::new(); 4],
            participants,
            phase: BonusPhase::Victory,
            result: None,
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
            self.result =
                Some(ResultSequence::new(self.balloons, self.participants));
            BonusPhase::Results
        } else {
            BonusPhase::Pumping
        }
    }

    /// Final rank after pumping ends.
    ///
    /// Equal positive scores share a rank. Distinct positive scores receive
    /// consecutive ranks; zero receives fourth place.
    #[must_use]
    pub fn rank(self, player: Player) -> Option<u8> {
        let result = self.result?;
        Some(result.rank(result.score(player)?))
    }

    // The instructions and ready setup each sample admitted stations.
    // Once the ready countdown starts, the competition roster is frozen.
    pub(super) const fn refresh_participants(&mut self, admitted: [bool; 4]) {
        if self.tick == 0
            && matches!(
                self.phase,
                BonusPhase::Instructions | BonusPhase::Ready
            )
        {
            self.participants = admitted;
        }
    }

    pub(super) const fn stage_setup_blank(self) -> bool {
        matches!(self.phase, BonusPhase::StageSetup)
            && self.tick >= STAGE_SETUP_BLANK
    }

    /// Fixed playing ticks in the current phase.
    #[must_use]
    pub const fn tick(self) -> u16 {
        self.tick
    }

    /// Visible balloon inflation, which can grow after the score is frozen.
    ///
    /// Absent stations and departed balloons return no visual.
    #[must_use]
    pub fn visual_inflation(self, player: Player) -> Option<u8> {
        let index = player.index();
        let _admitted = self
            .participants
            .get(index)
            .copied()
            .filter(|admitted| *admitted)?;
        if let Some(result) = self.result
            && !result.visible(result.score(player)?)
        {
            return None;
        }
        self.balloons.get(index).copied().map(Balloon::inflation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_pump(participants: [bool; 4]) -> Bonus {
        let mut bonus = Bonus::new(participants);
        for _ in 0..VICTORY_TICKS {
            bonus.advance([true; 4], true);
        }
        assert_eq!(bonus.phase(), BonusPhase::Instructions);
        for _ in 0..INSTRUCTION_MIN_TICKS {
            bonus.advance([true; 4], true);
        }
        assert_eq!(bonus.phase(), BonusPhase::Ready);
        for _ in 0..READY_TICKS {
            bonus.advance([true; 4], true);
        }
        assert_eq!(bonus.phase(), BonusPhase::Pumping);
        assert_eq!(bonus.inflation(Player::Marge), Some(0));
        bonus
    }

    #[test]
    fn pump_clock_matches_decimal_carry_and_timeout() {
        let mut bonus = start_pump([true; 4]);
        for tick in 0..PUMP_TICKS {
            let expected = match tick {
                0 => Some(1000),
                1 => Some(998),
                49 => Some(902),
                50 => Some(898),
                489 => Some(4),
                490 => Some(2),
                491 => Some(0),
                _ => None,
            };
            if let Some(value) = expected {
                assert_eq!(bonus.countdown(), Some(value));
            }
            bonus.advance([false; 4], false);
        }
        assert_eq!(bonus.countdown(), None);
        assert_eq!(bonus.phase(), BonusPhase::Results);
    }

    #[test]
    fn idle_timeout_preserves_zero_and_tied_ranks() {
        let mut bonus = start_pump([true, true, false, true]);
        for _ in 0..PUMP_TICKS {
            bonus.advance([false, false, true, false], false);
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_eq!(bonus.inflation(Player::Bart), None);
        assert_eq!(bonus.rank(Player::Marge), Some(4));
        assert_eq!(bonus.rank(Player::Lisa), Some(4));
        for _ in 0..600u16 {
            if bonus.phase() == BonusPhase::Complete {
                break;
            }
            bonus.advance([true; 4], true);
        }
        assert_eq!(bonus.phase(), BonusPhase::Complete);
        let result = bonus;
        bonus.advance([true; 4], true);
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
            let rapid_edge = tick.is_multiple_of(2);
            let sparse_edge = tick.is_multiple_of(24);
            rapid.advance([rapid_edge, false, false, false], rapid_edge);
            sparse.advance([sparse_edge, false, false, false], sparse_edge);
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
            bonus.advance([true, false, true, tick.is_multiple_of(2)], true);
            if bonus.phase() == BonusPhase::Results {
                break;
            }
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_eq!(bonus.inflation(Player::Marge), Some(BONUS_MAX_INFLATION));
        assert_eq!(bonus.rank(Player::Marge), Some(1));
        assert_eq!(bonus.rank(Player::Bart), Some(1));
        assert_eq!(bonus.rank(Player::Lisa), Some(2));
        assert_eq!(bonus.rank(Player::Homer), Some(4));
    }

    fn advance_quietly(bonus: &mut Bonus, ticks: u16) {
        for _ in 0..ticks {
            bonus.advance([false; 4], false);
        }
    }

    fn assert_departed(bonus: Bonus) {
        assert_eq!(bonus.phase(), BonusPhase::Complete);
        assert_eq!(bonus.visual_inflation(Player::Homer), None);
        assert_eq!(
            bonus.departure(Player::Homer),
            Some(BonusDeparture::Departed)
        );
        assert_eq!(
            bonus.departure(Player::Marge),
            Some(BonusDeparture::Departed)
        );
        assert_eq!(bonus.departure(Player::Bart), None);
        assert_eq!(bonus.displayed_result(Player::Marge), Some((1, 64)));
    }

    fn assert_other_balloon_grew(bonus: Bonus) {
        assert!(matches!(
            bonus.visual_inflation(Player::Homer),
            Some(48..=64)
        ));
    }

    fn assert_results_unchanged(bonus: Bonus, with_input: Bonus) {
        assert_eq!(bonus, with_input);
        assert_eq!(bonus.inflation(Player::Homer), Some(0));
        assert_eq!(bonus.displayed_result(Player::Homer), Some((4, 0)));
        assert_eq!(bonus.visual_inflation(Player::Bart), None);
    }

    fn winning_results() -> Bonus {
        let mut bonus = start_pump([true, true, false, false]);
        for tick in 0..200u16 {
            let edge = tick.is_multiple_of(2);
            bonus.advance([edge, false, false, false], edge);
            if bonus.phase() == BonusPhase::Results {
                break;
            }
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_eq!(bonus.inflation(Player::Marge), Some(64));
        assert_eq!(bonus.rank(Player::Homer), Some(4));
        bonus
    }

    #[test]
    fn departure_matches_fixed_point_hold_and_acceleration() {
        let checkpoints = [
            (0, 0, 0),
            (1, 0, 0),
            (2, 160, 0),
            (32, 4_960, 0),
            (33, 5_120, 160),
            (34, 5_288, 328),
            (159, 0x184c8 - 0x2800, 0x184c8 - 0x3b60),
        ];
        for (elapsed, balloon, character) in checkpoints {
            assert_eq!(
                BonusDeparture::displacement(elapsed),
                (balloon, character)
            );
        }
        assert!(matches!(
            BonusDeparture::at(158),
            BonusDeparture::Rising { .. }
        ));
        assert_eq!(BonusDeparture::at(159), BonusDeparture::Departed);
        assert_eq!(BonusDeparture::at(160), BonusDeparture::Departed);
    }

    #[test]
    fn departures_are_rank_staggered_and_exclude_nonparticipants() {
        let mut bonus = winning_results();
        let mut first = None;
        let mut fourth = None;
        for elapsed in 0..800u16 {
            assert_eq!(bonus.departure(Player::Bart), None);
            if first.is_none() && bonus.departure(Player::Marge).is_some() {
                first = Some(elapsed);
            }
            if fourth.is_none() && bonus.departure(Player::Homer).is_some() {
                fourth = Some(elapsed);
            }
            if bonus.phase() == BonusPhase::Complete {
                break;
            }
            bonus.advance([false; 4], false);
        }
        assert_eq!(
            fourth
                .zip(first)
                .map(|(late, early)| late.saturating_sub(early)),
            Some(45)
        );
        assert_departed(bonus);
    }

    #[test]
    fn results_reveal_then_count_without_changing_scores() {
        let mut bonus = winning_results();
        assert_eq!(bonus.displayed_result(Player::Marge), None);
        bonus.advance([false; 4], false);
        assert_eq!(bonus.displayed_result(Player::Marge), Some((1, 0)));
        assert_eq!(bonus.displayed_result(Player::Homer), None);
        advance_quietly(&mut bonus, 14);
        assert_eq!(bonus.displayed_result(Player::Homer), Some((4, 0)));
        advance_quietly(&mut bonus, 78);
        assert_eq!(bonus.displayed_result(Player::Marge), Some((1, 1)));
        assert_eq!(bonus.visual_inflation(Player::Homer), Some(0));
        advance_quietly(&mut bonus, 256);
        assert_eq!(bonus.displayed_result(Player::Marge), Some((1, 64)));
        assert_eq!(bonus.visual_inflation(Player::Homer), Some(0));
    }

    #[test]
    fn results_grow_and_depart_independently_of_input() {
        let mut bonus = winning_results();
        advance_quietly(&mut bonus, 349);
        let mut with_input = bonus;
        let mut first_departure = None;
        let mut last_departure = None;
        for elapsed in 0..400u16 {
            bonus.advance([false; 4], false);
            with_input.advance([true; 4], true);
            assert_results_unchanged(bonus, with_input);
            if bonus.visual_inflation(Player::Marge).is_none()
                && first_departure.is_none()
            {
                first_departure = Some(elapsed);
                assert_other_balloon_grew(bonus);
            }
            if bonus.phase() == BonusPhase::Complete {
                last_departure = Some(elapsed);
                break;
            }
        }
        assert_eq!(
            last_departure
                .zip(first_departure)
                .map(|(last, first)| last.saturating_sub(first)),
            Some(45)
        );
        assert_departed(bonus);
    }

    #[test]
    fn instruction_timeout_cannot_be_skipped_by_absent_station() {
        let mut bonus = Bonus::new([true, false, false, false]);
        for _ in 0..VICTORY_TICKS {
            bonus.advance([false; 4], false);
        }
        for _ in 1..INSTRUCTION_MAX_TICKS {
            bonus.advance([false, true, false, false], false);
            assert_eq!(bonus.phase(), BonusPhase::Instructions);
        }
        bonus.advance([false; 4], false);
        assert_eq!(bonus.phase(), BonusPhase::Ready);
    }
}
