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
//   - A bounded SDL2 playback queue and reusable PCM decoding storage.
// - Must-Not:
//   - Resample, mix, interpret original sound hardware or advance game time.
// - Allows:
//   - Inputs: Explicit playback configuration and complete PCM frames.
//   - Outputs: Queue occupancy, admission and playback control.
//   - Side effects: Native audio device enumeration, creation and sample
//     submission.
// - Split-When:
//   - Independent native audio backends require separate adapters.
// - Merge-When:
//   - Another adapter duplicates bounded native audio submission.
// - Summary:
//   - Exact-format native PCM playback with explicit backpressure.
// - Description:
//   - Checks capacity before decoding or submitting a frame range.
// - Usage:
//   - Open a paused device, queue complete frames and explicitly resume.
// - Defaults:
//   - Reject implicit sample-rate or channel conversion.

//! Bounded SDL2 audio playback on Linux.

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroUsize};
use std::ops::Range;

use games_assets::pcm::PcmSamples;
use sdl2::audio::{AudioFormat, AudioQueue, AudioSpecDesired};

const MAX_DEVICE_NAME: usize = 512;
const MAX_DEVICES: u32 = 64;

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
    /// SDL backend identifier.
    pub driver: &'static str,
    /// Exact output names, or None when enumeration is unsupported.
    pub names: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueueLayout {
    channels: usize,
    frequency: i32,
    max_bytes: u32,
    spec: PlaybackSpec,
}

impl QueueLayout {
    fn admit(
        self,
        samples: usize,
        queued_bytes: u32,
    ) -> Result<(), PlaybackError> {
        if samples == 0 || !samples.is_multiple_of(self.channels) {
            return Err(PlaybackError::InvalidFrames);
        }
        let bytes = u32::try_from(samples)
            .ok()
            .and_then(|count| count.checked_mul(2))
            .ok_or(PlaybackError::Capacity)?;
        if bytes > self.max_bytes.saturating_sub(queued_bytes) {
            return Err(PlaybackError::Capacity);
        }
        Ok(())
    }

    fn new(spec: PlaybackSpec) -> Result<Self, PlaybackError> {
        if !spec.buffer_frames.is_power_of_two()
            || u32::from(spec.buffer_frames.get())
                > spec.max_queued_frames.get()
        {
            return Err(PlaybackError::InvalidSpec);
        }
        let frequency = i32::try_from(spec.sample_rate.get())
            .map_err(|_error| PlaybackError::InvalidSpec)?;
        let max_bytes = spec
            .max_queued_frames
            .get()
            .checked_mul(NonZeroU32::from(spec.channels).get())
            .and_then(|count| count.checked_mul(2))
            .ok_or(PlaybackError::InvalidSpec)?;
        Ok(Self {
            channels: NonZeroUsize::from(spec.channels).get(),
            frequency,
            max_bytes,
            spec,
        })
    }

    fn pcm_range(
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
}

/// An exact-format native device, initially paused, with a bounded queue.
///
/// The reported queue size covers SDL's software queue, not samples
/// already transferred to the device or system audio server.
pub struct NativeAudio {
    device: AudioQueue<i16>,
    driver: &'static str,
    layout: QueueLayout,
    requested_device: Option<String>,
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
        self.device.clear();
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
        if name.is_some_and(|value| !valid_device(value)) {
            return Err(PlaybackError::InvalidDevice);
        }
        let layout = QueueLayout::new(spec)?;
        let sdl = sdl2::init().map_err(PlaybackError::Backend)?;
        let audio = sdl.audio().map_err(PlaybackError::Backend)?;
        let desired = AudioSpecDesired {
            freq: Some(layout.frequency),
            channels: Some(spec.channels.get()),
            samples: Some(spec.buffer_frames.get()),
        };
        let device = audio
            .open_queue::<i16, _>(name, &desired)
            .map_err(PlaybackError::Backend)?;
        let obtained = device.spec();
        if obtained.freq != layout.frequency
            || obtained.channels != spec.channels.get()
            || obtained.format != AudioFormat::s16_sys()
        {
            return Err(PlaybackError::FormatMismatch);
        }
        device.pause();
        Ok(Self {
            device,
            driver: audio.current_audio_driver(),
            layout,
            requested_device: name.map(str::to_owned),
            scratch: Vec::new(),
        })
    }

    /// Backend identifier and requested device name. None means the system
    /// default, whose physical route may change outside this process.
    #[must_use]
    pub fn output(&self) -> (&str, Option<&str>) {
        (self.driver, self.requested_device.as_deref())
    }

    /// Stop consuming software-queued frames.
    pub fn pause(&self) {
        self.device.pause();
    }

    /// Enumerate playback devices without opening or playing on them.
    /// # Errors
    /// Reports backend failure or enumeration beyond the bounded limit.
    pub fn playback_devices() -> Result<AudioDevices, PlaybackError> {
        let sdl = sdl2::init().map_err(PlaybackError::Backend)?;
        let audio = sdl.audio().map_err(PlaybackError::Backend)?;
        let names = audio
            .num_audio_playback_devices()
            .map(|count| {
                if count > MAX_DEVICES {
                    return Err(PlaybackError::Capacity);
                }
                (0..count)
                    .map(|index| {
                        validate_device_name(
                            audio
                                .audio_playback_device_name(index)
                                .map_err(PlaybackError::Backend)?,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        Ok(AudioDevices {
            driver: audio.current_audio_driver(),
            names,
        })
    }

    /// Decode and enqueue a verified canonical PCM frame range.
    ///
    /// No sample conversion other than little-endian decoding is performed.
    /// Scratch storage is reused, and only the admitted range is decoded.
    ///
    /// # Errors
    /// Rejects incompatible PCM, invalid ranges, capacity or backend failure.
    pub fn queue_pcm(
        &mut self,
        pcm: &PcmSamples<'_>,
        start: u64,
        frames: NonZeroU32,
    ) -> Result<(), PlaybackError> {
        let range = self.layout.pcm_range(pcm, start, frames)?;
        self.layout.admit(range.len(), self.device.size())?;
        self.scratch.clear();
        self.scratch
            .try_reserve_exact(range.len())
            .map_err(|_error| PlaybackError::Allocation)?;
        self.scratch
            .extend(pcm.samples().skip(range.start).take(range.len()));
        self.device
            .queue_audio(&self.scratch)
            .map_err(PlaybackError::Backend)
    }

    /// Enqueue already mixed interleaved signed 16-bit sample frames.
    ///
    /// # Errors
    /// Rejects empty or incomplete frames, capacity or backend failure.
    pub fn queue_samples(&self, samples: &[i16]) -> Result<(), PlaybackError> {
        self.layout.admit(samples.len(), self.device.size())?;
        self.device
            .queue_audio(samples)
            .map_err(PlaybackError::Backend)
    }

    /// Number of sample bytes pending in the native software queue.
    #[must_use]
    pub fn queued_bytes(&self) -> u32 {
        self.device.size()
    }

    /// Begin consuming software-queued frames.
    pub fn resume(&self) {
        self.device.resume();
    }
}

fn valid_device(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_DEVICE_NAME
        && !name.as_bytes().contains(&0)
}

fn validate_device_name(name: String) -> Result<String, PlaybackError> {
    if !valid_device(&name) {
        return Err(PlaybackError::InvalidDevice);
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::env;

    use games_assets::{AssetFormat, AssetRecord, ContentHash};

    use super::*;

    const PAYLOAD: [u8; 12] = [0, 128, 255, 127, 1, 0, 255, 255, 0, 0, 2, 0];

    fn configuration() -> PlaybackSpec {
        PlaybackSpec {
            buffer_frames: NonZeroU16::new(2).unwrap_or(NonZeroU16::MIN),
            channels: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
            max_queued_frames: NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            sample_rate: NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN),
        }
    }

    fn pcm() -> Result<PcmSamples<'static>, PlaybackError> {
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

    #[test]
    fn rejects_invalid_explicit_device_names_before_opening() {
        for name in ["", "bad\0name", &"a".repeat(513)] {
            assert!(matches!(
                NativeAudio::open_on(configuration(), Some(name)),
                Err(PlaybackError::InvalidDevice)
            ));
        }
    }

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
    fn rejects_incomplete_frames_and_software_queue_overflow() {
        let checked = QueueLayout::new(configuration());
        assert!(checked.is_ok());
        let Ok(layout) = checked else {
            return;
        };
        assert_eq!(layout.admit(0, 0), Err(PlaybackError::InvalidFrames));
        assert_eq!(layout.admit(1, 0), Err(PlaybackError::InvalidFrames));
        assert_eq!(layout.admit(6, 0), Ok(()));
        assert_eq!(layout.admit(4, 4), Ok(()));
        assert_eq!(layout.admit(4, 5), Err(PlaybackError::Capacity));
        assert_eq!(layout.admit(2, u32::MAX), Err(PlaybackError::Capacity));
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

    fn require(condition: bool, reason: &str) -> Result<(), PlaybackError> {
        if condition {
            Ok(())
        } else {
            Err(PlaybackError::Backend(reason.to_owned()))
        }
    }

    #[test]
    fn opt_in_native_audio_smoke() -> Result<(), PlaybackError> {
        if env::var_os("GAMES_NATIVE_AUDIO_SMOKE").is_none() {
            return Ok(());
        }
        let sdl = sdl2::init().map_err(PlaybackError::Backend)?;
        let _subsystem = sdl.audio().map_err(PlaybackError::Backend)?;
        let devices = NativeAudio::playback_devices().map_err(|error| {
            PlaybackError::Backend(format!("enumerate: {error:?}"))
        })?;
        let mut audio = NativeAudio::open(configuration())?;
        require(
            audio.output() == (devices.driver, None),
            "unexpected default output",
        )?;
        let source = pcm()?;
        require(audio.queued_bytes() == 0, "expected empty queue")?;
        audio.queue_pcm(&source, 1, NonZeroU32::MIN)?;
        require(audio.queued_bytes() == 4, "PCM range not queued")?;
        audio.queue_samples(&[0, 0, 1, -1])?;
        require(audio.queued_bytes() == 12, "queue size changed")?;
        require(
            audio.queue_samples(&[0, 0]) == Err(PlaybackError::Capacity),
            "queue overflow accepted",
        )?;
        require(audio.queued_bytes() == 12, "queue size changed")?;
        audio.clear();
        require(audio.queued_bytes() == 0, "expected empty queue")?;
        audio.resume();
        audio.pause();
        drop(audio);
        if let Some(name) =
            devices.names.as_ref().and_then(|names| names.first())
        {
            let named = NativeAudio::open_on(configuration(), Some(name))
                .map_err(|error| {
                    PlaybackError::Backend(format!("named open: {error:?}"))
                })?;
            require(
                named.output() == (devices.driver, Some(name.as_str())),
                "unexpected named output",
            )?;
        }
        Ok(())
    }
}
