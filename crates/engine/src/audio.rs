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
//   - Bounded, deterministic PCM timing, voice playback and stereo mixing.
// - Must-Not:
//   - Open native devices, decode source media or schedule title events.
// - Allows:
//   - Inputs: Borrowed or retained PCM, rates, gains and output frames.
//   - Outputs: Mixed stereo frames and explicit voice occupancy.
//   - Side effects: Mutates caller-owned output and playback positions.
// - Split-When:
//   - A distinct audio synthesis or timing contract is required.
// - Merge-When:
//   - Another runtime mixer duplicates PCM playback.
// - Summary:
//   - Allocation-free mixing after explicit bounded voice allocation.
// - Description:
//   - Convert rates with integer linear interpolation and clip final sums.
// - Usage:
//   - Bind mono or stereo clips and render into caller-provided frames.
// - Defaults:
//   - One-shot playback unless an explicit loop region is admitted. No implicit
//     replacement, device access or title policy.

//! Deterministic PCM playback independently of presentation and device rate.

use std::num::{NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize};
use std::ops::Range;
use std::ptr;
use std::rc::Rc;
use std::time::Duration;

/// Interleaved source sample layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channels {
    /// One sample per frame, duplicated to left and right.
    Mono,
    /// Two samples per frame, left followed by right.
    Stereo,
}

impl Channels {
    const fn width(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

/// A checked, nonempty, immutable PCM clip in native signed sample values.
#[derive(Clone, Copy, Debug)]
pub struct PcmClip<S> {
    channels: Channels,
    rate: NonZeroU32,
    samples: S,
}

/// A zero-copy borrowed PCM clip.
pub type Clip<'samples> = PcmClip<&'samples [i16]>;

/// A PCM clip retaining its immutable samples across host callbacks.
pub type OwnedClip = PcmClip<Rc<[i16]>>;

impl<S> PcmClip<S>
where
    S: AsRef<[i16]>,
{
    /// Number of complete source frames.
    #[must_use]
    pub fn frames(&self) -> usize {
        let samples = self.samples.as_ref();
        match self.channels {
            Channels::Mono => samples.len(),
            Channels::Stereo => samples.len() >> 1,
        }
    }

    /// Bind complete immutable interleaved samples without allocating.
    /// # Errors
    /// Rejects empty clips and partial frames.
    pub fn new(
        samples: S,
        channels: Channels,
        rate: NonZeroU32,
    ) -> Result<Self, AudioError> {
        if samples.as_ref().is_empty()
            || !samples.as_ref().len().is_multiple_of(channels.width())
        {
            return Err(AudioError::InvalidClip);
        }
        Ok(Self { channels, rate, samples })
    }

    /// Nominal source sample rate in frames per second.
    #[must_use]
    pub const fn rate(&self) -> NonZeroU32 {
        self.rate
    }

    fn stereo(&self, index: usize) -> [i16; 2] {
        let samples = self.samples.as_ref();
        let offset = index.saturating_mul(self.channels.width());
        let first = samples.get(offset).copied().unwrap_or(0);
        match self.channels {
            Channels::Mono => [first, first],
            Channels::Stereo => [
                first,
                samples.get(offset.saturating_add(1)).copied().unwrap_or(0),
            ],
        }
    }
}

/// Independent left and right gains, with 65535 representing unity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StereoGain {
    /// Left-channel gain in the inclusive range zero to unity.
    pub left: u16,
    /// Right-channel gain in the inclusive range zero to unity.
    pub right: u16,
}

impl StereoGain {
    /// Unattenuated left and right channels.
    pub const UNITY: Self = Self {
        left: u16::MAX,
        right: u16::MAX,
    };

    /// Apply an independent source level while preserving its stereo balance.
    #[must_use]
    pub fn attenuated(self, level: MasterVolume) -> Self {
        let scale = |gain| {
            u16::try_from(level.attenuate(i64::from(gain))).unwrap_or(u16::MAX)
        };
        Self {
            left: scale(self.left),
            right: scale(self.right),
        }
    }
}

/// Master output attenuation, independent of the game's individual gains.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MasterVolume {
    /// Preserve the original amplitude.
    #[default]
    Full,
    /// One half of the original amplitude.
    Half,
    /// Silence without stopping voices.
    Muted,
    /// One quarter of the original amplitude.
    Quarter,
    /// Three quarters of the original amplitude.
    ThreeQuarters,
}

impl MasterVolume {
    /// All supported levels in menu order.
    pub const ALL: [Self; 5] = [
        Self::Muted,
        Self::Quarter,
        Self::Half,
        Self::ThreeQuarters,
        Self::Full,
    ];

    const fn attenuate(self, sample: i64) -> i64 {
        match self {
            Self::Muted => 0,
            Self::Quarter => sample.saturating_div(4),
            Self::Half => sample.saturating_div(2),
            Self::ThreeQuarters => sample.saturating_mul(3).saturating_div(4),
            Self::Full => sample,
        }
    }

    /// Select the next lower level, wrapping after mute.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Full => Self::ThreeQuarters,
            Self::ThreeQuarters => Self::Half,
            Self::Half => Self::Quarter,
            Self::Quarter => Self::Muted,
            Self::Muted => Self::Full,
        }
    }

    /// Parse one of the supported percentages.
    /// # Errors
    /// Rejects percentages outside the supported menu levels.
    pub fn parse(text: &str) -> Result<Self, AudioError> {
        Self::ALL
            .into_iter()
            .find(|level| level.percent() == text)
            .ok_or(AudioError::InvalidVolume)
    }

    /// Portable, human-readable percentage.
    #[must_use]
    pub const fn percent(self) -> &'static str {
        match self {
            Self::Muted => "0",
            Self::Quarter => "25",
            Self::Half => "50",
            Self::ThreeQuarters => "75",
            Self::Full => "100",
        }
    }
}

/// Audio timing and playback admission failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    /// Unable to allocate the requested bounded voice storage.
    Allocation,
    /// The requested audio frame count exceeds the representable range.
    FrameOverflow,
    /// Empty or incomplete source frames.
    InvalidClip,
    /// The loop region is empty or extends beyond the clip.
    InvalidLoop,
    /// The master volume percentage is not a supported level.
    InvalidVolume,
    /// No free voice slot; existing playback remains unchanged.
    VoicesFull,
}

/// An independently controlled source in the shared mixer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioSource {
    /// Short or environmental sound effects.
    Effects,
    /// Music, including loops.
    Music,
}

/// Independent source levels before the final master attenuation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceVolumes {
    /// Effects source attenuation.
    pub effects: MasterVolume,
    /// Music source attenuation.
    pub music: MasterVolume,
}

impl SourceVolumes {
    const fn level(self, source: AudioSource) -> MasterVolume {
        match source {
            AudioSource::Effects => self.effects,
            AudioSource::Music => self.music,
        }
    }
}

/// A voice gain and its independently controlled source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceGain {
    /// Source level applied on top of the stereo gain.
    pub source: AudioSource,
    /// Original, unattenuated stereo gain.
    pub stereo: StereoGain,
}

impl SourceGain {
    const fn effects(stereo: StereoGain) -> Self {
        Self {
            source: AudioSource::Effects,
            stereo,
        }
    }
}

/// Source settings exposed only for admitted playback capabilities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceSettings {
    /// Current effects level, absent when no effects are available.
    pub effects: Option<MasterVolume>,
    /// Current music level, absent when no music is available.
    pub music: Option<MasterVolume>,
}

impl SourceSettings {
    /// Return the current level if the source is available.
    #[must_use]
    pub const fn get(self, source: AudioSource) -> Option<MasterVolume> {
        match source {
            AudioSource::Effects => self.effects,
            AudioSource::Music => self.music,
        }
    }

    /// Current mixer levels; absent sources retain neutral attenuation.
    #[must_use]
    pub const fn levels(self) -> SourceVolumes {
        SourceVolumes {
            effects: match self.effects {
                Some(level) => level,
                None => MasterVolume::Full,
            },
            music: match self.music {
                Some(level) => level,
                None => MasterVolume::Full,
            },
        }
    }
}

/// Convert explicit elapsed simulation time to complete audio frames.
///
/// Fractional frames carry forward exactly, so an identical elapsed
/// duration yields the same total regardless of its partition. The clock
/// never queries wall time, allocates storage or submits device audio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameClock {
    rate: NonZeroU32,
    remainder: u32,
}

impl FrameClock {
    /// Count complete frames for elapsed time, retaining the fraction.
    ///
    /// # Errors
    /// Rejects an unrepresentable frame count without changing the clock.
    pub fn advance(&mut self, elapsed: Duration) -> Result<u64, AudioError> {
        let denominator = 1_000_000_000u128;
        let scaled = elapsed
            .as_nanos()
            .checked_mul(NonZeroU128::from(self.rate).get())
            .and_then(|count| count.checked_add(u128::from(self.remainder)))
            .ok_or(AudioError::FrameOverflow)?;
        let frames = scaled
            .checked_div(denominator)
            .and_then(|count| u64::try_from(count).ok())
            .ok_or(AudioError::FrameOverflow)?;
        let remainder = scaled
            .checked_rem(denominator)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(AudioError::FrameOverflow)?;
        self.remainder = remainder;
        Ok(frames)
    }

    /// Start counting at the given nonzero output sample rate.
    #[must_use]
    pub const fn new(rate: NonZeroU32) -> Self {
        Self { rate, remainder: 0 }
    }
}

#[derive(Clone, Copy, Debug)]
struct LoopRegion {
    end: usize,
    start: usize,
}

impl LoopRegion {
    fn advance(self, frame: usize, advance: usize) -> usize {
        let remaining = self.end.saturating_sub(frame);
        if advance < remaining {
            return frame.saturating_add(advance);
        }
        let length = self.end.saturating_sub(self.start);
        let offset = advance
            .saturating_sub(remaining)
            .checked_rem(length)
            .unwrap_or(0);
        self.start.saturating_add(offset)
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice<S> {
    clip: PcmClip<S>,
    effective_gain: StereoGain,
    frame: usize,
    gain: SourceGain,
    looping: Option<LoopRegion>,
    phase: u64,
}

impl<S> Voice<S>
where
    S: AsRef<[i16]>,
{
    fn advance(&mut self, output_rate: u64) -> bool {
        let phase = self
            .phase
            .saturating_add(NonZeroU64::from(self.clip.rate()).get());
        // For source rates at or below the output rate, phase can cross at
        // most one source-frame boundary. Avoid division in that hot path.
        let (advance, remainder) = if phase < output_rate {
            (0, phase)
        } else if phase < output_rate.saturating_mul(2) {
            (1, phase.saturating_sub(output_rate))
        } else {
            (
                phase.checked_div(output_rate).unwrap_or(0),
                phase.checked_rem(output_rate).unwrap_or(0),
            )
        };
        self.phase = remainder;
        let steps = usize::try_from(advance).unwrap_or(usize::MAX);
        if let Some(region) = self.looping {
            self.frame = region.advance(self.frame, steps);
            return true;
        }
        self.frame = self.frame.saturating_add(steps);
        self.frame < self.clip.frames()
    }

    fn sample(&self, output_rate: u64) -> [i64; 2] {
        let [first_left, first_right] = self.clip.stereo(self.frame);
        let next = self.looping.map_or_else(
            || {
                self.frame
                    .saturating_add(1)
                    .min(self.clip.frames().saturating_sub(1))
            },
            |region| region.advance(self.frame, 1),
        );
        let [last_left, last_right] = self.clip.stereo(next);
        let fraction = i64::try_from(self.phase).unwrap_or(0);
        let divisor = i64::try_from(output_rate).unwrap_or(1);
        let interpolate = |first: i16, last: i16| {
            let base = i64::from(first);
            let delta = i64::from(last).saturating_sub(base);
            base.saturating_add(
                delta
                    .saturating_mul(fraction)
                    .checked_div(divisor)
                    .unwrap_or(0),
            )
        };
        let left = interpolate(first_left, last_left);
        let right = match self.clip.channels {
            Channels::Mono => left,
            Channels::Stereo => interpolate(first_right, last_right),
        };
        let scale = |sample: i64, gain: u16| {
            sample
                .saturating_mul(i64::from(gain))
                .checked_div(i64::from(u16::MAX))
                .unwrap_or(0)
        };
        [
            scale(left, self.effective_gain.left),
            scale(right, self.effective_gain.right),
        ]
    }
}

type VoiceSlot<S> = Option<Voice<S>>;

/// Stereo mixer with bounded, reusable one-shot and looping voice slots.
///
/// Rendering is independent of chunk size and allocates no memory. The
/// borrowed or owned sample storage determines the lifetime of each voice.
#[derive(Debug)]
pub struct PcmMixer<S> {
    rate: NonZeroU32,
    sources: SourceVolumes,
    voices: Vec<VoiceSlot<S>>,
    volume: MasterVolume,
}

/// Mixer using clips backed by caller-owned sample slices.
pub type Mixer<'samples> = PcmMixer<&'samples [i16]>;

/// Mixer retaining samples for the lifetime of each admitted voice.
pub type OwnedMixer = PcmMixer<Rc<[i16]>>;

impl<S> PcmMixer<S>
where
    S: AsRef<[i16]>,
{
    /// Number of currently playing voices.
    #[must_use]
    pub fn active(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// Number of currently playing voices on an independent source.
    #[must_use]
    pub fn active_on(&self, source: AudioSource) -> usize {
        self.voices
            .iter()
            .flatten()
            .filter(|voice| voice.gain.source == source)
            .count()
    }

    fn admit(
        &mut self,
        clip: PcmClip<S>,
        gain: SourceGain,
        looping: Option<LoopRegion>,
    ) -> Result<usize, AudioError> {
        let (index, slot) = self
            .voices
            .iter_mut()
            .enumerate()
            .find(|(_index, voice)| voice.is_none())
            .ok_or(AudioError::VoicesFull)?;
        *slot = Some(Voice {
            clip,
            effective_gain: gain
                .stereo
                .attenuated(self.sources.level(gain.source)),
            frame: 0,
            gain,
            looping,
            phase: 0,
        });
        Ok(index)
    }

    /// Stop all voices without releasing the allocated slots.
    pub fn clear(&mut self) {
        self.voices.fill_with(|| None);
    }

    /// Allocate a fixed number of reusable voice slots.
    /// # Errors
    /// Returns an allocation error if the requested storage cannot be reserved.
    pub fn new(
        rate: NonZeroU32,
        capacity: NonZeroU16,
    ) -> Result<Self, AudioError> {
        let mut voices = Vec::new();
        let count = NonZeroUsize::from(capacity).get();
        voices
            .try_reserve_exact(count)
            .map_err(|_error| AudioError::Allocation)?;
        voices.resize_with(count, || None);
        Ok(Self {
            rate,
            sources: SourceVolumes::default(),
            voices,
            volume: MasterVolume::default(),
        })
    }

    /// Start a clip in the first free slot and return its index.
    ///
    /// A slot can be reused once stopped or exhausted; an old index is not
    /// a persistent handle.
    /// # Errors
    /// Rejects a full mixer without replacing or advancing any voice.
    pub fn play(
        &mut self,
        clip: PcmClip<S>,
        gain: StereoGain,
    ) -> Result<usize, AudioError> {
        self.play_on(clip, SourceGain::effects(gain))
    }

    /// Play from frame zero, then repeat the specified source region.
    ///
    /// The end is exclusive. Interpolation wraps to the loop start rather
    /// than reading samples beyond the loop end. A loop remains active
    /// until explicitly stopped or cleared.
    /// # Errors
    /// Rejects empty or out-of-range regions and a full mixer.
    pub fn play_loop(
        &mut self,
        clip: PcmClip<S>,
        gain: StereoGain,
        region: Range<usize>,
    ) -> Result<usize, AudioError> {
        self.play_loop_on(clip, SourceGain::effects(gain), region)
    }

    /// Play a loop through an independently controlled source.
    /// # Errors
    /// Rejects empty or out-of-range regions and a full mixer.
    pub fn play_loop_on(
        &mut self,
        clip: PcmClip<S>,
        gain: SourceGain,
        region: Range<usize>,
    ) -> Result<usize, AudioError> {
        if region.is_empty() || region.end > clip.frames() {
            return Err(AudioError::InvalidLoop);
        }
        self.admit(
            clip,
            gain,
            Some(LoopRegion {
                end: region.end,
                start: region.start,
            }),
        )
    }

    /// Play a one-shot clip through an independently controlled source.
    /// # Errors
    /// Rejects a full mixer without replacing or advancing any voice.
    pub fn play_on(
        &mut self,
        clip: PcmClip<S>,
        gain: SourceGain,
    ) -> Result<usize, AudioError> {
        self.admit(clip, gain, None)
    }

    /// Device-independent output rate in stereo frames per second.
    #[must_use]
    pub const fn rate(&self) -> NonZeroU32 {
        self.rate
    }

    /// Overwrite complete stereo frames, producing silence when idle.
    ///
    /// Source voices advance by rational rate ratios. Final sums saturate
    /// only after all active voices contribute.
    pub fn render(&mut self, output: &mut [[i16; 2]]) {
        let rate = NonZeroU64::from(self.rate).get();
        for frame in output {
            *frame = mix_frame(&mut self.voices, rate, self.volume);
        }
    }

    /// Restart an already playing one-shot effect backed by this allocation.
    ///
    /// Other allocations, music and looping voices retain their progress.
    /// If the effect has expired, admit it in the first free slot instead.
    /// The result is a transient slot index, not a persistent voice handle.
    /// # Errors
    /// Rejects a full mixer if no matching one-shot effect is active.
    pub fn retrigger_effect(
        &mut self,
        clip: PcmClip<S>,
        gain: StereoGain,
    ) -> Result<usize, AudioError> {
        let effective_gain = gain.attenuated(self.sources.effects);
        let matching =
            self.voices
                .iter_mut()
                .enumerate()
                .find_map(|(index, slot)| {
                    let voice = slot.as_mut()?;
                    (voice.gain.source == AudioSource::Effects
                        && voice.looping.is_none()
                        && voice.clip.channels == clip.channels
                        && voice.clip.rate == clip.rate
                        && ptr::eq(
                            voice.clip.samples.as_ref(),
                            clip.samples.as_ref(),
                        ))
                    .then_some((index, voice))
                });
        if let Some((index, voice)) = matching {
            voice.frame = 0;
            voice.phase = 0;
            voice.gain = SourceGain::effects(gain);
            voice.effective_gain = effective_gain;
            return Ok(index);
        }
        self.play(clip, gain)
    }

    /// Change source attenuation for active and future voices.
    pub fn set_source_volumes(&mut self, sources: SourceVolumes) {
        if self.sources == sources {
            return;
        }
        self.sources = sources;
        for voice in self.voices.iter_mut().flatten() {
            voice.effective_gain = voice
                .gain
                .stereo
                .attenuated(sources.level(voice.gain.source));
        }
    }

    /// Set the master attenuation without interrupting active voices.
    pub const fn set_volume(&mut self, volume: MasterVolume) {
        self.volume = volume;
    }

    /// Stop a currently occupied slot without changing other voices.
    pub fn stop(&mut self, index: usize) -> bool {
        self.voices
            .get_mut(index)
            .is_some_and(|slot| slot.take().is_some())
    }

    /// Stop every voice on a source, preserving other voices and positions.
    pub fn stop_source(&mut self, source: AudioSource) {
        for slot in &mut self.voices {
            if slot
                .as_ref()
                .is_some_and(|voice| voice.gain.source == source)
            {
                *slot = None;
            }
        }
    }
}

fn mix_frame<S>(
    voices: &mut [VoiceSlot<S>],
    rate: u64,
    volume: MasterVolume,
) -> [i16; 2]
where
    S: AsRef<[i16]>,
{
    let mut sum = [0i64; 2];
    for slot in voices {
        let Some(voice) = slot else {
            continue;
        };
        for (channel, sample) in sum.iter_mut().zip(voice.sample(rate)) {
            *channel = channel.saturating_add(sample);
        }
        if !voice.advance(rate) {
            *slot = None;
        }
    }
    sum.map(|sample| {
        let attenuated = volume.attenuate(sample);
        i16::try_from(
            attenuated.clamp(i64::from(i16::MIN), i64::from(i16::MAX)),
        )
        .unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_clock_preserves_tick_fraction() -> Result<(), AudioError> {
        let interval = Duration::from_micros(16_896);
        let mut partitioned = FrameClock::new(rate(48_000));
        let mut whole = partitioned;
        for _ in 0u8..124u8 {
            if partitioned.advance(interval)? != 811 {
                return Err(AudioError::FrameOverflow);
            }
        }
        (partitioned.advance(interval)? == 812
            && whole.advance(interval.saturating_mul(125))? == 101_376
            && whole == partitioned
            && partitioned.advance(Duration::ZERO)? == 0
            && partitioned.advance(interval)? == 811)
            .then_some(())
            .ok_or(AudioError::FrameOverflow)
    }

    #[test]
    fn frame_clock_overflow_preserves_fraction() -> Result<(), AudioError> {
        let mut clock = FrameClock::new(rate(2));
        let first = clock.advance(Duration::from_millis(250))?;
        let before = clock;
        (first == 0
            && clock.advance(Duration::MAX) == Err(AudioError::FrameOverflow)
            && clock == before
            && clock.advance(Duration::from_millis(250))? == 1
            && FrameClock::new(rate(1)).advance(Duration::MAX)? == u64::MAX)
            .then_some(())
            .ok_or(AudioError::FrameOverflow)
    }

    fn rate(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    #[test]
    fn source_attenuation_preserves_independent_mix_and_timing()
    -> Result<(), AudioError> {
        let music =
            Clip::new(&[8000, -8000, 4000, -4000], Channels::Stereo, rate(1))?;
        let effect = Clip::new(&[4000, 8000], Channels::Mono, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN.saturating_add(1))?;
        let _track = mixer.play_loop(
            music,
            StereoGain::UNITY.attenuated(MasterVolume::Half),
            0..2,
        )?;
        let _cue = mixer.play(
            effect,
            StereoGain {
                left: 32768,
                right: 16384,
            }
            .attenuated(MasterVolume::Quarter),
        )?;
        let mut frames = [[0; 2]; 2];
        mixer.render(&mut frames);
        if frames != [[4499, -3749], [2999, -1499]] || mixer.active() != 1 {
            return Err(AudioError::InvalidVolume);
        }
        let _muted = mixer
            .play(effect, StereoGain::UNITY.attenuated(MasterVolume::Muted))?;
        mixer.render(&mut frames);
        (frames == [[3999, -3999], [1999, -1999]] && mixer.active() == 1)
            .then_some(())
            .ok_or(AudioError::InvalidVolume)
    }

    #[test]
    fn source_levels_change_active_voices_without_restarting_them()
    -> Result<(), AudioError> {
        let music = Clip::new(
            &[1000, -1000, 2000, -2000, 3000, -3000],
            Channels::Stereo,
            rate(1),
        )?;
        let effect =
            Clip::new(&[4000, 8000, 12000, 16000], Channels::Mono, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN.saturating_add(1))?;
        let _music = mixer.play_loop_on(
            music,
            SourceGain {
                source: AudioSource::Music,
                stereo: StereoGain::UNITY,
            },
            0..3,
        )?;
        let _effect = mixer.play(effect, StereoGain::UNITY)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[5000, 3000]] {
            return Err(AudioError::InvalidVolume);
        }
        mixer.set_source_volumes(SourceVolumes {
            effects: MasterVolume::Half,
            music: MasterVolume::Muted,
        });
        mixer.render(&mut frame);
        if frame != [[3999; 2]] || mixer.active() != 2 {
            return Err(AudioError::InvalidVolume);
        }
        mixer.set_source_volumes(SourceVolumes {
            effects: MasterVolume::Muted,
            music: MasterVolume::Full,
        });
        mixer.render(&mut frame);
        if frame != [[3000, -3000]] {
            return Err(AudioError::InvalidVolume);
        }
        mixer.render(&mut frame);
        if frame != [[1000, -1000]] || mixer.active() != 1 {
            return Err(AudioError::InvalidVolume);
        }
        let _muted = mixer.play(effect, StereoGain::UNITY)?;
        mixer.set_volume(MasterVolume::Half);
        mixer.render(&mut frame);
        (frame == [[1000, -1000]] && mixer.active() == 2)
            .then_some(())
            .ok_or(AudioError::InvalidVolume)
    }

    #[test]
    fn source_switch_preserves_unrelated_one_shot_progress()
    -> Result<(), AudioError> {
        let music = Clip::new(&[1000, -1000], Channels::Stereo, rate(1))?;
        let effect = Clip::new(&[4000, 8000, 12000], Channels::Mono, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN.saturating_add(1))?;
        let gain = SourceGain {
            source: AudioSource::Music,
            stereo: StereoGain::UNITY,
        };
        let _music = mixer.play_loop_on(music, gain, 0..1)?;
        let _effect = mixer.play(effect, StereoGain::UNITY)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[5000, 3000]] {
            return Err(AudioError::InvalidVolume);
        }
        mixer.stop_source(AudioSource::Music);
        if mixer.active_on(AudioSource::Music) != 0
            || mixer.active_on(AudioSource::Effects) != 1
        {
            return Err(AudioError::InvalidVolume);
        }
        let _next = mixer.play_on(music, gain)?;
        mixer.render(&mut frame);
        if frame != [[9000, 7000]] || mixer.active_on(AudioSource::Music) != 0 {
            return Err(AudioError::InvalidVolume);
        }
        mixer.stop_source(AudioSource::Music);
        mixer.render(&mut frame);
        (frame == [[12000; 2]] && mixer.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidVolume)
    }

    #[test]
    fn owned_effect_retrigger_restarts_only_the_matching_one_shot()
    -> Result<(), AudioError> {
        let cue =
            OwnedClip::new(Rc::from([1000, 2000]), Channels::Mono, rate(1))?;
        let other =
            OwnedClip::new(Rc::from([1000, 2000]), Channels::Mono, rate(1))?;
        let mut mixer =
            OwnedMixer::new(rate(1), NonZeroU16::MIN.saturating_add(2))?;
        let _music = mixer.play_on(cue.clone(), SourceGain {
            source: AudioSource::Music,
            stereo: StereoGain::UNITY,
        })?;
        let _other = mixer.play(other, StereoGain::UNITY)?;
        let voice = mixer.play(cue.clone(), StereoGain::UNITY)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[3000; 2]]
            || mixer.retrigger_effect(cue.clone(), StereoGain::UNITY)? != voice
            || mixer.active() != 3
        {
            return Err(AudioError::InvalidClip);
        }
        mixer.render(&mut frame);
        if frame != [[5000; 2]] || mixer.active() != 1 {
            return Err(AudioError::InvalidClip);
        }
        mixer.render(&mut frame);
        if frame != [[2000; 2]] || mixer.active() != 0 {
            return Err(AudioError::InvalidClip);
        }
        let _reused = mixer.retrigger_effect(cue, StereoGain::UNITY)?;
        mixer.render(&mut frame);
        (frame == [[1000; 2]] && mixer.active() == 1)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn master_volume_preserves_timing_and_scales_before_clipping()
    -> Result<(), AudioError> {
        let clip = Clip::new(
            &[20_000, -20_000, 24_000, -24_000],
            Channels::Mono,
            rate(1),
        )?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN.saturating_add(1))?;
        let _first = mixer.play(clip, StereoGain::UNITY)?;
        let _second = mixer.play(clip, StereoGain::UNITY)?;
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        if frame != [[i16::MAX; 2]] {
            return Err(AudioError::InvalidVolume);
        }
        mixer.set_volume(MasterVolume::Half);
        mixer.render(&mut frame);
        if frame != [[-20_000; 2]] {
            return Err(AudioError::InvalidVolume);
        }
        mixer.set_volume(MasterVolume::Muted);
        mixer.render(&mut frame);
        if frame != [[0; 2]] || mixer.active() != 2 {
            return Err(AudioError::InvalidVolume);
        }
        mixer.set_volume(MasterVolume::Quarter);
        mixer.render(&mut frame);
        (frame == [[-12_000; 2]] && mixer.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidVolume)
    }

    #[test]
    fn volume_levels_roundtrip_and_cycle() -> Result<(), AudioError> {
        let mut level = MasterVolume::default();
        for expected in [
            MasterVolume::Full,
            MasterVolume::ThreeQuarters,
            MasterVolume::Half,
            MasterVolume::Quarter,
            MasterVolume::Muted,
        ] {
            if level != expected
                || MasterVolume::parse(level.percent())? != level
            {
                return Err(AudioError::InvalidVolume);
            }
            level = level.next();
        }
        (level == MasterVolume::Full
            && MasterVolume::parse("101").is_err()
            && MasterVolume::parse("25.5").is_err())
        .then_some(())
        .ok_or(AudioError::InvalidVolume)
    }

    #[test]
    fn clip_rejects_empty_and_partial_frames() {
        assert!(Clip::new(&[], Channels::Mono, rate(1)).is_err());
        assert!(Clip::new(&[1], Channels::Stereo, rate(1)).is_err());
    }

    #[test]
    fn clearing_voices_discards_audio_and_reuses_slots()
    -> Result<(), AudioError> {
        let clip = Clip::new(&[1, 2, 3], Channels::Mono, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN)?;
        let slot = mixer.play(clip, StereoGain::UNITY)?;
        mixer.clear();
        let mut frame = [[1; 2]];
        mixer.render(&mut frame);
        (mixer.active() == 0
            && frame == [[0, 0]]
            && mixer.play(clip, StereoGain::UNITY)? == slot)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn mixing_saturates_after_all_voices() -> Result<(), AudioError> {
        let loud = Clip::new(&[30_000], Channels::Mono, rate(1))?;
        let quiet = Clip::new(&[-30_000, 30_000], Channels::Stereo, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN.saturating_add(2))?;
        let _first = mixer.play(loud, StereoGain::UNITY)?;
        let _second = mixer.play(loud, StereoGain::UNITY)?;
        let _third = mixer.play(quiet, StereoGain::UNITY)?;
        let mut frames = [[0; 2]; 2];
        mixer.render(&mut frames);
        (frames == [[30_000, i16::MAX], [0, 0]] && mixer.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn rational_playback_is_independent_of_render_chunks()
    -> Result<(), AudioError> {
        let clip = Clip::new(&[0, 12_000, -12_000], Channels::Mono, rate(2))?;
        let mut whole = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let mut split = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let gain = StereoGain { left: u16::MAX, right: 0 };
        let _whole_slot = whole.play(clip, gain)?;
        let _split_slot = split.play(clip, gain)?;
        let mut joined = [[0; 2]; 6];
        let mut first = [[0; 2]; 2];
        let mut last = [[0; 2]; 4];
        whole.render(&mut joined);
        split.render(&mut first);
        split.render(&mut last);
        (joined
            == [
                [0, 0],
                [8_000, 0],
                [4_000, 0],
                [-12_000, 0],
                [-12_000, 0],
                [0, 0],
            ]
            && joined.into_iter().eq(first.into_iter().chain(last))
            && split.active() == 0
            && whole.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    fn ratio_trace(
        source: u32,
        output: u32,
        expected: &[i16; 8],
        remaining: usize,
    ) -> Result<(), AudioError> {
        let source_samples = [0, 12_000, -12_000, 6_000, -6_000, 1_200];
        let clip = Clip::new(&source_samples, Channels::Mono, rate(source))?;
        let mut mixer = Mixer::new(rate(output), NonZeroU16::MIN)?;
        let _slot = mixer.play(clip, StereoGain::UNITY)?;
        let mut rendered = [[0; 2]; 8];
        mixer.render(&mut rendered);
        (rendered == expected.map(|sample| [sample; 2])
            && mixer.active() == remaining)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn rational_rate_boundaries_and_skipped_source_frames()
    -> Result<(), AudioError> {
        ratio_trace(5, 3, &[0, -4_000, 2_000, 1_200, 0, 0, 0, 0], 0)?;
        ratio_trace(8, 3, &[0, 0, 1_200, 0, 0, 0, 0, 0], 0)?;
        ratio_trace(u32::MAX, 1, &[0; 8], 0)?;
        ratio_trace(1, u32::MAX, &[0; 8], 1)?;
        let direct = [0, 12_000, -12_000, 6_000, -6_000, 1_200, 0, 0];
        ratio_trace(u32::MAX, u32::MAX, &direct, 0)?;
        ratio_trace(u32::MAX, u32::MAX.saturating_sub(1), &direct, 0)
    }

    #[test]
    fn mono_and_stereo_interpolation_preserve_independent_gains()
    -> Result<(), AudioError> {
        let mono = Clip::new(&[12_000, -12_000], Channels::Mono, rate(2))?;
        let stereo = Clip::new(
            &[12_000, -12_000, -12_000, 12_000],
            Channels::Stereo,
            rate(2),
        )?;
        let gain = StereoGain {
            left: 32_767,
            right: u16::MAX,
        };
        let mut mixer = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let _mono = mixer.play(mono, gain)?;
        let mut mono_frames = [[0; 2]; 4];
        mixer.render(&mut mono_frames);
        let expected_mono =
            [[5_999, 12_000], [-1_999, -4_000], [-5_999, -12_000], [0, 0]];
        let _stereo = mixer.play(stereo, gain)?;
        let mut stereo_frames = [[0; 2]; 4];
        mixer.render(&mut stereo_frames);
        let expected_stereo =
            [[5_999, -12_000], [-1_999, 4_000], [-5_999, 12_000], [0, 0]];
        (mono_frames == expected_mono
            && stereo_frames == expected_stereo
            && mixer.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn voice_admission_stop_and_reuse_are_explicit() -> Result<(), AudioError> {
        let clip = Clip::new(&[1, 2, 3], Channels::Mono, rate(1))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN)?;
        let slot = mixer.play(clip, StereoGain::UNITY)?;
        let rejected = mixer.play(clip, StereoGain::UNITY);
        let mut frame = [[0; 2]];
        mixer.render(&mut frame);
        (rejected == Err(AudioError::VoicesFull)
            && frame == [[1, 1]]
            && mixer.stop(slot)
            && !mixer.stop(slot)
            && mixer.play(clip, StereoGain::UNITY)? == slot)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }

    #[test]
    fn loop_preserves_intro_fraction_and_chunk_partition()
    -> Result<(), AudioError> {
        let clip =
            Clip::new(&[0, 12_000, -12_000, 30_000], Channels::Mono, rate(2))?;
        let mut whole = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let mut split = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let slot = whole.play_loop(clip, StereoGain::UNITY, 1..3)?;
        let _slot = split.play_loop(clip, StereoGain::UNITY, 1..3)?;
        let mut expected = [[0; 2]; 10];
        let mut first = [[0; 2]; 3];
        let mut last = [[0; 2]; 7];
        whole.render(&mut expected);
        split.render(&mut first);
        split.render(&mut last);
        let samples = [
            0, 8_000, 4_000, -12_000, 4_000, 4_000, -12_000, 4_000, 4_000,
            -12_000,
        ];
        (expected == samples.map(|sample| [sample; 2])
            && expected.into_iter().eq(first.into_iter().chain(last))
            && whole.active() == 1
            && split.active() == 1
            && whole.stop(slot)
            && whole.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidLoop)
    }

    #[test]
    fn loop_admission_and_multi_wrap_downsampling() -> Result<(), AudioError> {
        let clip = Clip::new(&[1, 2, 3, 4], Channels::Mono, rate(11))?;
        let mut mixer = Mixer::new(rate(1), NonZeroU16::MIN)?;
        for (start, end) in [(0, 0), (2, 1), (2, 2), (0, 5)] {
            if mixer.play_loop(clip, StereoGain::UNITY, start..end)
                != Err(AudioError::InvalidLoop)
            {
                return Err(AudioError::InvalidLoop);
            }
        }
        let slot = mixer.play_loop(clip, StereoGain::UNITY, 1..3)?;
        let mut output = [[0; 2]; 5];
        mixer.render(&mut output);
        if output != [[1; 2], [2; 2], [3; 2], [2; 2], [3; 2]]
            || mixer.play(clip, StereoGain::UNITY)
                != Err(AudioError::VoicesFull)
            || !mixer.stop(slot)
        {
            return Err(AudioError::InvalidLoop);
        }
        let _single = mixer.play_loop(clip, StereoGain::UNITY, 2..3)?;
        mixer.render(&mut output);
        mixer.clear();
        (output == [[1; 2], [3; 2], [3; 2], [3; 2], [3; 2]]
            && mixer.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidLoop)
    }

    #[test]
    fn retained_samples_survive_the_admitting_scope() -> Result<(), AudioError>
    {
        let mut owned = OwnedMixer::new(rate(3), NonZeroU16::MIN)?;
        {
            let samples = Rc::<[i16]>::from([0, 12_000, -12_000]);
            let clip =
                OwnedClip::new(Rc::clone(&samples), Channels::Mono, rate(2))?;
            let _slot = owned.play(clip, StereoGain::UNITY)?;
        }
        let mut borrowed = Mixer::new(rate(3), NonZeroU16::MIN)?;
        let clip = Clip::new(&[0, 12_000, -12_000], Channels::Mono, rate(2))?;
        let _slot = borrowed.play(clip, StereoGain::UNITY)?;
        let mut expected = [[0; 2]; 6];
        borrowed.render(&mut expected);
        let mut first = [[0; 2]; 2];
        let mut last = [[0; 2]; 4];
        owned.render(&mut first);
        owned.render(&mut last);
        (expected.into_iter().eq(first.into_iter().chain(last))
            && owned.active() == 0
            && borrowed.active() == 0)
            .then_some(())
            .ok_or(AudioError::InvalidClip)
    }
}
