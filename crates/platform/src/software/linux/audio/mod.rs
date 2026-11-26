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
//   - Opens admitted devices and verifies the obtained playback format.
// - Usage:
//   - Open a paused device, queue complete frames and explicitly resume.
// - Defaults:
//   - Reject implicit sample-rate or channel conversion.
//! Bounded SDL2 audio playback on Linux.

use std::fmt::{self, Debug, Formatter};

use sdl2::audio::{AudioFormat, AudioQueue, AudioSpecDesired};

use crate::ir::audio::{
    AudioDevices, PlaybackBackend, PlaybackError, PlaybackPort,
};
use crate::ir::security::audio::{
    AdmittedSamples, PlaybackRequest, admit_device_count,
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
    fn clear(&self) {
        self.device.clear();
    }

    fn output(&self) -> (&str, Option<&str>) {
        (self.driver, self.requested_device.as_deref())
    }

    fn pause(&self) {
        self.device.pause();
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

    fn resume(&self) {
        self.device.resume();
    }
}

/// Open the already admitted request and verify the obtained format.
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
    let obtained = device.spec();
    if obtained.freq != layout.frequency()
        || obtained.channels != spec.channels.get()
        || obtained.format != AudioFormat::s16_sys()
    {
        return Err(PlaybackError::FormatMismatch);
    }
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
    use std::num::NonZeroU32;

    use super::super::NATIVE_SMOKE_LOCK;
    use super::*;
    use crate::ir::audio::NativeAudio;
    use crate::ir::audio::fixtures::{configuration, pcm};
    use crate::ir::process::fixtures::{NativeSmoke, smoke_requested};
    fn require(condition: bool, reason: &str) -> Result<(), PlaybackError> {
        if condition {
            Ok(())
        } else {
            Err(PlaybackError::Backend(reason.to_owned()))
        }
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
        audio.clear();
        require(audio.queued_frames()? == 0, "expected empty queue")?;
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
