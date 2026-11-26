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
//   - Browser audio source scheduling timestamps.
// - Must-Not:
//   - Open a context, own PCM or advance simulation time.
// - Allows:
//   - Inputs: Observed audio time, startup lead and scheduled tail.
//   - Outputs: Admitted next source start time.
//   - Side effects: None.
// - Split-When:
//   - A distinct audio scheduling policy is required.
// - Merge-When:
//   - Another browser adapter duplicates source timestamp selection.
// - Summary:
//   - Preserve contiguous scheduled playback with bounded startup lead.
// - Description:
//   - Distinguish the queued source tail from an idle audio timeline.
// - Usage:
//   - Select a start before scheduling a new buffer source.
// - Defaults:
//   - Reserve one requested buffer period on idle startup or underrun.

//! Timestamp policy for browser PCM source scheduling.

use crate::ir::audio::PlaybackError;

pub(super) fn next_start(
    now: f64,
    lead: f64,
    tail: Option<f64>,
) -> Result<f64, PlaybackError> {
    let earliest = now + lead;
    if !earliest.is_finite()
        || now.is_sign_negative()
        || !lead.is_finite()
        || lead <= 0.0f64
        || tail.is_some_and(|end| !end.is_finite() || end.is_sign_negative())
    {
        return Err(PlaybackError::InvalidState);
    }
    // A live tail already has a scheduled start. Applying the startup lead
    // again would create a silent gap despite the source still playing.
    Ok(tail.filter(|end| *end > now).unwrap_or(earliest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuing_source_stays_contiguous() {
        for tail in [10.125f64, 10.25f64, 10.5f64] {
            assert_eq!(
                next_start(10.0, 0.25, Some(tail)).map(f64::to_bits),
                Ok(tail.to_bits()),
                "a queued future tail must not acquire a second startup lead"
            );
        }
    }

    #[test]
    fn idle_tail_reserves_startup_lead() {
        for tail in [None, Some(9.5f64), Some(10.0f64)] {
            assert_eq!(
                next_start(10.0, 0.25, tail).map(f64::to_bits),
                Ok(10.25f64.to_bits()),
                "idle or elapsed playback requires a fresh startup lead"
            );
        }
    }

    #[test]
    fn invalid_clock_or_tail_is_rejected() {
        for (now, lead, tail) in [
            (f64::NAN, 0.25f64, None),
            (-1.0f64, 0.25f64, None),
            (1.0f64, 0.0f64, None),
            (1.0f64, 0.25f64, Some(f64::INFINITY)),
        ] {
            assert_eq!(
                next_start(now, lead, tail),
                Err(PlaybackError::InvalidState)
            );
        }
    }
}
