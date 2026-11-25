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
//   - Local attack cue selection and bounded native audio presentation.
// - Must-Not:
//   - Interpret ROM bytes, duplicate mixing or influence simulation time.
// - Allows:
//   - Inputs: Canonical PCM and cumulative fixed-step station cues.
//   - Outputs: Bounded mixed stereo submissions to the shared audio host.
//   - Side effects: Local device playback and explicit voice admission.
// - Split-When:
//   - Additional independently scheduled music or audio buses are required.
// - Merge-When:
//   - Another title-local module duplicates native cue delivery.
// - Summary:
//   - Optional four-station attack sample playback.
// - Description:
//   - Observe each fixed-step cue once and refill a short software queue.
// - Usage:
//   - Load the verified local audio catalogue before opening the device.
// - Defaults:
//   - No device unless requested; no music or original audio parity claim.

//! Opt-in playback of observed attack-associated PCM clips.

use std::ffi::OsString;
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64};
use std::path::Path;
use std::time::Duration;

use games::audio::{AudioError, Channels, Clip, Mixer, StereoGain};
use games::input::InputEvent;
use games::replay::LiveSimulation;
use games_assets::ManifestLimits;
use games_assets::pcm::PcmSamples;
use games_platform_audio::linux::{NativeAudio, PlaybackSpec};
use games_platform_graphics::linux::RenderError;
use games_platform_graphics::linux::input::key_code;

use super::{Controls, KEYBOARD, failure, live, open_selected_bundle, size};
use crate::{Cabinet, TICK_INTERVAL};

const CHUNK: usize = 256;
const MAX_CHUNKS: u8 = 4;
const QUEUE_FRAMES: u32 = 1024;
const RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(47_999);
const SOURCE_RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(9_999);
// Observed PCM pitch 0xe9b with the 3,579,545 Hz chip clock:
// round(3,579,545 / (4096 - 0xe9b)). The catalogue stays nominal.
const ATTACK_RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(10_026);
// Observed volume and pan mapped to the shared 16-bit gain scale,
// including the reference board half-gain stereo routes.
const GAINS: [StereoGain; 4] = [
    StereoGain {
        left: 10_904,
        right: 4_888,
    },
    StereoGain {
        left: 14_768,
        right: 10_366,
    },
    StereoGain {
        left: 6_862,
        right: 9_776,
    },
    StereoGain {
        left: 4_056,
        right: 9_048,
    },
];
const CLIPS: [(&str, u64); 4] = [
    ("audio/sample-0004", 8335),
    ("audio/sample-0005", 3916),
    ("audio/sample-0006", 2944),
    ("audio/sample-0007", 3828),
];

pub(super) struct Samples {
    clips: [Vec<i16>; 4],
}

impl Samples {
    fn clips(&self) -> Result<[Clip<'_>; 4], RenderError> {
        let [marge, homer, bart, lisa] = &self.clips;
        let clip = |samples| {
            Clip::new(samples, Channels::Mono, ATTACK_RATE).map_err(failure)
        };
        Ok([clip(marge)?, clip(homer)?, clip(bart)?, clip(lisa)?])
    }

    pub(super) fn load(
        parent: &Path,
        hash: &OsString,
    ) -> Result<Self, RenderError> {
        let bundle = open_selected_bundle(parent, hash, ManifestLimits {
            manifest_bytes: size(65_536)?,
            payload_bytes: NonZeroU64::MIN.saturating_add(264_323),
            records: size(161)?,
            total_bytes: NonZeroU64::MIN.saturating_add(3_798_887),
        })?;
        let mut clips = Vec::with_capacity(CLIPS.len());
        for (key, frames) in CLIPS {
            let record = bundle
                .manifest()
                .record(key)
                .ok_or_else(|| failure("missing attack PCM"))?;
            let bytes = bundle.read_payload(key).map_err(failure)?;
            let pcm = PcmSamples::new(record, &bytes).map_err(failure)?;
            if pcm.channels() != NonZeroU8::MIN
                || pcm.sample_rate() != SOURCE_RATE
                || pcm.frames().get() != frames
            {
                return Err(failure("unexpected attack PCM format"));
            }
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(pcm.samples().len())
                .map_err(failure)?;
            samples.extend(pcm.samples());
            clips.push(samples);
        }
        Ok(Self {
            clips: clips.try_into().map_err(|_error| failure("clip count"))?,
        })
    }
}

pub(super) struct Playback<'samples> {
    clips: [Clip<'samples>; 4],
    cues: [u64; 4],
    device: NativeAudio,
    mixer: Mixer<'samples>,
    scratch: [[i16; 2]; CHUNK],
}

impl<'samples> Playback<'samples> {
    fn fill(&mut self) -> Result<(), RenderError> {
        for _ in 0..MAX_CHUNKS {
            if self.mixer.active() == 0
                || self.device.queued_bytes()
                    > QUEUE_FRAMES.saturating_sub(256).saturating_mul(4)
            {
                break;
            }
            self.mixer.render(&mut self.scratch);
            self.device
                .queue_samples(self.scratch.as_flattened())
                .map_err(failure)?;
        }
        Ok(())
    }

    pub(super) fn open(
        samples: &'samples Samples,
    ) -> Result<Self, RenderError> {
        let device = NativeAudio::open(PlaybackSpec {
            buffer_frames: NonZeroU16::MIN.saturating_add(255),
            channels: NonZeroU8::MIN.saturating_add(1),
            max_queued_frames: NonZeroU32::MIN.saturating_add(QUEUE_FRAMES - 1),
            sample_rate: RATE,
        })
        .map_err(failure)?;
        Ok(Self {
            clips: samples.clips()?,
            cues: [0; 4],
            device,
            mixer: Mixer::new(RATE, NonZeroU16::MIN.saturating_add(15))
                .map_err(failure)?,
            scratch: [[0; 2]; CHUNK],
        })
    }

    pub(super) fn resume(&self) {
        self.device.resume();
    }

    pub(super) fn smoke(&mut self) -> Result<(), RenderError> {
        let mut simulation = smoke_entry()?;
        self.update(simulation.state())?;
        if self.device.queued_bytes() != 0 {
            return Err(failure("entry incorrectly triggered attack audio"));
        }
        let _first = simulation
            .advance(TICK_INTERVAL.saturating_mul(4))
            .map_err(failure)?;
        self.update(simulation.state())?;
        if self.device.queued_bytes() != 0 {
            return Err(failure("attack audio started before cue"));
        }
        let _cue = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_submission(2)?;
        let _second = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        self.device.clear();
        self.update(simulation.state())?;
        self.verify_submission(4)?;
        for _ in 0..60u8 {
            self.device.clear();
            let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
            self.update(simulation.state())?;
        }
        if self.cues != [1; 4]
            || self.mixer.active() != 0
            || self.device.queued_bytes() != 0
        {
            return Err(failure("held attack repeated or audio did not drain"));
        }
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        cabinet: &Cabinet,
    ) -> Result<(), RenderError> {
        let current = cabinet.stations().map(|station| station.attack_cues);
        for (((previous, count), clip), gain) in
            self.cues.iter_mut().zip(current).zip(self.clips).zip(GAINS)
        {
            let new_cues = count.saturating_sub(*previous);
            *previous = count;
            // At most eight fixed ticks can execute in one host update.
            if new_cues > 8 {
                return Err(failure("attack cue count exceeded tick budget"));
            }
            for _ in 0..new_cues {
                play(&mut self.mixer, clip, gain)?;
            }
        }
        self.fill()
    }

    fn verify_submission(&self, active: usize) -> Result<(), RenderError> {
        if self.mixer.active() != active
            || self.device.queued_bytes() != QUEUE_FRAMES.saturating_mul(4)
            || self.scratch.iter().all(|frame| *frame == [0, 0])
        {
            return Err(failure("unexpected native attack audio submission"));
        }
        Ok(())
    }
}

fn play<'samples>(
    mixer: &mut Mixer<'samples>,
    clip: Clip<'samples>,
    gain: StereoGain,
) -> Result<(), RenderError> {
    match mixer.play(clip, gain) {
        Ok(_) | Err(AudioError::VoicesFull) => Ok(()),
        Err(error) => Err(failure(error)),
    }
}

fn smoke_entry() -> Result<LiveSimulation<Cabinet>, RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::new(false)?;
    let entries = KEYBOARD.into_iter().map(|[_, start, ..]| start);
    for key in entries {
        for down in [true, false] {
            controls.route(
                InputEvent::Key {
                    code: key_code(key),
                    down,
                },
                Duration::ZERO,
                &mut simulation,
            )?;
        }
    }
    let _entry = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    for [_, _, primary, ..] in KEYBOARD {
        controls.route(
            InputEvent::Key {
                code: key_code(primary),
                down: true,
            },
            TICK_INTERVAL,
            &mut simulation,
        )?;
    }
    Ok(simulation)
}

#[cfg(test)]
mod tests {
    use std::array;

    use super::*;

    #[test]
    fn isolated_attack_clips_use_observed_stereo_ratios_and_rate()
    -> Result<(), RenderError> {
        let samples = Samples {
            clips: array::from_fn(|_| vec![i16::MAX; 2]),
        };
        let expected = [[5451, 2443], [7383, 5182], [3430, 4887], [2027, 4523]];
        for ((clip, gain), frame) in
            samples.clips()?.into_iter().zip(GAINS).zip(expected)
        {
            if clip.rate().get() != 10_027 {
                return Err(failure("attack clip rate did not match pitch"));
            }
            let mut mixer =
                Mixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
            play(&mut mixer, clip, gain)?;
            let mut output = [[0; 2]];
            mixer.render(&mut output);
            if output != [frame] {
                return Err(failure("attack clip stereo gain mismatch"));
            }
        }
        Ok(())
    }
}
