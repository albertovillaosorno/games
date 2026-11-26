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
//   - An admitted SDL2 playback queue and device diagnostics.
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
//   - Opens admitted devices and reports the obtained playback format.
// - Usage:
//   - Open a paused device, queue complete frames and explicitly resume.
// - Defaults:
//   - Reject implicit sample-rate or channel conversion.
//! Bounded SDL2 audio playback on Linux.

use std::fmt::{self, Debug, Formatter};

use sdl2::audio::{AudioFormat, AudioQueue, AudioSpecDesired, AudioStatus};

use crate::ir::audio::{
    AudioDevices, PlaybackBackend, PlaybackControl, PlaybackError, PlaybackPort,
};
use crate::ir::security::audio::{
    AdmittedSamples, PlaybackObservation, PlaybackRequest, PlaybackState,
    admit_device_count,
};

/// Native device ownership; format and queue admission belong to the IR.
struct AudioBackend {
    device: AudioQueue<i16>,
    driver: &'static str,
    requested_device: Option<String>,
}

impl Debug for AudioBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioBackend").finish_non_exhaustive()
    }
}

impl PlaybackBackend for AudioBackend {
    fn clear(&self) -> Result<(), PlaybackError> {
        self.device.clear();
        Ok(())
    }

    fn observe_playback(&self) -> PlaybackObservation {
        let obtained = self.device.spec();
        PlaybackObservation {
            buffer_bytes: obtained.size,
            buffer_frames: obtained.samples,
            channels: obtained.channels,
            frequency: obtained.freq,
            signed_native_16: obtained.format == AudioFormat::s16_sys(),
            state: match self.device.status() {
                AudioStatus::Paused => PlaybackState::Paused,
                AudioStatus::Playing => PlaybackState::Running,
                AudioStatus::Stopped => PlaybackState::Stopped,
            },
        }
    }

    fn output(&self) -> (&str, Option<&str>) {
        (self.driver, self.requested_device.as_deref())
    }

    fn poll_control(&self) -> Result<PlaybackControl, PlaybackError> {
        Err(PlaybackError::InvalidState)
    }

    fn queue_samples(
        &self,
        samples: AdmittedSamples<'_>,
    ) -> Result<(), PlaybackError> {
        self.device
            .queue_audio(samples.samples())
            .map_err(PlaybackError::Backend)
    }

    fn queued_bytes(&self) -> u32 {
        self.device.size()
    }

    fn request_control(
        &self,
        next: PlaybackState,
    ) -> Result<PlaybackControl, PlaybackError> {
        match next {
            PlaybackState::Paused => self.device.pause(),
            PlaybackState::Running => self.device.resume(),
            PlaybackState::Stopped => return Err(PlaybackError::InvalidState),
        }
        Ok(PlaybackControl::Complete)
    }
}

/// Open the already admitted request; the IR admits the obtained format.
pub(in crate::software) fn open_audio(
    request: PlaybackRequest<'_>,
) -> Result<PlaybackPort, PlaybackError> {
    let layout = request.layout();
    let name = request.name();
    let spec = layout.spec();
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
    device.pause();
    Ok(Box::new(AudioBackend {
        device,
        driver: audio.current_audio_driver(),
        requested_device: name.map(str::to_owned),
    }))
}

/// Enumerate the native device names within the shared result budget.
pub(in crate::software) fn devices() -> Result<AudioDevices, PlaybackError> {
    let sdl = sdl2::init().map_err(PlaybackError::Backend)?;
    let audio = sdl.audio().map_err(PlaybackError::Backend)?;
    let names = audio
        .num_audio_playback_devices()
        .map(|count| {
            admit_device_count(count)?;
            (0..count)
                .map(|index| {
                    audio
                        .audio_playback_device_name(index)
                        .map_err(PlaybackError::Backend)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(AudioDevices {
        driver: audio.current_audio_driver(),
        names,
    })
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32};
    use std::time::Duration;

    use super::super::NATIVE_SMOKE_LOCK;
    use super::*;
    use crate::ir::audio::fixtures::{configuration, pcm};
    use crate::ir::audio::{NativeAudio, PlaybackSpec};
    use crate::ir::process::fixtures::{NativeSmoke, smoke_requested};
    use crate::ir::process::{wait_for_retry, write_status};
    fn require(condition: bool, reason: &str) -> Result<(), PlaybackError> {
        if condition {
            Ok(())
        } else {
            Err(PlaybackError::Backend(reason.to_owned()))
        }
    }

    // Use silence so opt-in device tests never emit an audible test tone.
    fn verify_running_queue(name: Option<&str>) -> Result<(), PlaybackError> {
        let audio = NativeAudio::open_on(
            PlaybackSpec {
                buffer_frames: NonZeroU16::MIN.saturating_add(255),
                max_queued_frames: NonZeroU32::MIN.saturating_add(1023),
                ..configuration()
            },
            name,
        )?;
        audio.queue_samples(&[0; 2048])?;
        wait_for_retry(Duration::from_millis(30))
            .map_err(|error| PlaybackError::Backend(error.to_string()))?;
        require(audio.queued_frames()? == 1024, "paused queue drained")?;
        require(
            audio.resume()? == PlaybackControl::Complete,
            "native control pending",
        )?;
        for _attempt in 0u32..100 {
            if audio.queued_frames()? == 0 {
                require(
                    audio.pause()? == PlaybackControl::Complete,
                    "native control pending",
                )?;
                let (driver, requested) = audio.output();
                return write_status(format_args!(
                    "Native audio drained 1024 frames: SDL {driver}, {}",
                    requested.unwrap_or("system default").escape_debug(),
                ))
                .map_err(|error| PlaybackError::Backend(error.to_string()));
            }
            wait_for_retry(Duration::from_millis(10))
                .map_err(|error| PlaybackError::Backend(error.to_string()))?;
        }
        Err(PlaybackError::Backend(String::from(
            "resumed native software queue did not drain",
        )))
    }

    #[test]
    fn opt_in_native_audio_smoke() -> Result<(), PlaybackError> {
        if !smoke_requested(NativeSmoke::Audio)
            .map_err(|error| PlaybackError::Backend(error.to_string()))?
        {
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
        require(audio.queued_frames()? == 0, "expected empty queue")?;
        audio.queue_pcm(&source, 1, NonZeroU32::MIN)?;
        require(audio.queued_frames()? == 1, "PCM range not queued")?;
        audio.queue_samples(&[0, 0, 1, -1])?;
        require(audio.queued_frames()? == 3, "queue size changed")?;
        require(
            audio.queue_samples(&[0, 0]) == Err(PlaybackError::Capacity),
            "queue overflow accepted",
        )?;
        require(audio.queued_frames()? == 3, "queue size changed")?;
        audio.clear()?;
        require(audio.queued_frames()? == 0, "expected empty queue")?;
        require(
            audio.resume()? == PlaybackControl::Complete,
            "native control pending",
        )?;
        audio.queue_samples(&[1, -1])?;
        require(
            audio.pause()? == PlaybackControl::Complete,
            "native control pending",
        )?;
        audio.clear()?;
        drop(audio);
        verify_running_queue(None)?;
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
            drop(named);
            verify_running_queue(Some(name))?;
        }
        Ok(())
    }
}
