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
//   - Bounded comparison, recurrence measurement and selection of PCM16.
// - Must-Not:
//   - Resample, infer audio events, play audio or bypass storage.
// - Allows:
//   - Inputs: Explicit WAV paths, frame windows, lags and resource budgets.
//   - Outputs: Frame match count, first difference or bounded recurrence score.
//   - Side effects: Read bounded regular files and write summary to stdout.
// - Split-When:
//   - Different comparison semantics require an independent policy.
// - Merge-When:
//   - Another host command duplicates exact PCM reference comparison.
// - Summary:
//   - Verify reproducibility of reference audio without storing extra data.
// - Description:
//   - Admit strict WAV layout and compare interleaved frames or samples.
// - Usage:
//   - Supply recordings, explicit search ranges and positive limits.
// - Defaults:
//   - Reject any differing sample, format or incomplete capture.

//! Exact local reference audio comparison.

use std::ffi::{OsStr, OsString};
use std::num::{NonZeroU64, NonZeroUsize};
use std::ops::Range;

use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
    SourceIdentity,
};
use games_platform::ir::importer::reference::WavePcm;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::staging::CanonicalStaging;
use games_platform::ir::toolchain;

use crate::{USAGE, parse_limit, publication, text};

/// An explicitly selected complete PCM frame window.
#[derive(Debug)]
pub(crate) struct Publication {
    identity: SourceIdentity,
    key: String,
    limit: NonZeroU64,
    parent: OsString,
    path: OsString,
    window: Range<u64>,
}

impl Publication {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [title, revision, parent, path, key, start, end, limit] =
            <[OsString; 8]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let identity = SourceIdentity {
            revision: text(&revision)?.to_owned(),
            title: text(&title)?.to_owned(),
        };
        identity
            .validate()
            .map_err(|error| format!("identity: {error:?}"))?;
        let frame = |value: &OsString| {
            text(value)?
                .parse::<u64>()
                .map_err(|_error| String::from("invalid frame index"))
        };
        let window = frame(&start)?..frame(&end)?;
        if window.is_empty() {
            return Err(String::from("empty PCM window"));
        }
        Ok(Self {
            identity,
            key: text(&key)?.to_owned(),
            limit: parse_limit(&limit)?,
            parent,
            path,
            window,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        publication::require_asset_parent(&self.parent, &self.identity.title)?;
        let source = snapshot(&self.path, self.limit)
            .map_err(|error| format!("source WAV: {error:?}"))?;
        let wave = WavePcm::new(&source, self.limit)
            .and_then(|pcm| pcm.slice(self.window))
            .map_err(|error| format!("PCM window: {error:?}"))?;
        let bytes = wave.payload();
        let size = u64::try_from(bytes.len())
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or_else(|| String::from("invalid PCM window size"))?;
        let digest = ContentHash::calculate(bytes);
        let record = AssetRecord {
            digest,
            format: wave.format(),
            key: self.key,
            size: size.get(),
        };
        let limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: size,
            records: NonZeroUsize::MIN,
            total_bytes: size,
        };
        let manifest = AssetManifest::new(self.identity, vec![record], limits)
            .map_err(|error| format!("PCM manifest: {error:?}"))?;
        let mut stage = CanonicalStaging::new(
            &self.parent,
            publication::staging_budget(limits)?,
        )
        .map_err(|error| format!("stage: {error:?}"))?;
        stage
            .write_payload(digest, bytes)
            .map_err(|error| format!("stage: {error:?}"))?;
        publication::finish(stage, &manifest, limits)
    }
}

/// Explicit, bounded lag search over a single reference recording.
#[derive(Debug)]
pub(crate) struct Recurrence {
    byte_limit: NonZeroU64,
    first_lag: NonZeroU64,
    frame_stride: NonZeroUsize,
    lag_step: NonZeroU64,
    last_lag: NonZeroU64,
    path: OsString,
    window: Range<u64>,
    work_limit: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Match {
    candidate_energy: u128,
    lag: u64,
    reference_energy: u128,
    squared_error: u128,
    ties: u64,
}

#[derive(Debug, Clone, Copy)]
struct SamplingPlan {
    candidates: u64,
    frame_width: usize,
}

impl Recurrence {
    fn admit(&self, wave: WavePcm<'_>) -> Result<SamplingPlan, String> {
        let AssetFormat::PcmS16Le { channels, frames, .. } = wave.format()
        else {
            return Err(String::from("not canonical PCM16"));
        };
        let invalid = || String::from("invalid lag or reference window");
        let duration = self
            .window
            .end
            .checked_sub(self.window.start)
            .and_then(NonZeroU64::new)
            .ok_or_else(invalid)?;
        let span = self
            .last_lag
            .get()
            .checked_sub(self.first_lag.get())
            .ok_or_else(invalid)?;
        if self
            .window
            .end
            .checked_add(self.last_lag.get())
            .is_none_or(|end| end > frames.get())
        {
            return Err(invalid());
        }
        let candidates = span
            .checked_div(self.lag_step.get())
            .and_then(|count| count.checked_add(1))
            .ok_or_else(invalid)?;
        let stride = u64::try_from(self.frame_stride.get())
            .map_err(|_error| invalid())?;
        let work = duration
            .get()
            .div_ceil(stride)
            .checked_mul(candidates)
            .and_then(|count| {
                count.checked_mul(NonZeroU64::from(channels).get())
            })
            .filter(|count| *count <= self.work_limit.get());
        if work.is_none() {
            return Err(String::from("lag search exceeds sample-pair budget"));
        }
        let width = NonZeroUsize::from(channels).get().saturating_mul(2);
        Ok(SamplingPlan {
            candidates,
            frame_width: width,
        })
    }

    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [path, start, end, first, last, step, stride, work, limit] =
            <[OsString; 9]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let frame = |value: &OsString| {
            text(value)?
                .parse::<u64>()
                .map_err(|_error| String::from("invalid frame index"))
        };
        Ok(Self {
            byte_limit: parse_limit(&limit)?,
            first_lag: parse_limit(&first)?,
            frame_stride: text(&stride)?
                .parse()
                .map_err(|_error| String::from("invalid frame stride"))?,
            lag_step: parse_limit(&step)?,
            last_lag: parse_limit(&last)?,
            path,
            window: frame(&start)?..frame(&end)?,
            work_limit: parse_limit(&work)?,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let source = snapshot(&self.path, self.byte_limit)
            .map_err(|error| format!("source WAV: {error:?}"))?;
        let wave = WavePcm::new(&source, self.byte_limit)
            .map_err(|error| format!("source WAV: {error:?}"))?;
        let result = self.search(wave)?;
        toolchain::write_line(&format!(
            "Best lag: {} frames; tied candidates: {}; squared error: {}; \
             reference energy: {}; candidate energy: {}",
            result.lag,
            result.ties,
            result.squared_error,
            result.reference_energy,
            result.candidate_energy
        ))
        .map_err(|error| error.to_string())
    }

    fn search(&self, wave: WavePcm<'_>) -> Result<Match, String> {
        let plan = self.admit(wave)?;
        let reference = wave
            .slice(self.window.clone())
            .map_err(|error| format!("reference window: {error:?}"))?;
        let mut best: Option<Match> = None;
        for index in 0..plan.candidates {
            let lag = self
                .first_lag
                .get()
                .saturating_add(index.saturating_mul(self.lag_step.get()));
            let candidate = wave
                .slice(
                    self.window.start.saturating_add(lag)
                        ..self.window.end.saturating_add(lag),
                )
                .map_err(|error| format!("candidate window: {error:?}"))?;
            let score = score_windows(
                reference,
                candidate,
                (plan.frame_width, self.frame_stride),
                lag,
            );
            match best.as_mut() {
                Some(current)
                    if current.squared_error == score.squared_error =>
                {
                    current.ties = current.ties.saturating_add(1);
                },
                Some(current)
                    if current.squared_error < score.squared_error => {},
                _ => best = Some(score),
            }
        }
        best.ok_or_else(|| String::from("empty lag search"))
    }
}

/// Compare two bounded complete PCM16 reference recordings.
/// # Errors
/// Rejects invalid files, unequal formats or any differing frame.
pub fn check(
    left: &OsStr,
    right: &OsStr,
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

fn score_windows(
    reference: WavePcm<'_>,
    candidate: WavePcm<'_>,
    sampling: (usize, NonZeroUsize),
    lag: u64,
) -> Match {
    let (width, stride) = sampling;
    let mut result = Match {
        candidate_energy: 0,
        lag,
        reference_energy: 0,
        squared_error: 0,
        ties: 1,
    };
    let pairs = reference
        .payload()
        .chunks_exact(width)
        .step_by(stride.get())
        .zip(
            candidate
                .payload()
                .chunks_exact(width)
                .step_by(stride.get()),
        );
    for (left_frame, right_frame) in pairs {
        for (left_sample, right_sample) in left_frame
            .as_chunks::<2>()
            .0
            .iter()
            .zip(right_frame.as_chunks::<2>().0)
        {
            let left_value = i32::from(i16::from_le_bytes(*left_sample));
            let right_value = i32::from(i16::from_le_bytes(*right_sample));
            let difference = left_value.saturating_sub(right_value);
            result.squared_error = result
                .squared_error
                .saturating_add(u128::from(difference.unsigned_abs()).pow(2));
            result.reference_energy = result
                .reference_energy
                .saturating_add(u128::from(left_value.unsigned_abs()).pow(2));
            result.candidate_energy = result
                .candidate_energy
                .saturating_add(u128::from(right_value.unsigned_abs()).pow(2));
        }
    }
    result
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

    fn repeated_wave() -> Result<Vec<u8>, String> {
        let mut source = WAVE.to_vec();
        // Three identical two-frame stereo periods, with distinct channels.
        let payload = WAVE
            .get(44..)
            .ok_or_else(|| String::from("fixture payload absent"))?;
        source.extend_from_slice(payload);
        source.extend_from_slice(payload);
        source
            .get_mut(4..8)
            .ok_or_else(|| String::from("fixture RIFF length absent"))?
            .copy_from_slice(&60u32.to_le_bytes());
        source
            .get_mut(40..44)
            .ok_or_else(|| String::from("fixture data length absent"))?
            .copy_from_slice(&24u32.to_le_bytes());
        Ok(source)
    }

    fn search_fixture(
        wave: WavePcm<'_>,
        lags: (u64, u64),
        work: u64,
        stride: usize,
    ) -> Result<Match, String> {
        let args = [
            "unused".to_owned(),
            "0".to_owned(),
            "2".to_owned(),
            lags.0.to_string(),
            lags.1.to_string(),
            "1".to_owned(),
            stride.to_string(),
            work.to_string(),
            "1000".to_owned(),
        ];
        let query =
            Recurrence::parse(args.into_iter().map(OsString::from).collect())?;
        query.search(wave)
    }

    #[test]
    fn recurrence_distinguishes_unique_and_tied_matches() -> Result<(), String>
    {
        let source = repeated_wave()?;
        let wave = WavePcm::new(&source, NonZeroU64::MAX)
            .map_err(|error| format!("{error:?}"))?;
        let unique = search_fixture(wave, (1, 3), 12, 1)?;
        let tied = search_fixture(wave, (1, 4), 16, 1)?;
        if unique.lag != 2
            || unique.ties != 1
            || unique.squared_error != 0
            || unique.reference_energy != 2_147_418_115
            || unique.candidate_energy != unique.reference_energy
            || tied.lag != 2
            || tied.ties != 2
        {
            return Err(String::from("recurrence score or tie differs"));
        }
        Ok(())
    }

    #[test]
    fn recurrence_checks_budget_and_shifted_window() -> Result<(), String> {
        let source = repeated_wave()?;
        let wave = WavePcm::new(&source, NonZeroU64::MAX)
            .map_err(|error| format!("{error:?}"))?;
        if search_fixture(wave, (1, 3), 11, 1).is_ok()
            || search_fixture(wave, (1, 3), 6, 2)?.lag != 2
            || search_fixture(wave, (1, 5), 100, 1).is_ok()
            || search_fixture(wave, (3, 1), 100, 1).is_ok()
            || search_fixture(wave, (0, 2), 100, 1).is_ok()
            || search_fixture(wave, (1, 2), 100, 0).is_ok()
        {
            return Err(String::from("recurrence bounds were not enforced"));
        }
        Ok(())
    }
}
