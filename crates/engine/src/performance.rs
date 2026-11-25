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
//   - Aggregation of comparable presentation and resource measurements.
// - Must-Not:
//   - Sample operating-system clocks, infer GPU use or alter game quality.
// - Allows:
//   - Inputs: Instrumented frame and latency durations with byte counts.
//   - Outputs: Measured throughput and resource peaks per quality profile.
//   - Side effects: Retains only bounded aggregate counters in memory.
// - Split-When:
//   - Platform GPU timings require a dedicated hardware adapter.
// - Merge-When:
//   - A shared measurement suite owns the same performance aggregates.
// - Summary:
//   - O(1)-memory frame, FPS, load, latency and byte accounting.
// - Description:
//   - Keeps source measurements separate from benchmark conclusions.
// - Usage:
//   - Feed actual instrumented frame observations to a collector.
// - Defaults:
//   - Zero observations yield no computed performance rate.
//
//! Portable bounded aggregation for performance observations.

use std::time::Duration;

/// A comparable presentation setup supplied by a title's host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QualityProfile {
    /// Presentation height in pixels.
    pub height: u32,
    /// Whether optional graphics effects are enabled.
    pub optional_effects: bool,
    /// Presentation width in pixels.
    pub width: u32,
}

/// A frame observation supplied by a measured host platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameObservation {
    /// Wall-time duration of this rendered frame.
    pub frame_time: Duration,
    /// Age of an applied input event, if one was actually measured.
    pub input_latency: Option<Duration>,
    /// Approximate working set bytes, if supported by the platform.
    pub resident_bytes: Option<u64>,
}

/// Constant-memory duration statistics without fabricated observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimingSummary {
    maximum: Duration,
    samples: u64,
    total: Duration,
}

impl TimingSummary {
    const EMPTY: Self = Self {
        maximum: Duration::ZERO,
        samples: 0,
        total: Duration::ZERO,
    };

    /// Return the largest measured duration, if a sample exists.
    #[must_use]
    pub const fn maximum(&self) -> Option<Duration> {
        if self.samples == 0 {
            None
        } else {
            Some(self.maximum)
        }
    }

    /// Return mean measured duration at nanosecond resolution.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        self.total
            .as_nanos()
            .checked_div(u128::from(self.samples))
            .map(Duration::from_nanos_u128)
    }

    fn record(&mut self, duration: Duration) -> Result<(), PerformanceError> {
        let total = self
            .total
            .checked_add(duration)
            .ok_or(PerformanceError::Overflow)?;
        let samples = self
            .samples
            .checked_add(1)
            .ok_or(PerformanceError::Overflow)?;
        self.maximum = self.maximum.max(duration);
        self.samples = samples;
        self.total = total;
        Ok(())
    }

    /// Return the number of actual measurements.
    #[must_use]
    pub const fn samples(&self) -> u64 {
        self.samples
    }

    /// Return the sum of observed durations.
    #[must_use]
    pub const fn total(&self) -> Duration {
        self.total
    }
}

/// Persistent measurement aggregates for one presentation quality profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceReport {
    /// Timings of all observed rendered frames.
    pub frame_time: TimingSummary,
    /// Timings of measured input events only, excluding unobserved inputs.
    pub input_latency: TimingSummary,
    /// Largest observed resident memory footprint, if measured.
    pub peak_resident_bytes: Option<u64>,
}

/// Bounded instrumentation that does not retain individual frame samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceCollector {
    binary_bytes: Option<u64>,
    load_time: Option<Duration>,
    profile: QualityProfile,
    report: PerformanceReport,
}

impl PerformanceCollector {
    /// Read the measured binary's size in bytes, if separately supplied.
    #[must_use]
    pub const fn binary_size(&self) -> Option<u64> {
        self.binary_bytes
    }

    /// Return the mean frame throughput, rounded down to whole FPS.
    #[must_use]
    pub fn frames_per_second(&self) -> Option<u64> {
        let elapsed_ns = self.report.frame_time.total().as_nanos();
        let numerator = u128::from(self.report.frame_time.samples())
            .checked_mul(1_000_000_000)?;
        u64::try_from(numerator.checked_div(elapsed_ns)?).ok()
    }

    /// Return the most recently reported full-load duration.
    #[must_use]
    pub const fn load_duration(&self) -> Option<Duration> {
        self.load_time
    }

    /// Construct an aggregate for one explicit, comparable quality profile.
    #[must_use]
    pub const fn new(profile: QualityProfile) -> Self {
        Self {
            binary_bytes: None,
            load_time: None,
            profile,
            report: PerformanceReport {
                frame_time: TimingSummary::EMPTY,
                input_latency: TimingSummary::EMPTY,
                peak_resident_bytes: None,
            },
        }
    }

    /// Record one frame observation, without allocating a sample history.
    ///
    /// No partial frame or resource measurement is kept on failure.
    ///
    /// # Errors
    ///
    /// Rejects cumulative duration or sample-count overflow.
    pub fn observe(
        &mut self,
        frame: FrameObservation,
    ) -> Result<(), PerformanceError> {
        let mut report = self.report;
        report.frame_time.record(frame.frame_time)?;
        if let Some(latency) = frame.input_latency {
            report.input_latency.record(latency)?;
        }
        report.peak_resident_bytes =
            report.peak_resident_bytes.max(frame.resident_bytes);
        self.report = report;
        Ok(())
    }

    /// Return the explicit quality profile used for these observations.
    #[must_use]
    pub const fn profile(&self) -> QualityProfile {
        self.profile
    }

    /// Record the observed distribution artifact size, not a source estimate.
    pub const fn record_binary_size(&mut self, bytes: u64) {
        self.binary_bytes = Some(bytes);
    }

    /// Record one complete loading duration from platform instrumentation.
    pub const fn record_load(&mut self, duration: Duration) {
        self.load_time = Some(duration);
    }

    /// Return the aggregate without exposing implementation-specific samples.
    #[must_use]
    pub const fn report(&self) -> PerformanceReport {
        self.report
    }
}

/// A measurement could not be represented without losing information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerformanceError {
    /// Duration or sample count cannot be safely accumulated.
    Overflow,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        FrameObservation, PerformanceCollector, PerformanceError,
        QualityProfile,
    };

    fn profile() -> QualityProfile {
        QualityProfile {
            height: 720,
            optional_effects: false,
            width: 1280,
        }
    }

    #[test]
    fn reports_real_units_from_supplied_frame_observations() {
        let mut collector = PerformanceCollector::new(profile());
        assert_eq!(collector.frames_per_second(), None);
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(10),
                input_latency: Some(Duration::from_millis(2)),
                resident_bytes: Some(1_000_000),
            }),
            Ok(())
        );
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(30),
                input_latency: Some(Duration::from_millis(15)),
                resident_bytes: Some(2_000_000),
            }),
            Ok(())
        );
        let report = collector.report();
        assert_eq!(
            (
                report.frame_time.samples(),
                report.frame_time.total(),
                report.frame_time.maximum(),
                report.frame_time.mean(),
            ),
            (
                2,
                Duration::from_millis(40),
                Some(Duration::from_millis(30)),
                Some(Duration::from_millis(20)),
            ),
        );
        assert_eq!(
            (
                report.input_latency.samples(),
                report.input_latency.maximum(),
                report.input_latency.mean(),
            ),
            (
                2,
                Some(Duration::from_millis(15)),
                Some(Duration::from_micros(8500)),
            ),
        );
        assert_eq!(report.peak_resident_bytes, Some(2_000_000));
        assert_eq!(collector.frames_per_second(), Some(50));
    }

    #[test]
    fn missing_measurements_are_not_recorded_as_zero() {
        let mut collector = PerformanceCollector::new(profile());
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(16),
                input_latency: None,
                resident_bytes: None,
            }),
            Ok(())
        );
        let report = collector.report();
        assert_eq!(
            (
                report.frame_time.samples(),
                report.input_latency.samples(),
                report.input_latency.maximum(),
                report.input_latency.mean(),
                report.peak_resident_bytes,
            ),
            (1, 0, None, None, None),
        );
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(16),
                input_latency: Some(Duration::ZERO),
                resident_bytes: Some(0),
            }),
            Ok(())
        );
        let measured = collector.report();
        assert_eq!(
            (
                measured.frame_time.samples(),
                measured.input_latency.samples(),
                measured.input_latency.mean(),
                measured.peak_resident_bytes,
            ),
            (2, 1, Some(Duration::ZERO), Some(0)),
        );
    }

    #[test]
    fn records_platform_load_size_and_quality_without_hidden_changes() {
        let mut collector = PerformanceCollector::new(profile());
        collector.record_load(Duration::from_millis(450));
        collector.record_binary_size(123_456);
        assert_eq!(collector.load_duration(), Some(Duration::from_millis(450)));
        assert_eq!(collector.binary_size(), Some(123_456));
        assert_eq!(collector.profile(), profile());
    }

    #[test]
    fn rejects_cumulative_duration_overflow_without_partial_changes() {
        let mut collector = PerformanceCollector::new(profile());
        let first = FrameObservation {
            frame_time: Duration::MAX,
            input_latency: None,
            resident_bytes: Some(1),
        };
        assert_eq!(collector.observe(first), Ok(()));
        let before = collector.report();
        assert_eq!(before.frame_time.mean(), Some(Duration::MAX));
        let second = FrameObservation {
            frame_time: Duration::from_nanos(1),
            input_latency: Some(Duration::from_secs(9)),
            resident_bytes: Some(100),
        };
        assert_eq!(collector.observe(second), Err(PerformanceError::Overflow));
        assert_eq!(collector.report(), before);
    }

    #[test]
    fn latency_overflow_does_not_partially_record_the_frame() {
        let mut collector = PerformanceCollector::new(profile());
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_nanos(1),
                input_latency: Some(Duration::MAX),
                resident_bytes: None,
            }),
            Ok(())
        );
        let before = collector.report();
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_nanos(1),
                input_latency: Some(Duration::from_nanos(1)),
                resident_bytes: Some(100),
            }),
            Err(PerformanceError::Overflow),
        );
        assert_eq!(collector.report(), before);
    }
}
