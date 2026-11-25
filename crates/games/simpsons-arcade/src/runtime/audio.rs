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

use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64};
use std::path::Path;
use std::time::Duration;

use games::audio::{
    AudioError, Channels, Clip, MasterVolume, Mixer, StereoGain,
};
use games::input::{InputEvent, LogicalAction};
use games::replay::LiveSimulation;
use games_assets::ManifestLimits;
use games_assets::pcm::PcmSamples;
use games_platform::ir::audio::{NativeAudio, PlaybackSpec};
use games_platform::ir::frame::RenderError;
use games_platform::ir::process::write_status;
use games_platform::ir::storage::bundle::MediaOverlay;

use super::launch::AudioSelection;
use super::{Controls, failure, live, open_selected_bundle, size, smoke_press};
use crate::{Action, Cabinet, Control, Phase, Player, TICK_INTERVAL};

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
        selection: &AudioSelection,
    ) -> Result<Self, RenderError> {
        let bundle =
            open_selected_bundle(parent, &selection.base, ManifestLimits {
                manifest_bytes: size(65_536)?,
                payload_bytes: NonZeroU64::MIN.saturating_add(264_323),
                records: size(161)?,
                total_bytes: NonZeroU64::MIN.saturating_add(3_798_887),
            })?;
        let replacement = selection
            .replacement
            .as_ref()
            .map(|hash| {
                open_selected_bundle(parent, hash, replacement_limits()?)
            })
            .transpose()?;
        let allowed = CLIPS.map(|(key, _frames)| key);
        let overlay = replacement
            .as_ref()
            .map(|changes| MediaOverlay::new(&bundle, changes, &allowed))
            .transpose()
            .map_err(failure)?;
        let mut clips = Vec::with_capacity(CLIPS.len());
        for (key, frames) in CLIPS {
            let (descriptor, payload) = overlay.as_ref().map_or_else(
                || (bundle.manifest().record(key), bundle.read_payload(key)),
                |changes| (changes.record(key), changes.read_payload(key)),
            );
            let record =
                descriptor.ok_or_else(|| failure("missing attack PCM"))?;
            let bytes = payload.map_err(failure)?;
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
    live_output: bool,
    mixer: Mixer<'samples>,
    running: bool,
    scratch: [[i16; 2]; CHUNK],
    volume: MasterVolume,
}

impl<'samples> Playback<'samples> {
    pub(super) const fn enable_output(&mut self) {
        self.live_output = true;
    }

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
        name: Option<&str>,
    ) -> Result<Self, RenderError> {
        let spec = PlaybackSpec {
            buffer_frames: NonZeroU16::MIN.saturating_add(255),
            channels: NonZeroU8::MIN.saturating_add(1),
            max_queued_frames: NonZeroU32::MIN.saturating_add(QUEUE_FRAMES - 1),
            sample_rate: RATE,
        };
        let device = NativeAudio::open_on(spec, name).map_err(failure)?;
        let (driver, requested) = device.output();
        write_status(format_args!(
            "Audio: SDL {driver}, requested output: {}",
            requested.unwrap_or("system default").escape_debug(),
        ))
        .map_err(failure)?;
        Ok(Self {
            clips: samples.clips()?,
            cues: [0; 4],
            device,
            live_output: false,
            mixer: Mixer::new(RATE, NonZeroU16::MIN.saturating_add(15))
                .map_err(failure)?,
            running: false,
            scratch: [[0; 2]; CHUNK],
            volume: MasterVolume::default(),
        })
    }

    fn set_running(&mut self, running: bool) {
        if self.running != running {
            if running {
                self.device.resume();
            } else {
                self.device.pause();
            }
            self.running = running;
        }
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
        self.smoke_pause(&mut simulation)?;
        self.smoke_drain(&mut simulation)?;
        self.smoke_reset(&mut simulation)?;
        self.smoke_maneuver(false)?;
        self.smoke_maneuver(true)
    }

    fn smoke_drain(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
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

    fn smoke_maneuver(&mut self, airborne: bool) -> Result<(), RenderError> {
        let mut simulation = live(false)?;
        let mut controls = Controls::new(false)?;
        self.update(simulation.state())?;
        smoke_join(&mut simulation, &mut controls)?;
        smoke_hold(&mut controls, &mut simulation, Control::Secondary)?;
        if airborne {
            for _ in 0..15u8 {
                let _step =
                    simulation.advance(TICK_INTERVAL).map_err(failure)?;
                self.update(simulation.state())?;
            }
            self.verify_quiet()?;
        }
        smoke_hold(&mut controls, &mut simulation, Control::Primary)?;
        let _before = simulation
            .advance(TICK_INTERVAL.saturating_mul(4))
            .map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_quiet()?;
        let _cue = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_submission(4)?;
        self.smoke_drain(&mut simulation)
    }

    fn smoke_pause(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
        let mut controls = Controls::new(false)?;
        smoke_press(&mut controls, simulation, Control::Pause)?;
        self.device.clear();
        self.update(simulation.state())?;
        if self.device.queued_bytes() != 0 || self.mixer.active() != 4 {
            return Err(failure("paused audio advanced"));
        }
        smoke_press(&mut controls, simulation, Control::Pause)?;
        self.update(simulation.state())?;
        self.verify_submission(4)
    }

    fn smoke_reset(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
        let clip = *self.clips.first().ok_or(RenderError::InvalidSprite)?;
        play(&mut self.mixer, clip, StereoGain::UNITY)?;
        self.fill()?;
        self.verify_submission(1)?;
        simulation
            .enqueue(simulation.next_tick(), vec![LogicalAction {
                action: Action {
                    control: Control::Primary,
                    player: Player::Marge,
                },
                down: false,
            }])
            .map_err(|error| failure(error.reason))?;
        let mut controls = Controls::new(false)?;
        for control in [
            Control::Pause,
            Control::Down,
            Control::Down,
            Control::Primary,
        ] {
            smoke_press(&mut controls, simulation, control)?;
        }
        self.update(simulation.state())?;
        if simulation.state().phase() != Phase::MainMenu
            || self.device.queued_bytes() != 0
            || self.mixer.active() != 0
        {
            return Err(failure(format!(
                "reset retained stale audio: {:?}, {}, {}",
                simulation.state().phase(),
                self.device.queued_bytes(),
                self.mixer.active()
            )));
        }
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        cabinet: &Cabinet,
    ) -> Result<(), RenderError> {
        let volume = cabinet.session().volume();
        if self.volume != volume {
            // Pending frames were already mixed at the previous volume.
            self.device.clear();
            self.mixer.set_volume(volume);
            self.volume = volume;
        }
        let current = cabinet.stations().map(|station| station.attack_cues);
        let phase = cabinet.phase();
        let playing = phase == Phase::Playing;
        self.set_running(self.live_output && playing);
        if matches!(phase, Phase::MainMenu | Phase::Exited) {
            self.cues = current;
            if self.device.queued_bytes() != 0 {
                self.device.clear();
            }
            self.mixer.clear();
        }
        if !playing {
            return Ok(());
        }
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

    fn verify_quiet(&self) -> Result<(), RenderError> {
        if self.cues != [0; 4]
            || self.mixer.active() != 0
            || self.device.queued_bytes() != 0
        {
            return Err(failure("audio played before an admitted cue"));
        }
        Ok(())
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

pub(super) fn list_devices() -> Result<(), RenderError> {
    let devices = NativeAudio::playback_devices().map_err(failure)?;
    write_status(format_args!("SDL audio backend: {}", devices.driver))
        .map_err(failure)?;
    if let Some(names) = devices.names {
        for name in names {
            write_status(format_args!("{}", name.escape_debug()))
                .map_err(failure)?;
        }
    } else {
        write_status(format_args!("Device enumeration unavailable"))
            .map_err(failure)?;
    }
    Ok(())
}

pub(super) fn open_optional<'samples>(
    samples: Option<&'samples Samples>,
    name: Option<&str>,
) -> Result<Option<Playback<'samples>>, RenderError> {
    if let Some(available) = samples {
        return Playback::open(available, name).map(Some);
    }
    write_status(format_args!("Audio: disabled (no audio bundle)"))
        .map_err(failure)?;
    Ok(None)
}

fn replacement_limits() -> Result<ManifestLimits, RenderError> {
    let frames = CLIPS.map(|(_key, count)| count);
    let bytes = |count: u64| {
        count
            .checked_mul(2)
            .and_then(NonZeroU64::new)
            .ok_or(RenderError::TooLarge)
    };
    Ok(ManifestLimits {
        manifest_bytes: size(4096)?,
        payload_bytes: bytes(frames.into_iter().max().unwrap_or(0))?,
        records: size(CLIPS.len())?,
        total_bytes: bytes(frames.into_iter().sum())?,
    })
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
    smoke_join(&mut simulation, &mut controls)?;
    smoke_hold(&mut controls, &mut simulation, Control::Primary)?;
    Ok(simulation)
}

fn smoke_hold(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    control: Control,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    for player in Player::ALL {
        controls.route(
            InputEvent::Key {
                code: controls.key(player, control)?.code(),
                down: true,
            },
            TICK_INTERVAL.saturating_mul(tick),
            simulation,
        )?;
    }
    Ok(())
}

fn smoke_join(
    simulation: &mut LiveSimulation<Cabinet>,
    controls: &mut Controls,
) -> Result<(), RenderError> {
    for player in Player::ALL {
        let key = controls.key(player, Control::Start)?;
        for down in [true, false] {
            controls.route(
                InputEvent::Key { code: key.code(), down },
                Duration::ZERO,
                simulation,
            )?;
        }
    }
    let _entry = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::array;
    use std::ffi::OsString;

    use games_assets::archive::ArchiveBudget;
    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, ContentHash, SourceIdentity,
    };
    use games_platform::ir::storage::fixtures::TestRoot;
    use games_platform::ir::storage::staging::CanonicalStaging;

    use super::*;

    type FixtureClip<'key> = (&'key str, u64, i16);

    fn publish_fixture(
        parent: &Path,
        clips: &[FixtureClip<'_>],
    ) -> Result<OsString, RenderError> {
        let limits = replacement_limits()?;
        let maximum = NonZeroU64::MIN.saturating_add(65_535);
        let budget = ArchiveBudget::new(
            maximum,
            maximum,
            NonZeroU64::MIN.saturating_add(7),
        );
        let mut stage =
            CanonicalStaging::new(parent, budget).map_err(failure)?;
        let mut records = Vec::new();
        for &(key, frames, value) in clips {
            let bytes = value
                .to_le_bytes()
                .repeat(usize::try_from(frames).map_err(failure)?);
            let digest = ContentHash::calculate(&bytes);
            let format = AssetFormat::PcmS16Le {
                channels: NonZeroU8::MIN,
                frames: NonZeroU64::new(frames).ok_or(RenderError::TooLarge)?,
                sample_rate: SOURCE_RATE,
            };
            stage.write_payload(digest, &bytes).map_err(failure)?;
            records.push(AssetRecord {
                digest,
                format,
                key: key.to_owned(),
                size: u64::try_from(bytes.len()).map_err(failure)?,
            });
        }
        let manifest = AssetManifest::new(
            SourceIdentity {
                title: String::from("simpsons-arcade"),
                revision: String::from("world-4p-set-1"),
            },
            records,
            limits,
        )
        .map_err(failure)?;
        let staged = stage.finish(&manifest, limits).map_err(failure)?;
        let digest = staged.digest().to_string();
        staged
            .publish()
            .map_err(failure)?
            .durability
            .map_err(failure)?;
        Ok(digest.into())
    }

    #[test]
    fn opted_in_audio_replacement_is_selective_and_fail_closed()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.path();
        let base = CLIPS.map(|(key, frames)| (key, frames, 12));
        let mut selection = AudioSelection {
            base: publish_fixture(parent, &base)?,
            replacement: None,
        };
        let original = Samples::load(parent, &selection)?;
        if original.clips.iter().flatten().any(|sample| *sample != 12) {
            return Err(failure("incorrect original clip samples"));
        }
        let (key, frames) = CLIPS
            .first()
            .copied()
            .ok_or_else(|| failure("missing fixture clip"))?;
        selection.replacement =
            Some(publish_fixture(parent, &[(key, frames, -7)])?);
        let modified = Samples::load(parent, &selection)?;
        let [first, rest @ ..] = &modified.clips;
        if first.iter().any(|sample| *sample != -7)
            || rest.iter().flatten().any(|sample| *sample != 12)
        {
            return Err(failure("replacement did not remain selective"));
        }
        for (invalid_key, invalid_frames) in
            [("audio/unknown", frames), (key, frames.saturating_sub(1))]
        {
            selection.replacement = Some(publish_fixture(parent, &[(
                invalid_key,
                invalid_frames,
                -7,
            )])?);
            if Samples::load(parent, &selection).is_ok() {
                return Err(failure("invalid replacement accepted"));
            }
        }
        Ok(())
    }

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
