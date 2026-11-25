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
//   - Opt-in gameplay submission and local and remote input age measurements.
// - Must-Not:
//   - Change tick assignment, rendering quality or gameplay.
// - Allows:
//   - Inputs: Admitted fixed ticks, completed updates and frame timings.
//   - Outputs: Bounded aggregate reports per gameplay presentation segment.
//   - Side effects: Explicit resident-memory samples and stdout reports.
// - Split-When:
//   - A different platform needs shared live submission instrumentation.
// - Merge-When:
//   - Another native title duplicates this measurement policy.
// - Summary:
//   - Measures executed input age, not physical display or network latency.
// - Description:
//   - Tracks oldest local and remote timestamps per pending input tick.
// - Usage:
//   - Enable explicitly and observe successfully submitted gameplay frames.
// - Defaults:
//   - No instrumentation in ordinary play.

use std::collections::BTreeMap;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::{DisplaySettings, FramePacing, PresentationLayout};
use games_platform::ir::frame::RenderError;
use games_platform::ir::performance::{FrameMeter, resident_bytes};

use super::failure;

const RSS_SAMPLE_FRAMES: u64 = 120;

/// Timing of one successful gameplay frame from host work to submission.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameSample {
    pub(super) completed: Instant,
    pub(super) display: DisplaySettings,
    pub(super) elapsed: Duration,
    pub(super) layout: PresentationLayout,
    pub(super) work: Duration,
}

struct Segment {
    cadence: FrameMeter,
    collector: PerformanceCollector,
    display: DisplaySettings,
    layout: PresentationLayout,
}

impl Segment {
    fn new(
        profile: QualityProfile,
        display: DisplaySettings,
        layout: PresentationLayout,
        first_submission: Instant,
    ) -> Self {
        Self {
            cadence: FrameMeter::new(
                PerformanceCollector::with_percentiles(
                    profile,
                    NonZeroU64::new(16_666_667),
                ),
                first_submission,
            ),
            collector: PerformanceCollector::with_percentiles(
                profile,
                NonZeroU64::new(16_666_667),
            ),
            display,
            layout,
        }
    }

    fn report(&self) -> Result<(), RenderError> {
        let profile = self.collector.profile();
        let report = self.collector.report();
        let cadence = self.cadence.collector();
        let intervals = cadence.report();
        writeln!(
            stdout().lock(),
            concat!(
                "Live gameplay {}x{} logical, {}x{} drawable {:?}: ",
                "{} submitted frames; ",
                "work mean {:?}; max {:?}; p95 {:?}; budget {:?}; ",
                "submission intervals {} samples; mean {:?}; ",
                "max {:?}; p95 {:?}; budget {:?}; whole FPS {:?}; ",
                "oldest applied local input age {} samples; mean {:?}; ",
                "p95 {:?}; ",
                "oldest applied remote decode age {} samples; mean {:?}; ",
                "p95 {:?}; sampled RSS peak {:?} bytes"
            ),
            profile.width,
            profile.height,
            self.layout.surface.width.get(),
            self.layout.surface.height.get(),
            self.display,
            report.frame_time.samples(),
            report.frame_time.mean(),
            report.frame_time.maximum(),
            self.collector.frame_percentile(95),
            report.frame_budget,
            intervals.frame_time.samples(),
            intervals.frame_time.mean(),
            intervals.frame_time.maximum(),
            cadence.frame_percentile(95),
            intervals.frame_budget,
            cadence.frames_per_second(),
            report.input_latency.samples(),
            report.input_latency.mean(),
            self.collector.input_percentile(95),
            intervals.input_latency.samples(),
            intervals.input_latency.mean(),
            cadence.input_percentile(95),
            report.peak_resident_bytes,
        )
        .map_err(failure)
    }
}

// Retain at most one timestamp per successfully admitted fixed tick.
struct InputLedger<Timestamp> {
    applied: Option<Timestamp>,
    pending: BTreeMap<u64, Timestamp>,
}

impl<Timestamp> Default for InputLedger<Timestamp> {
    fn default() -> Self {
        Self {
            applied: None,
            pending: BTreeMap::new(),
        }
    }
}

impl<Timestamp: Copy + Ord> InputLedger<Timestamp> {
    /// Keep the oldest observed timestamp per nonempty admitted batch.
    fn admit(&mut self, tick: u64, elapsed: Timestamp) {
        let _stored = self
            .pending
            .entry(tick)
            .and_modify(|oldest| *oldest = (*oldest).min(elapsed))
            .or_insert(elapsed);
    }

    /// Retain the oldest applied event until a gameplay frame is submitted.
    fn advance(&mut self, next_tick: u64) {
        while self
            .pending
            .first_key_value()
            .is_some_and(|(&tick, _)| tick < next_tick)
        {
            if let Some((_, elapsed)) = self.pending.pop_first() {
                self.applied = Some(
                    self.applied.map_or(elapsed, |oldest| oldest.min(elapsed)),
                );
            }
        }
    }
}

/// Retains bounded input metadata only for successfully admitted actions.
#[derive(Default)]
pub(super) struct LiveMeasurement {
    local: InputLedger<Duration>,
    remote: InputLedger<Instant>,
    segment: Option<Segment>,
}

impl LiveMeasurement {
    pub(super) fn admit(&mut self, tick: u64, elapsed: Duration) {
        self.local.admit(tick, elapsed);
    }

    pub(super) fn admit_remote(&mut self, tick: u64, completed_at: Instant) {
        self.remote.admit(tick, completed_at);
    }

    pub(super) fn advance(&mut self, next_tick: u64) {
        self.local.advance(next_tick);
        self.remote.advance(next_tick);
    }

    pub(super) fn finish(&mut self) -> Result<(), RenderError> {
        if let Some(segment) = self.segment.take() {
            segment.report()?;
        }
        self.local.applied = None;
        self.remote.applied = None;
        Ok(())
    }

    /// Do not combine gameplay intervals with menus or changed settings.
    pub(super) fn observe(
        &mut self,
        sample: FrameSample,
    ) -> Result<(), RenderError> {
        let profile = QualityProfile {
            frame_pacing: FramePacing::Uncapped,
            height: sample.layout.design.height.get(),
            optional_effects: false,
            width: sample.layout.design.width.get(),
        };
        let input_latency = self
            .local
            .applied
            .take()
            .and_then(|received| sample.elapsed.checked_sub(received));
        let remote_received = self.remote.applied.take();
        if self.segment.as_ref().is_some_and(|segment| {
            segment.layout != sample.layout || segment.display != sample.display
        }) {
            self.finish()?;
        }
        let first = self.segment.is_none();
        let segment = self.segment.get_or_insert_with(|| {
            Segment::new(
                profile,
                sample.display,
                sample.layout,
                sample.completed,
            )
        });
        if first {
            if let Some(received) = remote_received {
                segment
                    .cadence
                    .observe_input(received, sample.completed)
                    .map_err(failure)?;
            }
        } else {
            segment
                .cadence
                .observe_frame(sample.completed, remote_received, None)
                .map_err(failure)?;
        }
        let resident_bytes = segment
            .collector
            .report()
            .frame_time
            .samples()
            .is_multiple_of(RSS_SAMPLE_FRAMES)
            .then(|| resident_bytes().ok())
            .flatten();
        segment
            .collector
            .observe(FrameObservation {
                frame_time: sample.work,
                input_latency,
                resident_bytes,
            })
            .map_err(failure)
    }
}

#[cfg(test)]
mod tests {
    use games::presentation::{Dimensions, PresentationMode};

    use super::super::dimension;
    use super::{
        DisplaySettings, Duration, FrameSample, Instant, LiveMeasurement,
        RenderError,
    };

    #[test]
    fn only_executed_ticks_contribute_and_oldest_age_survives_skips() {
        let mut measurement = LiveMeasurement::default();
        let early = Duration::from_millis(10);
        let later = Duration::from_millis(20);
        measurement.admit(2, later);
        measurement.admit(2, early);
        measurement.admit(4, later);
        measurement.advance(2);
        assert_eq!(measurement.local.applied, None);
        measurement.advance(3);
        assert_eq!(measurement.local.applied, Some(early));
        measurement.advance(5);
        assert_eq!(measurement.local.applied, Some(early));
        assert_eq!(measurement.local.pending.len(), 0);
    }

    #[test]
    fn successful_frames_measure_applied_input_only_once()
    -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let design = Dimensions::new(dimension(320)?, dimension(224)?);
        let surface = Dimensions::new(dimension(960)?, dimension(672)?);
        let layout = PresentationMode::AspectFit.layout(surface, design);
        let sample = FrameSample {
            completed: Instant::now(),
            display: DisplaySettings::default(),
            elapsed: Duration::from_millis(30),
            layout,
            work: Duration::from_millis(3),
        };
        measurement.admit(1, Duration::from_millis(10));
        measurement.advance(1);
        measurement.observe(sample)?;
        measurement.advance(2);
        measurement.observe(sample)?;
        measurement.observe(sample)?;
        let segment = measurement
            .segment
            .as_ref()
            .ok_or(RenderError::InvalidSprite)?;
        let report = segment.collector.report();
        if report.frame_time.samples() != 3
            || report.input_latency.samples() != 1
            || report.input_latency.maximum() != Some(Duration::from_millis(20))
        {
            return Err(RenderError::InvalidSprite);
        }
        Ok(())
    }

    #[test]
    fn drawable_resize_starts_new_segment() -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let design = Dimensions::new(dimension(320)?, dimension(224)?);
        let first = Dimensions::new(dimension(960)?, dimension(672)?);
        let second = Dimensions::new(dimension(1200)?, dimension(672)?);
        let sample = FrameSample {
            completed: Instant::now(),
            display: DisplaySettings::default(),
            elapsed: Duration::from_millis(30),
            layout: PresentationMode::AspectFit.layout(first, design),
            work: Duration::from_millis(3),
        };
        measurement.observe(sample)?;
        measurement.observe(sample)?;
        let resized = PresentationMode::AspectFit.layout(second, design);
        measurement.observe(FrameSample {
            layout: resized,
            ..sample
        })?;
        let segment = measurement
            .segment
            .as_ref()
            .ok_or(RenderError::InvalidSprite)?;
        if segment.layout != resized
            || segment.collector.report().frame_time.samples() != 1
            || segment.cadence.collector().report().frame_time.samples() != 0
        {
            return Err(RenderError::InvalidSprite);
        }
        measurement.finish()?;
        measurement.observe(sample)?;
        if measurement.segment.as_ref().is_none_or(|next| {
            next.cadence.collector().report().frame_time.samples() != 0
        }) {
            return Err(RenderError::InvalidSprite);
        }
        Ok(())
    }

    #[test]
    fn pause_disarms_applied_events_without_losing_pending_ticks() {
        let mut measurement = LiveMeasurement::default();
        let first = Duration::from_millis(10);
        let next = Duration::from_millis(20);
        measurement.admit(1, first);
        measurement.admit(3, next);
        measurement.advance(2);
        assert_eq!(measurement.finish(), Ok(()));
        assert_eq!(measurement.local.applied, None);
        measurement.advance(4);
        assert_eq!(measurement.local.applied, Some(next));
    }

    #[test]
    fn remote_and_local_ages_are_independent_on_first_submission()
    -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let design = Dimensions::new(dimension(320)?, dimension(224)?);
        let completed = Instant::now();
        let decoded = completed
            .checked_sub(Duration::from_millis(7))
            .ok_or(RenderError::InvalidSprite)?;
        let sample = FrameSample {
            completed,
            display: DisplaySettings::default(),
            elapsed: Duration::from_millis(30),
            layout: PresentationMode::AspectFit.layout(design, design),
            work: Duration::from_millis(3),
        };
        measurement.admit(1, Duration::from_millis(20));
        measurement.admit_remote(1, completed);
        measurement.admit_remote(1, decoded);
        measurement.advance(2);
        measurement.observe(sample)?;
        let segment = measurement
            .segment
            .as_ref()
            .ok_or(RenderError::InvalidSprite)?;
        let remote = segment.cadence.collector().report();
        if remote.frame_time.samples() != 0
            || remote.input_latency.samples() != 1
            || remote.input_latency.mean() != Some(Duration::from_millis(7))
            || segment.collector.report().input_latency.mean()
                != Some(Duration::from_millis(10))
        {
            return Err(RenderError::InvalidSprite);
        }
        measurement.observe(sample)?;
        let following = measurement
            .segment
            .as_ref()
            .ok_or(RenderError::InvalidSprite)?;
        let following_remote = following.cadence.collector().report();
        if following_remote.frame_time.samples() != 1
            || following_remote.input_latency.samples() != 1
            || following.collector.report().input_latency.samples() != 1
        {
            return Err(RenderError::InvalidSprite);
        }
        Ok(())
    }

    #[test]
    fn remote_age_survives_pending_ticks_but_not_suspension()
    -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let early = Instant::now();
        let later = early
            .checked_add(Duration::from_millis(1))
            .ok_or(RenderError::InvalidSprite)?;
        measurement.admit_remote(1, early);
        measurement.admit_remote(3, later);
        measurement.advance(1);
        if measurement.remote.applied.is_some() {
            return Err(RenderError::InvalidSprite);
        }
        measurement.advance(2);
        if measurement.remote.applied != Some(early) {
            return Err(RenderError::InvalidSprite);
        }
        measurement.finish()?;
        if measurement.remote.applied.is_some() {
            return Err(RenderError::InvalidSprite);
        }
        measurement.advance(4);
        if measurement.remote.applied != Some(later) {
            return Err(RenderError::InvalidSprite);
        }
        Ok(())
    }

    #[test]
    fn cadence_distinguishes_work_from_submission_intervals()
    -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let design = Dimensions::new(dimension(320)?, dimension(224)?);
        let surface = Dimensions::new(dimension(960)?, dimension(672)?);
        let sample = FrameSample {
            completed: Instant::now(),
            display: DisplaySettings::default(),
            elapsed: Duration::ZERO,
            layout: PresentationMode::AspectFit.layout(surface, design),
            work: Duration::from_millis(3),
        };
        measurement.observe(sample)?;
        measurement.observe(FrameSample {
            completed: sample
                .completed
                .checked_add(Duration::from_millis(20))
                .ok_or(RenderError::InvalidSprite)?,
            ..sample
        })?;
        measurement.observe(FrameSample {
            completed: sample
                .completed
                .checked_add(Duration::from_millis(30))
                .ok_or(RenderError::InvalidSprite)?,
            ..sample
        })?;
        let segment = measurement
            .segment
            .as_ref()
            .ok_or(RenderError::InvalidSprite)?;
        let cadence = segment.cadence.collector();
        let intervals = cadence.report();
        if intervals.frame_time.samples() != 2
            || intervals.frame_time.mean() != Some(Duration::from_millis(15))
            || cadence.frames_per_second() != Some(66)
            || intervals.frame_budget.map(|budget| budget.missed) != Some(1)
            || segment.collector.report().frame_time.mean()
                != Some(Duration::from_millis(3))
        {
            return Err(RenderError::InvalidSprite);
        }
        Ok(())
    }
}
