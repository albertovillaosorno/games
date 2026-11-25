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
use std::num::NonZeroU32;

use games_assets::pcm::PcmSamples;
use sdl2::audio::{AudioFormat, AudioQueue, AudioSpecDesired};

use crate::ir::audio::{AudioDevices, PlaybackError, PlaybackSpec};
use crate::ir::security::audio::{
    MAX_DEVICES, QueueLayout, valid_device, validate_device_name,
};

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
            freq: Some(layout.frequency()),
            channels: Some(spec.channels.get()),
            samples: Some(spec.buffer_frames.get()),
        };
        let device = audio
            .open_queue::<i16, _>(name, &desired)
            .map_err(PlaybackError::Backend)?;
        let obtained = device.spec();
        if obtained.freq != layout.frequency()
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

#[cfg(test)]
mod tests {
    use std::env;

    use super::super::NATIVE_SMOKE_LOCK;
    use super::*;
    use crate::ir::audio::fixtures::{configuration, pcm};
    #[test]
    fn rejects_invalid_explicit_device_names_before_opening() {
        for name in ["", "bad\0name", &"a".repeat(513)] {
            assert!(matches!(
                NativeAudio::open_on(configuration(), Some(name)),
                Err(PlaybackError::InvalidDevice)
            ));
        }
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
        let _guard = NATIVE_SMOKE_LOCK
            .lock()
            .map_err(|error| PlaybackError::Backend(error.to_string()))?;
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
