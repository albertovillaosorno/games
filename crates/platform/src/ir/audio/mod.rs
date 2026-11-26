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
//   - Side effects: Delegates admitted playback requests to a private backend.
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

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};

use games_assets::pcm::PcmSamples;

use super::security::audio::{
    AdmittedSamples, PlaybackRequest, QueueLayout, validate_devices,
};

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
    /// The backend reported queue occupancy outside the admitted layout.
    InvalidQueue,
    /// The requested playback configuration is not representable.
    InvalidSpec,
    /// No playback adapter is available for the selected host.
    Unsupported,
}

/// The current native audio backend and its enumerated playback devices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioDevices {
    /// Adapter diagnostic identifier.
    pub driver: &'static str,
    /// Exact output names, or None when enumeration is unsupported.
    pub names: Option<Vec<String>>,
}

/// Adapter port for an admitted, exact-format playback queue.
pub(crate) trait PlaybackBackend: Debug {
    /// Clear pending frames without changing pause state.
    fn clear(&self);
    /// Backend and requested device diagnostics.
    fn output(&self) -> (&str, Option<&str>);
    /// Stop consuming queued samples.
    fn pause(&self);
    /// Submit admitted complete frames.
    fn queue_samples(
        &self,
        samples: AdmittedSamples<'_>,
    ) -> Result<(), PlaybackError>;
    /// Observe current software-queue occupancy.
    fn queued_bytes(&self) -> u32;
    /// Begin consuming queued samples.
    fn resume(&self);
}

pub(crate) type PlaybackPort = Box<dyn PlaybackBackend>;

/// Platform composition implements the admitted playback factory privately.
pub(crate) trait PlaybackFactory {
    fn devices() -> Result<AudioDevices, PlaybackError>;
    fn open(
        request: PlaybackRequest<'_>,
    ) -> Result<PlaybackPort, PlaybackError>;
}

/// An exact-format playback handle, initially paused, with a bounded queue.
///
/// The backend and device are private. The reported queue size excludes
/// samples already transferred to the physical device or audio server.
pub struct NativeAudio {
    backend: PlaybackPort,
    layout: QueueLayout,
    scratch: Vec<i16>,
}

impl Debug for NativeAudio {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAudio")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl NativeAudio {
    /// Discard pending software-queued frames without changing pause state.
    pub fn clear(&self) {
        self.backend.clear();
    }

    /// Open the default device at the explicitly requested signed PCM format.
    ///
    /// # Errors
    /// Rejects invalid configuration, unavailable devices or format changes.
    pub fn open(spec: PlaybackSpec) -> Result<Self, PlaybackError> {
        Self::open_on(spec, None)
    }

    /// Open an exact named device or the system default.
    /// # Errors
    /// Rejects malformed names, unavailable devices and format changes.
    pub fn open_on(
        spec: PlaybackSpec,
        name: Option<&str>,
    ) -> Result<Self, PlaybackError> {
        let request = PlaybackRequest::new(spec, name)?;
        let backend = <() as PlaybackFactory>::open(request)?;
        let layout = request.layout();
        Ok(Self {
            backend,
            layout,
            scratch: Vec::new(),
        })
    }

    /// Backend identifier and requested device name. None means the system
    /// default, whose physical route may change outside this process.
    #[must_use]
    pub fn output(&self) -> (&str, Option<&str>) {
        self.backend.output()
    }

    /// Stop consuming software-queued frames.
    pub fn pause(&self) {
        self.backend.pause();
    }

    /// Enumerate playback devices without opening or playing on them.
    /// # Errors
    /// Reports backend failure or enumeration beyond the bounded limit.
    pub fn playback_devices() -> Result<AudioDevices, PlaybackError> {
        validate_devices(<() as PlaybackFactory>::devices()?)
    }

    /// Decode and enqueue a verified canonical PCM frame range.
    ///
    /// No sample conversion other than little-endian decoding is performed.
    /// Scratch storage is reused, and only the admitted range is decoded.
    ///
    /// # Errors
    /// Rejects incompatible PCM, invalid ranges, queue or backend failure.
    pub fn queue_pcm(
        &mut self,
        pcm: &PcmSamples<'_>,
        start: u64,
        frames: NonZeroU32,
    ) -> Result<(), PlaybackError> {
        let range = self.layout.pcm_range(pcm, start, frames)?;
        let queued = self.backend.queued_bytes();
        self.layout.admit(range.len(), queued)?;
        self.scratch.clear();
        self.scratch
            .try_reserve_exact(range.len())
            .map_err(|_error| PlaybackError::Allocation)?;
        self.scratch
            .extend(pcm.samples().skip(range.start).take(range.len()));
        let admitted = self.layout.admit_samples(&self.scratch, queued)?;
        self.backend.queue_samples(admitted)
    }

    /// Enqueue already mixed interleaved signed 16-bit sample frames.
    ///
    /// # Errors
    /// Rejects empty or incomplete frames, queue or backend failure.
    pub fn queue_samples(&self, samples: &[i16]) -> Result<(), PlaybackError> {
        let admitted = self
            .layout
            .admit_samples(samples, self.backend.queued_bytes())?;
        self.backend.queue_samples(admitted)
    }

    /// Number of complete sample frames pending in the software queue.
    /// # Errors
    /// Rejects malformed or out-of-budget backend observations.
    pub fn queued_frames(&self) -> Result<u32, PlaybackError> {
        self.layout.queued_frames(self.backend.queued_bytes())
    }

    /// Begin consuming software-queued frames.
    pub fn resume(&self) {
        self.backend.resume();
    }
}

#[cfg(test)]
/// Shared synthetic PCM fixtures for hardware contract and backend tests.
pub mod fixtures {
    use std::num::NonZeroU64;

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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::fixtures::{configuration, pcm};
    use super::*;

    #[derive(Debug)]
    struct FakeBackend {
        observation: Cell<Option<u32>>,
        paused: Cell<bool>,
        samples: RefCell<Vec<i16>>,
        submission_fails: Cell<bool>,
    }

    impl PlaybackBackend for Rc<FakeBackend> {
        fn clear(&self) {
            self.samples.borrow_mut().clear();
        }

        fn output(&self) -> (&str, Option<&str>) {
            ("fake", None)
        }

        fn pause(&self) {
            self.paused.set(true);
        }

        fn queue_samples(
            &self,
            samples: AdmittedSamples<'_>,
        ) -> Result<(), PlaybackError> {
            if self.submission_fails.get() {
                return Err(PlaybackError::Backend(String::from("rejected")));
            }
            self.samples
                .borrow_mut()
                .extend_from_slice(samples.samples());
            Ok(())
        }

        fn queued_bytes(&self) -> u32 {
            self.observation.get().unwrap_or_else(|| {
                u32::try_from(self.samples.borrow().len())
                    .unwrap_or(u32::MAX)
                    .saturating_mul(2)
            })
        }

        fn resume(&self) {
            self.paused.set(false);
        }
    }

    type FakePlayback = (NativeAudio, Rc<FakeBackend>);

    fn fake() -> Result<FakePlayback, PlaybackError> {
        let backend = Rc::new(FakeBackend {
            observation: Cell::new(None),
            paused: Cell::new(true),
            samples: RefCell::new(Vec::new()),
            submission_fails: Cell::new(false),
        });
        let handle = NativeAudio {
            backend: Box::new(Rc::clone(&backend)),
            layout: QueueLayout::new(configuration())?,
            scratch: Vec::new(),
        };
        Ok((handle, backend))
    }

    #[test]
    fn malformed_requests_fail_before_opening_any_backend() {
        for name in ["", "bad\0name", &"a".repeat(513)] {
            assert!(matches!(
                NativeAudio::open_on(configuration(), Some(name)),
                Err(PlaybackError::InvalidDevice)
            ));
        }
        assert!(matches!(
            NativeAudio::open(PlaybackSpec {
                max_queued_frames: NonZeroU32::MIN,
                ..configuration()
            }),
            Err(PlaybackError::InvalidSpec)
        ));
    }

    #[test]
    fn verified_pcm_and_mixed_frames_share_queue_admission()
    -> Result<(), PlaybackError> {
        let (mut handle, state) = fake()?;
        let source = pcm()?;
        handle.queue_pcm(&source, 1, NonZeroU32::MIN)?;
        if state.samples.borrow().as_slice() != [1, -1] {
            return Err(PlaybackError::FormatMismatch);
        }
        handle.queue_samples(&[2, 3, 4, 5])?;
        if handle.queued_frames()? != 3
            || handle.queue_samples(&[6, 7]) != Err(PlaybackError::Capacity)
            || handle.queue_samples(&[6]) != Err(PlaybackError::InvalidFrames)
            || state.samples.borrow().as_slice() != [1, -1, 2, 3, 4, 5]
        {
            return Err(PlaybackError::InvalidFrames);
        }
        Ok(())
    }

    #[test]
    fn malformed_queue_observations_never_dispatch_samples()
    -> Result<(), PlaybackError> {
        let (mut handle, state) = fake()?;
        let source = pcm()?;
        for bytes in [1, 2, 5, 16, u32::MAX] {
            state.observation.set(Some(bytes));
            if handle.queued_frames() != Err(PlaybackError::InvalidQueue)
                || handle.queue_samples(&[1, -1])
                    != Err(PlaybackError::InvalidQueue)
                || handle.queue_pcm(&source, 0, NonZeroU32::MIN)
                    != Err(PlaybackError::InvalidQueue)
                || !state.samples.borrow().is_empty()
            {
                return Err(PlaybackError::Backend(format!(
                    "invalid observation admitted: {bytes}"
                )));
            }
        }
        state.observation.set(None);
        handle.queue_pcm(&source, 0, NonZeroU32::MIN)?;
        if handle.queued_frames()? != 1 {
            return Err(PlaybackError::InvalidQueue);
        }
        Ok(())
    }

    #[test]
    fn backend_rejection_is_reported_without_consuming_queue_budget()
    -> Result<(), PlaybackError> {
        let (mut handle, state) = fake()?;
        let source = pcm()?;
        state.submission_fails.set(true);
        let error = PlaybackError::Backend(String::from("rejected"));
        if handle.queue_samples(&[1, -1]) != Err(error.clone())
            || handle.queue_pcm(&source, 1, NonZeroU32::MIN) != Err(error)
            || handle.queued_frames()? != 0
        {
            return Err(PlaybackError::InvalidQueue);
        }
        state.submission_fails.set(false);
        handle.queue_pcm(&source, 1, NonZeroU32::MIN)?;
        if handle.queued_frames()? != 1
            || state.samples.borrow().as_slice() != [1, -1]
        {
            return Err(PlaybackError::InvalidFrames);
        }
        Ok(())
    }

    #[test]
    fn playback_control_is_independent_of_queued_frames()
    -> Result<(), PlaybackError> {
        let (handle, state) = fake()?;
        if handle.output() != ("fake", None) || !state.paused.get() {
            return Err(PlaybackError::InvalidSpec);
        }
        handle.queue_samples(&[1, -1])?;
        handle.resume();
        if state.paused.get() {
            return Err(PlaybackError::InvalidSpec);
        }
        handle.clear();
        if handle.queued_frames()? != 0 || state.paused.get() {
            return Err(PlaybackError::InvalidFrames);
        }
        handle.pause();
        if !state.paused.get() {
            return Err(PlaybackError::InvalidSpec);
        }
        Ok(())
    }
}
