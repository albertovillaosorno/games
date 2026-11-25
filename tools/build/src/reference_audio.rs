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
//   - Bounded exact comparison of private PCM16 reference recordings.
// - Must-Not:
//   - Resample, infer audio events, play audio or publish observations.
// - Allows:
//   - Inputs: Two explicit local WAV paths and a per-file byte budget.
//   - Outputs: Complete frame match count or first differing frame.
//   - Side effects: Read bounded regular files and write summary to stdout.
// - Split-When:
//   - Different comparison semantics require an independent policy.
// - Merge-When:
//   - Another host command duplicates exact PCM reference comparison.
// - Summary:
//   - Verify reproducibility of reference audio without storing extra data.
// - Description:
//   - Admit strict WAV capture layout and compare exact interleaved frames.
// - Usage:
//   - Supply two recordings and a positive maximum size for each.
// - Defaults:
//   - Reject any differing sample, format or incomplete capture.

//! Exact local reference audio comparison.

use std::num::{NonZeroU64, NonZeroUsize};
use std::path::Path;

use games_assets::AssetFormat;
use games_platform::ir::importer::reference::WavePcm;
use games_platform::ir::storage::snapshot;
use games_platform::ir::toolchain;

/// Compare two bounded complete PCM16 reference recordings.
/// # Errors
/// Rejects invalid files, unequal formats or any differing frame.
pub fn check(
    left: &Path,
    right: &Path,
    limit: NonZeroU64,
) -> Result<(), String> {
    let first = snapshot(left, limit)
        .map_err(|error| format!("left audio: {error:?}"))?;
    let second = snapshot(right, limit)
        .map_err(|error| format!("right audio: {error:?}"))?;
    let first_wave = WavePcm::new(&first, limit)
        .map_err(|error| format!("left WAV: {error:?}"))?;
    let second_wave = WavePcm::new(&second, limit)
        .map_err(|error| format!("right WAV: {error:?}"))?;
    let frames = compare(first_wave, second_wave)?;
    toolchain::write_line(&format!("Matched {frames} PCM sample frames"))
        .map_err(|error| error.to_string())
}

fn compare(left: WavePcm<'_>, right: WavePcm<'_>) -> Result<u64, String> {
    if left.format() != right.format() {
        return Err(String::from("audio layouts differ"));
    }
    let AssetFormat::PcmS16Le { channels, frames, .. } = left.format() else {
        return Err(String::from("not canonical PCM16"));
    };
    let width = NonZeroUsize::from(channels)
        .get()
        .checked_mul(2)
        .ok_or_else(|| String::from("frame width overflow"))?;
    let mismatch = left
        .payload()
        .chunks_exact(width)
        .zip(right.payload().chunks_exact(width))
        .position(|(left_frame, right_frame)| left_frame != right_frame);
    if let Some(index) = mismatch {
        return Err(format!("audio differs at sample frame {index}"));
    }
    Ok(frames.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAVE: [u8; 52] = [
        b'R', b'I', b'F', b'F', 44, 0, 0, 0, b'W', b'A', b'V', b'E', b'f',
        b'm', b't', b' ', 16, 0, 0, 0, 1, 0, 2, 0, 128, 187, 0, 0, 0, 238, 2,
        0, 4, 0, 16, 0, b'd', b'a', b't', b'a', 8, 0, 0, 0, 0, 128, 255, 127,
        1, 0, 255, 255,
    ];

    #[test]
    fn exact_comparison_includes_both_channels() -> Result<(), String> {
        let baseline = WavePcm::new(&WAVE, NonZeroU64::MAX)
            .map_err(|error| format!("{error:?}"))?;
        let mut different = WAVE;
        let last = different
            .last_mut()
            .ok_or_else(|| String::from("missing synthetic sample"))?;
        *last = 0;
        let control = WavePcm::new(&different, NonZeroU64::MAX)
            .map_err(|error| format!("{error:?}"))?;
        if compare(baseline, baseline) != Ok(2)
            || compare(baseline, control)
                != Err(String::from("audio differs at sample frame 1"))
        {
            return Err(String::from("exact PCM comparison differs"));
        }
        Ok(())
    }
}
