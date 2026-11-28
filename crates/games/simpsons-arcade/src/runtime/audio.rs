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
//   - Local effects and soundtrack selection and bounded audio presentation.
// - Must-Not:
//   - Interpret ROM bytes, duplicate mixing or influence simulation time.
// - Allows:
//   - Inputs: Canonical PCM and cumulative fixed-step combat cues.
//   - Outputs: Bounded mixed stereo submissions to the shared audio host.
//   - Side effects: Local device playback and explicit voice admission.
// - Split-When:
//   - Additional independently scheduled music or audio buses are required.
// - Merge-When:
//   - Another title-local module duplicates native cue delivery.
// - Summary:
//   - Optional combat effects and scene music samples.
// - Description:
//   - Observe each fixed-step cue once and refill a short software queue.
// - Usage:
//   - Load the verified local audio catalogue before opening the device.
// - Defaults:
//   - No device or music unless requested; no exact audio parity claim.

//! Opt-in playback of observed combat effects and scene music.

use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64};
use std::rc::Rc;
use std::time::Duration;

use games::audio::{
    AudioError, AudioSource, Channels, MasterVolume, OwnedClip, OwnedMixer,
    SourceGain, SourceSettings, StereoGain,
};
use games::input::{InputEvent, LogicalAction};
use games::presentation::DisplaySettings;
use games::replay::{InputStep as _, LiveSimulation};
use games::session::LocalSession;
use games::timing::{FixedTick, StepClock};
use games_assets::pcm::PcmSamples;
use games_assets::{AssetRecord, ContentHash, ManifestLimits};
use games_platform::ir::audio::{NativeAudio, PlaybackControl, PlaybackSpec};
use games_platform::ir::frame::RenderError;
use games_platform::ir::process::write_status;
use games_platform::ir::storage::bundle::{
    BundleSource, CanonicalBundle, MediaOverlay,
};
use games_platform::ir::storage::preferences::VolumeStore;

use super::launch::AudioSelection as LocalAudioSelection;
use super::{
    Controls, dimension, failure, fixed, live_with_session,
    open_selected_bundle, parse_hash, parse_hashes, size, smoke_press,
};
use crate::{
    Action, BonusPhase, Cabinet, CemeteryEntrance, CemeteryPhase, Control,
    ENEMY_CAPACITY, Phase, Player, Point, StreetLayout, TICK_INTERVAL,
    Wrestler, WrestlerAction,
};

const CHUNK: usize = 256;
// Cover two nominal presentation intervals plus a submission period.
// A shorter queue starved attack cues on the observed browser cadence.
const MAX_CHUNKS: u8 = 8;
const QUEUE_FRAMES: u32 = 2048;
const RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(47_999);
const MUSIC_KEY: &str = "audio/street-music";
const MUSIC_FRAMES: usize = 2_607_286;
const MUSIC_LOOP_START: usize = 175_680;
// Finite original service excerpt; no verified recurring seam yet.
const STAGE_TWO_TRACK: (&str, usize) = ("audio/stage-two-music", 2_880_000);
// Finite cemetery service excerpt; no verified recurring seam yet.
const CEMETERY_TRACK: (&str, usize) = ("audio/cemetery-music", 2_880_000);
// Bounded service captures: ten seconds of pumping music and the
// five-second results cue including its verified silent tail.
const BONUS_TRACKS: [(&str, usize); 2] = [
    ("audio/bonus-pumping", 480_000),
    ("audio/bonus-results", 240_000),
];
// The observed first three handoff commands have finite decay tails.
const HANDOFF_TRACKS: [(&str, usize); 4] = [
    ("audio/boss-defeat", 384_000),
    ("audio/victory", 288_000),
    ("audio/bonus-transition", 480_000),
    ("audio/bonus-instructions", 240_000),
];
// Baseline command offsets from the terminal response are 275 and 499
// reference frames. Native fixed ticks use the reference screen period;
// the bonus clock begins after the 128-tick wrestler defeat phase.
const VICTORY_CUE_TICK: u16 = 147;
const TRANSITION_CUE_TICK: u16 = 371;
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
const HURT_RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(11_185);
// Character-specific short ground-hit samples, pitch 0xec0.
const HURT_CLIPS: [(&str, u64); 4] = [
    ("audio/sample-0008", 7288),
    ("audio/sample-0009", 6644),
    ("audio/sample-0010", 4294),
    ("audio/sample-0011", 4770),
];
const HURT_GAINS: [StereoGain; 4] = [
    StereoGain {
        left: 29_464,
        right: 13_208,
    },
    StereoGain {
        left: 16_432,
        right: 11_534,
    },
    StereoGain {
        left: 16_206,
        right: 23_088,
    },
    StereoGain {
        left: 11_544,
        right: 25_752,
    },
];
// Original centered volume 63; one differential PCM byte is omitted.
const CEMETERY_AMBIENT_KEY: &str = "audio/sample-0054";
const CEMETERY_AMBIENT_FRAMES: u64 = 1574;
const CEMETERY_AMBIENT_TAIL: usize = 2;
const CEMETERY_AMBIENT_GAIN: StereoGain = StereoGain {
    left: 11_340,
    right: 11_340,
};
// Original centered volume 111; the programmed voice stops one byte
// before the end of the canonical differential PCM catalogue entry.
const CEMETERY_EFFECT_KEY: &str = "audio/sample-0070";
const CEMETERY_EFFECT_FRAMES: u64 = 5818;
const CEMETERY_EFFECT_TAIL: usize = 2;
const CEMETERY_EFFECT_GAIN: StereoGain = StereoGain {
    left: 19_980,
    right: 19_980,
};
const IMPACT_KEY: &str = "audio/sample-0056";
const IMPACT_FRAMES: u64 = 4258;
// Original centered volume 79, pitch 0xe40, differential PCM.
// Rounded playback rate for both effects using the original pitch 0xe40.
const PITCH_E40_RATE: NonZeroU32 = NonZeroU32::MIN.saturating_add(7989);
const IMPACT_GAIN: StereoGain = StereoGain {
    left: 14_220,
    right: 14_220,
};
const CLIPS: [(&str, u64); 4] = [
    ("audio/sample-0004", 8335),
    ("audio/sample-0005", 3916),
    ("audio/sample-0006", 2944),
    ("audio/sample-0007", 3828),
];

type SharedSamples = Rc<[i16]>;

/// Pinned effects samples, optional music and their source levels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSelection {
    /// Manifest providing attack clips and optional combat effects.
    pub base: ContentHash,
    /// Optional pumping and results music manifests, in that order.
    pub bonus_music: Option<[ContentHash; 2]>,
    /// Optional finite cemetery music manifest.
    pub cemetery_music: Option<ContentHash>,
    /// Independent effects attenuation before the master level.
    pub effects_volume: MasterVolume,
    /// Optional defeat, victory, transition and instructions manifests.
    pub handoff_music: Option<[ContentHash; 4]>,
    /// Optional canonical opening-street music manifest.
    pub music: Option<ContentHash>,
    /// Independent music attenuation before the master level.
    pub music_volume: MasterVolume,
    /// Optional compatible replacement manifest.
    pub replacement: Option<ContentHash>,
    /// Optional finite second-stage music manifest.
    pub stage_two_music: Option<ContentHash>,
}

impl AudioSelection {
    pub(super) fn from_local(
        selection: &LocalAudioSelection,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            base: parse_hash(&selection.base)?,
            bonus_music: selection
                .bonus_music
                .as_ref()
                .map(parse_hashes)
                .transpose()?,
            cemetery_music: selection
                .cemetery_music
                .as_deref()
                .map(parse_hash)
                .transpose()?,
            effects_volume: selection.effects_volume,
            handoff_music: selection
                .handoff_music
                .as_ref()
                .map(parse_hashes)
                .transpose()?,
            music: selection.music.as_deref().map(parse_hash).transpose()?,
            music_volume: selection.music_volume,
            replacement: selection
                .replacement
                .as_deref()
                .map(parse_hash)
                .transpose()?,
            stage_two_music: selection
                .stage_two_music
                .as_deref()
                .map(parse_hash)
                .transpose()?,
        })
    }

    fn validate_music(&self) -> Result<(), RenderError> {
        if self.cemetery_music.is_some() && self.stage_two_music.is_none() {
            return Err(failure("cemetery music requires second-stage music"));
        }
        if self.stage_two_music.is_some() && self.bonus_music.is_none() {
            return Err(failure("stage-two music requires bonus music"));
        }
        if self.handoff_music.is_some() && self.bonus_music.is_none() {
            return Err(failure("handoff music requires bonus music"));
        }
        if self.bonus_music.is_some() && self.music.is_none() {
            return Err(failure("bonus music requires street music"));
        }
        Ok(())
    }
}

pub(super) struct Samples {
    ambient_effect: Option<OwnedClip>,
    bonus_music: Option<[OwnedClip; 2]>,
    cemetery_music: Option<OwnedClip>,
    clips: [Rc<[i16]>; 4],
    effects_volume: MasterVolume,
    entrance_effect: Option<OwnedClip>,
    handoff_music: Option<[OwnedClip; 4]>,
    hurt: [Option<OwnedClip>; 4],
    impact: Option<OwnedClip>,
    music: Option<OwnedClip>,
    music_volume: MasterVolume,
    stage_two_music: Option<OwnedClip>,
}

impl Samples {
    fn cemetery_effects(&self) -> [CountedEffect; 2] {
        [
            CountedEffect::new(
                self.ambient_effect.clone(),
                CEMETERY_AMBIENT_GAIN,
            ),
            CountedEffect::new(
                self.entrance_effect.clone(),
                CEMETERY_EFFECT_GAIN,
            ),
        ]
    }

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
        selection: &AudioSelection,
    ) -> Result<Self, RenderError> {
        selection.validate_music()?;
        let bundle =
            open_selected_bundle(store, selection.base, base_limits()?)?;
        let replacement = selection
            .replacement
            .map(|hash| {
                open_selected_bundle(store, hash, replacement_limits()?)
            })
            .transpose()?;
        let clips = load_attack_clips(&bundle, replacement.as_ref())?;
        let [marge, homer, bart, lisa] = HURT_CLIPS.map(|(key, frames)| {
            load_optional_effect(&bundle, key, HURT_RATE, (frames, 0))
        });
        Ok(Self {
            ambient_effect: load_optional_effect(
                &bundle,
                CEMETERY_AMBIENT_KEY,
                PITCH_E40_RATE,
                (CEMETERY_AMBIENT_FRAMES, CEMETERY_AMBIENT_TAIL),
            )?,
            bonus_music: selection
                .bonus_music
                .map(|hashes| load_tracks(store, hashes, BONUS_TRACKS))
                .transpose()?,
            cemetery_music: selection
                .cemetery_music
                .map(|hash| load_cemetery_music(store, hash))
                .transpose()?,
            clips,
            effects_volume: selection.effects_volume,
            entrance_effect: load_optional_effect(
                &bundle,
                CEMETERY_EFFECT_KEY,
                PITCH_E40_RATE,
                (CEMETERY_EFFECT_FRAMES, CEMETERY_EFFECT_TAIL),
            )?,
            handoff_music: selection
                .handoff_music
                .map(|hashes| load_tracks(store, hashes, HANDOFF_TRACKS))
                .transpose()?,
            hurt: [marge?, homer?, bart?, lisa?],
            impact: load_optional_effect(
                &bundle,
                IMPACT_KEY,
                PITCH_E40_RATE,
                (IMPACT_FRAMES, 0),
            )?,
            music: selection
                .music
                .map(|hash| load_street_music(store, hash))
                .transpose()?,
            music_volume: selection.music_volume,
            stage_two_music: selection
                .stage_two_music
                .map(|hash| load_stage_two_music(store, hash))
                .transpose()?,
        })
    }

    fn source_settings(&self) -> SourceSettings {
        SourceSettings {
            effects: Some(self.effects_volume),
            music: self.music.as_ref().map(|_clip| self.music_volume),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MusicCue {
    Cemetery,
    Defeat,
    Instructions,
    Pumping,
    Quiet,
    Results,
    StageTwo,
    Street,
    Transition,
    Victory,
}

impl MusicCue {
    fn select(cabinet: &Cabinet) -> Self {
        if let Some(entrance) = cabinet.cemetery_entrance() {
            return if entrance.phase() == CemeteryPhase::Card
                || entrance.tick() == 0
            {
                Self::Quiet
            } else {
                Self::Cemetery
            };
        }
        if cabinet.stage_two() {
            return Self::StageTwo;
        }
        if let Some(bonus) = cabinet.bonus() {
            return match bonus.phase() {
                BonusPhase::Pumping => Self::Pumping,
                BonusPhase::Results => Self::Results,
                BonusPhase::Instructions => Self::Instructions,
                BonusPhase::Victory if bonus.tick() < VICTORY_CUE_TICK => {
                    Self::Defeat
                },
                BonusPhase::Victory if bonus.tick() < TRANSITION_CUE_TICK => {
                    Self::Victory
                },
                BonusPhase::Victory => Self::Transition,
                BonusPhase::StageSetup if bonus.tick() >= 1 => Self::StageTwo,
                BonusPhase::Complete
                | BonusPhase::Ready
                | BonusPhase::StageCard
                | BonusPhase::StageSetup => Self::Quiet,
            };
        }
        if cabinet.wrestler().is_some_and(|wrestler| {
            matches!(
                wrestler.action,
                WrestlerAction::Defeat { .. } | WrestlerAction::Cleared
            )
        }) {
            Self::Defeat
        } else {
            Self::Street
        }
    }
}

struct Soundtrack {
    bonus: Option<[OwnedClip; 2]>,
    cemetery: Option<OwnedClip>,
    cue: MusicCue,
    handoff: Option<[OwnedClip; 4]>,
    stage_two: Option<OwnedClip>,
    street: Option<OwnedClip>,
}

impl Soundtrack {
    fn clip(&self, cue: MusicCue) -> Option<&OwnedClip> {
        match cue {
            MusicCue::Cemetery => self.cemetery.as_ref(),
            MusicCue::Defeat => {
                self.handoff.as_ref().map(|[defeat, ..]| defeat)
            },
            MusicCue::Victory => {
                self.handoff.as_ref().map(|[_, victory, ..]| victory)
            },
            MusicCue::Transition => self
                .handoff
                .as_ref()
                .map(|[_, _, transition, _]| transition),
            MusicCue::Instructions => self
                .handoff
                .as_ref()
                .map(|[_, _, _, instructions]| instructions),
            MusicCue::Pumping => self.bonus.as_ref().map(|[pump, _]| pump),
            MusicCue::Results => {
                self.bonus.as_ref().map(|[_, results]| results)
            },
            MusicCue::StageTwo => self.stage_two.as_ref(),
            MusicCue::Street => self.street.as_ref(),
            MusicCue::Quiet => None,
        }
    }

    fn select(
        &mut self,
        mixer: &mut OwnedMixer,
        next: MusicCue,
    ) -> Result<bool, RenderError> {
        if self.cue == next {
            return Ok(false);
        }
        let track = self.clip(next);
        let changed = self.clip(self.cue).is_some() || track.is_some();
        mixer.stop_source(AudioSource::Music);
        let gain = SourceGain {
            source: AudioSource::Music,
            stereo: StereoGain::UNITY,
        };
        if let Some(clip) = track {
            let _voice = if next == MusicCue::Street {
                mixer.play_loop_on(
                    clip.clone(),
                    gain,
                    MUSIC_LOOP_START..MUSIC_FRAMES,
                )
            } else {
                mixer.play_on(clip.clone(), gain)
            }
            .map_err(failure)?;
        }
        self.cue = next;
        Ok(changed)
    }
}

struct CountedEffect {
    clip: Option<OwnedClip>,
    count: u8,
    gain: StereoGain,
}

impl CountedEffect {
    const fn new(clip: Option<OwnedClip>, gain: StereoGain) -> Self {
        Self { clip, count: 0, gain }
    }

    fn observe(
        &mut self,
        mixer: &mut OwnedMixer,
        count: u8,
    ) -> Result<(), RenderError> {
        let changed = count > self.count;
        self.count = count;
        if changed && let Some(clip) = &self.clip {
            return match mixer.retrigger_effect(clip.clone(), self.gain) {
                Ok(_) | Err(AudioError::VoicesFull) => Ok(()),
                Err(error) => Err(failure(error)),
            };
        }
        Ok(())
    }
}

pub(super) struct Playback {
    ambient: CountedEffect,
    clips: [OwnedClip; 4],
    cues: [u64; 4],
    device: NativeAudio,
    discard_queued: bool,
    entrance: CountedEffect,
    hurt: [Option<OwnedClip>; 4],
    hurt_cues: [u64; 4],
    impact: Option<OwnedClip>,
    impact_cues: u64,
    live_output: bool,
    mixer: OwnedMixer,
    music: Soundtrack,
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
        self.mixer.active_on(AudioSource::Music)
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
        let [ambient, entrance] = samples.cemetery_effects();
        Ok(Self {
            ambient,
            clips: samples.clips()?,
            cues: [0; 4],
            device,
            discard_queued: false,
            entrance,
            hurt: samples.hurt.clone(),
            hurt_cues: [0; 4],
            impact: samples.impact.clone(),
            impact_cues: 0,
            live_output: false,
            mixer,
            music: Soundtrack {
                bonus: samples.bonus_music.clone(),
                cemetery: samples.cemetery_music.clone(),
                cue: MusicCue::Quiet,
                handoff: samples.handoff_music.clone(),
                stage_two: samples.stage_two_music.clone(),
                street: samples.music.clone(),
            },
            scratch: [[0; 2]; CHUNK],
            sources,
            volume,
        })
    }

    fn play_effects(&mut self, cabinet: &Cabinet) -> Result<(), RenderError> {
        let stations = cabinet.stations();
        for (((previous, station), clip), gain) in self
            .cues
            .iter_mut()
            .zip(stations)
            .zip(&self.clips)
            .zip(GAINS)
        {
            // A station can emit at most one attack cue per fixed tick.
            play_cues(
                &mut self.mixer,
                (previous, station.attack_cues),
                (Some(clip), gain),
                8,
            )?;
        }
        for (((previous, station), clip), gain) in self
            .hurt_cues
            .iter_mut()
            .zip(stations)
            .zip(&self.hurt)
            .zip(HURT_GAINS)
        {
            // Each enemy can strike at most once within eight fixed ticks.
            play_cues(
                &mut self.mixer,
                (previous, station.hurt_cues),
                (clip.as_ref(), gain),
                ENEMY_CAPACITY,
            )?;
        }
        // A session cannot defeat more enemies than its admitted capacity.
        play_cues(
            &mut self.mixer,
            (&mut self.impact_cues, cabinet.impact_cues()),
            (self.impact.as_ref(), IMPACT_GAIN),
            ENEMY_CAPACITY,
        )?;
        let entrance = cabinet.cemetery_entrance();
        self.ambient.observe(
            &mut self.mixer,
            entrance.map_or(0, CemeteryEntrance::ambient_effect_cues),
        )?;
        self.entrance.observe(
            &mut self.mixer,
            entrance.map_or(0, CemeteryEntrance::entrance_effect_cues),
        )
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
        self.smoke_source_changes()?;
        self.smoke_impact()?;
        self.smoke_hurts()?;
        self.smoke_bonus_music()?;
        for roster in [
            [true, true, false, false],
            [true, true, true, false],
            [true, true, false, true],
            [true; 4],
        ] {
            self.smoke_cemetery_ambient(roster)?;
        }
        Ok(())
    }

    fn smoke_bonus_music(&mut self) -> Result<(), RenderError> {
        if self.music.bonus.is_none() {
            return Ok(());
        }
        let mut cabinet = smoke_campaign(
            self.smoke_session(),
            self.music.stage_two.is_some(),
        )?;
        self.update(&cabinet)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        self.update(&cabinet)?;
        self.verify_quiet()?;
        clear_smoke_wrestler(&mut cabinet);
        self.update(&cabinet)?;
        if self.music.cue != MusicCue::Defeat
            || (self.music.handoff.is_none()
                && (self.music_voices() != 0 || self.queued_frames()? != 0))
        {
            return Err(failure("defeat retained queued street music"));
        }
        self.smoke_handoff_music(&mut cabinet)?;
        self.smoke_bonus_phase(&mut cabinet, BonusPhase::Pumping)?;
        self.smoke_bonus_pause(&mut cabinet)?;
        self.smoke_bonus_phase(&mut cabinet, BonusPhase::Results)?;
        self.smoke_bonus_results(&cabinet)?;
        self.smoke_bonus_phase(&mut cabinet, BonusPhase::Complete)?;
        write_status(format_args!(
            "Bonus music PCM: street, pumping, results and silence verified"
        ))
        .map_err(failure)?;
        self.smoke_stage_two_music(&mut cabinet)?;
        self.smoke_cemetery_music()
    }

    fn smoke_bonus_pause(
        &mut self,
        cabinet: &mut Cabinet,
    ) -> Result<(), RenderError> {
        self.verify_submission(0)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Pause));
        self.device.clear().map_err(failure)?;
        self.update(cabinet)?;
        if cabinet.phase() != Phase::Paused
            || self.queued_frames()? != 0
            || self.music_voices() != 1
        {
            return Err(failure("paused bonus music advanced"));
        }
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Pause));
        self.update(cabinet)?;
        self.verify_submission(0)
    }

    fn smoke_bonus_phase(
        &mut self,
        cabinet: &mut Cabinet,
        expected: BonusPhase,
    ) -> Result<(), RenderError> {
        for tick in 0..2000 {
            cabinet.input_step(tick, &[]);
            self.update(cabinet)?;
            if cabinet
                .bonus()
                .is_some_and(|bonus| bonus.phase() == expected)
            {
                return Ok(());
            }
        }
        Err(failure(
            "bonus music smoke did not reach the expected phase",
        ))
    }

    fn smoke_bonus_results(
        &mut self,
        cabinet: &Cabinet,
    ) -> Result<(), RenderError> {
        if self.music.cue != MusicCue::Results {
            return Err(failure("results music not selected"));
        }
        self.verify_submission(0)?;
        // Hidden smoke has no consumer; drain the finite clip in bounded
        // queue-sized steps without changing its fixed-step phase.
        for _ in 0..120u8 {
            self.device.clear().map_err(failure)?;
            self.update(cabinet)?;
        }
        if self.music_voices() != 0 || self.queued_frames()? != 0 {
            return Err(failure("finished results music restarted"));
        }
        Ok(())
    }

    fn smoke_cemetery_ambient(
        &mut self,
        roster: [bool; 4],
    ) -> Result<(), RenderError> {
        if self.ambient.clip.is_none() {
            return Ok(());
        }
        let mut cabinet = smoke_campaign(self.smoke_session(), true)?;
        self.update(&cabinet)?;
        let starts: Vec<_> = Player::ALL
            .into_iter()
            .zip(roster)
            .filter(|(_, admitted)| *admitted)
            .flat_map(|(player, _)| {
                [true, false].map(|down| LogicalAction {
                    action: Action {
                        control: Control::Start,
                        player,
                    },
                    down,
                })
            })
            .collect();
        cabinet.input_step(0, &starts);
        cabinet.enter_stage_two(270);
        let admitted = cabinet.stations().map(crate::Station::can_fight);
        if admitted != roster {
            return Err(failure("ambient smoke roster differs"));
        }
        cabinet.cemetery = Some(CemeteryEntrance::new(admitted, Player::Marge));
        self.update(&cabinet)?;
        for tick in 1..=192u64 {
            cabinet.input_step(tick, &[]);
            self.update(&cabinet)?;
            if self.ambient.count != 0 || self.mixer.active() != 0 {
                return Err(failure("ambient effect leaked through the card"));
            }
        }
        self.smoke_cemetery_ambient_setup(&mut cabinet, roster)
    }

    fn smoke_cemetery_ambient_setup(
        &mut self,
        cabinet: &mut Cabinet,
        roster: [bool; 4],
    ) -> Result<(), RenderError> {
        let mut expected = 0u8;
        for tick in 1..=250u64 {
            let receipt =
                (13..=168).contains(&tick) && cabinet.board_phase == 0;
            cabinet.input_step(tick, &[]);
            self.device.clear().map_err(failure)?;
            self.update(cabinet)?;
            expected = expected.saturating_add(u8::from(receipt));
            if self.ambient.count != expected {
                return Err(failure("ambient effect admission timing differs"));
            }
            if !receipt {
                continue;
            }
            if self.mixer.active_on(AudioSource::Effects) == 0
                || self.queued_frames()? != QUEUE_FRAMES
            {
                return Err(failure("ambient PCM was not submitted"));
            }
            if expected == 1 {
                self.verify_submission(1)?;
            }
        }
        if expected != 10 || self.mixer.active_on(AudioSource::Effects) != 0 {
            return Err(failure(
                "ambient smoke did not admit and drain ten cues",
            ));
        }
        write_status(format_args!(
            "Cemetery ambient PCM: ten cues for {roster:?}"
        ))
        .map_err(failure)
    }

    fn smoke_cemetery_descent(
        &mut self,
        cabinet: &mut Cabinet,
    ) -> Result<(), RenderError> {
        for tick in 194u64..430 {
            cabinet.input_step(tick, &[]);
            let entrance = cabinet
                .cemetery_entrance()
                .ok_or_else(|| failure("cemetery entrance disappeared"))?;
            let elapsed = entrance.tick();
            let expected = u8::from(elapsed >= 26)
                .saturating_add(u8::from(elapsed >= 120));
            let changed = expected > self.entrance.count;
            if changed {
                self.device.clear().map_err(failure)?;
            }
            self.update(cabinet)?;
            if self.music.cue != MusicCue::Cemetery || self.music_voices() != 1
            {
                return Err(failure(
                    "cemetery soundtrack stopped before landing",
                ));
            }
            if self.entrance.count != expected {
                return Err(failure("cemetery entrance effect timing differs"));
            }
            if changed {
                self.verify_submission(usize::from(
                    self.entrance.clip.is_some(),
                ))?;
            }
        }
        if self.entrance.clip.is_some() {
            write_status(format_args!(
                "Cemetery entrance PCM: two timed one-shot admissions verified"
            ))
            .map_err(failure)?;
        }
        Ok(())
    }

    fn smoke_cemetery_music(&mut self) -> Result<(), RenderError> {
        if self.music.cemetery.is_none() {
            return Ok(());
        }
        let mut cabinet = smoke_campaign(self.smoke_session(), true)?;
        self.update(&cabinet)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        cabinet.enter_stage_two(270);
        self.update(&cabinet)?;
        self.verify_submission(0)?;
        enter_smoke_cemetery(&mut cabinet);
        self.update(&cabinet)?;
        for tick in 1..=192u64 {
            cabinet.input_step(tick, &[]);
            self.update(&cabinet)?;
            if self.music.cue != MusicCue::Quiet
                || self.music_voices() != 0
                || self.queued_frames()? != 0
            {
                return Err(failure("cemetery card retained music"));
            }
        }
        cabinet.input_step(193, &[]);
        self.update(&cabinet)?;
        if self.music.cue != MusicCue::Cemetery || self.music_voices() != 1 {
            return Err(failure("cemetery soundtrack did not start"));
        }
        self.verify_submission(0)?;
        self.smoke_bonus_pause(&mut cabinet)?;
        self.smoke_cemetery_descent(&mut cabinet)?;
        for _ in 0..1500u16 {
            self.device.clear().map_err(failure)?;
            self.update(&cabinet)?;
        }
        if self.music_voices() != 0 || self.queued_frames()? != 0 {
            return Err(failure("finished cemetery music restarted"));
        }
        write_status(format_args!(
            "Cemetery music PCM: card, setup, pause and finite end verified"
        ))
        .map_err(failure)
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

    fn smoke_handoff_cue(
        &mut self,
        cabinet: &mut Cabinet,
        expected: MusicCue,
    ) -> Result<(), RenderError> {
        for tick in 0..1000 {
            if self.music.cue == expected {
                break;
            }
            cabinet.input_step(tick, &[]);
            self.update(cabinet)?;
        }
        if self.music.cue != expected
            || self.music_voices() != 1
            || self.queued_frames()? != QUEUE_FRAMES
        {
            return Err(failure("handoff music cue not admitted"));
        }
        Ok(())
    }

    fn smoke_handoff_music(
        &mut self,
        cabinet: &mut Cabinet,
    ) -> Result<(), RenderError> {
        if self.music.handoff.is_none() {
            return Ok(());
        }
        for expected in [
            MusicCue::Defeat,
            MusicCue::Victory,
            MusicCue::Transition,
            MusicCue::Instructions,
        ] {
            self.smoke_handoff_cue(cabinet, expected)?;
        }
        write_status(format_args!(
            "Handoff music PCM: four finite cues verified"
        ))
        .map_err(failure)
    }

    fn smoke_hurt_station(
        &mut self,
        player: Player,
        depleted_stock: Option<u8>,
    ) -> Result<(), RenderError> {
        let layout = StreetLayout::DOWNTOWN;
        let position = layout
            .spawn
            .get(player.index())
            .ok_or(RenderError::InvalidSprite)?;
        let mut cabinet = Cabinet::with_encounter(layout, Point {
            x: position.x.saturating_add(18),
            y: position.y,
        })
        .map_err(failure)?;
        cabinet.session = self.smoke_session();
        self.update(&cabinet)?;
        cabinet.input_step(
            0,
            &[true, false].map(|down| LogicalAction {
                action: Action {
                    control: Control::Start,
                    player,
                },
                down,
            }),
        );
        if let Some(stocks) = depleted_stock {
            let slot = cabinet
                .slots
                .get_mut(player.index())
                .ok_or(RenderError::InvalidSprite)?;
            slot.station.health = 1;
            slot.station.display_health = 1;
            slot.station.stocks = stocks;
        }
        self.update(&cabinet)?;
        let mut simulation = fixed(cabinet)?;
        self.verify_quiet()?;
        for _ in 0..14u8 {
            let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
            self.device.clear().map_err(failure)?;
            self.update(simulation.state())?;
        }
        if let Some(stocks) = depleted_stock {
            let stations = simulation.state().stations();
            let station = stations
                .get(player.index())
                .ok_or(RenderError::InvalidSprite)?;
            if station.health != 0 || station.stocks != stocks.saturating_sub(1)
            {
                return Err(failure("ground hit did not deplete the stock"));
            }
        }
        let expected = Player::ALL.map(|station| u64::from(station == player));
        if self.hurt_cues != expected {
            return Err(failure("ground hit did not admit the station cue"));
        }
        self.verify_submission(1)?;
        self.update(simulation.state())?;
        self.verify_submission(1)
    }

    fn smoke_hurts(&mut self) -> Result<(), RenderError> {
        let available = self.hurt.each_ref().map(Option::is_some);
        for (player, present) in Player::ALL.into_iter().zip(available) {
            if !present {
                continue;
            }
            for stock in [None, Some(crate::PLAYER_STOCKS), Some(1)] {
                self.smoke_hurt_station(player, stock)?;
            }
        }
        let count = available.into_iter().filter(|present| *present).count();
        if count != 0 {
            write_status(format_args!(
                "Ground hurt PCM: {count} stations, three hit cases each"
            ))
            .map_err(failure)?;
        }
        Ok(())
    }

    fn smoke_impact(&mut self) -> Result<(), RenderError> {
        if self.impact.is_none() {
            return Ok(());
        }
        let mut cabinet =
            Cabinet::with_encounter(StreetLayout::DOWNTOWN, Point {
                x: 132,
                y: 190,
            })
            .map_err(failure)?;
        cabinet.session = self.smoke_session();
        let mut simulation = fixed(cabinet)?;
        let mut controls = Controls::new(false)?;
        self.update(simulation.state())?;
        smoke_press(&mut controls, &mut simulation, Control::Start)?;
        self.update(simulation.state())?;
        self.verify_quiet()?;
        smoke_press(&mut controls, &mut simulation, Control::Primary)?;
        for _ in 0..10u8 {
            let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
            self.device.clear().map_err(failure)?;
            self.update(simulation.state())?;
        }
        if self.impact_cues != 1 {
            return Err(failure("defeat did not admit exactly one impact cue"));
        }
        self.verify_submission(2)?;
        self.update(simulation.state())?;
        self.verify_submission(2)?;
        write_status(format_args!("Combat impact PCM: one cue verified"))
            .map_err(failure)
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
            || self.music.cue != MusicCue::Quiet
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

    fn smoke_stage_two_music(
        &mut self,
        cabinet: &mut Cabinet,
    ) -> Result<(), RenderError> {
        if self.music.stage_two.is_none() {
            return Ok(());
        }
        for tick in 0..220 {
            cabinet.input_step(tick, &[]);
            self.update(cabinet)?;
            if cabinet.stage_two() {
                break;
            }
            let setup_cue = cabinet.bonus().is_some_and(|bonus| {
                bonus.phase() == BonusPhase::StageSetup && bonus.tick() >= 1
            });
            let expected = if setup_cue {
                MusicCue::StageTwo
            } else {
                MusicCue::Quiet
            };
            if self.music.cue != expected
                || self.music_voices() != usize::from(setup_cue)
            {
                return Err(failure("stage setup soundtrack timing differs"));
            }
        }
        self.update(cabinet)?;
        if !cabinet.stage_two()
            || self.music.cue != MusicCue::StageTwo
            || self.music_voices() != 1
        {
            return Err(failure("second-stage music not selected"));
        }
        self.smoke_bonus_pause(cabinet)?;
        // No simulation step during draining: the finite selected cue
        // must remain exhausted until a new soundtrack state is entered.
        for _ in 0..1500u16 {
            self.device.clear().map_err(failure)?;
            self.update(cabinet)?;
        }
        if self.music_voices() != 0 || self.queued_frames()? != 0 {
            return Err(failure("finished stage-two music restarted"));
        }
        write_status(format_args!(
            "Second-stage music PCM: entry, pause and finite end verified"
        ))
        .map_err(failure)
    }

    pub(super) fn soundtrack_smoke(
        mut self,
    ) -> Result<SoundtrackSmoke, RenderError> {
        if self.music.stage_two.is_none() {
            return Err(failure("stage-two soundtrack selection is required"));
        }
        let mut cabinet = smoke_campaign(self.smoke_session(), true)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        let target = if self.music.cemetery.is_some() {
            MusicCue::Cemetery
        } else {
            MusicCue::StageTwo
        };
        self.enable_output();
        Ok(SoundtrackSmoke {
            cabinet,
            output: self,
            phase: SoundtrackSmokePhase::Street,
            start: None,
            target,
        })
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
        let phase = cabinet.phase();
        let playing = phase == Phase::Playing;
        if matches!(phase, Phase::MainMenu | Phase::Exited) {
            let stations = cabinet.stations();
            self.cues = stations.map(|station| station.attack_cues);
            self.hurt_cues = stations.map(|station| station.hurt_cues);
            self.impact_cues = cabinet.impact_cues();
            self.ambient.count = 0;
            self.entrance.count = 0;
            self.discard_queued = true;
            self.mixer.clear();
            self.music.cue = MusicCue::Quiet;
        }
        if playing
            && self
                .music
                .select(&mut self.mixer, MusicCue::select(cabinet))?
        {
            // Pending frames can still contain the previous track.
            self.discard_queued = true;
        }
        let ready = self.ready(self.live_output && playing)?;
        if !playing {
            return Ok(());
        }
        self.play_effects(cabinet)?;
        if ready {
            self.fill()
        } else {
            Ok(())
        }
    }

    fn verify_idle_effects(&self) -> Result<(), RenderError> {
        if self.music.street.is_some() {
            if self.music_voices() != 1 {
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
        if self.cues != [0; 4]
            || self.hurt_cues != [0; 4]
            || self.impact_cues != 0
            || self.ambient.count != 0
            || self.entrance.count != 0
        {
            return Err(failure("audio played before an admitted cue"));
        }
        self.verify_idle_effects()
    }

    fn verify_submission(&self, active: usize) -> Result<(), RenderError> {
        let levels = self.sources.levels();
        let audible = self.volume != MasterVolume::Muted
            && ((active != 0 && levels.effects != MasterVolume::Muted)
                || (self.music_voices() != 0
                    && levels.music != MasterVolume::Muted));
        if self.mixer.active() != active.saturating_add(self.music_voices())
            || self.queued_frames()? != QUEUE_FRAMES
            || self.scratch.iter().all(|frame| *frame == [0, 0]) == audible
        {
            return Err(failure("unexpected native audio submission"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct CemeteryEffectProbe {
    clock: StepClock,
    first_drained: bool,
    observed: u8,
    previous: Duration,
}

struct CemeterySmokeTicks<'cabinet>(&'cabinet mut Cabinet);

impl FixedTick for CemeterySmokeTicks<'_> {
    fn fixed_tick(&mut self, index: u64) {
        self.0.input_step(index, &[]);
    }
}

#[derive(Clone, Copy)]
enum SoundtrackSmokePhase {
    Cemetery,
    CemeteryCard,
    CemeteryEffects(CemeteryEffectProbe),
    Draining,
    Exhausted { since: Duration },
    Paused { since: Duration, queued: u32 },
    Pausing,
    Resuming,
    StageTwo,
    Street,
}

/// Finite host-consumed soundtrack diagnostic. Unlike the hidden native
/// smoke, this never clears the queue to accelerate the finite excerpt.
pub(super) struct SoundtrackSmoke {
    cabinet: Cabinet,
    output: Playback,
    phase: SoundtrackSmokePhase,
    start: Option<Duration>,
    target: MusicCue,
}

impl SoundtrackSmoke {
    fn begin_drain(&mut self, timestamp: Duration) -> Result<(), RenderError> {
        self.phase = if self.target == MusicCue::Cemetery
            && self.output.entrance.clip.is_some()
        {
            SoundtrackSmokePhase::CemeteryEffects(CemeteryEffectProbe {
                clock: StepClock::new(TICK_INTERVAL, dimension(8)?)
                    .map_err(failure)?,
                first_drained: false,
                observed: 0,
                previous: timestamp,
            })
        } else {
            SoundtrackSmokePhase::Draining
        };
        Ok(())
    }

    fn pause_track(&mut self) {
        self.cabinet
            .input_step(0, &bonus_smoke_pulse(Control::Pause));
        self.phase = SoundtrackSmokePhase::Pausing;
    }

    fn report(message: &str) -> Result<(), RenderError> {
        write_status(format_args!("Browser soundtrack: {message}"))
            .map_err(failure)
    }

    pub(super) fn step(
        &mut self,
        timestamp: Duration,
    ) -> Result<bool, RenderError> {
        let start = *self.start.get_or_insert(timestamp);
        if timestamp.saturating_sub(start) > Duration::from_secs(90) {
            return Err(failure("soundtrack smoke exceeded host time budget"));
        }
        self.output.update(&self.cabinet)?;
        if self.output.device.poll_control().map_err(failure)?
            == PlaybackControl::Pending
        {
            return Ok(true);
        }
        let queued = self.output.queued_frames()?;
        match self.phase {
            SoundtrackSmokePhase::Street => self.step_street(queued)?,
            SoundtrackSmokePhase::StageTwo => self.step_stage_two(queued)?,
            SoundtrackSmokePhase::CemeteryCard => {
                self.step_cemetery_card(queued)?;
            },
            SoundtrackSmokePhase::CemeteryEffects(probe) => {
                self.step_cemetery_effects(timestamp, probe)?;
            },
            SoundtrackSmokePhase::Cemetery
            | SoundtrackSmokePhase::Draining
            | SoundtrackSmokePhase::Exhausted { .. }
            | SoundtrackSmokePhase::Paused { .. }
            | SoundtrackSmokePhase::Pausing
            | SoundtrackSmokePhase::Resuming => {
                self.step_track(timestamp, queued)?;
            },
        }
        Ok(!matches!(
            self.phase,
            SoundtrackSmokePhase::Exhausted { since }
                if timestamp.saturating_sub(since)
                    >= Duration::from_millis(250)
        ))
    }

    fn step_cemetery_card(&mut self, queued: u32) -> Result<(), RenderError> {
        if self.output.music.cue != MusicCue::Quiet
            || self.output.music_voices() != 0
            || queued != 0
        {
            return Err(failure("cemetery card retained queued music"));
        }
        Self::report("cemetery card silent")?;
        for tick in 1..=192u64 {
            self.cabinet.input_step(tick, &[]);
            if MusicCue::select(&self.cabinet) != MusicCue::Quiet {
                return Err(failure("cemetery music started on the card"));
            }
        }
        self.cabinet.input_step(193, &[]);
        self.phase = SoundtrackSmokePhase::Cemetery;
        Ok(())
    }

    fn step_cemetery_effects(
        &mut self,
        timestamp: Duration,
        mut probe: CemeteryEffectProbe,
    ) -> Result<(), RenderError> {
        self.verify_voice(self.target)?;
        let elapsed = timestamp.saturating_sub(probe.previous);
        let _advanced = probe
            .clock
            .advance(elapsed, &mut CemeterySmokeTicks(&mut self.cabinet))
            .map_err(failure)?;
        probe.previous = timestamp;
        let entrance = self
            .cabinet
            .cemetery_entrance()
            .ok_or_else(|| failure("cemetery entrance disappeared"))?;
        let tick = entrance.tick();
        let expected =
            u8::from(tick >= 26).saturating_add(u8::from(tick >= 120));
        self.output.update(&self.cabinet)?;
        let active = self.output.mixer.active_on(AudioSource::Effects);
        if self.output.entrance.count != expected
            || active > 1
            || (expected == 0 && active != 0)
            || (expected == probe.observed
                && expected == 1
                && probe.first_drained
                && active != 0)
        {
            return Err(failure("browser entrance effect timing differs"));
        }
        if expected != probe.observed {
            if active != 1
                || expected != probe.observed.saturating_add(1)
                || (expected == 2 && !probe.first_drained)
            {
                return Err(failure(
                    "browser entrance effect admission differs",
                ));
            }
            probe.observed = expected;
            Self::report(if expected == 1 {
                "cemetery setup effect admitted"
            } else {
                "cemetery descent effect admitted"
            })?;
        }
        if expected == 1 && active == 0 {
            probe.first_drained = true;
        }
        if expected == 2 && active == 0 {
            Self::report("cemetery entrance effects drained")?;
            self.phase = SoundtrackSmokePhase::Draining;
        } else {
            self.phase = SoundtrackSmokePhase::CemeteryEffects(probe);
        }
        Ok(())
    }

    fn step_stage_two(&mut self, queued: u32) -> Result<(), RenderError> {
        if !self.cabinet.stage_two() {
            return Err(failure("second-stage fixture did not arrive"));
        }
        self.verify_voice(MusicCue::StageTwo)?;
        if queued == 0 {
            return Ok(());
        }
        Self::report("second-stage queued")?;
        if self.target == MusicCue::Cemetery {
            enter_smoke_cemetery(&mut self.cabinet);
            self.phase = SoundtrackSmokePhase::CemeteryCard;
        } else {
            self.pause_track();
        }
        Ok(())
    }

    fn step_street(&mut self, queued: u32) -> Result<(), RenderError> {
        self.verify_voice(MusicCue::Street)?;
        if queued != 0 {
            Self::report("street queued")?;
            enter_smoke_stage_two(&mut self.cabinet)?;
            self.phase = SoundtrackSmokePhase::StageTwo;
        }
        Ok(())
    }

    fn step_track(
        &mut self,
        timestamp: Duration,
        queued: u32,
    ) -> Result<(), RenderError> {
        if self.output.music.cue != self.target {
            return Err(failure("finite soundtrack was not selected"));
        }
        match self.phase {
            SoundtrackSmokePhase::Street
            | SoundtrackSmokePhase::StageTwo
            | SoundtrackSmokePhase::CemeteryCard
            | SoundtrackSmokePhase::CemeteryEffects(_) => {
                return Err(failure("soundtrack diagnostic state leaked"));
            },
            SoundtrackSmokePhase::Cemetery => {
                self.verify_voice(self.target)?;
                if queued != 0 {
                    Self::report("cemetery queued")?;
                    self.pause_track();
                }
            },
            SoundtrackSmokePhase::Pausing => {
                self.verify_voice(self.target)?;
                if self.cabinet.phase() != Phase::Paused {
                    return Err(failure("soundtrack did not enter pause"));
                }
                self.phase =
                    SoundtrackSmokePhase::Paused { since: timestamp, queued };
            },
            SoundtrackSmokePhase::Paused { since, queued: held } => {
                self.verify_pause(
                    timestamp.saturating_sub(since),
                    queued,
                    held,
                )?;
            },
            SoundtrackSmokePhase::Resuming => {
                self.verify_voice(self.target)?;
                if self.cabinet.phase() != Phase::Playing {
                    return Err(failure("soundtrack did not resume play"));
                }
                if queued != 0 {
                    Self::report("resumed")?;
                    self.begin_drain(timestamp)?;
                }
            },
            SoundtrackSmokePhase::Draining => {
                if self.output.music_voices() == 0 && queued == 0 {
                    self.phase =
                        SoundtrackSmokePhase::Exhausted { since: timestamp };
                }
            },
            SoundtrackSmokePhase::Exhausted { since } => {
                if self.output.music_voices() != 0 || queued != 0 {
                    return Err(failure("finite soundtrack restarted"));
                }
                if timestamp.saturating_sub(since) >= Duration::from_millis(250)
                {
                    Self::report("finite end")?;
                }
            },
        }
        Ok(())
    }

    fn verify_pause(
        &mut self,
        elapsed: Duration,
        queued: u32,
        held: u32,
    ) -> Result<(), RenderError> {
        self.verify_voice(self.target)?;
        if queued != held || self.cabinet.phase() != Phase::Paused {
            return Err(failure("paused soundtrack queue changed"));
        }
        if elapsed >= Duration::from_millis(150) {
            Self::report("pause held")?;
            self.cabinet
                .input_step(0, &bonus_smoke_pulse(Control::Pause));
            self.phase = SoundtrackSmokePhase::Resuming;
        }
        Ok(())
    }

    fn verify_voice(&self, cue: MusicCue) -> Result<(), RenderError> {
        if self.output.music.cue != cue || self.output.music_voices() != 1 {
            return Err(failure("expected soundtrack voice is absent"));
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
    stereo_limits(MUSIC_FRAMES)
}

pub(super) fn bonus_music_limits() -> Result<ManifestLimits, RenderError> {
    track_limits(BONUS_TRACKS)
}

pub(super) fn handoff_music_limits() -> Result<ManifestLimits, RenderError> {
    track_limits(HANDOFF_TRACKS)
}

pub(super) fn stage_two_music_limits() -> Result<ManifestLimits, RenderError> {
    stereo_limits(STAGE_TWO_TRACK.1)
}

pub(super) fn cemetery_music_limits() -> Result<ManifestLimits, RenderError> {
    stereo_limits(CEMETERY_TRACK.1)
}

fn track_limits<const COUNT: usize>(
    tracks: [(&str, usize); COUNT],
) -> Result<ManifestLimits, RenderError> {
    stereo_limits(
        tracks
            .into_iter()
            .map(|(_, frames)| frames)
            .max()
            .ok_or(RenderError::TooLarge)?,
    )
}

fn stereo_limits(frames: usize) -> Result<ManifestLimits, RenderError> {
    let bytes = u64::try_from(frames)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .and_then(NonZeroU64::new)
        .ok_or(RenderError::TooLarge)?;
    Ok(ManifestLimits {
        manifest_bytes: size(4096)?,
        payload_bytes: bytes,
        records: size(1)?,
        total_bytes: bytes,
    })
}

fn copy_samples<I>(source: I) -> Result<SharedSamples, RenderError>
where
    I: ExactSizeIterator<Item = i16>,
{
    let mut samples = Vec::new();
    samples.try_reserve_exact(source.len()).map_err(failure)?;
    samples.extend(source);
    Ok(Rc::from(samples))
}

fn mono_pcm<'payload>(
    record: &AssetRecord,
    bytes: &'payload [u8],
    rate: NonZeroU32,
    frames: u64,
) -> Result<PcmSamples<'payload>, RenderError> {
    let pcm = PcmSamples::new(record, bytes).map_err(failure)?;
    if pcm.channels() != NonZeroU8::MIN
        || pcm.sample_rate() != rate
        || pcm.frames().get() != frames
    {
        return Err(failure("unexpected effects PCM format"));
    }
    Ok(pcm)
}

fn load_attack_clips(
    bundle: &CanonicalBundle,
    replacement: Option<&CanonicalBundle>,
) -> Result<[SharedSamples; 4], RenderError> {
    let allowed = CLIPS.map(|(key, _frames)| key);
    let overlay = replacement
        .map(|changes| MediaOverlay::new(bundle, changes, &allowed))
        .transpose()
        .map_err(failure)?;
    let mut clips = Vec::with_capacity(CLIPS.len());
    for (key, frames) in CLIPS {
        let (descriptor, payload) = overlay.as_ref().map_or_else(
            || (bundle.manifest().record(key), bundle.view_payload(key)),
            |changes| (changes.record(key), changes.view_payload(key)),
        );
        let record = descriptor.ok_or_else(|| failure("missing attack PCM"))?;
        let bytes = payload.map_err(failure)?;
        let pcm = mono_pcm(record, &bytes, SOURCE_RATE, frames)?;
        clips.push(copy_samples(pcm.samples())?);
    }
    clips.try_into().map_err(|_error| failure("clip count"))
}

fn load_optional_effect(
    bundle: &CanonicalBundle,
    key: &str,
    rate: NonZeroU32,
    extent: (u64, usize),
) -> Result<Option<OwnedClip>, RenderError> {
    let (frames, trim) = extent;
    let Some(record) = bundle.manifest().record(key) else {
        return Ok(None);
    };
    let bytes = bundle.view_payload(key).map_err(failure)?;
    let pcm = mono_pcm(record, &bytes, rate, frames)?;
    let count = pcm
        .samples()
        .len()
        .checked_sub(trim)
        .ok_or(RenderError::TooLarge)?;
    let samples = copy_samples(pcm.samples().take(count))?;
    OwnedClip::new(samples, Channels::Mono, rate)
        .map(Some)
        .map_err(failure)
}

fn load_music(
    store: BundleSource<'_>,
    hash: ContentHash,
    (key, frames): (&str, usize),
    limits: ManifestLimits,
) -> Result<OwnedClip, RenderError> {
    let bundle = open_selected_bundle(store, hash, limits)?;
    let record = bundle
        .manifest()
        .record(key)
        .ok_or_else(|| failure("missing music PCM"))?;
    let bytes = bundle.view_payload(key).map_err(failure)?;
    let pcm = PcmSamples::new(record, &bytes).map_err(failure)?;
    if pcm.channels() != NonZeroU8::MIN.saturating_add(1)
        || pcm.sample_rate() != RATE
        || usize::try_from(pcm.frames().get()).ok() != Some(frames)
    {
        return Err(failure("unexpected music PCM format"));
    }
    OwnedClip::new(copy_samples(pcm.samples())?, Channels::Stereo, RATE)
        .map_err(failure)
}

fn load_cemetery_music(
    store: BundleSource<'_>,
    hash: ContentHash,
) -> Result<OwnedClip, RenderError> {
    load_music(store, hash, CEMETERY_TRACK, cemetery_music_limits()?)
}

fn load_stage_two_music(
    store: BundleSource<'_>,
    hash: ContentHash,
) -> Result<OwnedClip, RenderError> {
    load_music(store, hash, STAGE_TWO_TRACK, stage_two_music_limits()?)
}

fn load_street_music(
    store: BundleSource<'_>,
    hash: ContentHash,
) -> Result<OwnedClip, RenderError> {
    load_music(store, hash, (MUSIC_KEY, MUSIC_FRAMES), music_limits()?)
}

fn load_tracks<const COUNT: usize>(
    store: BundleSource<'_>,
    hashes: [ContentHash; COUNT],
    tracks: [(&str, usize); COUNT],
) -> Result<[OwnedClip; COUNT], RenderError> {
    let limits = track_limits(tracks)?;
    hashes
        .into_iter()
        .zip(tracks)
        .map(|(hash, track)| load_music(store, hash, track, limits))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_error| failure("music track count"))
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

fn play_cues(
    mixer: &mut OwnedMixer,
    (previous, count): (&mut u64, u64),
    (available, gain): (Option<&OwnedClip>, StereoGain),
    limit: usize,
) -> Result<(), RenderError> {
    let new_cues = count.saturating_sub(*previous);
    *previous = count;
    if usize::try_from(new_cues).map_err(failure)? > limit {
        return Err(failure("effect cue count exceeded admission budget"));
    }
    if let Some(clip) = available {
        for _ in 0..new_cues {
            play(mixer, clip.clone(), gain)?;
        }
    }
    Ok(())
}

fn smoke_campaign(
    session: LocalSession,
    stage_two: bool,
) -> Result<Cabinet, RenderError> {
    let mut cabinet = Cabinet::with_opening_street()
        .and_then(|state| state.with_scroll_limit(1360))
        .map_err(failure)?
        .with_late_wrestler(true)
        .with_bonus(true);
    if stage_two {
        cabinet = cabinet.with_stage_two(270).map_err(failure)?;
    }
    cabinet.session = session;
    Ok(cabinet)
}

fn clear_smoke_wrestler(cabinet: &mut Cabinet) {
    cabinet.wrestler = Some(Wrestler {
        action: WrestlerAction::Cleared,
        health: 0,
        ..Wrestler::new(0)
    });
    cabinet.admit_bonus();
}

const fn enter_smoke_cemetery(cabinet: &mut Cabinet) {
    cabinet.cemetery = Some(CemeteryEntrance::new(
        [true, false, false, false],
        Player::Marge,
    ));
}

fn enter_smoke_stage_two(cabinet: &mut Cabinet) -> Result<(), RenderError> {
    clear_smoke_wrestler(cabinet);
    for tick in 0..6000 {
        cabinet.input_step(tick, &[]);
        if cabinet.stage_two() {
            return Ok(());
        }
    }
    Err(failure(
        "soundtrack diagnostic did not reach the second stage",
    ))
}

fn bonus_smoke_pulse(control: Control) -> [LogicalAction<Action>; 2] {
    [true, false].map(|down| LogicalAction {
        action: Action {
            control,
            player: Player::Marge,
        },
        down,
    })
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
    use games_assets::{AssetFormat, AssetManifest, SourceIdentity};
    use games_platform::ir::storage::fixtures::TestRoot;
    use games_platform::ir::storage::staging::CanonicalStaging;

    use super::*;
    use crate::bonus::Bonus;

    type FixtureClip<'key> = (&'key str, u64, i16);

    #[test]
    fn soundtrack_follows_terminal_response_timing() -> Result<(), RenderError>
    {
        let mut cabinet = smoke_campaign(LocalSession::default(), false)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        cabinet.wrestler = Some(Wrestler {
            action: WrestlerAction::Defeat { tick: 0 },
            health: 0,
            ..Wrestler::new(0)
        });
        for elapsed in 0..=953u64 {
            let expected = match elapsed {
                0..=274 => MusicCue::Defeat,
                275..=498 => MusicCue::Victory,
                499..=952 => MusicCue::Transition,
                _ => MusicCue::Instructions,
            };
            if MusicCue::select(&cabinet) != expected
                || cabinet.bonus().is_some() != (elapsed >= 128)
            {
                return Err(failure("soundtrack shifted from terminal timing"));
            }
            cabinet.input_step(elapsed, &[]);
        }
        Ok(())
    }

    #[test]
    fn soundtrack_fixture_follows_bonus_handoff() -> Result<(), RenderError> {
        let mut cabinet = smoke_campaign(LocalSession::default(), true)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        if MusicCue::select(&cabinet) != MusicCue::Street || cabinet.stage_two()
        {
            return Err(failure("fixture did not start on the street"));
        }
        enter_smoke_stage_two(&mut cabinet)?;
        if !cabinet.stage_two()
            || MusicCue::select(&cabinet) != MusicCue::StageTwo
            || cabinet.phase() != Phase::Playing
        {
            return Err(failure("fixture did not enter stage two"));
        }
        Ok(())
    }

    #[test]
    fn soundtrack_starts_on_first_setup_tick_without_restarting()
    -> Result<(), RenderError> {
        let mut cabinet = smoke_campaign(LocalSession::default(), true)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        clear_smoke_wrestler(&mut cabinet);
        let start = (1..6000u64)
            .find(|&tick| {
                cabinet.input_step(tick, &[]);
                cabinet.bonus().is_some_and(|bonus| {
                    bonus.phase() == BonusPhase::StageSetup
                })
            })
            .ok_or_else(|| failure("fixture did not reach stage setup"))?;
        if cabinet.bonus().map(Bonus::tick) != Some(0)
            || MusicCue::select(&cabinet) != MusicCue::Quiet
        {
            return Err(failure("setup entry did not retain silence"));
        }
        let clip = OwnedClip::new(
            Rc::from((1..=24i16).collect::<Vec<_>>()),
            Channels::Stereo,
            RATE,
        )
        .map_err(failure)?;
        let mut soundtrack = Soundtrack {
            bonus: None,
            cemetery: None,
            cue: MusicCue::Quiet,
            handoff: None,
            stage_two: Some(clip),
            street: None,
        };
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        for elapsed in 1..=12u64 {
            cabinet.input_step(start.saturating_add(elapsed), &[]);
            let cue = MusicCue::select(&cabinet);
            let changed = soundtrack.select(&mut mixer, cue)?;
            let mut frame = [[0; 2]];
            mixer.render(&mut frame);
            let sample =
                i16::try_from(elapsed.saturating_mul(2)).map_err(failure)?;
            if cue != MusicCue::StageTwo
                || changed != (elapsed == 1)
                || cabinet.stage_two() != (elapsed >= 11)
                || frame != [[sample.saturating_sub(1), sample]]
            {
                return Err(failure("setup cue shifted or restarted"));
            }
        }
        Ok(())
    }

    #[test]
    fn cemetery_music_starts_once_after_the_card_and_survives_landing()
    -> Result<(), RenderError> {
        let mut cabinet = smoke_campaign(LocalSession::default(), true)?;
        cabinet.input_step(0, &bonus_smoke_pulse(Control::Start));
        cabinet.enter_stage_two(270);
        let cemetery = OwnedClip::new(
            Rc::from((1..=480i16).collect::<Vec<_>>()),
            Channels::Stereo,
            RATE,
        )
        .map_err(failure)?;
        let stage_two =
            OwnedClip::new(Rc::from([500; 4]), Channels::Stereo, RATE)
                .map_err(failure)?;
        let mut soundtrack = Soundtrack {
            bonus: None,
            cemetery: Some(cemetery),
            cue: MusicCue::Quiet,
            handoff: None,
            stage_two: Some(stage_two),
            street: None,
        };
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        verify_cemetery_card(&mut cabinet, &mut soundtrack, &mut mixer)?;
        verify_cemetery_landing(&mut cabinet, &mut soundtrack, &mut mixer)
    }

    fn verify_cemetery_card(
        cabinet: &mut Cabinet,
        soundtrack: &mut Soundtrack,
        mixer: &mut OwnedMixer,
    ) -> Result<(), RenderError> {
        let started = soundtrack.select(mixer, MusicCue::select(cabinet))?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if !started || frame != [[500; 2]] {
            return Err(failure("stage-two fixture did not play"));
        }
        enter_smoke_cemetery(cabinet);
        if !soundtrack.select(mixer, MusicCue::select(cabinet))? {
            return Err(failure("stage-two cue did not stop at card"));
        }
        for tick in 1..=192u64 {
            cabinet.input_step(tick, &[]);
            let cue = MusicCue::select(cabinet);
            let changed = soundtrack.select(mixer, cue)?;
            mixer.render(&mut frame);
            if cue != MusicCue::Quiet || changed || frame != [[0; 2]] {
                return Err(failure("card did not retain silence"));
            }
        }
        if cabinet.cemetery_entrance().map(CemeteryEntrance::tick) != Some(0) {
            return Err(failure("cemetery setup fixture did not start"));
        }
        Ok(())
    }

    fn verify_cemetery_landing(
        cabinet: &mut Cabinet,
        soundtrack: &mut Soundtrack,
        mixer: &mut OwnedMixer,
    ) -> Result<(), RenderError> {
        let mut frame = [[0; 2]];
        for elapsed in 1..=240u64 {
            cabinet.input_step(192u64.saturating_add(elapsed), &[]);
            let cue = MusicCue::select(cabinet);
            let changed = soundtrack.select(mixer, cue)?;
            mixer.render(&mut frame);
            let sample =
                i16::try_from(elapsed.saturating_mul(2)).map_err(failure)?;
            if cue != MusicCue::Cemetery
                || changed != (elapsed == 1)
                || frame != [[sample.saturating_sub(1), sample]]
            {
                return Err(failure("cemetery cue shifted or restarted"));
            }
        }
        let changed = soundtrack.select(mixer, MusicCue::select(cabinet))?;
        mixer.render(&mut frame);
        if cabinet.cemetery_entrance().map(CemeteryEntrance::phase)
            != Some(CemeteryPhase::Landed)
            || changed
            || frame != [[0; 2]]
        {
            return Err(failure("finished cemetery cue did not stay silent"));
        }
        Ok(())
    }

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
        let limits = base_limits()?;
        let maximum = NonZeroU64::MIN.saturating_add(131_071);
        let budget = ArchiveBudget::new(
            maximum,
            maximum,
            NonZeroU64::MIN.saturating_add(15),
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
                sample_rate: if matches!(
                    key,
                    IMPACT_KEY | CEMETERY_EFFECT_KEY | CEMETERY_AMBIENT_KEY
                ) {
                    PITCH_E40_RATE
                } else if HURT_CLIPS.iter().any(|(clip, _frames)| *clip == key)
                {
                    HURT_RATE
                } else {
                    SOURCE_RATE
                },
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
            bonus_music: None,
            cemetery_music: None,
            effects_volume: MasterVolume::Full,
            handoff_music: None,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
            stage_two_music: None,
        };
        let original = Samples::load(BundleSource::Local(parent), &selection)?;
        if original.ambient_effect.is_some()
            || original.impact.is_some()
            || original.entrance_effect.is_some()
            || original.hurt.iter().any(Option::is_some)
            || original
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
        let modified = Samples::load(BundleSource::Local(parent), &selection)?;
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
            if Samples::load(BundleSource::Local(parent), &selection).is_ok() {
                return Err(failure("invalid replacement accepted"));
            }
        }
        Ok(())
    }

    #[test]
    fn optional_entrance_effect_validates_stored_and_playback_extents()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.location();
        let mut clips = Vec::from(CLIPS.map(|(key, frames)| (key, frames, 12)));
        clips.push((CEMETERY_EFFECT_KEY, CEMETERY_EFFECT_FRAMES, i16::MAX));
        let mut selection = AudioSelection {
            base: publish_fixture(parent, &clips)?,
            bonus_music: None,
            cemetery_music: None,
            effects_volume: MasterVolume::Full,
            handoff_music: None,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
            stage_two_music: None,
        };
        let samples = Samples::load(BundleSource::Local(parent), &selection)?;
        let clip = samples
            .entrance_effect
            .ok_or_else(|| failure("missing entrance effect"))?;
        if clip.rate() != PITCH_E40_RATE || clip.frames() != 5816 {
            return Err(failure("entrance effect playback extent mismatch"));
        }
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        let mut effect = CountedEffect::new(Some(clip), CEMETERY_EFFECT_GAIN);
        effect.observe(&mut mixer, 1)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[9989; 2]] {
            return Err(failure("entrance effect center gain mismatch"));
        }
        let _entry = clips.pop();
        clips.push((CEMETERY_EFFECT_KEY, 5816, 12));
        selection.base = publish_fixture(parent, &clips)?;
        if Samples::load(BundleSource::Local(parent), &selection).is_ok() {
            return Err(failure("invalid entrance effect format accepted"));
        }
        Ok(())
    }

    #[test]
    fn repeated_entrance_observation_restarts_only_on_new_fixed_cues()
    -> Result<(), RenderError> {
        let clip = OwnedClip::new(
            Rc::from([1000, 2000, 3000, 4000]),
            Channels::Mono,
            RATE,
        )
        .map_err(failure)?;
        let mut effect = CountedEffect::new(Some(clip), CEMETERY_EFFECT_GAIN);
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        for (count, expected) in [
            (0, 0),
            (1, 304),
            (1, 609),
            (3, 304),
            (3, 609),
            (3, 914),
            (3, 1219),
            (3, 0),
            (0, 0),
            (1, 304),
        ] {
            effect.observe(&mut mixer, count)?;
            let mut frame = [[0; 2]];
            mixer.render(&mut frame);
            if frame != [[expected; 2]] {
                return Err(failure("entrance effect retrigger mismatch"));
            }
        }
        Ok(())
    }

    #[test]
    fn optional_ambient_effect_validates_extent_rate_and_center_gain()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.location();
        let mut clips = Vec::from(CLIPS.map(|(key, frames)| (key, frames, 12)));
        clips.push((CEMETERY_AMBIENT_KEY, CEMETERY_AMBIENT_FRAMES, i16::MAX));
        let mut selection = AudioSelection {
            base: publish_fixture(parent, &clips)?,
            bonus_music: None,
            cemetery_music: None,
            effects_volume: MasterVolume::Full,
            handoff_music: None,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
            stage_two_music: None,
        };
        let samples = Samples::load(BundleSource::Local(parent), &selection)?;
        let clip = samples
            .ambient_effect
            .ok_or_else(|| failure("missing ambient effect"))?;
        if clip.rate() != PITCH_E40_RATE || clip.frames() != 1572 {
            return Err(failure("ambient effect playback extent mismatch"));
        }
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        let mut effect = CountedEffect::new(Some(clip), CEMETERY_AMBIENT_GAIN);
        effect.observe(&mut mixer, 1)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[5669; 2]] {
            return Err(failure("ambient effect center gain mismatch"));
        }
        let _entry = clips.pop();
        clips.push((CEMETERY_AMBIENT_KEY, 1572, 12));
        selection.base = publish_fixture(parent, &clips)?;
        if Samples::load(BundleSource::Local(parent), &selection).is_ok() {
            return Err(failure("invalid ambient effect extent accepted"));
        }
        Ok(())
    }

    #[test]
    fn counted_ambient_and_entrance_effects_retrigger_independently()
    -> Result<(), RenderError> {
        let clip = |samples: [i16; 3]| {
            OwnedClip::new(Rc::from(samples), Channels::Mono, RATE)
                .map_err(failure)
        };
        let mut ambient =
            CountedEffect::new(Some(clip([100, 200, 300])?), StereoGain::UNITY);
        let mut entrance = CountedEffect::new(
            Some(clip([1000, 2000, 3000])?),
            StereoGain::UNITY,
        );
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN.saturating_add(1))
                .map_err(failure)?;
        let mut frame = [[0; 2]];
        for (ambient_count, entrance_count, expected) in [
            (1, 1, 1100),
            (2, 1, 2100),
            (2, 2, 1200),
            (2, 2, 2300),
            (2, 2, 3000),
            (2, 2, 0),
        ] {
            ambient.observe(&mut mixer, ambient_count)?;
            entrance.observe(&mut mixer, entrance_count)?;
            mixer.render(&mut frame);
            if frame != [[expected; 2]] {
                return Err(failure("independent cue retrigger mismatch"));
            }
        }
        Ok(())
    }

    #[test]
    fn optional_impact_clip_is_validated_and_not_replaceable()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.location();
        let mut clips = Vec::from(CLIPS.map(|(key, frames)| (key, frames, 12)));
        clips.push((IMPACT_KEY, IMPACT_FRAMES, i16::MAX));
        let mut selection = AudioSelection {
            base: publish_fixture(parent, &clips)?,
            bonus_music: None,
            cemetery_music: None,
            effects_volume: MasterVolume::Full,
            handoff_music: None,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
            stage_two_music: None,
        };
        let samples = Samples::load(BundleSource::Local(parent), &selection)?;
        let impact = samples.impact.ok_or_else(|| failure("missing impact"))?;
        if impact.rate() != PITCH_E40_RATE {
            return Err(failure("impact clip rate did not match pitch"));
        }
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        play(&mut mixer, impact, IMPACT_GAIN)?;
        let mut output = [[0; 2]];
        mixer.render(&mut output);
        if output != [[7109, 7109]] {
            return Err(failure("impact clip center gain mismatch"));
        }
        selection.replacement =
            Some(publish_fixture(parent, &[(IMPACT_KEY, IMPACT_FRAMES, -7)])?);
        if Samples::load(BundleSource::Local(parent), &selection).is_ok() {
            return Err(failure("impact unexpectedly replaceable"));
        }
        let _impact = clips.pop();
        clips.push((IMPACT_KEY, IMPACT_FRAMES.saturating_sub(1), 12));
        selection.base = publish_fixture(parent, &clips)?;
        selection.replacement = None;
        if Samples::load(BundleSource::Local(parent), &selection).is_ok() {
            return Err(failure("invalid impact format accepted"));
        }
        Ok(())
    }

    #[test]
    fn optional_hurt_clips_preserve_character_rate_and_stereo()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let parent = root.location();
        let mut clips = Vec::from(CLIPS.map(|(key, frames)| (key, frames, 12)));
        clips.extend(HURT_CLIPS.map(|(key, frames)| (key, frames, i16::MAX)));
        let selection = AudioSelection {
            base: publish_fixture(parent, &clips)?,
            bonus_music: None,
            cemetery_music: None,
            effects_volume: MasterVolume::Full,
            handoff_music: None,
            music: None,
            music_volume: MasterVolume::Full,
            replacement: None,
            stage_two_music: None,
        };
        let samples = Samples::load(BundleSource::Local(parent), &selection)?;
        let expected =
            [[14731, 6603], [8215, 5766], [8102, 11543], [5771, 12875]];
        for ((candidate, gain), frame) in
            samples.hurt.into_iter().zip(HURT_GAINS).zip(expected)
        {
            let clip = candidate.ok_or_else(|| failure("missing hurt clip"))?;
            if clip.rate() != HURT_RATE {
                return Err(failure("hurt clip rate did not match pitch"));
            }
            let mut mixer =
                OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
            play(&mut mixer, clip, gain)?;
            let mut output = [[0; 2]];
            mixer.render(&mut output);
            if output != [frame] {
                return Err(failure("hurt clip stereo gain mismatch"));
            }
        }
        Ok(())
    }

    #[test]
    fn bonus_music_switches_once_and_ends_without_restarting()
    -> Result<(), RenderError> {
        let street = OwnedClip::new(
            Rc::from(vec![100; MUSIC_FRAMES.saturating_mul(2)]),
            Channels::Stereo,
            RATE,
        )
        .map_err(failure)?;
        let stereo = |values: [i16; 4]| {
            OwnedClip::new(Rc::from(values), Channels::Stereo, RATE)
                .map_err(failure)
        };
        let mut soundtrack = Soundtrack {
            bonus: Some([stereo([1, 2, 3, 4])?, stereo([5, 6, 7, 8])?]),
            cemetery: Some(stereo([13, 14, 15, 16])?),
            cue: MusicCue::Quiet,
            handoff: None,
            stage_two: Some(stereo([9, 10, 11, 12])?),
            street: Some(street),
        };
        let mut mixer =
            OwnedMixer::new(RATE, NonZeroU16::MIN).map_err(failure)?;
        let mut frame = [[0; 2]];
        let started = soundtrack.select(&mut mixer, MusicCue::Street)?;
        mixer.render(&mut frame);
        if !started || frame != [[100; 2]] {
            return Err(failure("street track was not admitted"));
        }
        let stopped = soundtrack.select(&mut mixer, MusicCue::Quiet)?;
        mixer.render(&mut frame);
        if !stopped || frame != [[0; 2]] || mixer.active() != 0 {
            return Err(failure("street track survived defeat"));
        }
        for (cue, expected) in [
            (MusicCue::Pumping, [[1, 2], [3, 4]]),
            (MusicCue::Results, [[5, 6], [7, 8]]),
            (MusicCue::StageTwo, [[9, 10], [11, 12]]),
            (MusicCue::Cemetery, [[13, 14], [15, 16]]),
        ] {
            let admitted = soundtrack.select(&mut mixer, cue)?;
            let mut output = [[0; 2]; 2];
            mixer.render(&mut output);
            if !admitted || output != expected || mixer.active() != 0 {
                return Err(failure("bonus cue did not play exactly once"));
            }
            let repeated = soundtrack.select(&mut mixer, cue)?;
            mixer.render(&mut frame);
            if repeated || frame != [[0; 2]] {
                return Err(failure("finished bonus cue restarted"));
            }
        }
        Ok(())
    }

    #[test]
    fn isolated_attack_clips_use_observed_stereo_ratios_and_rate()
    -> Result<(), RenderError> {
        let samples = Samples {
            ambient_effect: None,
            bonus_music: None,
            cemetery_music: None,
            clips: array::from_fn(|_| Rc::from([i16::MAX; 2])),
            effects_volume: MasterVolume::Full,
            entrance_effect: None,
            handoff_music: None,
            hurt: array::from_fn(|_| None),
            impact: None,
            music: None,
            music_volume: MasterVolume::Full,
            stage_two_music: None,
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
