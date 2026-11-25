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
//   - Reusable deterministic input scripts for fixed-tick simulation.
// - Must-Not:
//   - Encode gameplay rules, transport protocols or renderer behaviour.
// - Allows:
//   - Inputs: Ordered actions keyed by simulation tick.
//   - Outputs: Replayable action slices and inspectable simulation state.
//   - Side effects: Delegates each tick to the supplied state machine.
// - Split-When:
//   - Real network transport or storage needs an independent adapter.
// - Merge-When:
//   - Shared input processing supersedes this replay adapter.
// - Summary:
//   - Deterministic scripted-input adapter.
// - Description:
//   - Feeds recorded public actions into the fixed-step scheduler.
// - Usage:
//   - Compose Scripted with a StepClock and InputStep implementation.
// - Defaults:
//   - Empty input on unrecorded ticks; insertion order within a tick.
//
//! Replay public action scripts without coupling to rendering frame rate.

use std::collections::BTreeMap;

use crate::timing::FixedTick;

/// A title-independent state machine driven by explicit public actions.
pub trait InputStep {
    /// The public action a consumer accepts for one simulation update.
    type Input;

    /// Apply ordered actions from a particular indexed simulation tick.
    fn input_step(&mut self, tick: u64, actions: &[Self::Input]);
}

/// Recorded public actions grouped by fixed simulation tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputScript<Input> {
    by_tick: BTreeMap<u64, Vec<Input>>,
}

impl<Input> Default for InputScript<Input> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Input> InputScript<Input> {
    /// Retrieve recorded actions for a tick in their original input order.
    #[must_use]
    pub fn actions(&self, tick: u64) -> &[Input] {
        self.by_tick.get(&tick).map_or(&[], Vec::as_slice)
    }

    /// Create an empty action recording.
    #[must_use]
    pub const fn new() -> Self {
        Self { by_tick: BTreeMap::new() }
    }

    /// Record another action without changing any existing action order.
    pub fn record(&mut self, tick: u64, input: Input) {
        self.by_tick.entry(tick).or_default().push(input);
    }
}

/// Connect a public action-driven simulation to the canonical fixed clock.
#[derive(Debug)]
pub struct Scripted<Simulation: InputStep> {
    inputs: InputScript<Simulation::Input>,
    state: Simulation,
}

impl<Simulation: InputStep> Scripted<Simulation> {
    /// Construct a reusable scripted simulation from a state and recording.
    pub const fn new(
        state: Simulation,
        inputs: InputScript<Simulation::Input>,
    ) -> Self {
        Self { inputs, state }
    }

    /// Inspect public state following a sequence of fixed simulation ticks.
    #[must_use]
    pub const fn state(&self) -> &Simulation {
        &self.state
    }
}

impl<Simulation: InputStep> FixedTick for Scripted<Simulation> {
    fn fixed_tick(&mut self, index: u64) {
        self.state.input_step(index, self.inputs.actions(index));
    }
}

#[cfg(test)]
mod tests {
    use std::iter::repeat_n;
    use std::num::NonZeroU32;
    use std::time::Duration;

    use super::{InputScript, InputStep, Scripted};
    use crate::timing::{StepClock, StepError};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Action {
        Move(i8),
        Shoot,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    struct ObservableState {
        position: i64,
        shots: u64,
        ticks: u64,
    }

    impl InputStep for ObservableState {
        type Input = Action;

        fn input_step(&mut self, _tick: u64, actions: &[Action]) {
            self.ticks = self.ticks.saturating_add(1);
            for action in actions {
                apply(self, action);
            }
        }
    }

    fn apply(state: &mut ObservableState, action: &Action) {
        match action {
            Action::Move(delta) => {
                state.position =
                    state.position.saturating_add(i64::from(*delta));
            },
            Action::Shoot => {
                state.shots = state.shots.saturating_add(1);
            },
        }
    }

    fn random(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        *state
    }

    fn script(seed: u64) -> InputScript<Action> {
        let mut recording = InputScript::new();
        let mut state = seed;
        for tick in 0u64..100u64 {
            if random(&mut state).trailing_zeros() >= 2 {
                recording.record(tick, Action::Shoot);
            }
            if random(&mut state) & 1 == 0 {
                recording.record(tick, Action::Move(2));
            } else {
                recording.record(tick, Action::Move(-1));
            }
        }
        recording
    }

    fn frames(seed: u64) -> Vec<Duration> {
        let mut state = seed;
        let mut remaining = 1_000u64;
        let mut result = Vec::new();
        while remaining > 0 {
            let length =
                (random(&mut state) & 31).saturating_add(1).min(remaining);
            result.push(Duration::from_millis(length));
            remaining = remaining.saturating_sub(length);
        }
        result
    }

    fn play(
        recording: InputScript<Action>,
        frames: impl IntoIterator<Item = Duration>,
    ) -> Result<ObservableState, StepError> {
        let budget = NonZeroU32::new(100).ok_or(StepError::InvalidInterval)?;
        let mut clock = StepClock::new(Duration::from_millis(10), budget)?;
        let mut state = Scripted::new(ObservableState::default(), recording);
        for elapsed in frames {
            let _report = clock.advance(elapsed, &mut state)?;
        }
        Ok(state.state().clone())
    }

    #[test]
    fn recorded_actions_keep_order_and_idle_ticks_are_empty() {
        let mut recording = InputScript::new();
        recording.record(3, Action::Move(7));
        recording.record(3, Action::Shoot);
        recording.record(3, Action::Move(-2));
        assert_eq!(recording.actions(3), &[
            Action::Move(7),
            Action::Shoot,
            Action::Move(-2)
        ],);
        assert_eq!(recording.actions(2), &[]);
    }

    #[test]
    fn seeded_replays_preserve_observable_state_across_frame_partitions() {
        for seed in 0u64..128u64 {
            let recording = script(seed);
            let fine = play(
                recording.clone(),
                repeat_n(Duration::from_millis(1), 1_000),
            );
            let variable = play(recording, frames(seed));
            assert!(fine.is_ok(), "fixed-step seed {seed}");
            assert_eq!(
                fine, variable,
                "render pacing changed replay at {seed}"
            );
        }
    }

    fn short_replay() -> Result<ObservableState, StepError> {
        let mut recording = InputScript::new();
        recording.record(0, Action::Move(3));
        recording.record(0, Action::Shoot);
        recording.record(2, Action::Move(-1));
        let budget = NonZeroU32::new(4).ok_or(StepError::InvalidInterval)?;
        let mut clock = StepClock::new(Duration::from_millis(10), budget)?;
        let mut simulation =
            Scripted::new(ObservableState::default(), recording);
        let _report =
            clock.advance(Duration::from_millis(30), &mut simulation)?;
        Ok(simulation.state().clone())
    }

    #[test]
    fn scripted_public_actions_reach_observable_state() {
        assert_eq!(
            short_replay(),
            Ok(ObservableState {
                position: 2,
                shots: 1,
                ticks: 3
            }),
        );
    }
}
