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
//   - Browser document monotonic clock observations.
// - Must-Not:
//   - Infer display latency or drive game simulation.
// - Allows:
//   - Inputs: Browser performance timestamps in milliseconds.
//   - Outputs: Comparable durations or typed observation errors.
//   - Side effects: Explicit read-only browser capability access.
// - Split-When:
//   - Additional measurements require an independent adapter.
// - Merge-When:
//   - A shared browser meter provides equivalent observations.
// - Summary:
//   - Connects the browser clock to portable measurement.
// - Description:
//   - Rejects invalid timestamps without substituting wall time.
// - Usage:
//   - Observe elapsed intervals within one document.
// - Defaults:
//   - Unsupported when no browser performance clock is available.

//! Browser document clock observations.

use std::io::{self, ErrorKind};
use std::time::Duration;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(in crate::software) fn now() -> io::Result<Duration> {
    let clock = web_sys::window()
        .and_then(|window| window.performance())
        .ok_or(ErrorKind::Unsupported)?;
    timestamp(clock.now())
}

pub(super) fn timestamp(milliseconds: f64) -> io::Result<Duration> {
    if milliseconds < 0.0f64 {
        return Err(ErrorKind::InvalidData.into());
    }
    Duration::try_from_secs_f64(milliseconds * 0.001f64)
        .map_err(|_error| ErrorKind::InvalidData.into())
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind};
    use std::time::Duration;

    use super::timestamp;

    #[test]
    fn converts_fractional_milliseconds() -> io::Result<()> {
        for (milliseconds, expected) in [
            (0.0f64, Duration::ZERO),
            (0.125f64, Duration::from_micros(125)),
            (1.5f64, Duration::from_micros(1500)),
            (1_234.5f64, Duration::from_micros(1_234_500)),
        ] {
            if timestamp(milliseconds)? != expected {
                return Err(io::Error::other("incorrect browser clock units"));
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_unrepresentable_timestamps() {
        for invalid in [
            -1.0f64,
            -f64::MIN_POSITIVE,
            f64::NAN,
            f64::INFINITY,
            f64::MAX,
        ] {
            assert_eq!(
                timestamp(invalid).err().map(|error| error.kind()),
                Some(ErrorKind::InvalidData),
                "invalid timestamp accepted: {invalid}"
            );
        }
    }
}
