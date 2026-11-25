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
//   - Scheduling regression tests across independent game consumers.
// - Must-Not:
//   - Assume original game assets or couple updates to renderer state.
// - Allows:
//   - Inputs: Synthetic elapsed time, seeded actions and consumer state.
//   - Outputs: Publicly observable replay and fixed-tick assertions.
//   - Side effects: None beyond synthetic in-memory simulation.
// - Split-When:
//   - A separate reusable gameplay harness requires new fixtures.
// - Merge-When:
//   - Shared deterministic simulation tests cover the same contracts.
// - Summary:
//   - Synthetic frame pacing and cross-consumer regression checks.
// - Description:
//   - Runs the same clock with independent, deterministic consumers.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No real clock, GPU, original media or network required.
//

use std::num::NonZeroU32;
use std::time::Duration;

use super::{FixedTick, StepClock, StepError};

#[derive(Debug, Default, Eq, PartialEq)]
struct Counter {
    count: u64,
    observed: Vec<u64>,
}

impl FixedTick for Counter {
    fn fixed_tick(&mut self, index: u64) {
        self.count = self.count.saturating_add(1);
        self.observed.push(index);
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Motion {
    position: i64,
    updates: u64,
}

impl FixedTick for Motion {
    fn fixed_tick(&mut self, _index: u64) {
        self.position = self.position.saturating_add(3);
        self.updates = self.updates.saturating_add(1);
    }
}

#[derive(Debug, Eq, PartialEq)]
struct MotionOutcome {
    backlog: Duration,
    state: Motion,
}

fn schedule(
    interval: Duration,
    budget: u32,
    frames: impl IntoIterator<Item = Duration>,
) -> Result<Counter, StepError> {
    let maximum = NonZeroU32::new(budget).ok_or(StepError::InvalidInterval)?;
    let mut clock = StepClock::new(interval, maximum)?;
    let mut state = Counter::default();
    for elapsed in frames {
        let _report = clock.advance(elapsed, &mut state)?;
    }
    Ok(state)
}

#[test]
fn different_render_frame_rates_produce_identical_fixed_updates() {
    let interval = Duration::from_millis(10);
    let fast = schedule(interval, 100, [Duration::from_millis(1); 200]);
    let slow = schedule(interval, 100, [Duration::from_millis(40); 5]);
    assert!(fast.is_ok(), "small frames should be schedulable");
    assert!(slow.is_ok(), "large frames should be schedulable");
    assert_eq!(fast, slow, "render pacing must not change simulation");
}

#[test]
fn catches_up_across_frames_without_dropping_time() {
    let outcome = replay_motion();
    assert_eq!(
        outcome,
        Ok(MotionOutcome {
            backlog: Duration::ZERO,
            state: Motion {
                position: 33,
                updates: 11
            },
        }),
        "backlog must be preserved and fully replayed",
    );
}

fn replay_motion() -> Result<MotionOutcome, StepError> {
    let budget = NonZeroU32::new(3).ok_or(StepError::InvalidInterval)?;
    let mut clock = StepClock::new(Duration::from_millis(10), budget)?;
    let mut state = Motion::default();
    let _initial_report =
        clock.advance(Duration::from_millis(110), &mut state)?;
    for _ in 0u8..3u8 {
        let _backlog_report = clock.advance(Duration::ZERO, &mut state)?;
    }
    Ok(MotionOutcome {
        backlog: clock.pending_time(),
        state,
    })
}

#[test]
fn rejects_zero_interval_and_keeps_fractional_remainder() {
    let maximum = NonZeroU32::new(5);
    assert!(maximum.is_some(), "five is nonzero");
    if let Some(budget) = maximum {
        assert_eq!(
            StepClock::new(Duration::ZERO, budget),
            Err(StepError::InvalidInterval),
        );
        if let Ok(mut clock) = StepClock::new(Duration::from_millis(10), budget)
        {
            let mut state = Counter::default();
            let report = clock.advance(Duration::from_millis(17), &mut state);
            assert!(report.is_ok(), "nonzero elapsed time");
            assert_eq!(clock.pending_time(), Duration::from_millis(7));
            assert_eq!(state.count, 1);
        }
    }
}

#[test]
fn long_frame_uses_budget_without_losing_time() {
    let budget = NonZeroU32::new(100);
    if let Some(limit) = budget {
        let new_clock = StepClock::new(Duration::from_micros(10), limit);
        if let Ok(mut clock) = new_clock {
            let mut counter = Counter::default();
            let first = clock.advance(Duration::from_secs(1), &mut counter);
            assert_eq!(first.map(|report| report.updates), Ok(100));
            assert_eq!(clock.pending_time(), Duration::from_millis(999));
            let second = clock.advance(Duration::ZERO, &mut counter);
            assert_eq!(second.map(|report| report.updates), Ok(100));
            assert_eq!(counter.count, 200);
            assert_eq!(clock.tick_index(), 200);
        }
    }
}
