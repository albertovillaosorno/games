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
//   - Native monotonic timing and explicit process or artifact measurement.
// - Must-Not:
//   - Render frames, infer GPU latency or modify simulation timing.
// - Allows:
//   - Inputs: Host timestamps, optional resident bytes and artifact paths.
//   - Outputs: Shared performance observations and typed timing errors.
//   - Side effects: Explicit read-only process status and file metadata access.
// - Split-When:
//   - GPU or platform-specific instrumentation needs a dedicated adapter.
// - Merge-When:
//   - A shared native meter provides the same timestamp and resource probes.
// - Summary:
//   - Connects native measurements to the portable performance collector.
// - Description:
//   - Measures host completion intervals, not physical display scanout.
// - Usage:
//   - Timestamp completed presentations and explicitly sample resources.
// - Defaults:
//   - Never fabricate missing observations or sample memory every frame.

//! Native timestamp and resource observations for the shared collector.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::{Duration, Instant};

use games::performance::{
    FrameObservation, PerformanceCollector, PerformanceError,
};

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

/// Measures elapsed time between explicit native presentation completions.
#[derive(Clone, Debug)]
pub struct FrameMeter {
    collector: PerformanceCollector,
    previous: Instant,
}

impl FrameMeter {
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
    pub const fn new(
        collector: PerformanceCollector,
        previous: Instant,
    ) -> Self {
        Self { collector, previous }
    }

    /// Record a completed presentation and optional applied input timestamp.
    ///
    /// This measures the host-observed interval and input-to-completion age,
    /// not GPU execution or display scanout. The host chooses when to sample
    /// memory and which applied input to observe.
    ///
    /// # Errors
    /// Rejects reversed timestamps or aggregate overflow without advancing
    /// the preceding presentation timestamp.
    pub fn observe_frame(
        &mut self,
        completed: Instant,
        input_received: Option<Instant>,
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
    /// Useful when the first successful presentation starts a new segment.
    /// The preceding presentation timestamp remains unchanged.
    ///
    /// # Errors
    /// Rejects reversed timestamps or aggregate overflow.
    pub fn observe_input(
        &mut self,
        received: Instant,
        completed: Instant,
    ) -> Result<(), MeasurementError> {
        self.collector
            .observe_input(elapsed(received, completed)?)?;
        Ok(())
    }

    /// Record the actual distribution artifact size using file metadata.
    ///
    /// # Errors
    /// Rejects unreadable paths and paths which are not regular files.
    pub fn record_artifact(&mut self, path: &Path) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "artifact must be a regular file",
            ));
        }
        self.collector.record_binary_size(metadata.len());
        Ok(())
    }

    /// Record a complete load interval observed by the host.
    ///
    /// # Errors
    /// Rejects reversed timestamps without changing the measured duration.
    pub fn record_load(
        &mut self,
        started: Instant,
        completed: Instant,
    ) -> Result<(), MeasurementError> {
        self.collector.record_load(elapsed(started, completed)?);
        Ok(())
    }
}

fn elapsed(
    started: Instant,
    completed: Instant,
) -> Result<Duration, MeasurementError> {
    completed
        .checked_duration_since(started)
        .ok_or(MeasurementError::TimestampOrder)
}

/// Read the process's approximate resident set size in bytes from procfs.
///
/// Call at an explicit sampling cadence rather than in every rendered frame.
///
/// # Errors
/// Returns an error if procfs is unavailable or its resident-size field is
/// missing, malformed or outside the supported byte range.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn resident_bytes() -> io::Result<u64> {
    memory_bytes("VmRSS:")
}

/// Read the approximate kernel-reported lifetime resident high-water mark.
///
/// This includes startup, loading and any earlier process work; it is not
/// a peak isolated to a benchmark interval. The kernel value is approximate.
///
/// # Errors
/// Returns an error if procfs or its high-water mark is unavailable or invalid.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn peak_resident_bytes() -> io::Result<u64> {
    memory_bytes("VmHWM:")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn memory_bytes(prefix: &str) -> io::Result<u64> {
    let status = fs::read_to_string("/proc/self/status")?;
    parse_memory_bytes(&status, prefix).ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidData, "invalid procfs memory field")
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn parse_memory_bytes(status: &str, prefix: &str) -> Option<u64> {
    let field = status.lines().find_map(|line| line.strip_prefix(prefix))?;
    let mut words = field.split_ascii_whitespace();
    let kibibytes = words.next()?.parse::<u64>().ok()?;
    if words.next()? != "kB" || words.next().is_some() {
        return None;
    }
    kibibytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fmt::Debug;
    use std::io::{self, ErrorKind};
    use std::time::{Duration, Instant};

    use games::performance::{PerformanceCollector, QualityProfile};
    use games::presentation::FramePacing;

    use super::{FrameMeter, MeasurementError};

    fn meter(started: Instant) -> FrameMeter {
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

    fn check<T>(actual: T, expected: T) -> io::Result<()>
    where
        T: Copy + Debug + PartialEq,
    {
        if actual == expected {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "expected {expected:?}, observed {actual:?}"
            )))
        }
    }

    fn later(started: Instant, milliseconds: u64) -> io::Result<Instant> {
        started
            .checked_add(Duration::from_millis(milliseconds))
            .ok_or_else(|| io::Error::other("test timestamp overflow"))
    }

    #[test]
    fn records_completion_intervals_and_observed_input_age() -> io::Result<()> {
        let started = Instant::now();
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
    fn first_input_age_does_not_create_an_interval() -> io::Result<()> {
        let started = Instant::now();
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
    fn rejects_reversed_timestamps_without_changing_interval() -> io::Result<()>
    {
        let started = Instant::now();
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
    fn measures_load_and_artifact_without_replacing_them_on_error()
    -> io::Result<()> {
        let started = Instant::now();
        let mut meter = meter(started);
        let completed = later(started, 25)?;
        check(meter.record_load(started, completed), Ok(()))?;
        check(
            meter.record_load(completed, started),
            Err(MeasurementError::TimestampOrder),
        )?;
        check(
            meter.collector().load_duration(),
            Some(Duration::from_millis(25)),
        )?;
        let executable = env::current_exe()?;
        meter.record_artifact(&executable)?;
        check(
            meter.collector().binary_size(),
            Some(executable.metadata()?.len()),
        )?;
        let directory = env::temp_dir();
        check(
            meter
                .record_artifact(&directory)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidInput),
        )?;
        check(
            meter.collector().binary_size(),
            Some(executable.metadata()?.len()),
        )?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn parses_kernel_resident_size_without_guessing_units() -> io::Result<()> {
        use super::parse_memory_bytes;

        check(
            parse_memory_bytes("Name: games\nVmRSS:\t123 kB\n", "VmRSS:"),
            Some(125_952),
        )?;
        check(parse_memory_bytes("VmRSS: 0 kB", "VmRSS:"), Some(0))?;
        for invalid in [
            "VmSize: 100 kB",
            "VmRSS: 100 MB",
            "VmRSS: 100 kB extra",
            "VmRSS: -1 kB",
            "VmRSS: 18446744073709551615 kB",
        ] {
            check(parse_memory_bytes(invalid, "VmRSS:"), None)?;
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn distinguishes_current_and_lifetime_residency() -> io::Result<()> {
        use super::parse_memory_bytes;

        let status = "VmRSS: 32 kB\nVmHWM: 80 kB\n";
        check(parse_memory_bytes(status, "VmRSS:"), Some(32_768))?;
        check(parse_memory_bytes(status, "VmHWM:"), Some(81_920))?;
        check(parse_memory_bytes("VmRSS: 32 kB", "VmHWM:"), None)?;
        check(parse_memory_bytes("VmHWM: 0 kB", "VmHWM:"), Some(0))?;
        check(parse_memory_bytes("VmHWM: 4 MB", "VmHWM:"), None)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_real_process_resident_memory() -> io::Result<()> {
        let bytes = super::resident_bytes()?;
        check(bytes > 0, true)?;
        check(bytes.checked_rem(1024), Some(0))?;
        let peak = super::peak_resident_bytes()?;
        check(peak > 0, true)?;
        check(peak.checked_rem(1024), Some(0))?;
        Ok(())
    }
}
