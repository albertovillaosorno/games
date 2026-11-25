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
//   - Opt-in native gameplay submission and local input age measurements.
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
//   - Measures applied local input age, not physical display latency.
// - Description:
//   - Tracks one oldest event timestamp per pending nonempty input tick.
// - Usage:
//   - Enable explicitly and observe successfully submitted gameplay frames.
// - Defaults:
//   - No instrumentation in ordinary play.

use std::collections::BTreeMap;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::time::Duration;

use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::{DisplaySettings, FramePacing, PresentationLayout};
use games_platform::performance::resident_bytes;
use games_platform_graphics::linux::RenderError;

use super::failure;

const RSS_SAMPLE_FRAMES: u64 = 120;

/// Timing of one successful gameplay frame from host work to submission.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameSample {
    pub(super) display: DisplaySettings,
    pub(super) elapsed: Duration,
    pub(super) layout: PresentationLayout,
    pub(super) work: Duration,
}

struct Segment {
    collector: PerformanceCollector,
    display: DisplaySettings,
    layout: PresentationLayout,
}

impl Segment {
    fn new(
        profile: QualityProfile,
        display: DisplaySettings,
        layout: PresentationLayout,
    ) -> Self {
        Self {
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
        writeln!(
            stdout().lock(),
            concat!(
                "Live gameplay {}x{} logical, {}x{} drawable {:?}: ",
                "{} submitted frames; ",
                "work mean {:?}; max {:?}; p95 {:?}; budget {:?}; ",
                "oldest applied local input age {} samples; mean {:?}; ",
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
            report.input_latency.samples(),
            report.input_latency.mean(),
            self.collector.input_percentile(95),
            report.peak_resident_bytes,
        )
        .map_err(failure)
    }
}

/// Retains bounded input metadata only for successfully admitted actions.
#[derive(Default)]
pub(super) struct LiveMeasurement {
    applied: Option<Duration>,
    pending: BTreeMap<u64, Duration>,
    segment: Option<Segment>,
}

impl LiveMeasurement {
    /// Keep the oldest observed timestamp per nonempty admitted batch.
    pub(super) fn admit(&mut self, tick: u64, elapsed: Duration) {
        let _stored = self
            .pending
            .entry(tick)
            .and_modify(|oldest| *oldest = (*oldest).min(elapsed))
            .or_insert(elapsed);
    }

    /// Retain the oldest applied event until a gameplay frame is submitted.
    pub(super) fn advance(&mut self, next_tick: u64) {
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

    pub(super) fn finish(&mut self) -> Result<(), RenderError> {
        if let Some(segment) = self.segment.take() {
            segment.report()?;
        }
        self.applied = None;
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
            .applied
            .take()
            .and_then(|received| sample.elapsed.checked_sub(received));
        if self.segment.as_ref().is_some_and(|segment| {
            segment.layout != sample.layout || segment.display != sample.display
        }) {
            self.finish()?;
        }
        let segment = self.segment.get_or_insert_with(|| {
            Segment::new(profile, sample.display, sample.layout)
        });
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
        DisplaySettings, Duration, FrameSample, LiveMeasurement, RenderError,
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
        assert_eq!(measurement.applied, None);
        measurement.advance(3);
        assert_eq!(measurement.applied, Some(early));
        measurement.advance(5);
        assert_eq!(measurement.applied, Some(early));
        assert_eq!(measurement.pending.len(), 0);
    }

    #[test]
    fn successful_frames_measure_applied_input_only_once()
    -> Result<(), RenderError> {
        let mut measurement = LiveMeasurement::default();
        let design = Dimensions::new(dimension(320)?, dimension(224)?);
        let surface = Dimensions::new(dimension(960)?, dimension(672)?);
        let layout = PresentationMode::AspectFit.layout(surface, design);
        let sample = FrameSample {
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
        {
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
        assert_eq!(measurement.applied, None);
        measurement.advance(4);
        assert_eq!(measurement.applied, Some(next));
    }
}
