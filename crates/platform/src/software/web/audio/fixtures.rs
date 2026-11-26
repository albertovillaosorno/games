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
//   - Isolated asynchronous browser audio diagnostic sequencing.
// - Must-Not:
//   - Play an audible test tone, read original media or alter user settings.
// - Allows:
//   - Inputs: Explicit invocation on an activated browser document.
//   - Outputs: Portable queue and transition assertions and diagnostics.
//   - Side effects: Temporary default audio context and silent playback.
// - Split-When:
//   - Additional audio scenarios require independent host resources.
// - Merge-When:
//   - A shared host diagnostic covers the same queue acceptance.
// - Summary:
//   - Observe real Web Audio control completion without blocking the page.
// - Description:
//   - Use an explicit bounded queue and release it after verified draining.
// - Usage:
//   - Invoke through the portable audio diagnostic in an isolated browser.
// - Defaults:
//   - No original assets or physical-audibility claim.

use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};

use js_sys::Promise;
use js_sys::futures::spawn_local;
use wasm_bindgen::JsValue;

use super::{BrowserAudio, create_audio};
use crate::ir::audio::{
    NativeAudio, PlaybackBackend, PlaybackControl, PlaybackError, PlaybackSpec,
};
use crate::ir::process::{write_diagnostic, write_status};
use crate::ir::security::audio::{PlaybackRequest, PlaybackState};

const SILENCE: [i16; 512] = [0; 512];
const ATTEMPTS: u16 = 300;

async fn delay() -> Result<(), PlaybackError> {
    let window = web_sys::window().ok_or(PlaybackError::Unsupported)?;
    let timer = Promise::new(&mut |resolve, reject| {
        if let Err(error) = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
        {
            let _rejected = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });
    let _elapsed = timer
        .await
        .map_err(|_error| failure("browser audio timer rejected"))?;
    Ok(())
}

async fn drain(
    mut queued: impl FnMut() -> Result<u32, PlaybackError>,
) -> Result<(), PlaybackError> {
    for _attempt in 0..ATTEMPTS {
        if queued()? == 0 {
            return Ok(());
        }
        delay().await?;
    }
    Err(failure("browser audio did not drain"))
}

fn failure(message: &str) -> PlaybackError {
    PlaybackError::Backend(message.to_owned())
}

async fn finish(audio: NativeAudio) -> Result<(), PlaybackError> {
    settle(|| audio.poll_control()).await?;
    drain(|| audio.queued_frames()).await?;
    audio.queue_samples(&SILENCE)?;
    require(audio.queued_frames()? == 256, "running submission charge")?;
    audio.queue_samples(&SILENCE)?;
    require(audio.queued_frames()? == 512, "running refill charge")?;
    audio.clear()?;
    require(
        audio.queued_frames()? == 0,
        "running clear retained samples",
    )?;
    require(
        audio.pause()? == PlaybackControl::Pending,
        "pause not pending",
    )?;
    settle(|| audio.poll_control()).await?;
    audio.queue_samples(&SILENCE)?;
    for _attempt in 0..10u8 {
        delay().await?;
        require(audio.queued_frames()? == 256, "paused queue drained")?;
    }
    audio.clear()?;
    require(audio.queued_frames()? == 0, "paused clear retained samples")?;
    audio.queue_samples(&SILENCE)?;
    require(
        audio.resume()? == PlaybackControl::Pending,
        "resume not pending",
    )?;
    settle(|| audio.poll_control()).await?;
    drain(|| audio.queued_frames()).await?;
    drop(audio);
    Ok(())
}

fn require(condition: bool, message: &str) -> Result<(), PlaybackError> {
    if condition {
        Ok(())
    } else {
        Err(failure(message))
    }
}

async fn settle(
    mut poll: impl FnMut() -> Result<PlaybackControl, PlaybackError>,
) -> Result<(), PlaybackError> {
    for _attempt in 0..ATTEMPTS {
        if poll()? == PlaybackControl::Complete {
            return Ok(());
        }
        delay().await?;
    }
    Err(failure("browser audio control did not settle"))
}

fn submit_silence(audio: &BrowserAudio) -> Result<(), PlaybackError> {
    audio
        .layout
        .admit_observation(audio.observe_playback(), PlaybackState::Running)?;
    let admitted =
        audio.layout.admit_samples(&SILENCE, audio.queued_bytes())?;
    audio.queue_samples(admitted)?;
    audio
        .layout
        .admit_submission(admitted, audio.queued_bytes(), audio.state())
}

fn scheduled_tail(audio: &BrowserAudio) -> Result<(f64, f64), PlaybackError> {
    audio
        .queue
        .borrow()
        .back()
        .and_then(|chunk| {
            chunk
                .source
                .as_ref()
                .map(|source| (source.end, chunk.buffer.duration()))
        })
        .ok_or_else(|| failure("missing scheduled audio tail"))
}

async fn verify_refill(audio: BrowserAudio) -> Result<(), PlaybackError> {
    settle(|| audio.poll_control()).await?;
    submit_silence(&audio)?;
    let (end, duration) = scheduled_tail(&audio)?;
    let spec = audio.layout.spec();
    let lead =
        f64::from(spec.buffer_frames.get()) / f64::from(spec.sample_rate.get());
    // Enter a real-clock interval in which applying a second startup lead
    // would introduce a gap. Leave headroom for host dispatch jitter.
    for _attempt in 0..ATTEMPTS {
        let remaining = end - audio.context.current_time();
        require(remaining > lead * 0.25, "missed near-tail refill window")?;
        if remaining < lead * 0.75 {
            submit_silence(&audio)?;
            let (next_end, next_duration) = scheduled_tail(&audio)?;
            require(
                next_duration.to_bits() == duration.to_bits()
                    && next_end.to_bits() == (end + duration).to_bits(),
                "near-tail refill introduced a discontinuity",
            )?;
            drain(|| Ok(audio.queued_bytes())).await?;
            return Ok(());
        }
        delay().await?;
    }
    Err(failure("browser audio clock did not reach refill window"))
}

pub(in crate::software) fn verify_fixture() -> Result<(), PlaybackError> {
    let spec = PlaybackSpec {
        buffer_frames: NonZeroU16::MIN.saturating_add(255),
        channels: NonZeroU8::MIN.saturating_add(1),
        max_queued_frames: NonZeroU32::MIN.saturating_add(511),
        sample_rate: NonZeroU32::MIN.saturating_add(47_999),
    };
    let devices = NativeAudio::playback_devices()?;
    require(
        devices.driver == "web-audio" && devices.names.is_none(),
        "browser output enumeration",
    )?;
    require(
        matches!(
            NativeAudio::open_on(spec, Some("named")),
            Err(PlaybackError::InvalidDevice)
        ),
        "named browser output admitted",
    )?;
    let audio = NativeAudio::open(spec)?;
    require(audio.output() == ("web-audio", None), "browser output")?;
    // The asymmetric non-silent PCM is copied only while paused and cleared
    // before any source is connected. All scheduled samples are silence.
    audio.queue_samples(&[i16::MIN, i16::MAX, -1, 1])?;
    require(audio.queued_frames()? == 2, "exact PCM receipt")?;
    audio.clear()?;
    audio.queue_samples(&SILENCE)?;
    audio.queue_samples(&SILENCE)?;
    require(audio.queued_frames()? == 512, "initial queue charge")?;
    require(
        audio.queue_samples(&[0, 0]) == Err(PlaybackError::Capacity),
        "browser queue exceeded capacity",
    )?;
    require(
        audio.resume()? == PlaybackControl::Pending,
        "initial resume not pending",
    )?;
    require(
        audio.clear() == Err(PlaybackError::TransitionPending),
        "pending clear admitted",
    )?;
    require(
        audio.queue_samples(&SILENCE) == Err(PlaybackError::TransitionPending),
        "pending submit admitted",
    )?;
    require(
        audio.pause() == Err(PlaybackError::TransitionPending),
        "pending pause admitted",
    )?;
    // A longer diagnostic period gives the real browser a wide observation
    // interval without changing the title's normal PCM queue policy.
    let period = NonZeroU16::MIN.saturating_add(8191);
    let request = PlaybackRequest::new(
        PlaybackSpec {
            buffer_frames: period,
            max_queued_frames: NonZeroU32::from(period),
            ..spec
        },
        None,
    )?;
    let refill = create_audio(request)?;
    request.admit_opened(
        refill.observe_playback(),
        refill.output().1,
        refill.queued_bytes(),
    )?;
    require(
        refill.request_control(PlaybackState::Running)?
            == PlaybackControl::Pending,
        "refill resume not pending",
    )?;
    write_status(format_args!("Browser audio diagnostic pending"))
        .map_err(|_error| failure("browser audio status unavailable"))?;
    spawn_local(async move {
        let result = async {
            finish(audio).await?;
            verify_refill(refill).await
        }
        .await;
        match result {
            Ok(()) => {
                let _status = write_status(format_args!(
                    "Browser audio diagnostic passed"
                ));
            },
            Err(error) => {
                let _diagnostic = write_diagnostic(format_args!(
                    "Browser audio diagnostic failed: {error:?}"
                ));
            },
        }
    });
    Ok(())
}
