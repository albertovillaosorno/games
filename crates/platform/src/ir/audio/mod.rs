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

#[cfg(any(test, feature = "test-fixtures"))]
/// Isolated host diagnostics and synthetic PCM test fixtures.
pub mod fixtures {
    #[cfg(test)]
    use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64};

    #[cfg(test)]
    use games_assets::{
        AssetFormat, AssetRecord, ContentHash, pcm::PcmSamples,
    };

    use super::PlaybackError;
    #[cfg(test)]
    use super::PlaybackSpec;

    pub(crate) trait AudioFixturePort {
        fn verify_playback() -> Result<(), PlaybackError>;
    }

    /// Start an isolated asynchronous host playback diagnostic.
    /// # Errors
    /// Reports unavailable diagnostics or synchronous admission failure.
    pub fn verify_playback() -> Result<(), PlaybackError> {
        <() as AudioFixturePort>::verify_playback()
    }

    #[cfg(test)]
    const PAYLOAD: [u8; 12] = [0, 128, 255, 127, 1, 0, 255, 255, 0, 0, 2, 0];

    #[cfg(test)]
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

    #[cfg(test)]
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

use std::cell::Cell;
use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};

use games_assets::pcm::PcmSamples;

use super::security::audio::{
    AdmittedSamples, PlaybackObservation, PlaybackRequest, PlaybackState,
    QueueLayout, validate_devices,
};

/// Explicit audio playback configuration and software-queue budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackSpec {
    /// Requested native buffer period in sample frames; a power of two.
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
    /// The observed device state does not match the admitted playback state.
    InvalidState,
    /// A previously requested pause or resume has not settled.
    TransitionPending,
    /// No playback adapter is available for the selected host.
    Unsupported,
}

/// Receipt for a pause or resume request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackControl {
    /// The requested state has been observed and admitted.
    Complete,
    /// The host has not completed the request; poll before further control.
    Pending,
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
    fn clear(&self) -> Result<(), PlaybackError>;
    /// Observe the application-facing format and current device state.
    fn observe_playback(&self) -> Result<PlaybackObservation, PlaybackError>;
    /// Backend and requested device diagnostics.
    fn output(&self) -> (&str, Option<&str>);
    /// Poll a previously pending control request without blocking.
    fn poll_control(&self) -> Result<PlaybackControl, PlaybackError>;
    /// Submit admitted complete frames.
    fn queue_samples(
        &self,
        samples: AdmittedSamples<'_>,
    ) -> Result<(), PlaybackError>;
    /// Observe current software-queue occupancy.
    fn queued_bytes(&self) -> Result<u32, PlaybackError>;
    /// Request a pause or resume, possibly completing asynchronously.
    fn request_control(
        &self,
        next: PlaybackState,
    ) -> Result<PlaybackControl, PlaybackError>;
}

pub(crate) type PlaybackPort = Box<dyn PlaybackBackend>;

/// Platform composition implements the admitted playback factory privately.
pub(crate) trait PlaybackFactory {
    fn devices() -> Result<AudioDevices, PlaybackError>;
    fn open(
        request: PlaybackRequest<'_>,
    ) -> Result<PlaybackPort, PlaybackError>;
}

#[derive(Clone, Copy, Debug)]
struct PendingControl {
    destination: PlaybackState,
    maximum_frames: u32,
    may_consume: bool,
}

/// An exact-format playback handle, initially paused, with a bounded queue.
///
/// The backend and device are private. The reported queue charge excludes
/// samples no longer tracked by the adapter. A backend may retain an entire
/// block charge until its consumption is observable.
/// A failed receipt reports an inconsistent device; it cannot retract
/// samples already copied by the adapter or played by the device.
pub struct NativeAudio {
    backend: PlaybackPort,
    layout: QueueLayout,
    pending: Cell<Option<PendingControl>>,
    scratch: Vec<i16>,
    state: Cell<PlaybackState>,
}

impl Debug for NativeAudio {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAudio")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl NativeAudio {
    fn admit_control(
        &self,
        mut request: PendingControl,
        result: Result<PlaybackControl, PlaybackError>,
    ) -> Result<PlaybackControl, PlaybackError> {
        let previous = self.state.get();
        let observed = self.backend.observe_playback()?;
        let expected = if matches!(&result, Ok(PlaybackControl::Complete))
            || (matches!(&result, Ok(PlaybackControl::Pending))
                && observed.state == request.destination)
        {
            request.destination
        } else {
            previous
        };
        self.layout.admit_observation(observed, expected)?;
        let bytes = self.backend.queued_bytes()?;
        self.layout.admit_queue_receipt(
            request.maximum_frames,
            bytes,
            request.may_consume,
        )?;
        request.maximum_frames = self.layout.queued_frames(bytes)?;
        match &result {
            Ok(PlaybackControl::Pending) => self.pending.set(Some(request)),
            Ok(PlaybackControl::Complete) => {
                self.state.set(request.destination);
                self.pending.set(None);
            },
            Err(_) => self.pending.set(None),
        }
        result
    }

    fn admit_opened(
        request: PlaybackRequest<'_>,
        backend: PlaybackPort,
    ) -> Result<Self, PlaybackError> {
        request.admit_opened(
            backend.observe_playback()?,
            backend.output().1,
            backend.queued_bytes()?,
        )?;
        Ok(Self {
            backend,
            layout: request.layout(),
            pending: Cell::new(None),
            scratch: Vec::new(),
            state: Cell::new(PlaybackState::Paused),
        })
    }

    /// Discard pending software-queued frames without changing playback state.
    /// # Errors
    /// Rejects unexpected device state or a nonempty queue receipt.
    pub fn clear(&self) -> Result<(), PlaybackError> {
        self.observe_state()?;
        self.backend.clear()?;
        self.observe_state()?;
        self.layout
            .admit_queue_receipt(0, self.backend.queued_bytes()?, false)
    }

    fn observe_state(&self) -> Result<(), PlaybackError> {
        if self.pending.get().is_some() {
            return Err(PlaybackError::TransitionPending);
        }
        self.layout.admit_observation(
            self.backend.observe_playback()?,
            self.state.get(),
        )
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
        Self::admit_opened(request, <() as PlaybackFactory>::open(request)?)
    }

    /// Backend identifier and requested device name. None means the system
    /// default, whose physical route may change outside this process.
    #[must_use]
    pub fn output(&self) -> (&str, Option<&str>) {
        self.backend.output()
    }

    /// Request a pause in software-queued frame consumption.
    ///
    /// A pending receipt requires polling; it does not confirm a pause.
    /// # Errors
    /// Rejects pending control, unexpected device state or queue receipt.
    pub fn pause(&self) -> Result<PlaybackControl, PlaybackError> {
        self.transition(PlaybackState::Paused)
    }

    /// Enumerate playback devices without opening or playing on them.
    /// # Errors
    /// Reports backend failure or enumeration beyond the bounded limit.
    pub fn playback_devices() -> Result<AudioDevices, PlaybackError> {
        validate_devices(<() as PlaybackFactory>::devices()?)
    }

    /// Poll pending playback control without blocking or issuing a new request.
    ///
    /// A pending receipt prevents queue operations and new controls until the
    /// backend settles and the resulting state and occupancy are admitted.
    /// With no pending request, this observes the confirmed device state.
    /// # Errors
    /// Rejects backend failure, unexpected device state or queue receipt.
    pub fn poll_control(&self) -> Result<PlaybackControl, PlaybackError> {
        if let Some(request) = self.pending.get() {
            self.admit_control(request, self.backend.poll_control())
        } else {
            self.observe_state()?;
            Ok(PlaybackControl::Complete)
        }
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
        self.observe_state()?;
        let _maximum = self
            .layout
            .admit(range.len(), self.backend.queued_bytes()?)?;
        self.scratch.clear();
        self.scratch
            .try_reserve_exact(range.len())
            .map_err(|_error| PlaybackError::Allocation)?;
        self.scratch
            .extend(pcm.samples().skip(range.start).take(range.len()));
        self.queue_samples(&self.scratch)
    }

    /// Enqueue already mixed interleaved signed 16-bit sample frames.
    ///
    /// # Errors
    /// Rejects empty or incomplete frames, queue or backend failure.
    pub fn queue_samples(&self, samples: &[i16]) -> Result<(), PlaybackError> {
        self.observe_state()?;
        let state = self.state.get();
        let admitted = self
            .layout
            .admit_samples(samples, self.backend.queued_bytes()?)?;
        self.backend.queue_samples(admitted)?;
        self.observe_state()?;
        self.layout.admit_submission(
            admitted,
            self.backend.queued_bytes()?,
            state,
        )
    }

    /// Number of complete sample frames charged to the software queue.
    /// # Errors
    /// Rejects malformed or out-of-budget backend observations.
    pub fn queued_frames(&self) -> Result<u32, PlaybackError> {
        self.observe_state()?;
        self.layout.queued_frames(self.backend.queued_bytes()?)
    }

    /// Poll any outstanding control, then admit the desired consumption state.
    ///
    /// Repeated requests for the confirmed state do not dispatch another
    /// control. A new desired state waits for an earlier pending receipt.
    /// # Errors
    /// Reports an invalid observation, queue receipt or backend failure.
    pub fn reconcile_running(
        &self,
        running: bool,
    ) -> Result<PlaybackControl, PlaybackError> {
        if self.pending.get().is_some()
            && self.poll_control()? == PlaybackControl::Pending
        {
            return Ok(PlaybackControl::Pending);
        }
        let next = if running {
            PlaybackState::Running
        } else {
            PlaybackState::Paused
        };
        if self.state.get() == next {
            self.observe_state()?;
            Ok(PlaybackControl::Complete)
        } else {
            self.transition(next)
        }
    }

    /// Request consumption of software-queued frames.
    ///
    /// A pending receipt requires polling; it does not confirm playback.
    /// # Errors
    /// Rejects pending control, unexpected device state or queue receipt.
    pub fn resume(&self) -> Result<PlaybackControl, PlaybackError> {
        self.transition(PlaybackState::Running)
    }

    fn transition(
        &self,
        next: PlaybackState,
    ) -> Result<PlaybackControl, PlaybackError> {
        self.observe_state()?;
        let previous = self.state.get();
        if next == PlaybackState::Stopped {
            return Err(PlaybackError::InvalidState);
        }
        let request = PendingControl {
            destination: next,
            maximum_frames: self
                .layout
                .queued_frames(self.backend.queued_bytes()?)?,
            may_consume: previous == PlaybackState::Running
                || next == PlaybackState::Running,
        };
        let result = self.backend.request_control(next);
        if matches!(&result, Ok(PlaybackControl::Pending)) {
            // A malformed pending observation must not permit another dispatch.
            self.pending.set(Some(request));
        }
        self.admit_control(request, result)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::fixtures::{configuration, pcm};
    use super::*;

    type ControlResult = Result<PlaybackControl, PlaybackError>;

    #[derive(Debug)]
    struct FakeBackend {
        after_submission: Cell<Option<u32>>,
        control_ignored: Cell<bool>,
        control_pending: Cell<bool>,
        control_result: RefCell<Option<ControlResult>>,
        observation: Cell<Option<u32>>,
        observation_fails: Cell<bool>,
        obtained: Cell<PlaybackObservation>,
        queue_reads_before_failure: Cell<Option<u32>>,
        requested_name: Option<&'static str>,
        samples: RefCell<Vec<i16>>,
        state: Cell<PlaybackState>,
        state_after_submission: Cell<Option<PlaybackState>>,
        submission_fails: Cell<bool>,
    }

    impl PlaybackBackend for Rc<FakeBackend> {
        fn clear(&self) -> Result<(), PlaybackError> {
            if !self.control_ignored.get() {
                self.samples.borrow_mut().clear();
            }
            Ok(())
        }

        fn observe_playback(
            &self,
        ) -> Result<PlaybackObservation, PlaybackError> {
            if self.observation_fails.get() {
                return Err(PlaybackError::Backend(String::from(
                    "format query failed",
                )));
            }
            Ok(PlaybackObservation {
                state: self.state.get(),
                ..self.obtained.get()
            })
        }

        fn output(&self) -> (&str, Option<&str>) {
            ("fake", self.requested_name)
        }

        fn poll_control(&self) -> Result<PlaybackControl, PlaybackError> {
            self.control_result
                .borrow_mut()
                .take()
                .unwrap_or(Ok(PlaybackControl::Pending))
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
            if let Some(bytes) = self.after_submission.take() {
                self.observation.set(Some(bytes));
            }
            if let Some(state) = self.state_after_submission.take() {
                self.state.set(state);
            }
            Ok(())
        }

        fn queued_bytes(&self) -> Result<u32, PlaybackError> {
            let remaining = self.queue_reads_before_failure.get();
            if remaining == Some(0) {
                return Err(PlaybackError::Backend(String::from(
                    "queue query failed",
                )));
            }
            self.queue_reads_before_failure
                .set(remaining.map(|count| count.saturating_sub(1)));
            Ok(self.observation.get().unwrap_or_else(|| {
                u32::try_from(self.samples.borrow().len())
                    .unwrap_or(u32::MAX)
                    .saturating_mul(2)
            }))
        }

        fn request_control(
            &self,
            next: PlaybackState,
        ) -> Result<PlaybackControl, PlaybackError> {
            if self.control_pending.get() {
                return Ok(PlaybackControl::Pending);
            }
            if !self.control_ignored.get() {
                self.state.set(next);
            }
            Ok(PlaybackControl::Complete)
        }
    }

    type FakePlayback = (NativeAudio, Rc<FakeBackend>);

    fn verify<T>(actual: &T, expected: &T) -> Result<(), PlaybackError>
    where
        T: Debug + PartialEq + ?Sized,
    {
        if actual == expected {
            Ok(())
        } else {
            Err(PlaybackError::Backend(format!(
                "{actual:?} != {expected:?}"
            )))
        }
    }

    fn fake_backend(requested_name: Option<&'static str>) -> Rc<FakeBackend> {
        Rc::new(FakeBackend {
            after_submission: Cell::new(None),
            control_ignored: Cell::new(false),
            control_pending: Cell::new(false),
            control_result: RefCell::new(None),
            observation: Cell::new(None),
            observation_fails: Cell::new(false),
            obtained: Cell::new(PlaybackObservation {
                buffer_bytes: 8,
                buffer_frames: 2,
                channels: 2,
                frequency: 48_000,
                signed_native_16: true,
                state: PlaybackState::Paused,
            }),
            queue_reads_before_failure: Cell::new(None),
            requested_name,
            samples: RefCell::new(Vec::new()),
            state: Cell::new(PlaybackState::Paused),
            state_after_submission: Cell::new(None),
            submission_fails: Cell::new(false),
        })
    }

    fn fake() -> Result<FakePlayback, PlaybackError> {
        let backend = fake_backend(None);
        let request = PlaybackRequest::new(configuration(), None)?;
        let handle =
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend)))?;
        Ok((handle, backend))
    }

    #[test]
    fn queue_receipts_allow_only_admitted_frames_and_running_consumption()
    -> Result<(), PlaybackError> {
        for (running, bytes, expected) in [
            (false, 0, Err(PlaybackError::InvalidQueue)),
            (false, 4, Err(PlaybackError::InvalidQueue)),
            (false, 8, Ok(())),
            (false, 12, Err(PlaybackError::InvalidQueue)),
            (true, 0, Ok(())),
            (true, 4, Ok(())),
            (true, 8, Ok(())),
            (true, 12, Err(PlaybackError::InvalidQueue)),
            (true, 6, Err(PlaybackError::InvalidQueue)),
            (true, 16, Err(PlaybackError::InvalidQueue)),
        ] {
            let (mut handle, backend) = fake()?;
            handle.queue_samples(&[2, 3])?;
            if running {
                verify(&handle.resume()?, &PlaybackControl::Complete)?;
            }
            backend.after_submission.set(Some(bytes));
            let source = pcm()?;
            let actual = handle.queue_pcm(&source, 1, NonZeroU32::MIN);
            if actual != expected
                || backend.samples.borrow().as_slice() != [2, 3, 1, -1]
            {
                return Err(PlaybackError::Backend(format!(
                    "{running}, {bytes}: {actual:?} != {expected:?}"
                )));
            }
        }
        Ok(())
    }

    #[test]
    fn failed_controls_do_not_commit_playback_state()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        handle.queue_samples(&[1, -1])?;
        backend.control_ignored.set(true);
        if handle.resume() != Err(PlaybackError::InvalidState)
            || handle.clear() != Err(PlaybackError::InvalidQueue)
            || handle.queued_frames()? != 1
        {
            return Err(PlaybackError::InvalidState);
        }
        backend.control_ignored.set(false);
        verify(&handle.resume()?, &PlaybackControl::Complete)?;
        backend.control_ignored.set(true);
        if handle.pause() != Err(PlaybackError::InvalidState) {
            return Err(PlaybackError::InvalidState);
        }
        handle.queue_samples(&[2, 3])?;
        backend.control_ignored.set(false);
        verify(&handle.pause()?, &PlaybackControl::Complete)?;
        handle.clear()?;
        if handle.queued_frames()? != 0
            || backend.state.get() != PlaybackState::Paused
        {
            return Err(PlaybackError::InvalidState);
        }
        Ok(())
    }

    #[test]
    fn changed_format_or_stopped_device_rejects_dispatch()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        let admitted = backend.obtained.get();
        backend.obtained.set(PlaybackObservation {
            frequency: 44_100,
            ..admitted
        });
        if handle.queue_samples(&[1, -1]) != Err(PlaybackError::FormatMismatch)
            || !backend.samples.borrow().is_empty()
        {
            return Err(PlaybackError::FormatMismatch);
        }
        backend.obtained.set(admitted);
        verify(&handle.resume()?, &PlaybackControl::Complete)?;
        backend
            .state_after_submission
            .set(Some(PlaybackState::Stopped));
        if handle.queue_samples(&[1, -1]) != Err(PlaybackError::InvalidState)
            || handle.queue_samples(&[2, 3]) != Err(PlaybackError::InvalidState)
            || handle.pause() != Err(PlaybackError::InvalidState)
            || handle.clear() != Err(PlaybackError::InvalidState)
            || handle.queued_frames() != Err(PlaybackError::InvalidState)
            || backend.samples.borrow().as_slice() != [1, -1]
        {
            return Err(PlaybackError::InvalidState);
        }
        Ok(())
    }

    #[test]
    fn desired_control_waits_and_reverses_without_duplicate_dispatch()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        handle.queue_samples(&[1, -1])?;
        backend.control_pending.set(true);
        verify(
            &handle.reconcile_running(false)?,
            &PlaybackControl::Complete,
        )?;
        for running in [true, true, false] {
            verify(
                &handle.reconcile_running(running)?,
                &PlaybackControl::Pending,
            )?;
            verify(&handle.clear(), &Err(PlaybackError::TransitionPending))?;
        }
        backend.state.set(PlaybackState::Running);
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.reconcile_running(false)?, &PlaybackControl::Pending)?;
        verify(&handle.reconcile_running(false)?, &PlaybackControl::Pending)?;
        verify(
            &handle.queued_frames(),
            &Err(PlaybackError::TransitionPending),
        )?;
        backend.state.set(PlaybackState::Paused);
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        for _attempt in 0..2u8 {
            verify(
                &handle.reconcile_running(false)?,
                &PlaybackControl::Complete,
            )?;
        }
        verify(&handle.queued_frames()?, &1)?;
        verify(backend.samples.borrow().as_slice(), &[1, -1])?;
        backend.state.set(PlaybackState::Stopped);
        verify(
            &handle.reconcile_running(false),
            &Err(PlaybackError::InvalidState),
        )
    }

    #[test]
    fn pending_control_blocks_queue_operations_until_settled()
    -> Result<(), PlaybackError> {
        let (mut handle, backend) = fake()?;
        let source = pcm()?;
        handle.queue_samples(&[1, -1, 2, -2])?;
        backend.control_pending.set(true);
        verify(&handle.resume()?, &PlaybackControl::Pending)?;
        verify(&handle.poll_control()?, &PlaybackControl::Pending)?;
        verify(
            &handle.queued_frames(),
            &Err(PlaybackError::TransitionPending),
        )?;
        verify(&handle.clear(), &Err(PlaybackError::TransitionPending))?;
        verify(&handle.pause(), &Err(PlaybackError::TransitionPending))?;
        verify(&handle.resume(), &Err(PlaybackError::TransitionPending))?;
        verify(
            &handle.queue_samples(&[3, -3]),
            &Err(PlaybackError::TransitionPending),
        )?;
        verify(
            &handle.queue_pcm(&source, 0, NonZeroU32::MIN),
            &Err(PlaybackError::TransitionPending),
        )?;
        verify(backend.samples.borrow().as_slice(), &[1, -1, 2, -2])?;
        backend.state.set(PlaybackState::Running);
        backend.observation.set(Some(4));
        verify(&handle.poll_control()?, &PlaybackControl::Pending)?;
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        verify(&handle.queued_frames()?, &1)?;
        verify(&handle.pause()?, &PlaybackControl::Pending)?;
        backend.state.set(PlaybackState::Paused);
        backend.observation.set(Some(0));
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        verify(&handle.queued_frames()?, &0)?;
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        Ok(())
    }

    #[test]
    fn pending_control_rejects_queue_growth_and_paused_drain()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        handle.queue_samples(&[1, -1, 2, -2])?;
        backend.control_pending.set(true);
        verify(&handle.pause()?, &PlaybackControl::Pending)?;
        for bytes in [4, 12, 9] {
            backend.observation.set(Some(bytes));
            verify(&handle.poll_control(), &Err(PlaybackError::InvalidQueue))?;
            verify(&handle.clear(), &Err(PlaybackError::TransitionPending))?;
        }
        backend.observation.set(Some(8));
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        verify(&handle.resume()?, &PlaybackControl::Pending)?;
        backend.state.set(PlaybackState::Running);
        backend.observation.set(Some(4));
        verify(&handle.poll_control()?, &PlaybackControl::Pending)?;
        backend.observation.set(Some(8));
        verify(&handle.poll_control(), &Err(PlaybackError::InvalidQueue))?;
        backend.observation.set(Some(0));
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        verify(&handle.queued_frames()?, &0)?;
        Ok(())
    }

    #[test]
    fn rejected_pending_control_restores_only_confirmed_state()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        handle.queue_samples(&[1, -1])?;
        backend.control_pending.set(true);
        verify(&handle.resume()?, &PlaybackControl::Pending)?;
        let failure = PlaybackError::Backend(String::from("denied"));
        drop(backend.control_result.replace(Some(Err(failure.clone()))));
        verify(&handle.poll_control(), &Err(failure))?;
        verify(&handle.queued_frames()?, &1)?;
        backend.control_pending.set(false);
        verify(&handle.resume()?, &PlaybackControl::Complete)?;
        backend.control_pending.set(true);
        verify(&handle.pause()?, &PlaybackControl::Pending)?;
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control(), &Err(PlaybackError::InvalidState))?;
        verify(&handle.clear(), &Err(PlaybackError::TransitionPending))?;
        Ok(())
    }

    #[test]
    fn incompatible_obtained_formats_never_expose_a_playback_handle()
    -> Result<(), PlaybackError> {
        let backend = fake_backend(None);
        let request = PlaybackRequest::new(configuration(), None)?;
        let valid = backend.obtained.get();
        for observation in [
            PlaybackObservation {
                frequency: 44_100,
                ..valid
            },
            PlaybackObservation { channels: 1, ..valid },
            PlaybackObservation {
                signed_native_16: false,
                ..valid
            },
            PlaybackObservation {
                buffer_bytes: 0,
                buffer_frames: 0,
                ..valid
            },
            PlaybackObservation { buffer_bytes: 7, ..valid },
        ] {
            backend.obtained.set(observation);
            if !matches!(
                NativeAudio::admit_opened(
                    request,
                    Box::new(Rc::clone(&backend))
                ),
                Err(PlaybackError::FormatMismatch)
            ) || Rc::strong_count(&backend) != 1
            {
                return Err(PlaybackError::FormatMismatch);
            }
        }
        backend.obtained.set(PlaybackObservation {
            buffer_bytes: 16,
            buffer_frames: 4,
            ..valid
        });
        let opened =
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend)))?;
        if opened.queued_frames()? != 0 {
            return Err(PlaybackError::InvalidQueue);
        }
        Ok(())
    }

    #[test]
    fn opening_requires_requested_device_paused_and_empty()
    -> Result<(), PlaybackError> {
        let backend = fake_backend(Some("selected"));
        let request = PlaybackRequest::new(configuration(), Some("selected"))?;
        let wrong = PlaybackRequest::new(configuration(), Some("other"))?;
        if !matches!(
            NativeAudio::admit_opened(wrong, Box::new(Rc::clone(&backend))),
            Err(PlaybackError::InvalidDevice)
        ) {
            return Err(PlaybackError::InvalidDevice);
        }
        backend.state.set(PlaybackState::Running);
        if !matches!(
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend))),
            Err(PlaybackError::InvalidState)
        ) {
            return Err(PlaybackError::InvalidState);
        }
        backend.state.set(PlaybackState::Paused);
        backend.observation.set(Some(4));
        if !matches!(
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend))),
            Err(PlaybackError::InvalidQueue)
        ) {
            return Err(PlaybackError::InvalidQueue);
        }
        backend.observation.set(None);
        let opened =
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend)))?;
        if opened.output() != ("fake", Some("selected")) {
            return Err(PlaybackError::InvalidDevice);
        }
        if opened.queued_frames()? != 0 {
            return Err(PlaybackError::InvalidQueue);
        }
        Ok(())
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
    fn format_query_failure_rejects_open_and_dispatch()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        backend.observation_fails.set(true);
        let failure =
            PlaybackError::Backend(String::from("format query failed"));
        let request = PlaybackRequest::new(configuration(), None)?;
        let opened =
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend)));
        verify(&opened.err(), &Some(failure.clone()))?;
        verify(&handle.queue_samples(&[1, -1]), &Err(failure.clone()))?;
        verify(&handle.resume(), &Err(failure))?;
        verify(&backend.state.get(), &PlaybackState::Paused)?;
        verify(backend.samples.borrow().as_slice(), &[])?;
        backend.observation_fails.set(false);
        verify(&handle.queued_frames()?, &0)
    }

    #[test]
    fn queue_query_failure_never_fabricates_a_submission_receipt()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        let failure =
            PlaybackError::Backend(String::from("queue query failed"));
        backend.queue_reads_before_failure.set(Some(0));
        let request = PlaybackRequest::new(configuration(), None)?;
        let opened =
            NativeAudio::admit_opened(request, Box::new(Rc::clone(&backend)));
        verify(&opened.err(), &Some(failure.clone()))?;
        verify(&handle.queue_samples(&[1, -1]), &Err(failure.clone()))?;
        verify(&handle.resume(), &Err(failure.clone()))?;
        verify(backend.samples.borrow().as_slice(), &[])?;
        verify(&backend.state.get(), &PlaybackState::Paused)?;
        // A failure after submission cannot retract samples already copied.
        backend.queue_reads_before_failure.set(Some(1));
        verify(&handle.queue_samples(&[2, -2]), &Err(failure))?;
        backend.queue_reads_before_failure.set(None);
        verify(backend.samples.borrow().as_slice(), &[2, -2])?;
        verify(&handle.queued_frames()?, &1)
    }

    #[test]
    fn failed_pending_query_does_not_admit_queue_operations()
    -> Result<(), PlaybackError> {
        let (handle, backend) = fake()?;
        handle.queue_samples(&[1, -1])?;
        backend.control_pending.set(true);
        verify(&handle.resume()?, &PlaybackControl::Pending)?;
        backend.queue_reads_before_failure.set(Some(0));
        let failure =
            PlaybackError::Backend(String::from("queue query failed"));
        verify(&handle.poll_control(), &Err(failure))?;
        verify(&handle.clear(), &Err(PlaybackError::TransitionPending))?;
        backend.queue_reads_before_failure.set(None);
        backend.state.set(PlaybackState::Running);
        drop(
            backend
                .control_result
                .replace(Some(Ok(PlaybackControl::Complete))),
        );
        verify(&handle.poll_control()?, &PlaybackControl::Complete)?;
        verify(&handle.queued_frames()?, &1)
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
        if handle.output() != ("fake", None)
            || state.state.get() != PlaybackState::Paused
        {
            return Err(PlaybackError::InvalidSpec);
        }
        handle.queue_samples(&[1, -1])?;
        verify(&handle.resume()?, &PlaybackControl::Complete)?;
        if state.state.get() != PlaybackState::Running {
            return Err(PlaybackError::InvalidSpec);
        }
        handle.clear()?;
        if handle.queued_frames()? != 0
            || state.state.get() != PlaybackState::Running
        {
            return Err(PlaybackError::InvalidFrames);
        }
        verify(&handle.pause()?, &PlaybackControl::Complete)?;
        if state.state.get() != PlaybackState::Paused {
            return Err(PlaybackError::InvalidSpec);
        }
        Ok(())
    }
}
