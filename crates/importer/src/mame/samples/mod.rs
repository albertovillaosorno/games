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
//   - Verified sound ROM assembly and bounded PCM sample extraction.
// - Must-Not:
//   - Execute program ROMs, schedule voices or synthesize music.
// - Allows:
//   - Inputs: Verified archive, sample member layout and clip descriptors.
//   - Outputs: Signed PCM samples and source-reference comparison counts.
//   - Side effects: Bounded allocation only.
// - Split-When:
//   - A distinct sample codec needs an independent implementation.
// - Merge-When:
//   - Another source adapter uses the same sample encoding and addressing.
// - Summary:
//   - Decode non-looping source sound samples without original execution.
// - Description:
//   - Preserve byte order, preincrement addressing and signed amplitudes.
// - Usage:
//   - Select a verified machine, then extract a bounded source clip.
// - Defaults:
//   - Reject missing members and unsupported observed playback modes.

//! Bounded Konami signed PCM and differential PCM sample conversion.

pub(super) mod catalogue;

use games_assets::archive::ArchiveBudget;

use super::allocated_bytes;
use crate::{ImportError, SourceSnapshot};

const RECORD_BYTES: usize = 24;
const VOICES: usize = 4;

/// A supported encoding in the selected machine's sample region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    /// Four-bit differential samples, low nibble before high nibble.
    Differential4,
    /// Signed eight-bit linear PCM.
    Pcm8,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Member {
    pub(super) bytes: usize,
    pub(super) name: &'static str,
}

/// A verified, assembled sample-only ROM region.
#[derive(Debug)]
pub struct SampleRom {
    bytes: Vec<u8>,
}

impl SampleRom {
    /// Borrow a forward, non-looping clip after the programmed start byte.
    /// The length is the number of encoded bytes, not output sample frames.
    /// # Errors
    /// Rejects empty clips and addresses outside the verified region.
    pub fn clip(
        &self,
        start: u32,
        length: u16,
        encoding: Encoding,
    ) -> Result<Clip<'_>, ImportError> {
        let first = usize::try_from(start)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(ImportError::UnsupportedMember)?;
        let end = first
            .checked_add(usize::from(length))
            .ok_or(ImportError::UnsupportedMember)?;
        let bytes = self
            .bytes
            .get(first..end)
            .filter(|bytes| !bytes.is_empty())
            .ok_or(ImportError::UnsupportedMember)?;
        Ok(Clip { bytes, encoding })
    }

    pub(super) fn load(
        source: &SourceSnapshot<'_>,
        budget: ArchiveBudget,
        members: &[Member],
    ) -> Result<Self, ImportError> {
        let size = members.iter().try_fold(0usize, |total, member| {
            total.checked_add(member.bytes).ok_or(ImportError::TooLarge)
        })?;
        let mut bytes = allocated_bytes(size)?;
        let mut found = vec![false; members.len()];
        source.visit_flat_zip(budget, |name, payload| {
            collect_member(&mut bytes, &mut found, members, (name, payload))
        })?;
        if size == 0 || found.iter().any(|seen| !seen) {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(Self { bytes })
    }
}

/// A bounded encoded clip. No playback rate or looping policy is implied.
#[derive(Clone, Copy, Debug)]
pub struct Clip<'source> {
    bytes: &'source [u8],
    encoding: Encoding,
}

impl<'source> Clip<'source> {
    fn observed_sample(self, position: u32) -> Option<i16> {
        let origin = match self.encoding {
            Encoding::Pcm8 => 1,
            Encoding::Differential4 => 2,
        };
        let Some(relative) = position.checked_sub(origin) else {
            return Some(0);
        };
        let index = usize::try_from(relative).ok()?;
        match self.encoding {
            Encoding::Pcm8 => self.bytes.get(index).map(|byte| {
                i16::from(i8::from_le_bytes([*byte])).saturating_mul(256)
            }),
            Encoding::Differential4 => self.samples().nth(index),
        }
    }

    /// Decode the clip in playback order to signed sixteen-bit PCM.
    /// Each original signed eight-bit value is scaled by 256.
    pub fn samples(self) -> impl Iterator<Item = i16> + 'source {
        let encoding = self.encoding;
        self.bytes
            .iter()
            .copied()
            .flat_map(move |byte| match encoding {
                Encoding::Pcm8 => [Some(i8::from_le_bytes([byte])), None],
                Encoding::Differential4 => [
                    Some(differential_step(byte & 0x0f)),
                    Some(differential_step(byte >> 4)),
                ],
            })
            .flatten()
            .scan(0i8, move |level, value| {
                *level = match encoding {
                    Encoding::Pcm8 => value,
                    Encoding::Differential4 => level.wrapping_add(value),
                };
                Some(i16::from(*level).saturating_mul(256))
            })
    }
}

/// Counts of compared active observations, separated by sample encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationReport {
    /// Active differential PCM observations.
    pub differential: usize,
    /// Complete four-voice frames in the input.
    pub frames: usize,
    /// Observed active samples with a different decoded amplitude.
    pub mismatches: usize,
    /// Active linear PCM observations.
    pub pcm: usize,
}

#[derive(Clone, Copy, Debug)]
struct Observation {
    encoding: Encoding,
    frame: u32,
    length: u16,
    looping: bool,
    output: i8,
    playing: bool,
    position: u32,
    reverse: bool,
    start: u32,
    voice: u8,
}

impl Observation {
    fn compare(
        self,
        rom: &SampleRom,
        report: &mut ObservationReport,
    ) -> Result<(), ImportError> {
        if !self.playing {
            return Ok(());
        }
        if self.looping || self.reverse {
            return Err(ImportError::UnsupportedMember);
        }
        let clip = rom.clip(self.start, self.length, self.encoding)?;
        let actual = clip
            .observed_sample(self.position)
            .ok_or(ImportError::UnsupportedMember)?;
        match self.encoding {
            Encoding::Pcm8 => report.pcm = report.pcm.saturating_add(1),
            Encoding::Differential4 => {
                report.differential = report.differential.saturating_add(1);
            },
        }
        if actual != i16::from(self.output).saturating_mul(256) {
            report.mismatches = report.mismatches.saturating_add(1);
        }
        Ok(())
    }

    const fn parse(record: &[u8; RECORD_BYTES]) -> Result<Self, ImportError> {
        let [frame, start, position, settings, state, flags] =
            record.as_chunks::<4>().0
        else {
            return Err(ImportError::UnsupportedMember);
        };
        let [low, high, _pitch_low, pitch_high] = *settings;
        let [voice, output, playing, reverse] = *state;
        let [looping, differential, volume, pan] = *flags;
        if voice >= 4
            || playing > 1
            || reverse > 1
            || looping > 1
            || differential > 1
            || pitch_high > 15
            || volume > 127
            || pan > 7
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(Self {
            encoding: if differential == 0 {
                Encoding::Pcm8
            } else {
                Encoding::Differential4
            },
            frame: u32::from_le_bytes(*frame),
            length: u16::from_le_bytes([low, high]),
            looping: looping != 0,
            output: i8::from_le_bytes([output]),
            playing: playing != 0,
            position: u32::from_le_bytes(*position),
            reverse: reverse != 0,
            start: u32::from_le_bytes(*start),
            voice,
        })
    }
}

fn collect_member(
    bytes: &mut [u8],
    found: &mut [bool],
    members: &[Member],
    entry: (&str, &[u8]),
) -> Result<(), ImportError> {
    let (name, payload) = entry;
    let mut offset = 0usize;
    for (member, seen) in members.iter().zip(found) {
        let end = offset
            .checked_add(member.bytes)
            .ok_or(ImportError::TooLarge)?;
        if member.name != name {
            offset = end;
            continue;
        }
        if *seen || payload.len() != member.bytes {
            return Err(ImportError::UnsupportedMember);
        }
        bytes
            .get_mut(offset..end)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(payload);
        *seen = true;
        break;
    }
    Ok(())
}

/// Compare complete four-voice frames from a local observation bridge.
/// # Errors
/// Rejects malformed records, missing or reordered voices, nonconsecutive
/// frames, unsupported loop/reverse modes and invalid active clip extents.
pub fn compare_observations(
    rom: &SampleRom,
    raw: &[u8],
) -> Result<ObservationReport, ImportError> {
    let (records, remainder) = raw.as_chunks::<RECORD_BYTES>();
    let (frames, partial) = records.as_chunks::<VOICES>();
    if frames.is_empty() || !partial.is_empty() || !remainder.is_empty() {
        return Err(ImportError::UnsupportedMember);
    }
    let mut report = ObservationReport {
        differential: 0,
        frames: frames.len(),
        mismatches: 0,
        pcm: 0,
    };
    for (index, frame) in frames.iter().enumerate() {
        let expected =
            u32::try_from(index).map_err(|_error| ImportError::TooLarge)?;
        for (voice, record) in frame.iter().enumerate() {
            let observed = Observation::parse(record)?;
            if observed.frame != expected
                || usize::from(observed.voice) != voice
            {
                return Err(ImportError::UnsupportedMember);
            }
            observed.compare(rom, &mut report)?;
        }
    }
    Ok(report)
}

// K053260 differential steps are signed powers of two, except zero.
// The eight-bit accumulator wraps in hardware.
fn differential_step(code: u8) -> i8 {
    match code {
        1..=7 => 1i8.wrapping_shl(u32::from(code.saturating_sub(1))),
        8..=15 => 1i8
            .wrapping_shl(u32::from(15u8.saturating_sub(code)))
            .wrapping_neg(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_member_order_and_exact_extents() -> Result<(), ImportError> {
        let members = [Member { bytes: 2, name: "first" }, Member {
            bytes: 1,
            name: "second",
        }];
        let mut bytes = [0u8; 3];
        let mut found = [false; 2];
        collect_member(&mut bytes, &mut found, &members, ("second", &[3]))?;
        collect_member(&mut bytes, &mut found, &members, ("unused", &[4]))?;
        if found != [false, true]
            || collect_member(&mut bytes, &mut found, &members, ("first", &[1]))
                .is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        collect_member(&mut bytes, &mut found, &members, ("first", &[1, 2]))?;
        (bytes == [1, 2, 3]
            && found == [true, true]
            && collect_member(
                &mut bytes,
                &mut found,
                &members,
                ("first", &[1, 2]),
            )
            .is_err())
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn differential_samples_use_low_nibble_and_wrap() -> Result<(), ImportError>
    {
        let rom = SampleRom {
            bytes: vec![0, 0x77, 0x8f, 0x10],
        };
        let clip = rom.clip(0, 3, Encoding::Differential4)?;
        (clip.samples().collect::<Vec<_>>()
            == [16384, -32768, 32512, -256, -256, 0]
            && clip.observed_sample(1) == Some(0)
            && clip.observed_sample(2) == Some(16384)
            && clip.observed_sample(7) == Some(0)
            && clip.observed_sample(8).is_none())
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn linear_samples_skip_the_programmed_start() -> Result<(), ImportError> {
        let rom = SampleRom {
            bytes: vec![0x42, 0x80, 0xff, 0, 0x7f],
        };
        let clip = rom.clip(0, 4, Encoding::Pcm8)?;
        (clip.samples().collect::<Vec<_>>() == [-32768, -256, 0, 32512]
            && clip.observed_sample(0) == Some(0)
            && clip.observed_sample(4) == Some(32512)
            && clip.observed_sample(5).is_none()
            && rom.clip(0, 5, Encoding::Pcm8).is_err()
            && rom.clip(0, 0, Encoding::Pcm8).is_err())
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn observations_require_full_ordered_frames() -> Result<(), ImportError> {
        let rom = SampleRom { bytes: vec![0, 0x7f] };
        let mut raw = [0u8; RECORD_BYTES.saturating_mul(VOICES)];
        for (voice, record) in
            raw.as_chunks_mut::<RECORD_BYTES>().0.iter_mut().enumerate()
        {
            *record.get_mut(16).ok_or(ImportError::UnsupportedMember)? =
                u8::try_from(voice).map_err(|_error| ImportError::TooLarge)?;
        }
        let first = raw
            .first_chunk_mut::<RECORD_BYTES>()
            .ok_or(ImportError::UnsupportedMember)?;
        *first.get_mut(8).ok_or(ImportError::UnsupportedMember)? = 1;
        *first.get_mut(12).ok_or(ImportError::UnsupportedMember)? = 1;
        *first.get_mut(17).ok_or(ImportError::UnsupportedMember)? = 0x7f;
        *first.get_mut(18).ok_or(ImportError::UnsupportedMember)? = 1;
        let report = compare_observations(&rom, &raw)?;
        if report
            != (ObservationReport {
                differential: 0,
                frames: 1,
                mismatches: 0,
                pcm: 1,
            })
            || compare_observations(&rom, &[]).is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        *raw.get_mut(17).ok_or(ImportError::UnsupportedMember)? = 0x7e;
        if compare_observations(&rom, &raw)?.mismatches != 1 {
            return Err(ImportError::UnsupportedMember);
        }
        *raw.get_mut(19).ok_or(ImportError::UnsupportedMember)? = 1;
        compare_observations(&rom, &raw)
            .is_err()
            .then_some(())
            .ok_or(ImportError::UnsupportedMember)
    }
}
