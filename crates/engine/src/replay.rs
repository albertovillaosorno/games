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
#[path = "../../../tests/engine/replay.rs"]
mod tests;
