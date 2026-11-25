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
//   - Finite native graphics diagnostic workload and observation reporting.
// - Must-Not:
//   - Interpret titles, change simulation timing or infer scanout latency.
// - Allows:
//   - Inputs: Verified persistent atlas scene, commands and frame count.
//   - Outputs: Host frame timing, sampled RSS, load and artifact observations.
//   - Side effects: Native presentation and explicit host resource sampling.
// - Split-When:
//   - Other benchmark workload families require separate composition.
// - Merge-When:
//   - A shared workload driver can express the same measurement schedule.
// - Summary:
//   - Measure native imported graphics scene submission with fixed quality.
// - Description:
//   - Warm the renderer, then measure finite frames without pixel readback.
// - Usage:
//   - Call only after verified scene preparation and a pixel check.
// - Defaults:
//   - Hidden uncapped window, no input observations or inferred GPU timings.

//! Imported graphics diagnostic host workload measurement.

use std::env;
use std::io::{Write as _, stdout};
use std::num::{NonZeroU32, NonZeroU64};
use std::time::{Duration, Instant};

use games::performance::{
    PercentileRange, PerformanceCollector, QualityProfile,
};
use games::presentation::FramePacing;
use games_platform::performance::{FrameMeter, resident_bytes};
use games_platform_graphics::linux::sprites::{SceneSession, SpriteDraw};
use games_platform_graphics::linux::{FrameLayout, RenderError};

use crate::render_check::present;

const SIXTY_HZ_NS: NonZeroU64 = NonZeroU64::MIN.saturating_add(16_666_666);
const WARMUP_FRAMES: u32 = 8;

/// Explicit finite benchmark schedule and observed load interval.
#[derive(Clone, Copy, Debug)]
pub struct Schedule {
    /// Number of measured presentations, excluding warmup.
    pub frames: NonZeroU32,
    /// Completion of the initial verified atlas upload.
    pub loaded: Instant,
    /// Start of local bundle and palette preparation.
    pub started: Instant,
}

/// Measure a prepared scene using the existing native observation meter.
/// # Errors
/// Rejects failed presentations, timing overflow or artifact metadata failure.
pub fn measure(
    scene: &mut SceneSession<'_, '_>,
    commands: &[SpriteDraw],
    layout: FrameLayout,
    schedule: Schedule,
) -> Result<PerformanceCollector, RenderError> {
    measure_workload(layout, schedule, || present(scene, commands))
}

/// Measure any prepared workload with the same native host timing policy.
/// # Errors
/// Rejects failed workload steps, timing overflow or artifact failure.
pub fn measure_workload<Draw>(
    layout: FrameLayout,
    schedule: Schedule,
    mut draw: Draw,
) -> Result<PerformanceCollector, RenderError>
where
    Draw: FnMut() -> Result<(), RenderError>,
{
    for _frame in 0..WARMUP_FRAMES {
        draw()?;
    }
    let profile = QualityProfile {
        frame_pacing: FramePacing::Uncapped,
        height: layout.height().get(),
        optional_effects: false,
        width: layout.width().get(),
    };
    let collector =
        PerformanceCollector::with_percentiles(profile, Some(SIXTY_HZ_NS));
    let mut meter = FrameMeter::new(collector, Instant::now());
    for frame in 1..=schedule.frames.get() {
        draw()?;
        let completed = Instant::now();
        let rss = if frame == 1
            || frame.is_multiple_of(60)
            || frame == schedule.frames.get()
        {
            resident_bytes().ok()
        } else {
            None
        };
        meter.observe_frame(completed, None, rss).map_err(|error| {
            RenderError::Backend(format!("timing: {error:?}"))
        })?;
    }
    meter
        .record_load(schedule.started, schedule.loaded)
        .map_err(|error| RenderError::Backend(format!("load: {error:?}")))?;
    let path = env::current_exe()
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    meter
        .record_artifact(&path)
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    Ok(meter.into_collector())
}

/// Report measured quantities without inventing latency or VRAM data.
/// # Errors
/// Rejects a failed summary write.
pub fn report(
    collector: &PerformanceCollector,
    renderer: &str,
    workload: &str,
) -> Result<(), String> {
    let profile = collector.profile();
    let report = collector.report();
    let frames = report.frame_time;
    let mean = duration(frames.mean());
    let maximum = duration(frames.maximum());
    let p50 = percentile(collector.frame_percentile(50));
    let p95 = percentile(collector.frame_percentile(95));
    let p99 = percentile(collector.frame_percentile(99));
    let fps = observed(collector.frames_per_second());
    let load = duration(collector.load_duration());
    let rss = observed(report.peak_resident_bytes);
    let bytes = observed(collector.binary_size());
    let misses = observed(report.frame_budget.map(|budget| budget.missed));
    let mut out = stdout().lock();
    writeln!(
        out,
        "Host submission: {renderer}, {}x{} logical, {workload}, uncapped",
        profile.width, profile.height,
    )
    .and_then(|()| {
        writeln!(
            out,
            "Frames: {}, mean: {mean}, max: {maximum}, FPS: {fps}",
            frames.samples(),
        )
    })
    .and_then(|()| {
        writeln!(
            out,
            "Frame percentile ranges: p50 {p50}, p95 {p95}, p99 {p99}"
        )
    })
    .and_then(|()| writeln!(out, "60 Hz target misses: {misses}, load: {load}"))
    .and_then(|()| {
        writeln!(
            out,
            "Sampled peak RSS bytes: {rss}, executable bytes: {bytes}",
        )
    })
    .and_then(|()| {
        writeln!(
            out,
            "Physical input/display latency and GPU/VRAM usage: not measured",
        )
    })
    .map_err(|error| error.to_string())
}

fn duration(value: Option<Duration>) -> String {
    value.map_or_else(
        || String::from("unobserved"),
        |time| format!("{} ns", time.as_nanos()),
    )
}

fn percentile(range: Option<PercentileRange>) -> String {
    range.map_or_else(
        || String::from("unobserved"),
        |bounds| {
            format!(
                "{}..={} ns",
                bounds.lower.as_nanos(),
                bounds.upper.as_nanos()
            )
        },
    )
}

fn observed(value: Option<u64>) -> String {
    value.map_or_else(|| String::from("unobserved"), |count| count.to_string())
}
