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
//   - Admission and conversion of the embedded KDSC sample catalogue.
// - Must-Not:
//   - Run original code or infer semantic sound names or playback events.
// - Allows:
//   - Inputs: Verified sample region, expected count and canonical limits.
//   - Outputs: Ordered mono PCM16 assets at declared nominal sample rates.
//   - Side effects: Calls the private canonical conversion sink.
// - Split-When:
//   - A source uses a different sample directory representation.
// - Merge-When:
//   - Another adapter implements the same bounded catalogue format.
// - Summary:
//   - Convert the complete verified sample catalogue without publishing ROMs.
// - Description:
//   - Validate all extents and tags before decoding one bounded clip at a time.
// - Usage:
//   - Use the selected machine's pinned sample region and catalogue count.
// - Defaults:
//   - Reject ambiguous, gapped, overlapping or out-of-bounds sample entries.

//! Strict sample directory admission and canonical conversion.

use std::num::{NonZeroU8, NonZeroU32, NonZeroU64, NonZeroUsize};

use games_assets::{AssetFormat, AssetManifest, ContentHash, ManifestLimits};

use super::{Clip, Encoding, SampleRom};
use crate::ImportError;
use crate::conversion::{Conversion, preflight_payloads};

const HEADER_BYTES: usize = 32;

#[derive(Clone, Copy, Debug)]
struct Catalogue<'source> {
    headers: &'source [[u8; HEADER_BYTES]],
    rom: &'source [u8],
}

impl<'source> Catalogue<'source> {
    fn new(
        rom: &'source [u8],
        count: NonZeroUsize,
    ) -> Result<Self, ImportError> {
        let header_end = count
            .get()
            .checked_mul(HEADER_BYTES)
            .ok_or(ImportError::TooLarge)?;
        let headers = rom
            .get(..header_end)
            .ok_or(ImportError::UnsupportedMember)?
            .as_chunks::<HEADER_BYTES>()
            .0;
        let mut expected = header_end;
        for header in headers {
            let entry = Entry::parse(header)?;
            if entry.start != expected
                || rom.get(entry.start..entry.end).is_none()
            {
                return Err(ImportError::UnsupportedMember);
            }
            expected = entry.end;
        }
        Ok(Self { headers, rom })
    }

    fn write<Stage>(
        self,
        limits: ManifestLimits,
        output: &mut Conversion<'_, Stage>,
    ) -> Result<(), ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        preflight_payloads(
            limits,
            self.headers
                .iter()
                .map(|header| Entry::parse(header)?.size()),
        )?;
        for (index, header) in self.headers.iter().enumerate() {
            let entry = Entry::parse(header)?;
            let bytes = self
                .rom
                .get(entry.start..entry.end)
                .ok_or(ImportError::UnsupportedMember)?;
            let clip = Clip {
                bytes,
                encoding: entry.encoding,
            };
            let frames = entry.frames()?;
            let size = entry.size()?;
            let mut payload = Vec::new();
            payload
                .try_reserve_exact(
                    usize::try_from(size)
                        .map_err(|_error| ImportError::TooLarge)?,
                )
                .map_err(|_error| ImportError::TooLarge)?;
            for sample in clip.samples() {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
            output.write(
                &key(index),
                AssetFormat::PcmS16Le {
                    channels: NonZeroU8::MIN,
                    frames,
                    sample_rate: entry.rate,
                },
                &payload,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    encoding: Encoding,
    end: usize,
    rate: NonZeroU32,
    start: usize,
}

impl Entry {
    fn frames(self) -> Result<NonZeroU64, ImportError> {
        let count = self
            .end
            .checked_sub(self.start)
            .and_then(|size| u64::try_from(size).ok())
            .ok_or(ImportError::TooLarge)?;
        let multiplier = match self.encoding {
            Encoding::Differential4 => 2,
            Encoding::Pcm8 => 1,
        };
        count
            .checked_mul(multiplier)
            .and_then(NonZeroU64::new)
            .ok_or(ImportError::TooLarge)
    }

    fn parse(header: &[u8; HEADER_BYTES]) -> Result<Self, ImportError> {
        let (fields, tag) = header.split_at(16);
        let [first, duplicate, last, rate_bytes] = fields.as_chunks::<4>().0
        else {
            return Err(ImportError::UnsupportedMember);
        };
        let encoding = match tag {
            b"KDSC PCM        " => Encoding::Pcm8,
            b"KDSC KADPCM     " => Encoding::Differential4,
            _ => return Err(ImportError::UnsupportedMember),
        };
        if first != duplicate {
            return Err(ImportError::UnsupportedMember);
        }
        let start = usize::try_from(u32::from_le_bytes(*first))
            .map_err(|_error| ImportError::TooLarge)?;
        let end = usize::try_from(u32::from_le_bytes(*last))
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(ImportError::TooLarge)?;
        if start >= end {
            return Err(ImportError::UnsupportedMember);
        }
        let rate = NonZeroU32::new(u32::from_le_bytes(*rate_bytes))
            .ok_or(ImportError::UnsupportedMember)?;
        Ok(Self {
            encoding,
            end,
            rate,
            start,
        })
    }

    fn size(self) -> Result<u64, ImportError> {
        self.frames()?
            .get()
            .checked_mul(2)
            .ok_or(ImportError::TooLarge)
    }
}

pub(in crate::mame) fn convert<Stage>(
    rom: &SampleRom,
    profile: ManifestLimits,
    limits: ManifestLimits,
    output: &mut Conversion<'_, Stage>,
) -> Result<(), ImportError>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
{
    Catalogue::new(&rom.bytes, profile.records)?.write(limits, output)
}

fn key(index: usize) -> String {
    format!("audio/sample-{index:04}")
}

pub(in crate::mame) fn validate_manifest(
    manifest: &AssetManifest,
    count: NonZeroUsize,
) -> Result<(), ImportError> {
    if manifest.records().len() != count.get() {
        return Err(ImportError::UnsupportedMember);
    }
    for index in 0..count.get() {
        if !matches!(
            manifest.record(&key(index)).map(|record| record.format),
            Some(AssetFormat::PcmS16Le { channels, .. })
                if channels == NonZeroU8::MIN
        ) {
            return Err(ImportError::UnsupportedMember);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use games_assets::SourceIdentity;

    use super::*;
    use crate::SourceSnapshot;
    use crate::conversion::convert_snapshot;

    fn header(first: u32, last: u32, rate: u32, tag: &[u8; 16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [first, first, last, rate] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(tag);
        bytes
    }

    fn reject_short_budget(
        source: &SourceSnapshot<'_>,
        rom: &SampleRom,
        limits: ManifestLimits,
    ) -> Result<(), ImportError> {
        let short = ManifestLimits {
            total_bytes: NonZeroU64::MIN.saturating_add(10),
            ..limits
        };
        let rejected = convert_snapshot(
            source,
            short,
            |_digest, _payload| Err(ImportError::StageFailed),
            |_verified, output| convert(rom, limits, short, output),
        );
        (rejected == Err(games_assets::AssetError::TooLarge.into()))
            .then_some(())
            .ok_or(ImportError::StageFailed)
    }

    #[test]
    fn canonical_conversion_preserves_amplitude_rate_and_order()
    -> Result<(), ImportError> {
        let mut bytes = header(64, 65, 8000, b"KDSC PCM        ");
        bytes.extend(header(66, 67, 12000, b"KDSC KADPCM     "));
        bytes.extend([0x80, 0x7f, 0x77, 0x8f]);
        let identity = SourceIdentity {
            revision: String::from("revision"),
            title: String::from("synthetic"),
        };
        let source = SourceSnapshot::new(
            b"synthetic",
            &identity,
            NonZeroU64::MIN.saturating_add(8),
        )?;
        let limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(7),
            records: NonZeroUsize::MIN.saturating_add(1),
            total_bytes: NonZeroU64::MIN.saturating_add(11),
        };
        let rom = SampleRom { bytes };
        reject_short_budget(&source, &rom, limits)?;
        let mut payloads = Vec::new();
        let manifest = convert_snapshot(
            &source,
            limits,
            |_digest, payload| {
                payloads.push(payload.to_vec());
                Ok(())
            },
            |_verified, output| convert(&rom, limits, limits, output),
        )?;
        validate_manifest(&manifest, limits.records)?;
        let first = manifest
            .record(&key(0))
            .ok_or(ImportError::UnsupportedMember)?;
        let second = manifest
            .record(&key(1))
            .ok_or(ImportError::UnsupportedMember)?;
        (payloads
            == [vec![0, 0x80, 0, 0x7f], vec![
                0, 0x40, 0, 0x80, 0, 0x7f, 0, 0xff,
            ]]
            && first.format
                == AssetFormat::PcmS16Le {
                    channels: NonZeroU8::MIN,
                    frames: NonZeroU64::MIN.saturating_add(1),
                    sample_rate: NonZeroU32::MIN.saturating_add(7999),
                }
            && second.format
                == AssetFormat::PcmS16Le {
                    channels: NonZeroU8::MIN,
                    frames: NonZeroU64::MIN.saturating_add(3),
                    sample_rate: NonZeroU32::MIN.saturating_add(11_999),
                })
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn catalogue_admits_contiguous_inclusive_ranges() -> Result<(), ImportError>
    {
        let mut rom = header(64, 65, 8000, b"KDSC PCM        ");
        rom.extend(header(66, 67, 12000, b"KDSC KADPCM     "));
        rom.extend([0x80, 0x7f, 0x77, 0x8f]);
        let count = NonZeroUsize::MIN.saturating_add(1);
        let catalogue = Catalogue::new(&rom, count)?;
        let mut entries = catalogue.headers.iter().map(Entry::parse);
        let first = entries.next().ok_or(ImportError::UnsupportedMember)??;
        let second = entries.next().ok_or(ImportError::UnsupportedMember)??;
        (first.frames()?.get() == 2
            && second.frames()?.get() == 4
            && first.rate.get() == 8000
            && second.rate.get() == 12000
            && catalogue.rom.get(first.start..first.end) == Some(&[0x80, 0x7f])
            && catalogue.rom.get(second.start..second.end)
                == Some(&[0x77, 0x8f]))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn catalogue_rejects_bad_tags_gaps_and_extents() -> Result<(), ImportError>
    {
        let mut valid = header(32, 32, 8000, b"KDSC PCM        ");
        valid.push(0);
        let count = NonZeroUsize::MIN;
        let _catalogue = Catalogue::new(&valid, count)?;
        for (offset, bad) in [(0, 33), (4, 33), (8, 33), (16, 0)] {
            let mut invalid = valid.clone();
            *invalid
                .get_mut(offset)
                .ok_or(ImportError::UnsupportedMember)? = bad;
            if Catalogue::new(&invalid, count).is_ok() {
                return Err(ImportError::UnsupportedMember);
            }
        }
        let mut zero = valid;
        zero.get_mut(12..16)
            .ok_or(ImportError::UnsupportedMember)?
            .fill(0);
        if Catalogue::new(&zero, count).is_ok() {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }
}
