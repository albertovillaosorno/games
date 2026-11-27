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
//   - Local attack and street music selection and bounded audio presentation.
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
//   - Optional four-station attack samples and street music.
// - Description:
//   - Observe each fixed-step cue once and refill a short software queue.
// - Usage:
//   - Load the verified local audio catalogue before opening the device.
// - Defaults:
//   - No device or music unless requested; no exact audio parity claim.

//! Opt-in playback of observed attack-associated PCM and street music.

use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64};
use std::rc::Rc;
use std::time::Duration;

use games::audio::{
    AudioError, AudioSource, Channels, MasterVolume, OwnedClip, OwnedMixer,
    SourceGain, SourceSettings, StereoGain,
};
use games::input::{InputEvent, LogicalAction};
use games::presentation::DisplaySettings;
use games::replay::LiveSimulation;
use games::session::LocalSession;
use games_assets::pcm::PcmSamples;
use games_assets::{ContentHash, ManifestLimits};
use games_platform::ir::audio::{NativeAudio, PlaybackControl, PlaybackSpec};
use games_platform::ir::frame::RenderError;
use games_platform::ir::process::write_status;
use games_platform::ir::storage::bundle::{BundleSource, MediaOverlay};
use games_platform::ir::storage::preferences::VolumeStore;

use super::launch::AudioSelection as LocalAudioSelection;
use super::{
    Controls, failure, live_with_session, open_selected_bundle, parse_hash,
    size, smoke_press,
};
use crate::{Action, Cabinet, Control, Phase, Player, TICK_INTERVAL};

const CHUNK: usize = 256;
// Cover two nominal presentation intervals plus a submission period.
// A shorter queue starved attack cues on the observed browser cadence.
const MAX_CHUNKS: u8 = 8;
const QUEUE_FRAMES: u32 = 2048;
const RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(47_999);
const MUSIC_KEY: &str = "audio/street-music";
const MUSIC_FRAMES: usize = 2_607_286;
const MUSIC_LOOP_START: usize = 175_680;
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

type SharedSamples = Rc<[i16]>;

/// Pinned attack samples, optional music and their source levels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSelection {
    /// Manifest providing the four observed attack clips.
    pub base: ContentHash,
    /// Independent attack attenuation before the master level.
    pub effects_volume: MasterVolume,
    /// Optional canonical opening-street music manifest.
    pub music: Option<ContentHash>,
    /// Independent music attenuation before the master level.
    pub music_volume: MasterVolume,
    /// Optional compatible replacement manifest.
    pub replacement: Option<ContentHash>,
}

impl AudioSelection {
    pub(super) fn from_local(
        selection: &LocalAudioSelection,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            base: parse_hash(&selection.base)?,
            effects_volume: selection.effects_volume,
            music: selection.music.as_deref().map(parse_hash).transpose()?,
            music_volume: selection.music_volume,
            replacement: selection
                .replacement
                .as_deref()
                .map(parse_hash)
                .transpose()?,
        })
    }
}

pub(super) struct Samples {
    clips: [Rc<[i16]>; 4],
    effects_volume: MasterVolume,
    music: Option<OwnedClip>,
    music_volume: MasterVolume,
}

impl Samples {
    fn clips(&self) -> Result<[OwnedClip; 4], RenderError> {
        let [marge, homer, bart, lisa] = &self.clips;
        let clip = |samples: &Rc<[i16]>| {
            OwnedClip::new(Rc::clone(samples), Channels::Mono, ATTACK_RATE)
                .map_err(failure)
        };
        Ok([clip(marge)?, clip(homer)?, clip(bart)?, clip(lisa)?])
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        selection: AudioSelection,
    ) -> Result<Self, RenderError> {
        let bundle =
            open_selected_bundle(store, selection.base, base_limits()?)?;
        let replacement = selection
            .replacement
            .map(|hash| {
                open_selected_bundle(store, hash, replacement_limits()?)
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
                || (bundle.manifest().record(key), bundle.view_payload(key)),
                |changes| (changes.record(key), changes.view_payload(key)),
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
            clips.push(copy_samples(pcm)?);
        }
        Ok(Self {
            clips: clips.try_into().map_err(|_error| failure("clip count"))?,
            effects_volume: selection.effects_volume,
            music: selection
                .music
                .map(|hash| load_music(store, hash))
                .transpose()?,
            music_volume: selection.music_volume,
        })
    }

    fn source_settings(&self) -> SourceSettings {
        SourceSettings {
            effects: Some(self.effects_volume),
            music: self.music.as_ref().map(|_clip| self.music_volume),
        }
    }
}

pub(super) struct Playback {
    clips: [OwnedClip; 4],
    cues: [u64; 4],
    device: NativeAudio,
    discard_queued: bool,
    live_output: bool,
    mixer: OwnedMixer,
    music: Option<OwnedClip>,
    music_started: bool,
    scratch: [[i16; 2]; CHUNK],
    sources: SourceSettings,
    volume: MasterVolume,
}

impl Playback {
    pub(super) const fn enable_output(&mut self) {
        self.live_output = true;
    }

    fn fill(&mut self) -> Result<(), RenderError> {
        for _ in 0..MAX_CHUNKS {
            if self.mixer.active() == 0
                || self.queued_frames()? > QUEUE_FRAMES.saturating_sub(256)
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

    fn music_voices(&self) -> usize {
        usize::from(self.music_started)
    }

    pub(super) fn open(
        samples: &Samples,
        name: Option<&str>,
        sources: SourceSettings,
        volume: MasterVolume,
    ) -> Result<Self, RenderError> {
        if sources.effects.is_none()
            || sources.music.is_some() != samples.music.is_some()
        {
            return Err(failure("audio source capability mismatch"));
        }
        let mut mixer = OwnedMixer::new(
            RATE,
            NonZeroU16::MIN.saturating_add(
                15u16.saturating_add(u16::from(samples.music.is_some())),
            ),
        )
        .map_err(failure)?;
        mixer.set_source_volumes(sources.levels());
        mixer.set_volume(volume);
        let spec = PlaybackSpec {
            buffer_frames: NonZeroU16::MIN.saturating_add(255),
            channels: NonZeroU8::MIN.saturating_add(1),
            max_queued_frames: NonZeroU32::MIN.saturating_add(QUEUE_FRAMES - 1),
            sample_rate: RATE,
        };
        let device = NativeAudio::open_on(spec, name).map_err(failure)?;
        let (driver, requested) = device.output();
        write_status(format_args!(
            "Audio: {driver}, requested output: {}",
            requested.unwrap_or("system default").escape_debug(),
        ))
        .map_err(failure)?;
        write_status(format_args!(
            "Audio levels (percent): master={}, effects={}, music={}",
            volume.percent(),
            sources.effects.map_or("unavailable", MasterVolume::percent),
            sources.music.map_or("unavailable", MasterVolume::percent),
        ))
        .map_err(failure)?;
        Ok(Self {
            clips: samples.clips()?,
            cues: [0; 4],
            device,
            discard_queued: false,
            live_output: false,
            mixer,
            music: samples.music.clone(),
            music_started: false,
            scratch: [[0; 2]; CHUNK],
            sources,
            volume,
        })
    }

    fn queued_frames(&self) -> Result<u32, RenderError> {
        self.device.queued_frames().map_err(failure)
    }

    fn ready(&mut self, running: bool) -> Result<bool, RenderError> {
        if self.discard_queued {
            // A host must settle its earlier control before any queue
            // operation. Never enqueue stale-volume or reset-session PCM.
            if self.device.poll_control().map_err(failure)?
                == PlaybackControl::Pending
            {
                return Ok(false);
            }
            if self.queued_frames()? != 0 {
                self.device.clear().map_err(failure)?;
            }
            self.discard_queued = false;
        }
        Ok(self.device.reconcile_running(running).map_err(failure)?
            == PlaybackControl::Complete)
    }

    pub(super) fn smoke(&mut self) -> Result<(), RenderError> {
        let mut simulation = smoke_entry(self.smoke_session())?;
        self.update(simulation.state())?;
        self.verify_quiet()?;
        let _first = simulation
            .advance(TICK_INTERVAL.saturating_mul(4))
            .map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_quiet()?;
        let _cue = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        // The hidden smoke has no device consumer. Refill after admission.
        self.device.clear().map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_submission(2)?;
        let _second = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        self.device.clear().map_err(failure)?;
        self.update(simulation.state())?;
        self.verify_submission(4)?;
        self.smoke_pause(&mut simulation)?;
        self.smoke_drain(&mut simulation)?;
        self.smoke_reset(&mut simulation)?;
        self.smoke_maneuver(false)?;
        self.smoke_maneuver(true)?;
        self.smoke_source_changes()
    }

    fn smoke_drain(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
        for _ in 0..60u8 {
            self.device.clear().map_err(failure)?;
            let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
            self.update(simulation.state())?;
        }
        if self.cues != [1; 4] {
            return Err(failure("held attack repeated"));
        }
        self.verify_idle_effects()
    }

    fn smoke_maneuver(&mut self, airborne: bool) -> Result<(), RenderError> {
        let mut simulation = live_with_session(false, self.smoke_session())?;
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
        // The hidden smoke has no device consumer. Refill after admission.
        self.device.clear().map_err(failure)?;
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
        self.device.clear().map_err(failure)?;
        self.update(simulation.state())?;
        if self.queued_frames()? != 0
            || self.mixer.active() != 4usize.saturating_add(self.music_voices())
        {
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
        let clip = self
            .clips
            .first()
            .ok_or(RenderError::InvalidSprite)?
            .clone();
        play(&mut self.mixer, clip, StereoGain::UNITY)?;
        self.device.clear().map_err(failure)?;
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
            || self.queued_frames()? != 0
            || self.mixer.active() != 0
            || self.music_started
        {
            return Err(failure(format!(
                "reset retained stale audio: {:?}, {}, {}",
                simulation.state().phase(),
                self.queued_frames()?,
                self.mixer.active()
            )));
        }
        Ok(())
    }

    fn smoke_session(&self) -> LocalSession {
        LocalSession::with_preferences(
            DisplaySettings::default(),
            self.volume,
            false,
        )
        .with_source_settings(self.sources)
    }

    fn smoke_settings(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
        controls: &mut Controls,
    ) -> Result<(), RenderError> {
        for control in [
            Control::Down,
            Control::Primary,
            Control::Down,
            Control::Down,
            Control::Primary,
        ] {
            smoke_press(controls, simulation, control)?;
        }
        if simulation.state().phase() != Phase::Audio {
            return Err(failure("paused audio settings did not open"));
        }
        for source in [AudioSource::Effects, AudioSource::Music] {
            if self.sources.get(source).is_some() {
                self.smoke_source_change(simulation, controls, source)?;
            }
        }
        for _ in 0u8..2 {
            smoke_press(controls, simulation, Control::Pause)?;
        }
        Ok(())
    }

    fn smoke_source_change(
        &mut self,
        simulation: &mut LiveSimulation<Cabinet>,
        controls: &mut Controls,
        source: AudioSource,
    ) -> Result<(), RenderError> {
        let original = self.sources;
        let active = self.mixer.active();
        smoke_press(controls, simulation, Control::Down)?;
        smoke_press(controls, simulation, Control::Primary)?;
        self.update(simulation.state())?;
        if self.sources.get(source)
            != original.get(source).map(MasterVolume::next)
            || self.queued_frames()? != 0
            || self.mixer.active() != active
        {
            return Err(failure(
                "source setting retained stale queued audio or reset voices",
            ));
        }
        for _ in 0u8..4 {
            smoke_press(controls, simulation, Control::Primary)?;
        }
        self.update(simulation.state())?;
        if self.sources != original || self.mixer.active() != active {
            return Err(failure("source setting restoration reset audio"));
        }
        Ok(())
    }

    fn smoke_source_changes(&mut self) -> Result<(), RenderError> {
        let mut simulation = live_with_session(false, self.smoke_session())?;
        let mut controls = Controls::new(false)?;
        self.update(simulation.state())?;
        smoke_join(&mut simulation, &mut controls)?;
        self.update(simulation.state())?;
        let clip = self
            .clips
            .first()
            .ok_or(RenderError::InvalidSprite)?
            .clone();
        play(&mut self.mixer, clip, StereoGain::UNITY)?;
        self.device.clear().map_err(failure)?;
        self.fill()?;
        self.verify_submission(1)?;
        smoke_press(&mut controls, &mut simulation, Control::Pause)?;
        self.update(simulation.state())?;
        self.smoke_settings(&mut simulation, &mut controls)?;
        if simulation.state().phase() != Phase::Paused
            || self.queued_frames()? != 0
        {
            return Err(failure(
                "source settings did not return to the paused session",
            ));
        }
        smoke_press(&mut controls, &mut simulation, Control::Pause)?;
        self.update(simulation.state())?;
        self.verify_submission(1)
    }

    fn sync_levels(&mut self, session: LocalSession) {
        let volume = session.volume();
        let sources = session.source_settings();
        if self.volume != volume || self.sources != sources {
            // Pending frames were already mixed at the previous levels.
            self.discard_queued = true;
            self.mixer.set_volume(volume);
            self.mixer.set_source_volumes(sources.levels());
            self.volume = volume;
            self.sources = sources;
        }
    }

    pub(super) fn update(
        &mut self,
        cabinet: &Cabinet,
    ) -> Result<(), RenderError> {
        self.sync_levels(cabinet.session());
        let current = cabinet.stations().map(|station| station.attack_cues);
        let phase = cabinet.phase();
        let playing = phase == Phase::Playing;
        if matches!(phase, Phase::MainMenu | Phase::Exited) {
            self.cues = current;
            self.discard_queued = true;
            self.mixer.clear();
            self.music_started = false;
        }
        let ready = self.ready(self.live_output && playing)?;
        if !playing {
            return Ok(());
        }
        if !self.music_started
            && let Some(clip) = &self.music
        {
            let _voice = self
                .mixer
                .play_loop_on(
                    clip.clone(),
                    SourceGain {
                        source: AudioSource::Music,
                        stereo: StereoGain::UNITY,
                    },
                    MUSIC_LOOP_START..MUSIC_FRAMES,
                )
                .map_err(failure)?;
            self.music_started = true;
        }
        for (((previous, count), clip), gain) in self
            .cues
            .iter_mut()
            .zip(current)
            .zip(&self.clips)
            .zip(GAINS)
        {
            let new_cues = count.saturating_sub(*previous);
            *previous = count;
            // At most eight fixed ticks can execute in one host update.
            if new_cues > 8 {
                return Err(failure("attack cue count exceeded tick budget"));
            }
            for _ in 0..new_cues {
                play(&mut self.mixer, clip.clone(), gain)?;
            }
        }
        if ready {
            self.fill()
        } else {
            Ok(())
        }
    }

    fn verify_idle_effects(&self) -> Result<(), RenderError> {
        if self.music.is_some() {
            if !self.music_started {
                return Err(failure("street music did not start"));
            }
            return self.verify_submission(0);
        }
        if self.mixer.active() != 0 || self.queued_frames()? != 0 {
            return Err(failure("attack audio did not drain"));
        }
        Ok(())
    }

    fn verify_quiet(&self) -> Result<(), RenderError> {
        if self.cues != [0; 4] {
            return Err(failure("audio played before an admitted cue"));
        }
        self.verify_idle_effects()
    }

    fn verify_submission(&self, active: usize) -> Result<(), RenderError> {
        let levels = self.sources.levels();
        let audible = self.volume != MasterVolume::Muted
            && ((active != 0 && levels.effects != MasterVolume::Muted)
                || (self.music_started && levels.music != MasterVolume::Muted));
        if self.mixer.active() != active.saturating_add(self.music_voices())
            || self.queued_frames()? != QUEUE_FRAMES
            || self.scratch.iter().all(|frame| *frame == [0, 0]) == audible
        {
            return Err(failure("unexpected native audio submission"));
        }
        Ok(())
    }
}

pub(super) fn list_devices() -> Result<(), RenderError> {
    let devices = NativeAudio::playback_devices().map_err(failure)?;
    write_status(format_args!("Audio backend: {}", devices.driver))
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

pub(super) fn source_settings(
    samples: Option<&Samples>,
    store: Option<&VolumeStore>,
) -> Result<SourceSettings, RenderError> {
    let defaults =
        samples.map_or_else(SourceSettings::default, Samples::source_settings);
    store.map_or(Ok(defaults), |preferences| {
        preferences.resolve_sources(defaults).map_err(failure)
    })
}

pub(super) fn open_optional(
    samples: Option<&Samples>,
    name: Option<&str>,
    sources: SourceSettings,
    volume: MasterVolume,
) -> Result<Option<Playback>, RenderError> {
    if let Some(available) = samples {
        return Playback::open(available, name, sources, volume).map(Some);
    }
    write_status(format_args!("Audio: disabled (no audio bundle)"))
        .map_err(failure)?;
    Ok(None)
}

pub(super) fn base_limits() -> Result<ManifestLimits, RenderError> {
    Ok(ManifestLimits {
        manifest_bytes: size(65_536)?,
        payload_bytes: NonZeroU64::MIN.saturating_add(264_323),
        records: size(161)?,
        total_bytes: NonZeroU64::MIN.saturating_add(3_798_887),
    })
}

pub(super) fn music_limits() -> Result<ManifestLimits, RenderError> {
    let bytes = u64::try_from(MUSIC_FRAMES)
        .ok()
        .and_then(|frames| frames.checked_mul(4))
        .and_then(NonZeroU64::new)
        .ok_or(RenderError::TooLarge)?;
    Ok(ManifestLimits {
        manifest_bytes: size(4096)?,
        payload_bytes: bytes,
        records: size(1)?,
        total_bytes: bytes,
    })
}

fn copy_samples(pcm: PcmSamples<'_>) -> Result<SharedSamples, RenderError> {
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(pcm.samples().len())
        .map_err(failure)?;
    samples.extend(pcm.samples());
    Ok(Rc::from(samples))
}

fn load_music(
    store: BundleSource<'_>,
    hash: ContentHash,
) -> Result<OwnedClip, RenderError> {
    let bundle = open_selected_bundle(store, hash, music_limits()?)?;
    let record = bundle
        .manifest()
        .record(MUSIC_KEY)
        .ok_or_else(|| failure("missing street music PCM"))?;
    let bytes = bundle.view_payload(MUSIC_KEY).map_err(failure)?;
    let pcm = PcmSamples::new(record, &bytes).map_err(failure)?;
    if pcm.channels() != NonZeroU8::MIN.saturating_add(1)
        || pcm.sample_rate() != RATE
        || usize::try_from(pcm.frames().get()).ok() != Some(MUSIC_FRAMES)
    {
        return Err(failure("unexpected street music PCM format"));
    }
    OwnedClip::new(copy_samples(pcm)?, Channels::Stereo, RATE).map_err(failure)
}

pub(super) fn replacement_limits() -> Result<ManifestLimits, RenderError> {
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

fn play(
    mixer: &mut OwnedMixer,
    clip: OwnedClip,
    gain: StereoGain,
) -> Result<(), RenderError> {
    match mixer.play(clip, gain) {
        Ok(_) | Err(AudioError::VoicesFull) => Ok(()),
        Err(error) => Err(failure(error)),
    }
}

fn smoke_entry(
    session: LocalSession,
) -> Result<LiveSimulation<Cabinet>, RenderError> {
    let mut simulation = live_with_session(false, session)?;
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
    use std::ffi::OsStr;

    use games_assets::archive::ArchiveBudget;
    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, SourceIdentity,
    };
    use games_platform::ir::storage::fixtures::TestRoot;
    use games_platform::ir::storage::staging::CanonicalStaging;

    use super::*;

    type FixtureClip<'key> = (&'key str, u64, i16);

    #[test]
    fn attack_queue_covers_two_ticks_and_a_submission()
    -> Result<(), RenderError> {
        let frames = u32::try_from(CHUNK).map_err(failure)?;
        let rate = f64::from(RATE.get());
        let buffered = Duration::from_secs_f64(f64::from(QUEUE_FRAMES) / rate);
        let period = Duration::from_secs_f64(f64::from(frames) / rate);
        if buffered < TICK_INTERVAL.saturating_mul(2).saturating_add(period)
            || QUEUE_FRAMES != frames.saturating_mul(u32::from(MAX_CHUNKS))
        {
            return Err(failure("attack audio refill budget is insufficient"));
        }
        Ok(())
    }

    fn publish_fixture(
        parent: &OsStr,
        clips: &[FixtureClip<'_>],
    ) -> Result<ContentHash, RenderError> {
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
        let digest = staged.digest();
        staged
            .publish()
            .map_err(failure)?
            .durability
            .map_err(failure)?;
        Ok(digest)
    }

    #[test]
    fn opted_in_audio_replacement_is_selective_and_fail_closed()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.location();
        let base = CLIPS.map(|(key, frames)| (key, frames, 12));
        let mut selection = AudioSelection {
            base: publish_fixture(parent, &base)?,
            effects_volume: MasterVolume::Full,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
        };
        let original = Samples::load(BundleSource::Local(parent), selection)?;
        if original
            .clips
            .iter()
            .flat_map(|clip| clip.iter())
            .any(|sample| *sample != 12)
        {
            return Err(failure("incorrect original clip samples"));
        }
        let (key, frames) = CLIPS
            .first()
            .copied()
            .ok_or_else(|| failure("missing fixture clip"))?;
        selection.replacement =
            Some(publish_fixture(parent, &[(key, frames, -7)])?);
        let modified = Samples::load(BundleSource::Local(parent), selection)?;
        let [first, rest @ ..] = &modified.clips;
        if first.iter().any(|sample| *sample != -7)
            || rest
                .iter()
                .flat_map(|clip| clip.iter())
                .any(|sample| *sample != 12)
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
            if Samples::load(BundleSource::Local(parent), selection).is_ok() {
                return Err(failure("invalid replacement accepted"));
            }
        }
        Ok(())
    }

    #[test]
    fn isolated_attack_clips_use_observed_stereo_ratios_and_rate()
    -> Result<(), RenderError> {
        let samples = Samples {
            clips: array::from_fn(|_| Rc::from([i16::MAX; 2])),
            effects_volume: MasterVolume::Full,
            music: None,
            music_volume: MasterVolume::Full,
        };
        let expected = [[5451, 2443], [7383, 5182], [3430, 4887], [2027, 4523]];
        let clips = samples.clips()?;
        drop(samples);
        for ((clip, gain), frame) in clips.into_iter().zip(GAINS).zip(expected)
        {
            if clip.rate().get() != 10_027 {
                return Err(failure("attack clip rate did not match pitch"));
            }
            let mut mixer =
                OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
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
