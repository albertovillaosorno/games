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
//   - Deterministic live inputs, replay scripts and observable checkpoints.
// - Must-Not:
//   - Encode gameplay rules, transport protocols or renderer behaviour.
// - Allows:
//   - Inputs: Ordered actions keyed by simulation tick.
//   - Outputs: Replayable actions, inspectable state and checkpoint results.
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
//   - Compose Scripted or LiveSimulation with the canonical StepClock.
// - Defaults:
//   - Empty input on unrecorded ticks; insertion order within a tick.
//
//! Drive live and replayed public actions independently of frame rate.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt::{Debug, Formatter, Result as FormatResult};
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::timing::{FixedTick, StepClock, StepError, StepReport};

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

/// Explicit bounds for actions waiting to enter fixed simulation ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveInputLimits {
    /// Maximum tick distance from the next unexecuted tick, inclusive.
    pub future_ticks: u64,
    /// Maximum total queued action count, regardless of their target ticks.
    pub pending_actions: NonZeroUsize,
}

/// Why a complete live action batch was rejected without being enqueued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueRejection {
    /// The batch would exceed the configured action count.
    Capacity,
    /// The batch is beyond the future window or cannot execute.
    Future,
    /// The requested simulation tick has already executed.
    Late,
}

/// A rejected batch returned to its caller without losing any input.
#[derive(Debug, Eq, PartialEq)]
pub struct RejectedInput<Input> {
    /// Original ordered actions, unchanged.
    pub actions: Vec<Input>,
    /// The reason no action was admitted.
    pub reason: QueueRejection,
}

/// Admission either succeeds or returns the complete rejected batch.
pub type EnqueueResult<Input> = Result<(), RejectedInput<Input>>;

/// Own the fixed clock, bounded live action queue and public simulation.
#[derive(Debug)]
pub struct LiveSimulation<Simulation: InputStep> {
    clock: StepClock,
    inputs: InputScript<Simulation::Input>,
    limits: LiveInputLimits,
    pending: usize,
    state: Simulation,
}

struct LiveConsumer<'live, Simulation: InputStep> {
    inputs: &'live mut InputScript<Simulation::Input>,
    pending: &'live mut usize,
    state: &'live mut Simulation,
}

impl<Simulation: InputStep> FixedTick for LiveConsumer<'_, Simulation> {
    fn fixed_tick(&mut self, index: u64) {
        if let Some(actions) = self.inputs.by_tick.remove(&index) {
            *self.pending = self.pending.saturating_sub(actions.len());
            self.state.input_step(index, &actions);
        } else {
            self.state.input_step(index, &[]);
        }
    }
}

impl<Simulation: InputStep> LiveSimulation<Simulation> {
    /// Execute eligible fixed updates and consume only their queued actions.
    ///
    /// # Errors
    /// A clock error leaves both queued input and simulation unchanged.
    pub fn advance(
        &mut self,
        elapsed: Duration,
    ) -> Result<StepReport, StepError> {
        let mut consumer = LiveConsumer {
            inputs: &mut self.inputs,
            pending: &mut self.pending,
            state: &mut self.state,
        };
        self.clock.advance(elapsed, &mut consumer)
    }

    /// Admit a complete ordered action batch for a specific unexecuted tick.
    ///
    /// Repeated batches for one tick retain insertion order. The caller
    /// retains ownership of the batch on rejection and may handle or retry
    /// it explicitly; dropping a rejected release could leave an action held.
    ///
    /// # Errors
    /// Rejects a late tick, excessive lookahead or insufficient queue space.
    pub fn enqueue(
        &mut self,
        tick: u64,
        actions: Vec<Simulation::Input>,
    ) -> EnqueueResult<Simulation::Input> {
        let room = self
            .limits
            .pending_actions
            .get()
            .saturating_sub(self.pending);
        let rejection = match tick.checked_sub(self.clock.tick_index()) {
            None => Some(QueueRejection::Late),
            Some(distance)
                if tick == u64::MAX || distance > self.limits.future_ticks =>
            {
                Some(QueueRejection::Future)
            },
            Some(_) if actions.len() > room => Some(QueueRejection::Capacity),
            Some(_) => None,
        };
        if let Some(reason) = rejection {
            return Err(RejectedInput { actions, reason });
        }
        if !actions.is_empty() {
            self.pending = self.pending.saturating_add(actions.len());
            match self.inputs.by_tick.entry(tick) {
                Entry::Occupied(mut entry) => entry.get_mut().extend(actions),
                Entry::Vacant(entry) => {
                    let _stored = entry.insert(actions);
                },
            }
        }
        Ok(())
    }

    /// Admit actions according to their timestamp from the clock's zero.
    ///
    /// A timestamp's target tick is independent of render cadence. Late
    /// events are not silently moved to a different tick; the host can
    /// explicitly retry the returned batch on the next unexecuted tick.
    /// # Errors
    /// Rejects late, unrepresentable, too-far-future or oversized batches.
    pub fn enqueue_at(
        &mut self,
        elapsed: Duration,
        actions: Vec<Simulation::Input>,
    ) -> EnqueueResult<Simulation::Input> {
        let Some(tick) = self.clock.tick_at(elapsed) else {
            return Err(RejectedInput {
                actions,
                reason: QueueRejection::Future,
            });
        };
        self.enqueue(tick, actions)
    }

    /// Start with a simulation aligned to the supplied fixed clock.
    #[must_use]
    pub const fn new(
        state: Simulation,
        clock: StepClock,
        limits: LiveInputLimits,
    ) -> Self {
        Self {
            clock,
            inputs: InputScript::new(),
            limits,
            pending: 0,
            state,
        }
    }

    /// Return the next fixed tick that can receive actions.
    #[must_use]
    pub const fn next_tick(&self) -> u64 {
        self.clock.tick_index()
    }

    /// Return the count of admitted actions not yet delivered.
    #[must_use]
    pub const fn pending_actions(&self) -> usize {
        self.pending
    }

    /// Inspect simulation state after the most recent executed tick.
    #[must_use]
    pub const fn state(&self) -> &Simulation {
        &self.state
    }
}

/// Failure to match a sequence of reference observations after fixed ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointError {
    /// The replay has not yet reached a required reference tick.
    Incomplete(u64),
    /// A reference must contain at least one strictly ordered checkpoint.
    InvalidSequence,
    /// The expected observation differed from the actual state at this tick.
    Mismatch(u64),
    /// The simulation skipped a required reference tick.
    Skipped(u64),
}

/// Compare a scripted simulation against title-defined public observations.
///
/// Checkpoints are borrowed and sampled only after their indexed updates.
/// The first divergence is retained across subsequent rendered frames.
pub struct CheckpointReplay<'reference, Simulation, Observation, Observe>
where
    Simulation: InputStep,
{
    failure: Option<CheckpointError>,
    observe: Observe,
    remaining: &'reference [(u64, Observation)],
    scripted: Scripted<Simulation>,
}

impl<'reference, Simulation, Observation, Observe>
    CheckpointReplay<'reference, Simulation, Observation, Observe>
where
    Simulation: InputStep,
    Observation: PartialEq,
    Observe: Fn(&Simulation) -> Observation,
{
    /// Inspect the first mismatch, skipped tick or pending checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error until every checkpoint is reached and matches.
    pub const fn finish(&self) -> Result<(), CheckpointError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if let Some((tick, _expected)) = self.remaining.first() {
            return Err(CheckpointError::Incomplete(*tick));
        }
        Ok(())
    }

    /// Construct a replay using public observations in ascending tick order.
    ///
    /// # Errors
    ///
    /// Rejects empty, duplicate or decreasing checkpoint indices.
    pub fn new(
        scripted: Scripted<Simulation>,
        checkpoints: &'reference [(u64, Observation)],
        observe: Observe,
    ) -> Result<Self, CheckpointError> {
        if checkpoints.is_empty()
            || !checkpoints
                .iter()
                .map(|(tick, _expected)| *tick)
                .is_sorted_by(|left, right| left < right)
        {
            return Err(CheckpointError::InvalidSequence);
        }
        Ok(Self {
            failure: None,
            observe,
            remaining: checkpoints,
            scripted,
        })
    }

    /// Inspect the current simulation independently of comparison status.
    #[must_use]
    pub const fn state(&self) -> &Simulation {
        self.scripted.state()
    }
}

impl<Simulation, Observation, Observe> Debug
    for CheckpointReplay<'_, Simulation, Observation, Observe>
where
    Simulation: InputStep,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> FormatResult {
        f.debug_struct("CheckpointReplay")
            .field("failure", &self.failure)
            .field("remaining", &self.remaining.len())
            .finish_non_exhaustive()
    }
}

impl<Simulation, Observation, Observe> FixedTick
    for CheckpointReplay<'_, Simulation, Observation, Observe>
where
    Simulation: InputStep,
    Observation: PartialEq,
    Observe: Fn(&Simulation) -> Observation,
{
    fn fixed_tick(&mut self, index: u64) {
        self.scripted.fixed_tick(index);
        if self.failure.is_some() {
            return;
        }
        let Some(((tick, expected), remaining)) = self.remaining.split_first()
        else {
            return;
        };
        if index > *tick {
            self.failure = Some(CheckpointError::Skipped(*tick));
        } else if index == *tick {
            if (self.observe)(self.scripted.state()) == *expected {
                self.remaining = remaining;
            } else {
                self.failure = Some(CheckpointError::Mismatch(*tick));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::iter::repeat_n;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use super::{
        CheckpointError, CheckpointReplay, InputScript, InputStep,
        LiveInputLimits, LiveSimulation, QueueRejection, RejectedInput,
        Scripted,
    };
    use crate::timing::{FixedTick as _, StepClock, StepError};

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

    type OrderedActions = Vec<(u64, Vec<Action>)>;

    #[derive(Debug, Default, Eq, PartialEq)]
    struct OrderedState(OrderedActions);

    impl InputStep for OrderedState {
        type Input = Action;

        fn input_step(&mut self, tick: u64, actions: &[Action]) {
            self.0.push((tick, actions.to_vec()));
        }
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

    fn checkpoint_script() -> Scripted<ObservableState> {
        let mut recording = InputScript::new();
        recording.record(0, Action::Move(3));
        recording.record(1, Action::Shoot);
        recording.record(2, Action::Move(-1));
        Scripted::new(ObservableState::default(), recording)
    }

    fn observation(state: &ObservableState) -> (i64, u64) {
        (state.position, state.shots)
    }

    #[test]
    fn compares_reference_after_ticks_across_render_partitions() {
        let checkpoints = [(0, (3, 0)), (1, (3, 1)), (2, (2, 1))];
        for frame_lengths in [[10u64, 10, 10], [25, 0, 5]] {
            let replay = CheckpointReplay::new(
                checkpoint_script(),
                &checkpoints,
                observation,
            );
            let budget = NonZeroU32::new(2);
            assert!(replay.is_ok());
            assert!(budget.is_some());
            let (Ok(mut comparison), Some(limit)) = (replay, budget) else {
                return;
            };
            let clock = StepClock::new(Duration::from_millis(10), limit);
            assert!(clock.is_ok());
            let Ok(mut scheduler) = clock else {
                return;
            };
            assert_eq!(
                comparison.finish(),
                Err(CheckpointError::Incomplete(0))
            );
            for elapsed in frame_lengths {
                assert!(
                    scheduler
                        .advance(
                            Duration::from_millis(elapsed),
                            &mut comparison
                        )
                        .is_ok()
                );
            }
            assert_eq!(comparison.finish(), Ok(()));
            assert_eq!(comparison.state().ticks, 3);
        }
    }

    #[test]
    fn retains_first_mismatch_even_when_final_state_matches() {
        let checkpoints = [(0, (7, 0)), (2, (2, 1))];
        let replay = CheckpointReplay::new(
            checkpoint_script(),
            &checkpoints,
            observation,
        );
        assert!(replay.is_ok());
        if let Ok(mut comparison) = replay {
            for tick in 0u64..3u64 {
                comparison.fixed_tick(tick);
            }
            assert_eq!(observation(comparison.state()), (2, 1));
            assert_eq!(comparison.finish(), Err(CheckpointError::Mismatch(0)));
        }
    }

    #[test]
    fn refuses_empty_duplicate_and_out_of_order_references() {
        let duplicate = [(2, (0, 0)), (2, (0, 0))];
        let reversed = [(2, (0, 0)), (1, (0, 0))];
        for checkpoints in [duplicate.as_slice(), reversed.as_slice(), &[]] {
            assert_eq!(
                CheckpointReplay::new(
                    checkpoint_script(),
                    checkpoints,
                    observation
                )
                .err(),
                Some(CheckpointError::InvalidSequence),
            );
        }
    }

    #[test]
    fn distinguishes_pending_and_skipped_checkpoints() {
        let checkpoints = [(0, (3, 0)), (1, (3, 1))];
        let replay = CheckpointReplay::new(
            checkpoint_script(),
            &checkpoints,
            observation,
        );
        assert!(replay.is_ok());
        if let Ok(mut comparison) = replay {
            comparison.fixed_tick(0);
            assert_eq!(
                comparison.finish(),
                Err(CheckpointError::Incomplete(1))
            );
            comparison.fixed_tick(2);
            assert_eq!(comparison.finish(), Err(CheckpointError::Skipped(1)));
        }
    }

    fn live_clock() -> Option<StepClock> {
        let clock = StepClock::new(
            Duration::from_millis(10),
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        );
        assert!(clock.is_ok(), "test clock must be valid");
        clock.ok()
    }

    fn limits(actions: usize, future_ticks: u64) -> LiveInputLimits {
        LiveInputLimits {
            future_ticks,
            pending_actions: NonZeroUsize::new(actions)
                .unwrap_or(NonZeroUsize::MIN),
        }
    }

    fn queue_example(live: &mut LiveSimulation<OrderedState>) {
        assert_eq!(live.enqueue(2, vec![Action::Shoot]), Ok(()));
        assert_eq!(live.enqueue(0, vec![Action::Move(3)]), Ok(()));
        assert_eq!(live.enqueue(0, vec![Action::Shoot]), Ok(()));
        assert_eq!(live.enqueue(2, vec![Action::Move(-2)]), Ok(()));
        assert_eq!(live.pending_actions(), 4);
    }

    #[test]
    fn live_actions_match_replay_across_render_partitions() {
        let Some(clock) = live_clock() else {
            return;
        };
        let mut live =
            LiveSimulation::new(OrderedState::default(), clock, limits(4, 2));
        queue_example(&mut live);
        let mut script = InputScript::new();
        script.record(0, Action::Move(3));
        script.record(0, Action::Shoot);
        script.record(2, Action::Shoot);
        script.record(2, Action::Move(-2));
        let mut replay = Scripted::new(OrderedState::default(), script);
        let mut replay_clock = clock;
        for elapsed in [25u64, 0, 5] {
            assert!(live.advance(Duration::from_millis(elapsed)).is_ok());
        }
        for elapsed in [1u64; 30] {
            assert!(
                replay_clock
                    .advance(Duration::from_millis(elapsed), &mut replay,)
                    .is_ok()
            );
        }
        assert_eq!(live.state(), replay.state());
        assert_eq!(live.state().0, [
            (0, vec![Action::Move(3), Action::Shoot]),
            (1, vec![]),
            (2, vec![Action::Shoot, Action::Move(-2)]),
        ]);
        assert_eq!(live.next_tick(), 3);
        assert_eq!(live.pending_actions(), 0);
    }

    fn reject_before_advance(live: &mut LiveSimulation<OrderedState>) {
        assert_eq!(live.enqueue(1, vec![Action::Move(1)]), Ok(()));
        assert_eq!(
            live.enqueue(0, vec![Action::Move(2), Action::Shoot]),
            Err(RejectedInput {
                actions: vec![Action::Move(2), Action::Shoot],
                reason: QueueRejection::Capacity,
            })
        );
        assert_eq!(
            live.enqueue(2, vec![Action::Shoot]),
            Err(RejectedInput {
                actions: vec![Action::Shoot],
                reason: QueueRejection::Future,
            })
        );
        assert_eq!(live.enqueue(1, vec![]), Ok(()));
        assert_eq!(live.pending_actions(), 1);
    }

    #[test]
    fn rejected_batches_are_returned_and_never_partially_enqueued() {
        let Some(clock) = live_clock() else {
            return;
        };
        let mut live =
            LiveSimulation::new(OrderedState::default(), clock, limits(2, 1));
        reject_before_advance(&mut live);
        assert!(live.advance(Duration::from_millis(10)).is_ok());
        assert_eq!(
            live.enqueue(0, vec![Action::Shoot]),
            Err(RejectedInput {
                actions: vec![Action::Shoot],
                reason: QueueRejection::Late,
            })
        );
        assert_eq!(live.enqueue(2, vec![Action::Shoot]), Ok(()));
        assert!(live.advance(Duration::from_millis(20)).is_ok());
        assert_eq!(live.state().0, [
            (0, vec![]),
            (1, vec![Action::Move(1)]),
            (2, vec![Action::Shoot]),
        ]);
        assert_eq!(live.pending_actions(), 0);
        assert_eq!(live.enqueue(3, vec![Action::Shoot]), Ok(()));
    }

    #[test]
    fn clock_error_preserves_live_state_and_pending_actions() {
        let Some(clock) = live_clock() else {
            return;
        };
        let mut live =
            LiveSimulation::new(OrderedState::default(), clock, limits(1, 0));
        assert_eq!(live.enqueue(0, vec![Action::Shoot]), Ok(()));
        assert!(live.advance(Duration::from_millis(1)).is_ok());
        assert_eq!(live.advance(Duration::MAX), Err(StepError::TimeOverflow));
        assert_eq!(live.next_tick(), 0);
        assert_eq!(live.pending_actions(), 1);
        assert_eq!(live.state(), &OrderedState::default());
        assert!(live.advance(Duration::from_millis(9)).is_ok());
        assert_eq!(live.state().0, [(0, vec![Action::Shoot])]);
        assert_eq!(live.pending_actions(), 0);
    }

    #[test]
    fn timestamped_actions_match_across_frame_partitions() {
        let Some(clock) = live_clock() else {
            return;
        };
        let expected = [
            (0, vec![Action::Move(3)]),
            (1, vec![Action::Shoot, Action::Move(-2)]),
            (2, vec![]),
        ];
        for frames in [[10u64, 10, 10], [25, 0, 5]] {
            let mut live = LiveSimulation::new(
                OrderedState::default(),
                clock,
                limits(3, 2),
            );
            for (millis, action) in [
                (9, Action::Move(3)),
                (10, Action::Shoot),
                (19, Action::Move(-2)),
            ] {
                assert_eq!(
                    live.enqueue_at(Duration::from_millis(millis), vec![
                        action
                    ]),
                    Ok(())
                );
            }
            for elapsed in frames {
                assert!(live.advance(Duration::from_millis(elapsed)).is_ok());
            }
            assert_eq!(live.state().0, expected);
            for (timestamp, reason) in [
                (Duration::from_millis(29), QueueRejection::Late),
                (Duration::MAX, QueueRejection::Future),
            ] {
                assert_eq!(
                    live.enqueue_at(timestamp, vec![Action::Shoot]),
                    Err(RejectedInput {
                        actions: vec![Action::Shoot],
                        reason,
                    })
                );
            }
            assert_eq!(live.pending_actions(), 0);
        }
    }

    #[test]
    fn rejects_a_tick_that_cannot_have_a_representable_successor() {
        let Some(clock) = live_clock() else {
            return;
        };
        let mut live = LiveSimulation::new(
            OrderedState::default(),
            clock,
            limits(1, u64::MAX),
        );
        assert_eq!(
            live.enqueue(u64::MAX, vec![Action::Shoot]),
            Err(RejectedInput {
                actions: vec![Action::Shoot],
                reason: QueueRejection::Future,
            })
        );
        assert_eq!(live.pending_actions(), 0);
    }
}
