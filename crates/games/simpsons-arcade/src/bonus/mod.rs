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

mod opposition;

use std::array::from_fn;

pub use opposition::AutomaticDeparture;
use opposition::Pump;

use super::{Player, stage_card_departed};

// Baseline instruction entry is 953 reference frames after the terminal
// response, of which 128 are the separate wrestler defeat phase.
const VICTORY_TICKS: u16 = 825;
// The activity gate opens after the counter has exceeded 208.
const INSTRUCTION_MIN_TICKS: u16 = 210;
const INSTRUCTION_MAX_TICKS: u16 = 384;
// A 96-tick countdown bracketed by phase setup and pump entry.
const READY_TICKS: u16 = 98;
// Neutral timeout includes the packed-decimal countdown and entry.
const PUMP_CLOCK_TICKS: u16 = 491;
const PUMP_TICKS: u16 = PUMP_CLOCK_TICKS + 1;
const BALLOON_SIZE_LIMIT: u16 = 416;
const BALLOON_NEUTRAL_SIZE: u16 = 160;
const RESULT_GROUP_TICKS: u16 = 14;
const RESULT_REVEAL_HOLD: u16 = 60;
const RESULT_COUNT_TICKS: u16 = 4;
const RESULT_INFLATION_TARGET: u16 = 192;
const RESULT_EXIT_STAGGER: u16 = 15;
const RESULT_EXIT_TICKS: u16 = 160;
const DEPARTURE_HOLD_TICKS: u16 = 32;
const DEPARTURE_SPEED: u32 = 160;
const DEPARTURE_ACCELERATION: u32 = 8;
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
        self.size_units().saturating_sub(BALLOON_NEUTRAL_SIZE)
    }

    const fn new() -> Self {
        Self {
            height: INVERSE_BASE,
            rest: 0,
            turn: 0,
            width: INVERSE_BASE,
        }
    }

    fn size_units(self) -> u16 {
        let [width, _] = self.width.to_be_bytes();
        let [height, _] = self.height.to_be_bytes();
        u16::from(0u8.wrapping_sub(width))
            .saturating_add(u16::from(0u8.wrapping_sub(height)))
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
    fn advance(
        &mut self,
        balloons: &mut [Balloon; 4],
        tick: u16,
        automatic: [bool; 4],
    ) -> bool {
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
                self.scores.into_iter().zip(automatic).all(
                    |(score, automated)| {
                        self.departure_complete(score, automated)
                    },
                )
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

    fn departure_complete(self, score: Option<u8>, automatic: bool) -> bool {
        score.is_none_or(|points| {
            self.stage_tick >= self.exit_ticks(points, automatic)
        })
    }

    fn departure_elapsed(self, score: u8) -> Option<u16> {
        if self.stage != ResultStage::Exit {
            return None;
        }
        self.stage_tick.checked_sub(self.exit_start(score))
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

    fn exit_ticks(self, score: u8, automatic: bool) -> u16 {
        let duration = if automatic {
            opposition::EXIT_TICKS
        } else {
            RESULT_EXIT_TICKS
        };
        self.exit_start(score).saturating_add(duration)
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

    fn visible(self, score: u8, automatic: bool) -> bool {
        self.stage != ResultStage::Exit
            || self.stage_tick < self.exit_ticks(score, automatic)
    }
}

/// Read-only bonus state, in character rather than physical-seat order.
///
/// Axis scheduling remains an independent native choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bonus {
    balloons: [Balloon; 4],
    opponents: Option<[Option<Pump>; 4]>,
    participants: [bool; 4],
    phase: BonusPhase,
    result: Option<ResultSequence>,
    scan_start: usize,
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
                self.scan_start = usize::from(self.tick).rem_euclid(4);
                self.refresh_opponents();
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
        let automatic = self.automatic_mask();
        if let Some(result) = &mut self.result
            && result.advance(&mut self.balloons, self.tick, automatic)
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
        if stage_card_departed(self.tick, active_edge) {
            self.phase = BonusPhase::StageSetup;
            self.tick = 0;
        }
        false
    }

    /// Motion of a ranked automatic competitor, distinct from a human exit.
    #[must_use]
    pub fn automatic_departure(
        self,
        player: Player,
    ) -> Option<AutomaticDeparture> {
        if !self.is_automatic(player) {
            return None;
        }
        let result = self.result?;
        result
            .departure_elapsed(result.score(player)?)
            .map(AutomaticDeparture::at)
    }

    fn automatic_mask(self) -> [bool; 4] {
        self.opponents
            .map_or([false; 4], |pumps| pumps.map(|pump| pump.is_some()))
    }

    /// Whether the automatic pump fired on the current playing tick.
    #[must_use]
    pub fn automatic_pressed(self, player: Player) -> bool {
        self.phase == BonusPhase::Pumping
            && self.tick != 0
            && self
                .opponents
                .and_then(|pumps| pumps.get(player.index()).copied())
                .flatten()
                .is_some_and(Pump::pressed)
    }

    fn contenders(self) -> [bool; 4] {
        let automatic = self.automatic_mask();
        from_fn(|index| {
            self.participants.get(index) == Some(&true)
                || automatic.get(index) == Some(&true)
        })
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
        Some(if self.tick >= PUMP_CLOCK_TICKS {
            0
        } else {
            seconds.saturating_mul(100).saturating_add(hundredths)
        })
    }

    /// Departure motion once this admitted station's ranked delay expires.
    #[must_use]
    pub fn departure(self, player: Player) -> Option<BonusDeparture> {
        if self.is_automatic(player) {
            return None;
        }
        let result = self.result?;
        result
            .departure_elapsed(result.score(player)?)
            .map(BonusDeparture::at)
    }

    /// Revealed rank and animated point count during the results sequence.
    #[must_use]
    pub fn displayed_result(self, player: Player) -> Option<(u8, u8)> {
        let result = self.result?;
        result.displayed(result.score(player)?, self.tick)
    }

    pub(super) fn human_rank(self, player: Player) -> Option<u8> {
        let _human = self
            .participants
            .get(player.index())
            .filter(|admitted| **admitted)?;
        self.rank(player)
    }

    /// Inflation for an admitted character, frozen when pumping finishes.
    #[must_use]
    pub fn inflation(self, player: Player) -> Option<u8> {
        if let Some(result) = self.result {
            return result.score(player);
        }
        self.visual_inflation(player)
    }

    /// Whether this seat's competitor is controlled by the bonus itself.
    #[must_use]
    pub fn is_automatic(self, player: Player) -> bool {
        self.opponents
            .and_then(|pumps| pumps.get(player.index()).copied())
            .flatten()
            .is_some()
    }

    pub(super) const fn new(participants: [bool; 4]) -> Self {
        Self {
            balloons: [Balloon::new(); 4],
            opponents: None,
            participants,
            phase: BonusPhase::Victory,
            result: None,
            scan_start: 0,
            tick: 0,
        }
    }

    /// The current victory or bonus phase.
    #[must_use]
    pub const fn phase(self) -> BonusPhase {
        self.phase
    }

    fn pump(&mut self, presses: [bool; 4]) -> BonusPhase {
        for (index, ((admitted, balloon), pressed)) in self
            .participants
            .iter()
            .zip(&mut self.balloons)
            .zip(presses)
            .enumerate()
        {
            if *admitted {
                balloon.advance(pressed);
            } else if let Some(Some(pump)) = self
                .opponents
                .as_mut()
                .and_then(|pumps| pumps.get_mut(index))
            {
                balloon.advance(pump.advance());
            }
        }
        if self.tick >= PUMP_TICKS
            || self
                .balloons
                .iter()
                .any(|balloon| balloon.size_units() >= BALLOON_SIZE_LIMIT)
        {
            self.result =
                Some(ResultSequence::new(self.balloons, self.contenders()));
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

    fn refresh_opponents(&mut self) {
        if self.opponents.is_some() {
            self.opponents = Some(if self.participants.contains(&true) {
                Pump::roster(self.participants, self.scan_start)
            } else {
                [None; 4]
            });
        }
    }

    // The instructions and ready setup each sample admitted stations.
    // Once the ready countdown starts, the competition roster is frozen.
    pub(super) fn refresh_participants(&mut self, admitted: [bool; 4]) {
        if self.tick == 0
            && matches!(
                self.phase,
                BonusPhase::Instructions | BonusPhase::Ready
            )
        {
            self.participants = admitted;
            self.refresh_opponents();
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

    fn visible_balloon(self, player: Player) -> Option<Balloon> {
        let index = player.index();
        let automatic = self.is_automatic(player);
        if !automatic && self.participants.get(index) != Some(&true) {
            return None;
        }
        if let Some(result) = self.result
            && !result.visible(result.score(player)?, automatic)
        {
            return None;
        }
        self.balloons.get(index).copied()
    }

    /// Visible inverse width and height high bytes, independently of score.
    ///
    /// Absent stations and retired balloons return no visual axes.
    #[must_use]
    pub fn visual_axes(self, player: Player) -> Option<(u8, u8)> {
        let balloon = self.visible_balloon(player)?;
        let [width, _] = balloon.width.to_be_bytes();
        let [height, _] = balloon.height.to_be_bytes();
        Some((width, height))
    }

    /// Visible balloon inflation, which can grow after the score is frozen.
    ///
    /// Absent stations and departed balloons return no visual.
    #[must_use]
    pub fn visual_inflation(self, player: Player) -> Option<u8> {
        self.visible_balloon(player).map(Balloon::inflation)
    }

    pub(super) fn with_opponents(mut self, enabled: bool) -> Self {
        if enabled {
            self.opponents = Some([None; 4]);
            self.refresh_opponents();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy)]
    struct ExpectedScore {
        points: u8,
        rank: u8,
        reveal: u16,
    }

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
    fn competition_roster_resamples_then_freezes_all_station_masks() {
        for mask in 0..16u8 {
            verify_competition_roster(
                Player::ALL.map(|player| mask & (1 << player.index()) != 0),
            );
        }
    }

    #[test]
    fn automatic_rosters_freeze_and_ignore_nonhuman_pump_edges() {
        for mask in 0..16u8 {
            let admitted = from_fn(|index| mask & (1 << index) != 0);
            verify_automatic_roster(admitted);
        }
    }

    fn assert_automatic_roster(bonus: Bonus, admitted: [bool; 4]) {
        let occupied = admitted.contains(&true);
        assert_eq!(
            Player::ALL.map(|player| bonus.is_automatic(player)),
            admitted.map(|human| occupied && !human)
        );
        assert_roster(bonus, [occupied; 4]);
    }

    fn verify_automatic_roster(admitted: [bool; 4]) {
        let other = admitted.map(|human| !human);
        let mut bonus = Bonus::new([false; 4]).with_opponents(true);
        advance_quietly(&mut bonus, VICTORY_TICKS);
        bonus.refresh_participants(other);
        assert_automatic_roster(bonus, other);
        bonus.advance([false; 4], false);
        bonus.refresh_participants(admitted);
        assert_automatic_roster(bonus, other);
        advance_quietly(&mut bonus, INSTRUCTION_MAX_TICKS - 1);
        assert_eq!(bonus.phase(), BonusPhase::Ready);
        bonus.refresh_participants(admitted);
        assert_automatic_roster(bonus, admitted);
        bonus.advance([false; 4], false);
        bonus.refresh_participants(other);
        assert_automatic_roster(bonus, admitted);
        advance_quietly(&mut bonus, READY_TICKS - 1);
        assert_eq!(bonus.phase(), BonusPhase::Pumping);
        assert_automatic_roster(bonus, admitted);

        let mut with_input = bonus;
        for _ in 0..PUMP_TICKS {
            bonus.advance([false; 4], false);
            with_input.refresh_participants(other);
            with_input.advance(other, true);
            assert_eq!(bonus, with_input);
            if bonus.phase() == BonusPhase::Results {
                break;
            }
        }
        assert_eq!(bonus.phase(), BonusPhase::Results);
        assert_automatic_results(bonus, admitted);
    }

    fn assert_automatic_scores(bonus: Bonus, admitted: [bool; 4]) {
        for (player, human) in Player::ALL.into_iter().zip(admitted) {
            assert_eq!(bonus.human_rank(player), human.then_some(4));
            match (human, admitted.contains(&true)) {
                (true, _) => assert_eq!(bonus.inflation(player), Some(0)),
                (false, true) => {
                    assert!(bonus.inflation(player).is_some_and(|n| n > 0));
                },
                (false, false) => assert_eq!(bonus.inflation(player), None),
            }
        }
    }

    fn assert_automatic_results(mut bonus: Bonus, admitted: [bool; 4]) {
        assert_automatic_scores(bonus, admitted);
        let scores = Player::ALL.map(|player| bonus.inflation(player));
        let ranks = Player::ALL.map(|player| bonus.rank(player));
        let mut with_input = bonus;
        for _ in 0..1000u16 {
            bonus.advance([false; 4], false);
            with_input.refresh_participants([true; 4]);
            with_input.advance([true; 4], true);
            assert_eq!(bonus, with_input);
            assert_eq!(
                Player::ALL.map(|player| bonus.inflation(player)),
                scores
            );
            assert_eq!(Player::ALL.map(|player| bonus.rank(player)), ranks);
            if bonus.phase() == BonusPhase::Complete {
                break;
            }
        }
        assert_eq!(bonus.phase(), BonusPhase::Complete);
        assert_eq!(
            Player::ALL.map(|player| bonus.visual_axes(player)),
            [None; 4]
        );
    }

    fn verify_competition_roster(admitted: [bool; 4]) {
        let other = admitted.map(|active| !active);
        let mut bonus = Bonus::new([false; 4]);
        advance_quietly(&mut bonus, VICTORY_TICKS);
        bonus.refresh_participants(other);
        assert_eq!(bonus.phase(), BonusPhase::Instructions);
        assert_roster(bonus, other);
        bonus.advance([false; 4], false);
        bonus.refresh_participants(admitted);
        assert_roster(bonus, other);
        advance_quietly(&mut bonus, INSTRUCTION_MAX_TICKS - 1);
        assert_eq!(bonus.phase(), BonusPhase::Ready);
        bonus.refresh_participants(admitted);
        assert_roster(bonus, admitted);
        bonus.advance([false; 4], false);
        bonus.refresh_participants(other);
        assert_roster(bonus, admitted);
        advance_quietly(&mut bonus, READY_TICKS - 1);
        assert_eq!(bonus.phase(), BonusPhase::Pumping);
        bonus.refresh_participants(other);
        bonus.advance([true; 4], true);
        assert_eq!(
            Player::ALL.map(|player| bonus.inflation(player)),
            admitted.map(|active| active.then_some(3))
        );
        advance_quietly(&mut bonus, PUMP_TICKS - 1);
        assert_eq!(bonus.phase(), BonusPhase::Results);
        bonus.refresh_participants(other);
        assert_roster(bonus, admitted);
    }

    fn assert_roster(bonus: Bonus, expected: [bool; 4]) {
        assert_eq!(
            Player::ALL.map(|player| bonus.inflation(player).is_some()),
            expected
        );
    }

    #[test]
    fn pump_clock_matches_decimal_carry_and_timeout() {
        let mut bonus = start_pump([true; 4]);
        let mut seconds = 10u16;
        let mut hundredths = 0u16;
        for _ in 0..PUMP_TICKS {
            assert_eq!(
                bonus.countdown(),
                Some(seconds.saturating_mul(100).saturating_add(hundredths))
            );
            if hundredths > 2 {
                hundredths = hundredths.saturating_sub(2);
            } else if seconds > 0 {
                seconds = seconds.saturating_sub(1);
                hundredths = 98;
            } else {
                hundredths = 0;
            }
            bonus.advance([false; 4], false);
        }
        assert_eq!(seconds, 0);
        assert_eq!(hundredths, 0);
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
    fn visual_axes_expose_independent_growth_without_score_rounding() {
        let mut bonus = start_pump([true, true, false, false]);
        assert_eq!(bonus.visual_axes(Player::Marge), Some((0xb0, 0xb0)));
        assert_eq!(bonus.visual_axes(Player::Bart), None);
        bonus.advance([true, false, false, false], true);
        assert_eq!(bonus.visual_axes(Player::Marge), Some((0xa2, 0xb0)));
        bonus.advance([true, false, false, false], true);
        assert_eq!(bonus.visual_axes(Player::Marge), Some((0xa2, 0xa2)));
        assert_eq!(bonus.visual_axes(Player::Homer), Some((0xb0, 0xb0)));
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
    fn inverse_axis_matches_bounded_reference_domain() {
        for minimum in [0x2000, 0x4000] {
            verify_axis_domain(minimum, true, 0x1c00);
            verify_axis_domain(minimum, false, 0x1800);
        }
    }

    fn verify_axis_domain(minimum: u16, growing: bool, strength: u64) {
        for initial in minimum..=0xb000 {
            // Independent widened arithmetic models the two truncated
            // products and the strict, rather than inclusive, clamp.
            let value = u64::from(initial);
            let squared = value.saturating_pow(2).checked_div(65_536);
            let delta = squared
                .unwrap_or_default()
                .saturating_mul(strength)
                .checked_div(65_536)
                .unwrap_or_default();
            let candidate = if growing {
                value.saturating_sub(delta)
            } else {
                value.saturating_add(delta)
            };
            let expected = candidate.clamp(u64::from(minimum), 0xb000);
            let mut actual = initial;
            let within = Balloon::adjust(&mut actual, minimum, growing);
            assert_eq!(u64::from(actual), expected);
            assert_eq!(within, candidate == expected);
        }
    }

    #[test]
    fn size_limit_checks_every_station_without_score_rounding()
    -> Result<(), &'static str> {
        let initial = start_pump([true; 4]);
        for station in 0..4 {
            for (width, height, expected) in [
                (0x4000, 0x2000, BonusPhase::Results),
                (0x40ff, 0x20ff, BonusPhase::Results),
                (0x4100, 0x2000, BonusPhase::Pumping),
                (0x4000, 0x2100, BonusPhase::Pumping),
                (0x4100, 0x2100, BonusPhase::Pumping),
                (0xb000, 0xb000, BonusPhase::Pumping),
            ] {
                let mut bonus = initial;
                let balloon =
                    bonus.balloons.get_mut(station).ok_or("station")?;
                // Hold the sampled size through the next update.
                balloon.width = width;
                balloon.height = height;
                balloon.rest = 1;
                bonus.advance([false; 4], false);
                (bonus.phase() == expected)
                    .then_some(())
                    .ok_or("unexpected size-limit outcome")?;
            }
        }
        Ok(())
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
    fn departures_match_every_ranked_motion_tick() {
        let motion = expected_departure_motion();
        // All station subsets, tied scores, four distinct ranks and
        // zero-score ties exercise the independent ranked delays.
        for points in [[64, 0, 32, 32], [64, 32, 16, 8], [0; 4], [1; 4]] {
            for mask in 0u8..16 {
                assert_departure_timeline(masked_scores(points, mask), &motion);
            }
        }
    }

    fn masked_scores(points: [u8; 4], mut mask: u8) -> [Option<u8>; 4] {
        points.map(|score| {
            let admitted = mask & 1 != 0;
            mask >>= 1u32;
            admitted.then_some(score)
        })
    }

    fn expected_departure_motion() -> [BonusDeparture; 159] {
        // Integrate the observed velocities one tick at a time rather
        // than reuse the closed-form displacement calculation.
        let mut balloon = 0u32;
        let mut character = 0u32;
        let mut velocity = 160u32;
        from_fn(|elapsed| {
            if elapsed >= 34 {
                velocity = velocity.saturating_add(8);
            }
            if elapsed >= 2 {
                balloon = balloon.saturating_add(velocity);
            }
            if elapsed >= 33 {
                character = character.saturating_add(velocity);
            }
            BonusDeparture::Rising { balloon, character }
        })
    }

    fn expected_departure(
        group: Option<ExpectedScore>,
        tick: u16,
        motion: &[BonusDeparture; 159],
    ) -> Option<BonusDeparture> {
        let elapsed =
            tick.checked_sub(u16::from(group?.rank).saturating_mul(15))?;
        Some(
            motion
                .get(usize::from(elapsed))
                .copied()
                .unwrap_or(BonusDeparture::Departed),
        )
    }

    fn assert_departure_timeline(
        scores: [Option<u8>; 4],
        motion: &[BonusDeparture; 159],
    ) {
        let mut ordered: Vec<_> = scores.into_iter().flatten().collect();
        ordered.sort_unstable_by(|left, right| right.cmp(left));
        ordered.dedup();
        let groups = expected_result_groups(scores, &ordered);
        let mut bonus = Bonus::new(scores.map(|score| score.is_some()));
        bonus.phase = BonusPhase::Results;
        bonus.result = Some(ResultSequence {
            scores,
            stage: ResultStage::Exit,
            stage_tick: 0,
        });
        let end = groups
            .into_iter()
            .flatten()
            .map(|group| {
                160u16.saturating_add(u16::from(group.rank).saturating_mul(15))
            })
            .max()
            .unwrap_or(1);
        for tick in 0..=end {
            for (player, group) in Player::ALL.into_iter().zip(groups) {
                assert_eq!(
                    bonus.departure(player),
                    expected_departure(group, tick, motion),
                );
                assert_eq!(
                    bonus.inflation(player),
                    group.map(|sample| sample.points),
                );
            }
            assert_eq!(
                bonus.phase(),
                if tick == end {
                    BonusPhase::Complete
                } else {
                    BonusPhase::Results
                },
            );
            bonus.advance([true; 4], true);
        }
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
    fn results_preserve_rank_groups_across_all_station_orderings() {
        // Absence, zero, four ordered positive scores and the maximum
        // exercise all tie and admission patterns in cabinet order.
        let scores = [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(BONUS_MAX_INFLATION),
        ];
        for ordinal in 0..scores.len().pow(4) {
            let mut remaining = ordinal;
            let sample = from_fn(|_| {
                let digit =
                    remaining.checked_rem(scores.len()).unwrap_or_default();
                remaining =
                    remaining.checked_div(scores.len()).unwrap_or_default();
                scores.get(digit).copied().flatten()
            });
            assert_result_groups(sample);
        }
    }

    fn assert_result_groups(scores: [Option<u8>; 4]) {
        let mut ordered: Vec<_> = scores.into_iter().flatten().collect();
        ordered.sort_unstable_by(|left, right| right.cmp(left));
        ordered.dedup();
        let mut bonus = Bonus::new(scores.map(|score| score.is_some()));
        bonus.phase = BonusPhase::Results;
        bonus.result = Some(ResultSequence {
            scores,
            stage: ResultStage::Reveal,
            stage_tick: 0,
        });
        let groups = expected_result_groups(scores, &ordered);
        let group_count = u16::try_from(ordered.len()).unwrap_or_default();
        let first_count = 61u16.saturating_add(group_count.saturating_mul(14));
        let maximum = ordered.first().copied().unwrap_or_default();
        let last_count =
            first_count.saturating_add(u16::from(maximum).saturating_mul(4));
        let mut count = 0u8;
        let mut next_count = first_count.saturating_add(4);
        for tick in 0..=last_count {
            if tick == next_count {
                count = count.saturating_add(1);
                next_count = next_count.saturating_add(4);
            }
            assert_result_readouts(bonus, groups, tick, count);
            bonus.advance([true; 4], true);
        }
    }

    fn expected_result_groups(
        scores: [Option<u8>; 4],
        ordered: &[u8],
    ) -> [Option<ExpectedScore>; 4] {
        let mut groups = [None; 4];
        let mut reveal = 1u16;
        for (index, group) in ordered.iter().enumerate() {
            let rank = if *group == 0 {
                4
            } else {
                u8::try_from(index.saturating_add(1)).unwrap_or_default()
            };
            for (expected, _) in groups
                .iter_mut()
                .zip(scores)
                .filter(|(_, score)| *score == Some(*group))
            {
                *expected = Some(ExpectedScore {
                    points: *group,
                    rank,
                    reveal,
                });
            }
            reveal = reveal.saturating_add(14);
        }
        groups
    }

    fn assert_result_readouts(
        bonus: Bonus,
        groups: [Option<ExpectedScore>; 4],
        tick: u16,
        count: u8,
    ) {
        for (player, group) in Player::ALL.into_iter().zip(groups) {
            assert_eq!(bonus.rank(player), group.map(|sample| sample.rank));
            let expected = group
                .filter(|sample| tick >= sample.reveal)
                .map(|sample| (sample.rank, count.min(sample.points)));
            assert_eq!(
                bonus.displayed_result(player),
                expected,
                "player {player:?}, tick {tick}",
            );
        }
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
