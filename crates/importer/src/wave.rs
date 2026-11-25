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
//   - Exact canonical-header RIFF PCM16 reference file admission.
// - Must-Not:
//   - Accept truncated or ambiguous chunks, resample or execute media.
// - Allows:
//   - Inputs: Bounded immutable WAV bytes and explicit byte limit.
//   - Outputs: Borrowed PCM payload and canonical format metadata.
//   - Side effects: None.
// - Split-When:
//   - Additional WAVE chunk layouts or codecs require their own policies.
// - Merge-When:
//   - An approved strict raw PCM dependency provides equivalent checks.
// - Summary:
//   - Read the exact uncompressed WAV shape produced by reference captures.
// - Description:
//   - Admit only RIFF WAVE with a 16-byte PCM fmt and one exact data chunk.
// - Usage:
//   - Bind the bounded capture and use its PCM bytes for comparison.
// - Defaults:
//   - Reject extended, reordered, duplicate or trailing chunks.

//! Strict signed PCM16 WAV reference capture reader.

use std::num::{NonZeroU8, NonZeroU32, NonZeroU64};

use games_assets::AssetFormat;

use crate::ImportError;

/// Fixed size of the supported canonical WAV header.
pub const WAVE_HEADER_BYTES: usize = 44;

/// Exact borrowed PCM16 payload from a canonical-header WAV.
#[derive(Clone, Copy, Debug)]
pub struct WavePcm<'source> {
    format: AssetFormat,
    payload: &'source [u8],
}

impl<'source> WavePcm<'source> {
    /// Canonical source-independent PCM representation.
    #[must_use]
    pub const fn format(self) -> AssetFormat {
        self.format
    }

    /// Parse an exact 44-byte RIFF WAVE header followed by complete PCM16.
    ///
    /// Additional chunks and extended formats are intentionally unsupported.
    /// # Errors
    /// Rejects oversized, malformed, empty or unsupported captures.
    pub fn new(
        source: &'source [u8],
        max_bytes: NonZeroU64,
    ) -> Result<Self, ImportError> {
        let size = u64::try_from(source.len())
            .map_err(|_error| ImportError::TooLarge)?;
        if size > max_bytes.get() {
            return Err(ImportError::TooLarge);
        }
        let (header, payload) = source
            .split_first_chunk::<WAVE_HEADER_BYTES>()
            .ok_or(ImportError::InvalidWave)?;
        validate_header(header, size)?;
        let format = pcm_format(
            header,
            u64::try_from(payload.len())
                .map_err(|_error| ImportError::TooLarge)?,
        )?;
        Ok(Self { format, payload })
    }

    /// Borrow the exact signed little-endian interleaved PCM bytes.
    #[must_use]
    pub const fn payload(self) -> &'source [u8] {
        self.payload
    }
}

fn field<const N: usize>(
    header: &[u8; WAVE_HEADER_BYTES],
    offset: usize,
) -> Result<[u8; N], ImportError> {
    let end = offset.checked_add(N).ok_or(ImportError::InvalidWave)?;
    header
        .get(offset..end)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ImportError::InvalidWave)
}

fn pcm_format(
    header: &[u8; WAVE_HEADER_BYTES],
    bytes: u64,
) -> Result<AssetFormat, ImportError> {
    let channels = u16::from_le_bytes(field(header, 22)?);
    let sample_rate = u32::from_le_bytes(field(header, 24)?);
    let byte_rate = u32::from_le_bytes(field(header, 28)?);
    let alignment = u16::from_le_bytes(field(header, 32)?);
    let channel_count = u8::try_from(channels)
        .ok()
        .filter(|count| *count <= 8)
        .and_then(NonZeroU8::new)
        .ok_or(ImportError::InvalidWave)?;
    let frame_bytes = u64::from(channels)
        .checked_mul(2)
        .ok_or(ImportError::InvalidWave)?;
    let rate = NonZeroU32::new(sample_rate).ok_or(ImportError::InvalidWave)?;
    if u64::from(alignment) != frame_bytes
        || u64::from(sample_rate).checked_mul(frame_bytes)
            != Some(u64::from(byte_rate))
        || bytes.checked_rem(frame_bytes) != Some(0)
    {
        return Err(ImportError::InvalidWave);
    }
    let frames = bytes
        .checked_div(frame_bytes)
        .and_then(NonZeroU64::new)
        .ok_or(ImportError::InvalidWave)?;
    Ok(AssetFormat::PcmS16Le {
        channels: channel_count,
        frames,
        sample_rate: rate,
    })
}

fn validate_header(
    header: &[u8; WAVE_HEADER_BYTES],
    size: u64,
) -> Result<(), ImportError> {
    if field::<4>(header, 0)? != *b"RIFF"
        || field::<4>(header, 8)? != *b"WAVE"
        || field::<4>(header, 12)? != *b"fmt "
        || field::<4>(header, 36)? != *b"data"
        || u32::from_le_bytes(field(header, 16)?) != 16
        || u16::from_le_bytes(field(header, 20)?) != 1
        || u16::from_le_bytes(field(header, 34)?) != 16
    {
        return Err(ImportError::InvalidWave);
    }
    let riff_size = u64::from(u32::from_le_bytes(field(header, 4)?));
    let data_size = u64::from(u32::from_le_bytes(field(header, 40)?));
    if riff_size.checked_add(8) != Some(size)
        || data_size.checked_add(
            u64::try_from(WAVE_HEADER_BYTES)
                .map_err(|_error| ImportError::InvalidWave)?,
        ) != Some(size)
    {
        return Err(ImportError::InvalidWave);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPTURE: [u8; 52] = [
        b'R', b'I', b'F', b'F', 44, 0, 0, 0, b'W', b'A', b'V', b'E', b'f',
        b'm', b't', b' ', 16, 0, 0, 0, 1, 0, 2, 0, 128, 187, 0, 0, 0, 238, 2,
        0, 4, 0, 16, 0, b'd', b'a', b't', b'a', 8, 0, 0, 0, 0, 128, 255, 127,
        1, 0, 255, 255,
    ];

    #[test]
    fn admits_exact_stereo_pcm() {
        let result = WavePcm::new(&CAPTURE, NonZeroU64::MAX);
        assert!(result.is_ok());
        let Ok(wave) = result else {
            return;
        };
        assert_eq!(wave.payload(), CAPTURE.get(44..).unwrap_or_default());
        assert_eq!(wave.format(), AssetFormat::PcmS16Le {
            channels: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
            frames: NonZeroU64::new(2).unwrap_or(NonZeroU64::MIN),
            sample_rate: NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN),
        });
    }

    #[test]
    fn rejects_truncated_ambiguous_and_oversized_captures() {
        for end in 0..CAPTURE.len() {
            assert!(
                WavePcm::new(
                    CAPTURE.get(..end).unwrap_or_default(),
                    NonZeroU64::MAX
                )
                .is_err()
            );
        }
        let mut appended = CAPTURE.to_vec();
        appended.push(0);
        assert!(WavePcm::new(&appended, NonZeroU64::MAX).is_err());
        assert!(matches!(
            WavePcm::new(&CAPTURE, NonZeroU64::MIN),
            Err(ImportError::TooLarge)
        ));
        for offset in [0usize, 4, 12, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
            let mut malformed = CAPTURE;
            if let Some(value) = malformed.get_mut(offset) {
                *value ^= 1;
            }
            assert!(WavePcm::new(&malformed, NonZeroU64::MAX).is_err());
        }
    }
}
