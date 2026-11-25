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
//   - Checked access to interleaved canonical signed PCM samples.
// - Must-Not:
//   - Decode source containers, mix playback or own native audio devices.
// - Allows:
//   - Inputs: Verified canonical descriptors and immutable payload bytes.
//   - Outputs: Borrowed frames and native-endian signed sample values.
//   - Side effects: None.
// - Split-When:
//   - Playback mixing requires separate timing and voice policies.
// - Merge-When:
//   - Another canonical PCM reader duplicates frame addressing.
// - Summary:
//   - Allocation-free canonical PCM frame and sample access.
// - Description:
//   - Check exact identity and extent before borrowing interleaved samples.
// - Usage:
//   - Bind a verified record and request complete frames or signed samples.
// - Defaults:
//   - Reject other formats, malformed metadata and mismatched bytes.

//! Verified, allocation-free interleaved PCM access.

use std::num::{NonZeroU8, NonZeroU32, NonZeroU64, NonZeroUsize};

use crate::{AssetError, AssetFormat, AssetRecord};

/// Borrowed canonical interleaved signed 16-bit little-endian audio.
#[derive(Clone, Copy, Debug)]
pub struct PcmSamples<'payload> {
    bytes: &'payload [u8],
    channels: NonZeroU8,
    frames: NonZeroU64,
    sample_rate: NonZeroU32,
}

impl<'payload> PcmSamples<'payload> {
    /// Complete interleaved PCM payload.
    #[must_use]
    pub const fn bytes(&self) -> &'payload [u8] {
        self.bytes
    }

    /// Number of interleaved channels.
    #[must_use]
    pub const fn channels(&self) -> NonZeroU8 {
        self.channels
    }

    /// Borrow the complete interleaved bytes of one sample frame.
    #[must_use]
    pub fn frame(&self, index: u64) -> Option<&'payload [u8]> {
        if index >= self.frames.get() {
            return None;
        }
        let width = NonZeroUsize::from(self.channels).get().checked_mul(2)?;
        let start = usize::try_from(index).ok()?.checked_mul(width)?;
        let end = start.checked_add(width)?;
        self.bytes.get(start..end)
    }

    /// Number of complete sample frames.
    #[must_use]
    pub const fn frames(&self) -> NonZeroU64 {
        self.frames
    }

    /// Check PCM shape and exact payload hash before borrowing the bytes.
    /// # Errors
    /// Rejects non-PCM descriptors, malformed extents or mismatched content.
    pub fn new(
        record: &AssetRecord,
        bytes: &'payload [u8],
    ) -> Result<Self, AssetError> {
        let AssetFormat::PcmS16Le {
            channels,
            frames,
            sample_rate,
        } = record.format
        else {
            return Err(AssetError::InvalidFormat);
        };
        record.verify(bytes)?;
        Ok(Self {
            bytes,
            channels,
            frames,
            sample_rate,
        })
    }

    /// Return one signed sample at a zero-based frame and channel.
    #[must_use]
    pub fn sample(&self, frame: u64, channel: u8) -> Option<i16> {
        let bytes = self.frame(frame)?;
        let pair = bytes.as_chunks::<2>().0.get(usize::from(channel))?;
        Some(i16::from_le_bytes(*pair))
    }

    /// Samples per second per channel.
    #[must_use]
    pub const fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    /// Iterate signed samples in their exact interleaved order.
    pub fn samples(&self) -> impl ExactSizeIterator<Item = i16> + 'payload {
        self.bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(i16::from_le_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContentHash;

    const PCM: [u8; 8] = [0, 128, 255, 127, 1, 0, 255, 255];

    fn record() -> AssetRecord {
        AssetRecord {
            digest: ContentHash::calculate(&PCM),
            format: AssetFormat::PcmS16Le {
                channels: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
                frames: NonZeroU64::new(2).unwrap_or(NonZeroU64::MIN),
                sample_rate: NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN),
            },
            key: String::from("sound/test"),
            size: 8,
        }
    }

    #[test]
    fn reads_exact_pcm_metadata() {
        let result = PcmSamples::new(&record(), &PCM);
        assert!(result.is_ok());
        let Ok(pcm) = result else {
            return;
        };
        assert_eq!(pcm.channels().get(), 2);
        assert_eq!(pcm.frames().get(), 2);
        assert_eq!(pcm.sample_rate().get(), 48_000);
        assert_eq!(pcm.bytes(), PCM);
    }

    #[test]
    fn reads_exact_sample_frames() {
        let result = PcmSamples::new(&record(), &PCM);
        assert!(result.is_ok());
        let Ok(pcm) = result else {
            return;
        };
        assert_eq!(pcm.frame(1), PCM.get(4..8));
        assert_eq!(pcm.sample(0, 0), Some(i16::MIN));
        assert_eq!(pcm.sample(0, 1), Some(i16::MAX));
        assert_eq!(pcm.sample(1, 0), Some(1));
        assert_eq!(pcm.sample(1, 1), Some(-1));
        assert_eq!(pcm.sample(1, 2), None);
        assert_eq!(pcm.frame(u64::MAX), None);
    }

    #[test]
    fn reads_interleaved_sample_iterator() {
        let result = PcmSamples::new(&record(), &PCM);
        assert!(result.is_ok());
        let Ok(pcm) = result else {
            return;
        };
        assert_eq!(pcm.samples().collect::<Vec<_>>(), [
            i16::MIN,
            i16::MAX,
            1,
            -1
        ]);
    }

    #[test]
    fn rejects_other_formats_and_tampering() {
        let mut descriptor = record();
        assert!(
            PcmSamples::new(&descriptor, PCM.get(..6).unwrap_or_default())
                .is_err()
        );
        assert!(PcmSamples::new(&descriptor, &[0; 8]).is_err());
        descriptor.format = AssetFormat::Data;
        assert!(PcmSamples::new(&descriptor, &PCM).is_err());
    }
}
