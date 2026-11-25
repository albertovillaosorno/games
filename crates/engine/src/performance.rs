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

use std::num::NonZeroU64;
use std::time::Duration;

use crate::presentation::FramePacing;

// Duration::MAX has a 94-bit nanosecond representation. Each doubling
// beginning at eight nanoseconds consumes eight counters, plus 0..=7.
const HISTOGRAM_BUCKETS: usize = 736;

/// A comparable presentation setup supplied by a title's host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QualityProfile {
    /// Rendering cap policy used during this observation.
    pub frame_pacing: FramePacing,
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

/// Inclusive bounds on the nearest-rank percentile of measured durations.
///
/// The histogram resolves the eight smallest nanosecond values exactly and
/// uses eight subdivisions per doubling thereafter. A range is not an exact
/// measured sample, and should not be printed as a point estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PercentileRange {
    /// Smallest possible duration at this rank.
    pub lower: Duration,
    /// Largest possible duration at this rank.
    pub upper: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TimingHistogram {
    counts: [u64; HISTOGRAM_BUCKETS],
}

impl TimingHistogram {
    const EMPTY: Self = Self {
        counts: [0; HISTOGRAM_BUCKETS],
    };

    fn bucket_mut(
        &mut self,
        duration: Duration,
    ) -> Result<&mut u64, PerformanceError> {
        let index =
            histogram_index(duration).ok_or(PerformanceError::Overflow)?;
        self.counts.get_mut(index).ok_or(PerformanceError::Overflow)
    }

    fn percentile(
        &self,
        summary: &TimingSummary,
        percentile: u8,
    ) -> Option<PercentileRange> {
        if !(1..=100).contains(&percentile) {
            return None;
        }
        let maximum = summary.maximum()?;
        if percentile == 100 {
            return Some(PercentileRange {
                lower: maximum,
                upper: maximum,
            });
        }
        let mut rank = u128::from(summary.samples())
            .saturating_mul(u128::from(percentile))
            .div_ceil(100);
        for (index, count) in self.counts.iter().copied().enumerate() {
            if rank <= u128::from(count) {
                return histogram_range(index, maximum);
            }
            rank = rank.saturating_sub(u128::from(count));
        }
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Distributions {
    frame: TimingHistogram,
    input: TimingHistogram,
}

impl Distributions {
    fn record(
        &mut self,
        observation: FrameObservation,
    ) -> Result<(), PerformanceError> {
        let frame = self.frame.bucket_mut(observation.frame_time)?;
        let frame_count =
            frame.checked_add(1).ok_or(PerformanceError::Overflow)?;
        if let Some(latency) = observation.input_latency {
            let input = self.input.bucket_mut(latency)?;
            let input_count =
                input.checked_add(1).ok_or(PerformanceError::Overflow)?;
            *input = input_count;
        }
        *frame = frame_count;
        Ok(())
    }
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

/// Counts frames exceeding an explicit benchmark target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameBudgetReport {
    /// Number of measured frames strictly longer than the target.
    pub missed: u64,
    /// Maximum acceptable frame duration, independent of a rendering cap.
    pub target: Duration,
}

/// Persistent measurement aggregates for one presentation quality profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceReport {
    /// Deadline misses, only when a benchmark target was configured.
    pub frame_budget: Option<FrameBudgetReport>,
    /// Timings of all observed rendered frames.
    pub frame_time: TimingSummary,
    /// Timings of measured input events only, excluding unobserved inputs.
    pub input_latency: TimingSummary,
    /// Largest observed resident memory footprint, if measured.
    pub peak_resident_bytes: Option<u64>,
}

/// Bounded instrumentation that does not retain individual frame samples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceCollector {
    binary_bytes: Option<u64>,
    distributions: Option<Box<Distributions>>,
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

    /// Estimate the nearest-rank frame percentile as an inclusive range.
    ///
    /// Returns no range when distributions were not enabled, no samples were
    /// observed, or the requested percentile is outside 1 through 100.
    #[must_use]
    pub fn frame_percentile(&self, percentile: u8) -> Option<PercentileRange> {
        self.distributions
            .as_ref()?
            .frame
            .percentile(&self.report.frame_time, percentile)
    }

    /// Return the mean frame throughput, rounded down to whole FPS.
    #[must_use]
    pub fn frames_per_second(&self) -> Option<u64> {
        let elapsed_ns = self.report.frame_time.total().as_nanos();
        let numerator = u128::from(self.report.frame_time.samples())
            .checked_mul(1_000_000_000)?;
        u64::try_from(numerator.checked_div(elapsed_ns)?).ok()
    }

    /// Estimate the nearest-rank percentile of measured input ages.
    ///
    /// Unobserved input is not counted as zero. Returns no range when
    /// distributions are disabled or the percentile or sample set is invalid.
    #[must_use]
    pub fn input_percentile(&self, percentile: u8) -> Option<PercentileRange> {
        self.distributions
            .as_ref()?
            .input
            .percentile(&self.report.input_latency, percentile)
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
            distributions: None,
            load_time: None,
            profile,
            report: PerformanceReport {
                frame_budget: None,
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
        if let Some(budget) = &mut report.frame_budget
            && frame.frame_time > budget.target
        {
            budget.missed = budget
                .missed
                .checked_add(1)
                .ok_or(PerformanceError::Overflow)?;
        }
        if let Some(latency) = frame.input_latency {
            report.input_latency.record(latency)?;
        }
        report.peak_resident_bytes =
            report.peak_resident_bytes.max(frame.resident_bytes);
        if let Some(distributions) = &mut self.distributions {
            distributions.record(frame)?;
        }
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

    /// Construct a collector with an explicit nonzero frame-time target.
    ///
    /// A presentation cap limits frequency; it is not a benchmark target.
    #[must_use]
    pub const fn with_frame_budget(
        profile: QualityProfile,
        target_ns: NonZeroU64,
    ) -> Self {
        let mut collector = Self::new(profile);
        collector.report.frame_budget = Some(FrameBudgetReport {
            missed: 0,
            target: Duration::from_nanos(target_ns.get()),
        });
        collector
    }

    /// Begin measurement with bounded optional timing distributions.
    ///
    /// Allocates two fixed histograms, not a growing frame history.
    /// Ordinary collectors allocate neither histogram.
    #[must_use]
    pub fn with_percentiles(
        profile: QualityProfile,
        target_ns: Option<NonZeroU64>,
    ) -> Self {
        let mut collector = target_ns.map_or_else(
            || Self::new(profile),
            |target| Self::with_frame_budget(profile, target),
        );
        collector.distributions = Some(Box::new(Distributions {
            frame: TimingHistogram::EMPTY,
            input: TimingHistogram::EMPTY,
        }));
        collector
    }
}

/// A measurement could not be represented without losing information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerformanceError {
    /// Duration or sample count cannot be safely accumulated.
    Overflow,
}

fn histogram_index(duration: Duration) -> Option<usize> {
    let nanos = duration.as_nanos();
    if nanos < 8 {
        return usize::try_from(nanos).ok();
    }
    let shift = nanos.ilog2().checked_sub(3)?;
    let bucket = u128::from(shift)
        .checked_mul(8)?
        .checked_add(nanos.checked_shr(shift)?)?;
    usize::try_from(bucket).ok()
}

fn histogram_range(index: usize, maximum: Duration) -> Option<PercentileRange> {
    if index < 8 {
        let exact = Duration::from_nanos(u64::try_from(index).ok()?);
        return Some(PercentileRange {
            lower: exact,
            upper: exact,
        });
    }
    let shift = u32::try_from(index.checked_sub(8)?.checked_div(8)?).ok()?;
    let mantissa =
        u128::try_from(index.checked_rem(8)?).ok()?.checked_add(8)?;
    let lower = mantissa.checked_shl(shift)?;
    let upper = mantissa
        .checked_add(1)?
        .checked_shl(shift)?
        .checked_sub(1)?;
    Some(PercentileRange {
        lower: Duration::from_nanos_u128(lower),
        upper: Duration::from_nanos_u128(upper.min(maximum.as_nanos())),
    })
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroU64};
    use std::time::Duration;

    use super::{
        FrameBudgetReport, FrameObservation, PercentileRange,
        PerformanceCollector, PerformanceError, QualityProfile,
    };
    use crate::presentation::FramePacing;

    fn profile() -> QualityProfile {
        QualityProfile {
            frame_pacing: FramePacing::Uncapped,
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

    #[test]
    fn explicit_frame_budget_counts_only_strict_overruns() {
        let target = NonZeroU64::new(20_000_000).unwrap_or(NonZeroU64::MIN);
        let mut collector =
            PerformanceCollector::with_frame_budget(profile(), target);
        for milliseconds in [10, 20, 30, 20] {
            assert_eq!(
                collector.observe(FrameObservation {
                    frame_time: Duration::from_millis(milliseconds),
                    input_latency: None,
                    resident_bytes: None,
                }),
                Ok(())
            );
        }
        assert_eq!(collector.frames_per_second(), Some(50));
        assert_eq!(
            collector.report().frame_budget,
            Some(FrameBudgetReport {
                missed: 1,
                target: Duration::from_millis(20),
            })
        );
        assert_eq!(collector.report().frame_time.samples(), 4);
    }

    #[test]
    fn render_cap_does_not_implicitly_define_a_benchmark_target() {
        let cap = NonZeroU32::new(60).unwrap_or(NonZeroU32::MIN);
        let capped = QualityProfile {
            frame_pacing: FramePacing::Capped(cap),
            ..profile()
        };
        let mut collector = PerformanceCollector::new(capped);
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_millis(30),
                input_latency: None,
                resident_bytes: None,
            }),
            Ok(())
        );
        assert_eq!(collector.report().frame_budget, None);
        assert_ne!(collector.profile(), profile());
    }

    #[test]
    fn rejected_observation_does_not_count_as_a_budget_miss() {
        let mut collector =
            PerformanceCollector::with_frame_budget(profile(), NonZeroU64::MIN);
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
                frame_time: Duration::from_millis(20),
                input_latency: Some(Duration::from_nanos(1)),
                resident_bytes: Some(100),
            }),
            Err(PerformanceError::Overflow)
        );
        assert_eq!(collector.report(), before);
    }
    #[test]
    fn percentile_ranges_contain_the_exact_nearest_rank() {
        let samples = [
            0,
            1,
            7,
            8,
            15,
            16,
            17,
            31,
            32,
            33,
            1000,
            500_000,
            16_666_667,
            30_000_000,
            1_000_000_000,
        ];
        let mut collector =
            PerformanceCollector::with_percentiles(profile(), None);
        for nanos in samples {
            let duration = Duration::from_nanos(nanos);
            assert_eq!(
                collector.observe(FrameObservation {
                    frame_time: duration,
                    input_latency: Some(duration),
                    resident_bytes: None,
                }),
                Ok(())
            );
        }
        for percentile in 1..=100 {
            let index = usize::from(percentile)
                .saturating_mul(samples.len())
                .div_ceil(100)
                .saturating_sub(1);
            let expected =
                samples.get(index).copied().map(Duration::from_nanos);
            let range = collector.frame_percentile(percentile);
            assert_eq!(range, collector.input_percentile(percentile));
            assert!(range.zip(expected).is_some_and(|(bounds, exact)| {
                bounds.lower <= exact && exact <= bounds.upper
            }));
        }
    }

    #[test]
    fn percentiles_distinguish_disabled_missing_and_measured_zero() {
        let mut collector =
            PerformanceCollector::with_percentiles(profile(), None);
        assert_eq!(collector.frame_percentile(50), None);
        let zero = FrameObservation {
            frame_time: Duration::ZERO,
            input_latency: None,
            resident_bytes: None,
        };
        assert_eq!(collector.observe(zero), Ok(()));
        assert_eq!(
            collector.frame_percentile(50),
            Some(PercentileRange {
                lower: Duration::ZERO,
                upper: Duration::ZERO,
            })
        );
        assert_eq!(collector.input_percentile(50), None);
        assert_eq!(
            collector.observe(FrameObservation {
                input_latency: Some(Duration::ZERO),
                ..zero
            }),
            Ok(())
        );
        assert_eq!(
            collector.input_percentile(99),
            collector.frame_percentile(99)
        );
    }

    #[test]
    fn disabled_distributions_and_invalid_percentiles_are_unobserved() {
        let zero = FrameObservation {
            frame_time: Duration::ZERO,
            input_latency: None,
            resident_bytes: None,
        };
        let mut ordinary = PerformanceCollector::new(profile());
        assert_eq!(ordinary.observe(zero), Ok(()));
        assert_eq!(ordinary.frame_percentile(50), None);
        let mut enabled =
            PerformanceCollector::with_percentiles(profile(), None);
        assert_eq!(enabled.observe(zero), Ok(()));
        assert_eq!(enabled.frame_percentile(0), None);
        assert_eq!(enabled.frame_percentile(101), None);
    }

    #[test]
    fn maximum_duration_and_failed_observations_preserve_distributions() {
        let mut collector =
            PerformanceCollector::with_percentiles(profile(), None);
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_nanos(1),
                input_latency: Some(Duration::MAX),
                resident_bytes: None,
            }),
            Ok(())
        );
        assert!(collector.input_percentile(50).is_some_and(|range| {
            range.lower <= Duration::MAX && range.upper == Duration::MAX
        }));
        assert_eq!(
            collector.input_percentile(100),
            Some(PercentileRange {
                lower: Duration::MAX,
                upper: Duration::MAX,
            })
        );
        let before = collector.clone();
        assert_eq!(
            collector.observe(FrameObservation {
                frame_time: Duration::from_nanos(20),
                input_latency: Some(Duration::from_nanos(1)),
                resident_bytes: Some(100),
            }),
            Err(PerformanceError::Overflow)
        );
        assert_eq!(collector, before);
    }
}
