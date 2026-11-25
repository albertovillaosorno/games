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
//   - Canonical deterministic fixed-update scheduling.
// - Must-Not:
//   - Own title rules, rendering or operating-system clocks.
// - Allows:
//   - Inputs: Explicit elapsed durations and configurable tick policy.
//   - Outputs: Ordered updates and remaining simulation time.
//   - Side effects: Calls supplied simulation callback.
// - Split-When:
//   - A genuinely different scheduling contract becomes necessary.
// - Merge-When:
//   - Another shared mechanism duplicates fixed-step scheduling.
// - Summary:
//   - Frame-rate-independent simulation scheduling.
// - Description:
//   - Advances a shared fixed clock without discarding unprocessed time.
// - Usage:
//   - Configure a fixed interval and invoke advance for every frame.
// - Defaults:
//   - Never sample the host clock or couple updates to rendered frames.
//

//! Deterministic, configurable scheduling shared by independent game rules.

use std::num::{NonZeroU32, NonZeroU128};
use std::time::Duration;

/// A simulation that advances through indexed, fixed-duration ticks.
///
/// The same tick index must produce the same observable transition when
/// a consumer receives identical initial state and external inputs.
pub trait FixedTick {
    /// Advance simulation once, independently of rendering frequency.
    fn fixed_tick(&mut self, index: u64);
}

/// Failures that would otherwise violate deterministic scheduling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepError {
    /// A zero duration cannot define a simulation interval.
    InvalidInterval,
    /// The indexed tick counter cannot represent another update.
    TickOverflow,
    /// The accumulated elapsed time exceeded `Duration` capacity.
    TimeOverflow,
}

/// A deterministic fixed-step scheduler with a bounded per-frame work budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepClock {
    accumulated: Duration,
    interval: Duration,
    max_updates: NonZeroU32,
    next_tick: u64,
}

/// Observable scheduling results for one frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepReport {
    /// Unprocessed elapsed time remains available for future frames.
    pub backlog: Duration,
    /// The index of the next unexecuted simulation update.
    pub next_tick: u64,
    /// The number of fixed simulation updates executed for this frame.
    pub updates: u32,
}

impl StepClock {
    /// Advance elapsed time and deliver ordered fixed ticks to the consumer.
    ///
    /// Exceeding the per-frame budget retains rather than discards extra time.
    /// A subsequent frame with zero elapsed duration can drain that backlog.
    ///
    /// # Errors
    ///
    /// Returns an error without changing clock or consumer state if elapsed
    /// time cannot be accumulated or the tick counter would overflow.
    pub fn advance<Consumer>(
        &mut self,
        elapsed: Duration,
        consumer: &mut Consumer,
    ) -> Result<StepReport, StepError>
    where
        Consumer: FixedTick,
    {
        let available_time = self
            .accumulated
            .checked_add(elapsed)
            .ok_or(StepError::TimeOverflow)?;
        let eligible = available_time
            .as_nanos()
            .checked_div(self.interval.as_nanos())
            .unwrap_or(0)
            .min(NonZeroU128::from(self.max_updates).get());
        let updates = u32::try_from(eligible)
            .map_err(|_error| StepError::TickOverflow)?;
        let elapsed_ticks = self
            .interval
            .checked_mul(updates)
            .ok_or(StepError::TimeOverflow)?;
        let remaining = available_time
            .checked_sub(elapsed_ticks)
            .ok_or(StepError::TimeOverflow)?;
        let next_tick = self
            .next_tick
            .checked_add(u64::from(updates))
            .ok_or(StepError::TickOverflow)?;
        for index in self.next_tick..next_tick {
            consumer.fixed_tick(index);
        }
        self.accumulated = remaining;
        self.next_tick = next_tick;
        Ok(StepReport {
            backlog: remaining,
            next_tick,
            updates,
        })
    }

    /// Construct a shared clock with explicit tick interval and work budget.
    ///
    /// # Errors
    ///
    /// Rejects a zero-length simulation interval.
    pub const fn new(
        interval: Duration,
        max_updates: NonZeroU32,
    ) -> Result<Self, StepError> {
        if interval.is_zero() {
            return Err(StepError::InvalidInterval);
        }
        Ok(Self {
            accumulated: Duration::ZERO,
            interval,
            max_updates,
            next_tick: 0,
        })
    }

    /// Return total unprocessed elapsed simulation time.
    #[must_use]
    pub const fn pending_time(&self) -> Duration {
        self.accumulated
    }

    /// Map an absolute elapsed timestamp to its fixed simulation tick.
    ///
    /// Time is measured from this clock's original zero, independently of
    /// frame partitions and backlog. A timestamp exactly on a tick boundary
    /// belongs to the new tick. Returns None for an unexecutable tick index.
    #[must_use]
    pub fn tick_at(&self, elapsed: Duration) -> Option<u64> {
        let tick = elapsed
            .as_nanos()
            .checked_div(self.interval.as_nanos())
            .and_then(|index| u64::try_from(index).ok())?;
        // Executing a tick must leave a representable successor.
        (tick < u64::MAX).then_some(tick)
    }

    /// Return the index of the next fixed update.
    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.next_tick
    }
}

#[cfg(test)]
mod tests {
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
        let maximum =
            NonZeroU32::new(budget).ok_or(StepError::InvalidInterval)?;
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
            if let Ok(mut clock) =
                StepClock::new(Duration::from_millis(10), budget)
            {
                let mut state = Counter::default();
                let report =
                    clock.advance(Duration::from_millis(17), &mut state);
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
}
