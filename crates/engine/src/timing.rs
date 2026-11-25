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

    /// Return the index of the next fixed update.
    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.next_tick
    }
}

#[cfg(test)]
#[path = "../../../tests/engine/timing.rs"]
mod tests;
