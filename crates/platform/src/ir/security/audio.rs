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
//   - Bounded PCM playback, device names and obtained-format admission.
// - Must-Not:
//   - Resample, mix, interpret original sound hardware or advance game time.
// - Allows:
//   - Inputs: Explicit playback configuration and complete PCM frames.
//   - Outputs: Queue occupancy, admission and playback control.
//   - Side effects: None.
// - Split-When:
//   - Additional PCM formats require independent admission policies.
// - Merge-When:
//   - Another adapter duplicates bounded native audio submission.
// - Summary:
//   - Exact-format portable PCM admission with explicit backpressure.
// - Description:
//   - Checks capacity before decoding or submitting a frame range.
// - Usage:
//   - Check explicit PCM requirements before device access.
// - Defaults:
//   - Reject invalid formats and unbounded requests.

//! Shared audio security admission before backend access.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::ops::Range;

use games_assets::pcm::PcmSamples;

use crate::ir::audio::{AudioDevices, PlaybackError, PlaybackSpec};

const MAX_DEVICE_NAME: usize = 512;
/// Maximum number of admitted playback devices.
pub const MAX_DEVICES: u32 = 64;

/// Actual application-facing playback format and initial device state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlaybackObservation {
    pub buffer_bytes: u32,
    pub buffer_frames: u16,
    pub channels: u8,
    pub frequency: i32,
    pub paused: bool,
    pub signed_native_16: bool,
}

/// Device open request whose name and layout have passed shared admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackRequest<'device> {
    layout: QueueLayout,
    name: Option<&'device str>,
}

impl<'device> PlaybackRequest<'device> {
    /// Admit the opened device before exposing its playback handle.
    ///
    /// Native buffer negotiation may change the period size but not the
    /// application-facing signed sample representation or frame geometry.
    pub(crate) fn admit_opened(
        self,
        observed: PlaybackObservation,
        reported_name: Option<&str>,
        queued_bytes: u32,
    ) -> Result<(), PlaybackError> {
        if reported_name != self.name {
            return Err(PlaybackError::InvalidDevice);
        }
        if observed.frequency != self.layout.frequency()
            || observed.channels != self.layout.spec().channels.get()
            || !observed.signed_native_16
            || observed.buffer_frames == 0
            || u32::from(observed.buffer_frames)
                .checked_mul(self.layout.frame_bytes)
                != Some(observed.buffer_bytes)
        {
            return Err(PlaybackError::FormatMismatch);
        }
        if !observed.paused {
            return Err(PlaybackError::InvalidState);
        }
        if queued_bytes != 0 {
            return Err(PlaybackError::InvalidQueue);
        }
        Ok(())
    }

    /// Admitted queue geometry.
    #[must_use]
    pub const fn layout(self) -> QueueLayout {
        self.layout
    }

    /// Explicit device name, or None for the system default.
    #[must_use]
    pub const fn name(self) -> Option<&'device str> {
        self.name
    }

    /// Admit a device name and exact playback layout before dispatch.
    /// # Errors
    /// Rejects invalid names, formats or queue budgets.
    pub fn new(
        spec: PlaybackSpec,
        name: Option<&'device str>,
    ) -> Result<Self, PlaybackError> {
        if name.is_some_and(|value| !valid_device(value)) {
            return Err(PlaybackError::InvalidDevice);
        }
        Ok(Self {
            layout: QueueLayout::new(spec)?,
            name,
        })
    }
}

/// Complete interleaved samples admitted against the current queue budget.
#[derive(Clone, Copy, Debug)]
pub struct AdmittedSamples<'samples> {
    samples: &'samples [i16],
}

impl<'samples> AdmittedSamples<'samples> {
    /// The exact admitted signed PCM slice.
    #[must_use]
    pub const fn samples(self) -> &'samples [i16] {
        self.samples
    }
}

/// Validated PCM queue geometry and admission policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueLayout {
    channels: usize,
    frame_bytes: u32,
    frequency: i32,
    max_bytes: u32,
    spec: PlaybackSpec,
}

impl QueueLayout {
    /// Admit complete interleaved samples within remaining queue capacity.
    /// # Errors
    /// Rejects partial frames, invalid observations or exceeded queue limits.
    pub fn admit(
        self,
        samples: usize,
        queued_bytes: u32,
    ) -> Result<(), PlaybackError> {
        if samples == 0 || !samples.is_multiple_of(self.channels) {
            return Err(PlaybackError::InvalidFrames);
        }
        let queued = self.queued_frames(queued_bytes)?;
        let frames = samples
            .checked_div(self.channels)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(PlaybackError::Capacity)?;
        if frames > self.spec.max_queued_frames.get().saturating_sub(queued) {
            return Err(PlaybackError::Capacity);
        }
        Ok(())
    }

    /// Admit complete samples and bind the verified slice for dispatch.
    /// # Errors
    /// Rejects incomplete frames, invalid observations or queue overflow.
    pub fn admit_samples(
        self,
        samples: &[i16],
        queued_bytes: u32,
    ) -> Result<AdmittedSamples<'_>, PlaybackError> {
        self.admit(samples.len(), queued_bytes)?;
        Ok(AdmittedSamples { samples })
    }

    /// Checked signed sample rate for the backend request.
    #[must_use]
    pub const fn frequency(self) -> i32 {
        self.frequency
    }

    /// Validate the requested playback format and capacity.
    /// # Errors
    /// Rejects unrepresentable rates and invalid queue budgets.
    pub fn new(spec: PlaybackSpec) -> Result<Self, PlaybackError> {
        if !spec.buffer_frames.is_power_of_two()
            || u32::from(spec.buffer_frames.get())
                > spec.max_queued_frames.get()
        {
            return Err(PlaybackError::InvalidSpec);
        }
        let frequency = i32::try_from(spec.sample_rate.get())
            .map_err(|_error| PlaybackError::InvalidSpec)?;
        let frame_bytes =
            NonZeroU32::from(spec.channels).get().saturating_mul(2);
        let max_bytes = spec
            .max_queued_frames
            .get()
            .checked_mul(frame_bytes)
            .ok_or(PlaybackError::InvalidSpec)?;
        Ok(Self {
            channels: NonZeroUsize::from(spec.channels).get(),
            frame_bytes,
            frequency,
            max_bytes,
            spec,
        })
    }

    /// Resolve a verified PCM frame range to interleaved sample indices.
    /// # Errors
    /// Rejects format mismatches and out-of-bounds frame ranges.
    pub fn pcm_range(
        self,
        pcm: &PcmSamples<'_>,
        start: u64,
        frames: NonZeroU32,
    ) -> Result<Range<usize>, PlaybackError> {
        if pcm.channels() != self.spec.channels
            || pcm.sample_rate() != self.spec.sample_rate
        {
            return Err(PlaybackError::FormatMismatch);
        }
        let end = start
            .checked_add(NonZeroU64::from(frames).get())
            .filter(|end| *end <= pcm.frames().get())
            .ok_or(PlaybackError::InvalidFrames)?;
        let first = usize::try_from(start)
            .ok()
            .and_then(|value| value.checked_mul(self.channels))
            .ok_or(PlaybackError::InvalidFrames)?;
        let last = usize::try_from(end)
            .ok()
            .and_then(|value| value.checked_mul(self.channels))
            .ok_or(PlaybackError::InvalidFrames)?;
        Ok(first..last)
    }

    /// Validate a backend byte observation and express it in sample frames.
    /// # Errors
    /// Rejects partial frames and observations beyond the admitted budget.
    pub fn queued_frames(
        self,
        queued_bytes: u32,
    ) -> Result<u32, PlaybackError> {
        if queued_bytes > self.max_bytes
            || !queued_bytes.is_multiple_of(self.frame_bytes)
        {
            return Err(PlaybackError::InvalidQueue);
        }
        queued_bytes
            .checked_div(self.frame_bytes)
            .ok_or(PlaybackError::InvalidQueue)
    }

    /// Original playback request admitted by this layout.
    #[must_use]
    pub const fn spec(self) -> PlaybackSpec {
        self.spec
    }
}

/// Admit the number of enumerated devices before allocation.
/// # Errors
/// Rejects device counts beyond the shared enumeration budget.
pub const fn admit_device_count(count: u32) -> Result<(), PlaybackError> {
    if count > MAX_DEVICES {
        Err(PlaybackError::Capacity)
    } else {
        Ok(())
    }
}

/// Whether a device name is bounded, nonempty and free of NUL bytes.
#[must_use]
pub fn valid_device(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_DEVICE_NAME
        && !name.as_bytes().contains(&0)
}

/// Admit a complete adapter enumeration before returning it to consumers.
/// # Errors
/// Rejects oversized inventories and invalid device names.
pub fn validate_devices(
    devices: AudioDevices,
) -> Result<AudioDevices, PlaybackError> {
    if let Some(names) = devices.names.as_ref() {
        let count = u32::try_from(names.len())
            .map_err(|_error| PlaybackError::Capacity)?;
        admit_device_count(count)?;
        if !names.iter().all(|name| valid_device(name)) {
            return Err(PlaybackError::InvalidDevice);
        }
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroU16};

    use super::*;
    use crate::ir::audio::fixtures::{configuration, pcm};
    #[test]
    fn checks_device_extent_and_budget() {
        let spec = configuration();
        assert_eq!(
            QueueLayout::new(PlaybackSpec {
                max_queued_frames: NonZeroU32::MAX,
                ..spec
            }),
            Err(PlaybackError::InvalidSpec)
        );
        assert_eq!(
            QueueLayout::new(PlaybackSpec {
                sample_rate: NonZeroU32::MAX,
                ..spec
            }),
            Err(PlaybackError::InvalidSpec)
        );
        assert_eq!(
            QueueLayout::new(PlaybackSpec {
                buffer_frames: NonZeroU16::new(3).unwrap_or(NonZeroU16::MIN),
                ..spec
            }),
            Err(PlaybackError::InvalidSpec)
        );
    }

    #[test]
    fn rejects_incomplete_frames_and_software_queue_overflow()
    -> Result<(), PlaybackError> {
        let layout = QueueLayout::new(configuration())?;
        for (samples, queued, expected) in [
            (0, 0, Err(PlaybackError::InvalidFrames)),
            (1, 0, Err(PlaybackError::InvalidFrames)),
            (6, 0, Ok(())),
            (4, 4, Ok(())),
            (4, 8, Err(PlaybackError::Capacity)),
            (4, 5, Err(PlaybackError::InvalidQueue)),
            (2, u32::MAX, Err(PlaybackError::InvalidQueue)),
        ] {
            let actual = layout.admit(samples, queued);
            if actual != expected {
                return Err(PlaybackError::Backend(format!(
                    "admission: expected {expected:?}, observed {actual:?}"
                )));
            }
        }
        Ok(())
    }

    #[test]
    fn frame_occupancy_preserves_caller_selected_channel_geometry()
    -> Result<(), PlaybackError> {
        for channels in [1u8, 2, 6, u8::MAX] {
            verify_channel_layout(channels)?;
        }
        Ok(())
    }

    fn verify_channel_layout(channels: u8) -> Result<(), PlaybackError> {
        let layout = QueueLayout::new(PlaybackSpec {
            channels: NonZeroU8::new(channels).unwrap_or(NonZeroU8::MIN),
            ..configuration()
        })?;
        let bytes = u32::from(channels).saturating_mul(2);
        let samples = usize::from(channels).saturating_mul(2);
        for (queued, expected) in [
            (0, Ok(0)),
            (bytes, Ok(1)),
            (bytes.saturating_mul(3), Ok(3)),
            (bytes.saturating_sub(1), Err(PlaybackError::InvalidQueue)),
            (bytes.saturating_mul(4), Err(PlaybackError::InvalidQueue)),
        ] {
            let actual = layout.queued_frames(queued);
            if actual != expected {
                return Err(PlaybackError::Backend(format!(
                    "{channels} channels: expected {expected:?}, \
                     observed {actual:?}"
                )));
            }
        }
        if layout.admit(samples, bytes).is_err()
            || layout.admit(samples, bytes.saturating_mul(2))
                != Err(PlaybackError::Capacity)
        {
            return Err(PlaybackError::Capacity);
        }
        Ok(())
    }

    #[test]
    fn validates_canonical_pcm_ranges_and_formats() {
        let inputs = QueueLayout::new(configuration())
            .and_then(|layout| pcm().map(|audio| (layout, audio)));
        assert!(inputs.is_ok());
        let Ok((layout, audio)) = inputs else {
            return;
        };
        let frames = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let result = layout.pcm_range(&audio, 1, frames);
        assert!(result.is_ok());
        let Ok(range) = result else {
            return;
        };
        assert_eq!(range, 2..6);
        assert_eq!(
            audio
                .samples()
                .skip(range.start)
                .take(range.len())
                .collect::<Vec<_>>(),
            [1, -1, 0, 2]
        );
        assert_eq!(
            layout.pcm_range(&audio, 2, frames),
            Err(PlaybackError::InvalidFrames)
        );
        assert_eq!(
            layout.pcm_range(&audio, u64::MAX, frames),
            Err(PlaybackError::InvalidFrames)
        );
        let incompatible = QueueLayout::new(PlaybackSpec {
            channels: NonZeroU8::MIN,
            ..configuration()
        });
        assert!(incompatible.is_ok());
        let Ok(wrong) = incompatible else {
            return;
        };
        assert_eq!(
            wrong.pcm_range(&audio, 0, frames),
            Err(PlaybackError::FormatMismatch)
        );
    }

    #[test]
    fn device_names_are_bounded_and_nonempty() {
        for name in ["", "bad\0name", &"a".repeat(513)] {
            assert!(!valid_device(name));
        }
        assert!(valid_device("default"));
    }

    #[test]
    fn enumeration_admits_bounded_valid_names() {
        assert_eq!(admit_device_count(MAX_DEVICES), Ok(()));
        for names in
            [None, Some(Vec::new()), Some(vec![String::from("default")])]
        {
            let devices = AudioDevices { driver: "fake", names };
            assert_eq!(validate_devices(devices.clone()), Ok(devices));
        }
    }

    #[test]
    fn enumeration_rejects_malformed_or_oversized_results() {
        assert_eq!(
            admit_device_count(MAX_DEVICES.saturating_add(1)),
            Err(PlaybackError::Capacity)
        );
        let oversized = AudioDevices {
            driver: "fake",
            names: Some(vec![String::from("default"); 65]),
        };
        assert_eq!(validate_devices(oversized), Err(PlaybackError::Capacity));
        for name in ["", "bad\0name", &"a".repeat(513)] {
            let devices = AudioDevices {
                driver: "fake",
                names: Some(vec![name.to_owned()]),
            };
            assert_eq!(
                validate_devices(devices),
                Err(PlaybackError::InvalidDevice)
            );
        }
    }
}
