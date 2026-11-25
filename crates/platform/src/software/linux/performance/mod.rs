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

pub use crate::ir::measurement::MeasurementError;
use crate::ir::measurement::{
    FrameMeter as PortableFrameMeter, MeasurementTimestamp,
};

/// Whether the selected host includes native measurement services.
pub const AVAILABLE: bool = true;

/// Native timestamp specialization of the portable measurement meter.
pub type FrameMeter = PortableFrameMeter<Instant>;

impl MeasurementTimestamp for Instant {
    fn checked_elapsed(self, earlier: Self) -> Option<Duration> {
        self.checked_duration_since(earlier)
    }
}

impl FrameMeter {
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
        self.record_artifact_bytes(metadata.len());
        Ok(())
    }
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
    fn native_timestamps_feed_the_portable_meter() -> io::Result<()> {
        let started = Instant::now();
        let mut meter = meter(started);
        let completed = later(started, 10)?;
        check(meter.observe_frame(completed, Some(started), None), Ok(()))?;
        check(
            meter.collector().report().frame_time.total(),
            Duration::from_millis(10),
        )?;
        check(
            meter.collector().report().input_latency.mean(),
            Some(Duration::from_millis(10)),
        )
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
