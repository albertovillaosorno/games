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
    /// Host-reported age of the applied input event.
    pub input_latency: Duration,
    /// Approximate working set bytes at this observation.
    pub resident_bytes: u64,
}

/// Persistent measurement aggregates for one presentation quality profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceReport {
    /// Number of sampled frames.
    pub frames: u64,
    /// Largest observed input age.
    pub max_input_latency: Duration,
    /// Largest observed resident memory footprint.
    pub peak_resident_bytes: u64,
    /// Slowest sampled frame time.
    pub slowest_frame: Duration,
    /// Total observed frame time.
    pub total_frame_time: Duration,
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
        let elapsed_ns = self.report.total_frame_time.as_nanos();
        if self.report.frames == 0 || elapsed_ns == 0 {
            return None;
        }
        let numerator =
            u128::from(self.report.frames).checked_mul(1_000_000_000)?;
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
                frames: 0,
                max_input_latency: Duration::ZERO,
                peak_resident_bytes: 0,
                slowest_frame: Duration::ZERO,
                total_frame_time: Duration::ZERO,
            },
        }
    }

    /// Record one frame observation, without allocating a sample history.
    ///
    /// # Errors
    ///
    /// Rejects cumulative duration or frame-count overflow.
    pub fn observe(
        &mut self,
        frame: FrameObservation,
    ) -> Result<(), PerformanceError> {
        let total = self
            .report
            .total_frame_time
            .checked_add(frame.frame_time)
            .ok_or(PerformanceError::Overflow)?;
        let frames = self
            .report
            .frames
            .checked_add(1)
            .ok_or(PerformanceError::Overflow)?;
        self.report.total_frame_time = total;
        self.report.frames = frames;
        self.report.slowest_frame =
            self.report.slowest_frame.max(frame.frame_time);
        self.report.max_input_latency =
            self.report.max_input_latency.max(frame.input_latency);
        self.report.peak_resident_bytes =
            self.report.peak_resident_bytes.max(frame.resident_bytes);
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
    /// Duration or frame count cannot be safely accumulated.
    Overflow,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{FrameObservation, PerformanceCollector, QualityProfile};

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
                input_latency: Duration::from_millis(2),
                resident_bytes: 1_000_000,
            }),
            Ok(())
        );
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(30),
                input_latency: Duration::from_millis(15),
                resident_bytes: 2_000_000,
            }),
            Ok(())
        );
        let report = collector.report();
        assert_eq!(report.frames, 2);
        assert_eq!(report.total_frame_time, Duration::from_millis(40));
        assert_eq!(report.slowest_frame, Duration::from_millis(30));
        assert_eq!(report.max_input_latency, Duration::from_millis(15));
        assert_eq!(report.peak_resident_bytes, 2_000_000);
        assert_eq!(collector.frames_per_second(), Some(50));
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
            input_latency: Duration::ZERO,
            resident_bytes: 1,
        };
        assert_eq!(collector.observe(first), Ok(()));
        let before = collector.report();
        let second = FrameObservation {
            frame_time: Duration::from_nanos(1),
            input_latency: Duration::from_secs(9),
            resident_bytes: 100,
        };
        assert!(collector.observe(second).is_err());
        assert_eq!(collector.report(), before);
    }
}
