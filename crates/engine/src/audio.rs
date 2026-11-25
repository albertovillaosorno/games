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
//   - Bounded, deterministic PCM voice playback and stereo mixing.
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

use std::num::{NonZeroU16, NonZeroU32, NonZeroU64, NonZeroUsize};

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

/// Playback admission failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    /// Unable to allocate the requested bounded voice storage.
    Allocation,
    /// Empty or incomplete source frames.
    InvalidClip,
    /// No free voice slot; existing playback remains unchanged.
    VoicesFull,
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
        let advance = phase.checked_div(output_rate).unwrap_or(0);
        self.phase = phase.checked_rem(output_rate).unwrap_or(0);
        self.frame = self
            .frame
            .saturating_add(usize::try_from(advance).unwrap_or(usize::MAX));
        self.frame < self.clip.frames()
    }

    fn sample(self, output_rate: u64) -> [i64; 2] {
        let current = self.clip.stereo(self.frame);
        let next = self.clip.stereo(
            self.frame
                .saturating_add(1)
                .min(self.clip.frames().saturating_sub(1)),
        );
        let fraction = i64::try_from(self.phase).unwrap_or(0);
        let divisor = i64::try_from(output_rate).unwrap_or(1);
        let mut scaled = [0i64; 2];
        for (((output, first), last), gain) in scaled
            .iter_mut()
            .zip(current)
            .zip(next)
            .zip([self.gain.left, self.gain.right])
        {
            let base = i64::from(first);
            let delta = i64::from(last).saturating_sub(base);
            let sample = base.saturating_add(
                delta
                    .saturating_mul(fraction)
                    .checked_div(divisor)
                    .unwrap_or(0),
            );
            *output = sample
                .saturating_mul(i64::from(gain))
                .checked_div(i64::from(u16::MAX))
                .unwrap_or(0);
        }
        scaled
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

    fn rate(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    #[test]
    fn clip_rejects_empty_and_partial_frames() {
        assert!(Clip::new(&[], Channels::Mono, rate(1)).is_err());
        assert!(Clip::new(&[1], Channels::Stereo, rate(1)).is_err());
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
