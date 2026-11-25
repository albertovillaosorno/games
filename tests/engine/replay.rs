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
//   - Seeded, replayable public-action behaviour regression scenarios.
// - Must-Not:
//   - Freeze implementation details or assert unverified original gameplay.
// - Allows:
//   - Inputs: Synthetic actions, known seeds and frame time partitions.
//   - Outputs: Observable state and deterministic property assertions.
//   - Side effects: None beyond in-memory simulation.
// - Split-When:
//   - Real title-owned scenario fixtures require independent ownership.
// - Merge-When:
//   - Another reusable public-action test suite covers this contract.
// - Summary:
//   - Deterministic scripted-input and frame-pacing property tests.
// - Description:
//   - Compares final public state across generated render schedules.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Fixed seeds; no GPU, wall clock, network or original media.
//

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
            match action {
                Action::Move(delta) => {
                    self.position =
                        self.position.saturating_add(i64::from(*delta));
                },
                Action::Shoot => {
                    self.shots = self.shots.saturating_add(1);
                },
            }
        }
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
        let length = (random(&mut state) & 31).saturating_add(1).min(remaining);
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
        let fine =
            play(recording.clone(), repeat_n(Duration::from_millis(1), 1_000));
        let variable = play(recording, frames(seed));
        assert!(fine.is_ok(), "fixed-step seed {seed}");
        assert_eq!(fine, variable, "render pacing changed replay at {seed}");
    }
}

fn short_replay() -> Result<ObservableState, StepError> {
    let mut recording = InputScript::new();
    recording.record(0, Action::Move(3));
    recording.record(0, Action::Shoot);
    recording.record(2, Action::Move(-1));
    let budget = NonZeroU32::new(4).ok_or(StepError::InvalidInterval)?;
    let mut clock = StepClock::new(Duration::from_millis(10), budget)?;
    let mut simulation = Scripted::new(ObservableState::default(), recording);
    let _report = clock.advance(Duration::from_millis(30), &mut simulation)?;
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
