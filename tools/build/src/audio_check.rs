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
//   - Bounded composition of canonical sample loading, mixing and native queue.
// - Must-Not:
//   - Decode original media, infer event mappings or publish private audio.
// - Allows:
//   - Inputs: Trusted bundle digest, two clip keys and an output frame count.
//   - Outputs: Deterministic stereo mix and native queue admission summary.
//   - Side effects: Reads private assets and opens an initially paused device.
// - Split-When:
//   - Interactive game playback requires independent host composition.
// - Merge-When:
//   - Another diagnostic duplicates canonical audio consumption.
// - Summary:
//   - Verify chunk-invariant mixing and paused native audio submission.
// - Description:
//   - Load selected PCM, compare distinct render partitions and check queues.
// - Usage:
//   - Select a pinned bundle and clips with an explicit finite frame count.
// - Defaults:
//   - Never start audible playback or write original-derived data.

//! Finite canonical audio consumption diagnostic.

use std::ffi::OsString;
use std::io::{Write as _, stdout};
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use games::audio::{Channels, Clip, Mixer, StereoGain};
use games_assets::ContentHash;
use games_assets::pcm::PcmSamples;
use games_importer::mame::Machine;
use games_importer::selection::AssetSelection;
use games_platform::ir::audio::{NativeAudio, PlaybackSpec};
use games_platform::ir::storage::bundle::CanonicalBundle;

use crate::{USAGE, text};

const CHUNK: usize = 1024;
const MAX_FRAMES: u32 = 480_000;
const OUTPUT_RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(47_999);

/// Explicit finite sample selection and output frame count.
#[derive(Debug)]
pub struct Arguments {
    digest: ContentHash,
    frames: NonZeroU32,
    left: String,
    parent: PathBuf,
    right: String,
}

impl Arguments {
    /// Parse a pinned bundle, two sample keys and an explicit frame limit.
    /// # Errors
    /// Rejects incomplete arguments, invalid digests and excessive work.
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, digest, left, right, frames] =
            <[OsString; 5]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let count: NonZeroU32 = text(&frames)?
            .parse()
            .map_err(|_error| String::from("invalid output frame count"))?;
        if count.get() > MAX_FRAMES {
            return Err(String::from("audio frame count exceeds limit"));
        }
        Ok(Self {
            digest: text(&digest)?
                .parse()
                .map_err(|_error| String::from("invalid manifest hash"))?,
            frames: count,
            left: text(&left)?.to_owned(),
            parent: parent.into(),
            right: text(&right)?.to_owned(),
        })
    }

    /// Verify canonical payloads and exercise bounded native submissions.
    /// # Errors
    /// Rejects invalid assets, allocation, divergent mix or queue failures.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let selection = AssetSelection::Audio(machine);
        let bundle = CanonicalBundle::open(
            &self.parent,
            self.digest,
            selection.limits(),
        )
        .map_err(|error| format!("bundle: {error:?}"))?;
        selection
            .validate_manifest(bundle.manifest())
            .map_err(|error| format!("manifest: {error:?}"))?;
        let left = Decoded::load(&bundle, &self.left)?;
        let right = Decoded::load(&bundle, &self.right)?;
        let mut whole = stereo_pair(&left, &right)?;
        let mut split = stereo_pair(&left, &right)?;
        let device = NativeAudio::open(PlaybackSpec {
            buffer_frames: NonZeroU16::MIN.saturating_add(255),
            channels: NonZeroU8::MIN.saturating_add(1),
            max_queued_frames: NonZeroU32::MIN.saturating_add(1023),
            sample_rate: OUTPUT_RATE,
        })
        .map_err(|error| format!("device: {error:?}"))?;
        let frames = NonZeroUsize::try_from(self.frames)
            .map_err(|_error| String::from("frame count not representable"))?;
        let MixReport { audible, mixing } =
            verify_mix(&mut whole, &mut split, &device, frames.get())?;
        let count = self.frames;
        let active = whole.active();
        let mut out = stdout().lock();
        writeln!(
            out,
            "{count} frames: {audible} nonzero, {active} active at 48000 Hz"
        )
        .and_then(|()| {
            writeln!(
                out,
                "Whole-block mix wall time: {} ns (excludes split and queue)",
                mixing.as_nanos()
            )
        })
        .map_err(|error| error.to_string())
    }
}

#[derive(Debug)]
struct Decoded {
    rate: NonZeroU32,
    samples: Vec<i16>,
}

impl Decoded {
    fn clip(&self) -> Result<Clip<'_>, String> {
        Clip::new(&self.samples, Channels::Mono, self.rate)
            .map_err(|error| format!("clip: {error:?}"))
    }

    fn load(bundle: &CanonicalBundle, key: &str) -> Result<Self, String> {
        let record = bundle
            .manifest()
            .record(key)
            .ok_or_else(|| format!("missing sample {key}"))?;
        let bytes = bundle
            .read_payload(key)
            .map_err(|error| format!("payload: {error:?}"))?;
        let pcm = PcmSamples::new(record, &bytes)
            .map_err(|error| format!("PCM: {error:?}"))?;
        if pcm.channels() != NonZeroU8::MIN {
            return Err(String::from("expected mono sample"));
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(pcm.samples().len())
            .map_err(|_error| String::from("sample allocation failed"))?;
        samples.extend(pcm.samples());
        Ok(Self {
            rate: pcm.sample_rate(),
            samples,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct MixReport {
    audible: usize,
    mixing: Duration,
}

fn stereo_pair<'samples>(
    left: &'samples Decoded,
    right: &'samples Decoded,
) -> Result<Mixer<'samples>, String> {
    let mut mixer = Mixer::new(OUTPUT_RATE, NonZeroU16::MIN.saturating_add(1))
        .map_err(|error| format!("mixer: {error:?}"))?;
    let _left = mixer
        .play(left.clip()?, StereoGain { left: u16::MAX, right: 0 })
        .map_err(|error| format!("left: {error:?}"))?;
    let _right = mixer
        .play(right.clip()?, StereoGain { left: 0, right: u16::MAX })
        .map_err(|error| format!("right: {error:?}"))?;
    Ok(mixer)
}

fn verify_mix(
    whole: &mut Mixer<'_>,
    split: &mut Mixer<'_>,
    device: &NativeAudio,
    frames: usize,
) -> Result<MixReport, String> {
    let mut expected = [[0; 2]; CHUNK];
    let mut actual = [[0; 2]; CHUNK];
    let mut remaining = frames;
    let mut audible = 0usize;
    let mut mixing = Duration::ZERO;
    while remaining != 0 {
        let count = remaining.min(CHUNK);
        let block = expected
            .get_mut(..count)
            .ok_or_else(|| String::from("invalid mix extent"))?;
        let partition = actual
            .get_mut(..count)
            .ok_or_else(|| String::from("invalid mix extent"))?;
        let started = Instant::now();
        whole.render(block);
        mixing = mixing
            .checked_add(started.elapsed())
            .ok_or_else(|| String::from("mix timing overflow"))?;
        for segment in partition.chunks_mut(113) {
            split.render(segment);
        }
        if block != partition || whole.active() != split.active() {
            return Err(String::from("mix depends on chunk boundaries"));
        }
        audible = audible.saturating_add(
            block.iter().filter(|frame| **frame != [0, 0]).count(),
        );
        let samples = block.as_flattened();
        device
            .queue_samples(samples)
            .map_err(|error| format!("queue: {error:?}"))?;
        let expected_bytes = u32::try_from(count)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| String::from("queue size overflow"))?;
        if device.queued_bytes() != expected_bytes {
            return Err(String::from("native queue extent differs"));
        }
        device.clear();
        remaining = remaining.saturating_sub(count);
    }
    Ok(MixReport { audible, mixing })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_or_unbounded_work() {
        let digest = "0".repeat(64);
        let valid = ["root", digest.as_str(), "audio/a", "audio/b", "1024"];
        assert!(
            Arguments::parse(valid.into_iter().map(OsString::from).collect())
                .is_ok()
        );
        for count in ["0", "480001", "-1"] {
            let mut invalid = valid;
            invalid[4] = count;
            assert!(
                Arguments::parse(
                    invalid.into_iter().map(OsString::from).collect()
                )
                .is_err()
            );
        }
        assert!(
            Arguments::parse(
                valid.into_iter().take(4).map(OsString::from).collect()
            )
            .is_err()
        );
    }
}
