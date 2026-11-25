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
//   - Portable PCM playback requests, outcomes and errors.
// - Must-Not:
//   - Resample, mix, interpret original sound hardware or advance game time.
// - Allows:
//   - Inputs: Explicit playback configuration and complete PCM frames.
//   - Outputs: Queue occupancy, admission and playback control.
//   - Side effects: None.
// - Split-When:
//   - Additional playback formats require separate typed requests.
// - Merge-When:
//   - Another adapter duplicates bounded native audio submission.
// - Summary:
//   - Exact-format portable PCM playback with explicit backpressure.
// - Description:
//   - Describes bounded playback requests and device observations.
// - Usage:
//   - Check explicit PCM requirements before device access.
// - Defaults:
//   - Reject invalid formats and unbounded requests.

//! Portable PCM playback requests and outcomes.

use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};

/// Selected native playback handle, unavailable without an adapter.
pub use crate::selected::NativeAudio;

/// Explicit audio playback configuration and software-queue budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackSpec {
    /// Native audio buffer size in sample frames; a power of two.
    pub buffer_frames: NonZeroU16,
    /// Number of interleaved channels.
    pub channels: NonZeroU8,
    /// Maximum number of complete sample frames in the software queue.
    pub max_queued_frames: NonZeroU32,
    /// Sample frames per second.
    pub sample_rate: NonZeroU32,
}

/// Failure to open the device or admit a complete audio submission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaybackError {
    /// Scratch storage could not be reserved.
    Allocation,
    /// The native audio backend reported an error.
    Backend(String),
    /// The submission would exceed the explicit software queue budget.
    Capacity,
    /// PCM or the obtained device has an incompatible format.
    FormatMismatch,
    /// An explicit device name is empty, oversized or invalid.
    InvalidDevice,
    /// An empty, incomplete or out-of-range frame submission.
    InvalidFrames,
    /// The requested playback configuration is not representable.
    InvalidSpec,
}

/// The current native audio backend and its enumerated playback devices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioDevices {
    /// Adapter diagnostic identifier.
    pub driver: &'static str,
    /// Exact output names, or None when enumeration is unsupported.
    pub names: Option<Vec<String>>,
}

#[cfg(test)]
/// Shared synthetic PCM fixtures for hardware contract and backend tests.
pub mod fixtures {
    use std::num::NonZeroU64;

    use games_assets::pcm::PcmSamples;
    use games_assets::{AssetFormat, AssetRecord, ContentHash};

    use super::*;
    const PAYLOAD: [u8; 12] = [0, 128, 255, 127, 1, 0, 255, 255, 0, 0, 2, 0];

    /// Small bounded stereo playback request.
    #[must_use]
    pub fn configuration() -> PlaybackSpec {
        PlaybackSpec {
            buffer_frames: NonZeroU16::new(2).unwrap_or(NonZeroU16::MIN),
            channels: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
            max_queued_frames: NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            sample_rate: NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN),
        }
    }

    /// Verify a tiny canonical stereo sample fixture.
    /// # Errors
    /// Reports invalid fixture format or digest.
    pub fn pcm() -> Result<PcmSamples<'static>, PlaybackError> {
        let spec = configuration();
        PcmSamples::new(
            &AssetRecord {
                digest: ContentHash::calculate(&PAYLOAD),
                format: AssetFormat::PcmS16Le {
                    channels: spec.channels,
                    frames: NonZeroU64::new(3).unwrap_or(NonZeroU64::MIN),
                    sample_rate: spec.sample_rate,
                },
                key: String::from("test/audio"),
                size: 12,
            },
            &PAYLOAD,
        )
        .map_err(|_error| PlaybackError::InvalidFrames)
    }
}
