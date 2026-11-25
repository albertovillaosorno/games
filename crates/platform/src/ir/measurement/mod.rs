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
//   - Host-independent measurement and delegated resource observation.
// - Must-Not:
//   - Access native services or estimate device execution latency.
// - Allows:
//   - Inputs: Comparable timestamps, resource bytes and shared collector.
//   - Outputs: Measured frame and load durations or typed failures.
//   - Side effects: Delegated read-only observations and in-memory updates.
// - Split-When:
//   - Another observation has independent aggregation requirements.
// - Merge-When:
//   - A shared measurement contract covers the same timing policy.
// - Summary:
//   - Portable frame measurement over an injected monotonic time domain.
// - Description:
//   - Rejects invalid ordering before recording any affected observation.
// - Usage:
//   - Supply timestamps from one clock and report actual observations.
// - Defaults:
//   - Missing input and resource observations remain absent.

//! Portable frame measurement, clock and resource observation.

use std::io::Result as IoResult;
use std::time::Duration;

use games::performance::{
    FrameObservation, PerformanceCollector, PerformanceError,
};

/// Private measurement source selected by the platform composition.
pub(crate) trait MeasurementSource {
    fn executable_bytes() -> IoResult<u64>;
    fn now() -> IoResult<Duration>;
    fn peak_resident_bytes() -> IoResult<u64>;
    fn resident_bytes() -> IoResult<u64>;
}

/// Comparable points in one monotonic time domain.
pub trait MeasurementTimestamp: Copy {
    /// Return the elapsed duration, or absence for reversed timestamps.
    ///
    /// Both points must originate from the same clock.
    fn checked_elapsed(self, earlier: Self) -> Option<Duration>;
}

impl MeasurementTimestamp for Duration {
    fn checked_elapsed(self, earlier: Self) -> Option<Duration> {
        self.checked_sub(earlier)
    }
}

/// A host measurement could not be recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementError {
    /// The shared aggregate rejected an observation.
    Aggregate(PerformanceError),
    /// The end timestamp precedes the corresponding start timestamp.
    TimestampOrder,
}

impl From<PerformanceError> for MeasurementError {
    fn from(error: PerformanceError) -> Self {
        Self::Aggregate(error)
    }
}

/// Measures intervals using explicitly supplied comparable timestamps.
#[derive(Clone, Debug)]
pub struct FrameMeter<T> {
    collector: PerformanceCollector,
    previous: T,
}

impl<T: MeasurementTimestamp> FrameMeter<T> {
    /// Access the accumulated observations without changing the meter.
    #[must_use]
    pub const fn collector(&self) -> &PerformanceCollector {
        &self.collector
    }

    /// Recover the collector, including its optional distributions.
    #[must_use]
    pub fn into_collector(self) -> PerformanceCollector {
        self.collector
    }

    /// Begin measurement at the preceding presentation or workload start.
    #[must_use]
    pub const fn new(collector: PerformanceCollector, previous: T) -> Self {
        Self { collector, previous }
    }

    /// Record a completed presentation and optional applied input timestamp.
    ///
    /// These are host observations, not GPU execution or display scanout.
    ///
    /// # Errors
    /// Rejects reversed timestamps or aggregate overflow without advancing
    /// the preceding presentation timestamp.
    pub fn observe_frame(
        &mut self,
        completed: T,
        input_received: Option<T>,
        resident_bytes: Option<u64>,
    ) -> Result<(), MeasurementError> {
        let frame_time = elapsed(self.previous, completed)?;
        let input_latency = input_received
            .map(|received| elapsed(received, completed))
            .transpose()?;
        self.collector.observe(FrameObservation {
            frame_time,
            input_latency,
            resident_bytes,
        })?;
        self.previous = completed;
        Ok(())
    }

    /// Record applied input age without fabricating a frame interval.
    ///
    /// The preceding presentation timestamp remains unchanged.
    ///
    /// # Errors
    /// Rejects reversed timestamps or aggregate overflow.
    pub fn observe_input(
        &mut self,
        received: T,
        completed: T,
    ) -> Result<(), MeasurementError> {
        self.collector
            .observe_input(elapsed(received, completed)?)?;
        Ok(())
    }

    /// Record the measured distribution artifact size.
    pub const fn record_artifact_bytes(&mut self, bytes: u64) {
        self.collector.record_binary_size(bytes);
    }

    /// Record a complete load interval observed by the host.
    ///
    /// # Errors
    /// Rejects reversed timestamps without changing the measured duration.
    pub fn record_load(
        &mut self,
        started: T,
        completed: T,
    ) -> Result<(), MeasurementError> {
        self.collector.record_load(elapsed(started, completed)?);
        Ok(())
    }
}

fn elapsed<T>(started: T, completed: T) -> Result<Duration, MeasurementError>
where
    T: MeasurementTimestamp,
{
    completed
        .checked_elapsed(started)
        .ok_or(MeasurementError::TimestampOrder)
}
/// Actual byte length of the current executable.
///
/// # Errors
/// Reports unavailable executable identity, metadata or non-file artifacts.
pub fn executable_bytes() -> IoResult<u64> {
    <() as MeasurementSource>::executable_bytes()
}

/// Read the process-wide monotonic observation clock.
///
/// The origin is arbitrary, shared within this process and distinct from
/// wall time, simulation ticks and device input session clocks.
///
/// # Errors
/// Reports an unavailable observation clock.
pub fn now() -> IoResult<Duration> {
    <() as MeasurementSource>::now()
}

/// Observe approximate process-lifetime peak resident bytes.
///
/// # Errors
/// Reports unsupported, unavailable or malformed resource observations.
pub fn peak_resident_bytes() -> IoResult<u64> {
    <() as MeasurementSource>::peak_resident_bytes()
}

/// Observe approximate current process resident bytes.
///
/// Sample at an explicit cadence rather than on every rendered frame.
///
/// # Errors
/// Reports unsupported, unavailable or malformed resource observations.
pub fn resident_bytes() -> IoResult<u64> {
    <() as MeasurementSource>::resident_bytes()
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::time::Duration;

    use games::performance::{PerformanceCollector, QualityProfile};
    use games::presentation::FramePacing;

    use super::{FrameMeter, MeasurementError};

    fn meter(started: Duration) -> FrameMeter<Duration> {
        FrameMeter::new(
            PerformanceCollector::new(QualityProfile {
                frame_pacing: FramePacing::Uncapped,
                height: 720,
                optional_effects: false,
                width: 1280,
            }),
            started,
        )
    }

    fn check<T>(actual: T, expected: T) -> Result<(), String>
    where
        T: Copy + Debug + PartialEq,
    {
        if actual == expected {
            Ok(())
        } else {
            Err(format!("expected {expected:?}, observed {actual:?}"))
        }
    }

    fn later(started: Duration, milliseconds: u64) -> Result<Duration, String> {
        started
            .checked_add(Duration::from_millis(milliseconds))
            .ok_or_else(|| String::from("test timestamp overflow"))
    }

    #[test]
    fn records_completion_intervals_and_observed_input_age()
    -> Result<(), String> {
        let started = Duration::ZERO;
        let mut meter = meter(started);
        let first = later(started, 10)?;
        let second = later(started, 40)?;
        check(meter.observe_frame(first, None, Some(1024)), Ok(()))?;
        check(
            meter.observe_frame(second, Some(later(started, 30)?), None),
            Ok(()),
        )?;
        let report = meter.collector().report();
        check(report.frame_time.mean(), Some(Duration::from_millis(20)))?;
        check(report.input_latency.mean(), Some(Duration::from_millis(10)))?;
        check(report.input_latency.samples(), 1)?;
        check(report.peak_resident_bytes, Some(1024))?;
        check(meter.collector().frames_per_second(), Some(50))?;
        Ok(())
    }

    #[test]
    fn first_input_age_does_not_create_an_interval() -> Result<(), String> {
        let started = Duration::ZERO;
        let completed = later(started, 15)?;
        let mut meter = meter(completed);
        check(meter.observe_input(started, completed), Ok(()))?;
        check(
            meter.observe_input(completed, started),
            Err(MeasurementError::TimestampOrder),
        )?;
        check(meter.collector().report().frame_time.samples(), 0)?;
        check(
            meter.collector().report().input_latency.mean(),
            Some(Duration::from_millis(15)),
        )?;
        check(
            meter.observe_frame(later(completed, 5)?, None, None),
            Ok(()),
        )?;
        check(
            meter.collector().report().frame_time.mean(),
            Some(Duration::from_millis(5)),
        )?;
        check(meter.collector().report().input_latency.samples(), 1)
    }

    #[test]
    fn rejects_reversed_timestamps_without_changing_interval()
    -> Result<(), String> {
        let started = Duration::ZERO;
        let mut meter = meter(later(started, 10)?);
        check(
            meter.observe_frame(started, None, Some(999)),
            Err(MeasurementError::TimestampOrder),
        )?;
        let completed = later(started, 20)?;
        check(
            meter.observe_frame(completed, Some(later(started, 30)?), None),
            Err(MeasurementError::TimestampOrder),
        )?;
        check(meter.collector().report().frame_time.samples(), 0)?;
        check(meter.observe_frame(completed, None, None), Ok(()))?;
        check(
            meter.collector().report().frame_time.total(),
            Duration::from_millis(10),
        )?;
        check(meter.collector().report().peak_resident_bytes, None)?;
        Ok(())
    }

    #[test]
    fn load_and_artifact_observations_are_independent() -> Result<(), String> {
        let started = Duration::ZERO;
        let completed = Duration::from_millis(25);
        let mut meter = meter(started);
        check(meter.record_load(started, completed), Ok(()))?;
        check(
            meter.record_load(completed, started),
            Err(MeasurementError::TimestampOrder),
        )?;
        check(
            meter.collector().load_duration(),
            Some(Duration::from_millis(25)),
        )?;
        meter.record_artifact_bytes(42);
        check(meter.collector().binary_size(), Some(42))?;
        check(meter.collector().report().frame_time.samples(), 0)?;
        Ok(())
    }
}
