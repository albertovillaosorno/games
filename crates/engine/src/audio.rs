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
//   - Inputs: Borrowed signed PCM samples, rates, gains and output frames.
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
//   - No implicit replacement, looping, device access or title policy.

//! Deterministic PCM playback independently of presentation and device rate.

use std::num::{NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize};
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
pub struct Clip<'samples> {
    channels: Channels,
    rate: NonZeroU32,
    samples: &'samples [i16],
}

impl<'samples> Clip<'samples> {
    /// Number of complete source frames.
    #[must_use]
    pub const fn frames(self) -> usize {
        match self.channels {
            Channels::Mono => self.samples.len(),
            Channels::Stereo => self.samples.len() >> 1,
        }
    }

    /// Bind complete interleaved samples without allocating.
    /// # Errors
    /// Rejects empty clips and partial frames.
    pub const fn new(
        samples: &'samples [i16],
        channels: Channels,
        rate: NonZeroU32,
    ) -> Result<Self, AudioError> {
        if samples.is_empty() || !samples.len().is_multiple_of(channels.width())
        {
            return Err(AudioError::InvalidClip);
        }
        Ok(Self { channels, rate, samples })
    }

    /// Nominal source sample rate in frames per second.
    #[must_use]
    pub const fn rate(self) -> NonZeroU32 {
        self.rate
    }

    fn stereo(self, index: usize) -> [i16; 2] {
        let offset = index.saturating_mul(self.channels.width());
        let first = self.samples.get(offset).copied().unwrap_or(0);
        match self.channels {
            Channels::Mono => [first, first],
            Channels::Stereo => [
                first,
                self.samples
                    .get(offset.saturating_add(1))
                    .copied()
                    .unwrap_or(0),
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
    /// No free voice slot; existing playback remains unchanged.
    VoicesFull,
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
struct Voice<'samples> {
    clip: Clip<'samples>,
    frame: usize,
    gain: StereoGain,
    phase: u64,
}

impl Voice<'_> {
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
        self.frame = self
            .frame
            .saturating_add(usize::try_from(advance).unwrap_or(usize::MAX));
        self.frame < self.clip.frames()
    }

    fn sample(self, output_rate: u64) -> [i64; 2] {
        let [first_left, first_right] = self.clip.stereo(self.frame);
        let [last_left, last_right] = self.clip.stereo(
            self.frame
                .saturating_add(1)
                .min(self.clip.frames().saturating_sub(1)),
        );
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
        [scale(left, self.gain.left), scale(right, self.gain.right)]
    }
}

/// One-shot stereo mixer with bounded, reusable voice slots.
///
/// Rendering is independent of chunk size and allocates no memory. The
/// caller owns all borrowed clip samples for the mixer's lifetime.
#[derive(Debug)]
pub struct Mixer<'samples> {
    rate: NonZeroU32,
    voices: Vec<Option<Voice<'samples>>>,
}

impl<'samples> Mixer<'samples> {
    /// Number of currently playing voices.
    #[must_use]
    pub fn active(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// Stop all voices without releasing the allocated slots.
    pub fn clear(&mut self) {
        self.voices.fill(None);
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
        voices.resize(count, None);
        Ok(Self { rate, voices })
    }

    /// Start a clip in the first free slot and return its index.
    ///
    /// A slot can be reused once stopped or exhausted; an old index is not
    /// a persistent handle.
    /// # Errors
    /// Rejects a full mixer without replacing or advancing any voice.
    pub fn play(
        &mut self,
        clip: Clip<'samples>,
        gain: StereoGain,
    ) -> Result<usize, AudioError> {
        let (index, slot) = self
            .voices
            .iter_mut()
            .enumerate()
            .find(|(_index, voice)| voice.is_none())
            .ok_or(AudioError::VoicesFull)?;
        *slot = Some(Voice {
            clip,
            frame: 0,
            gain,
            phase: 0,
        });
        Ok(index)
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
            *frame = mix_frame(&mut self.voices, rate);
        }
    }

    /// Stop a currently occupied slot without changing other voices.
    pub fn stop(&mut self, index: usize) -> bool {
        self.voices
            .get_mut(index)
            .is_some_and(|slot| slot.take().is_some())
    }
}

fn mix_frame(voices: &mut [Option<Voice<'_>>], rate: u64) -> [i16; 2] {
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
        i16::try_from(sample.clamp(i64::from(i16::MIN), i64::from(i16::MAX)))
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
}
