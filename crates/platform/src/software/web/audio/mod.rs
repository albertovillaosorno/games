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
//   - Browser audio context, PCM buffer submission and source lifetime.
// - Must-Not:
//   - Mix, resample, interpret title cues or advance simulation time.
// - Allows:
//   - Inputs: Admitted exact signed PCM and explicit control requests.
//   - Outputs: Bounded queue charges and asynchronous control receipts.
//   - Side effects: Explicit Web Audio source and context operations.
// - Split-When:
//   - An independent browser audio transport is required.
// - Merge-When:
//   - Another adapter owns the same browser playback resources.
// - Summary:
//   - Default-output Web Audio playback with bounded buffer ownership.
// - Description:
//   - Convert exact signed samples into planar floating-point buffers.
// - Usage:
//   - Open an admitted queue and poll explicit control requests.
// - Defaults:
//   - No source starts before an explicit, successful resume request.

//! Browser audio playback and asynchronous context control.

#[cfg(feature = "test-fixtures")]
pub(in crate::software) use fixtures::verify_fixture;
#[cfg(feature = "test-fixtures")]
mod fixtures;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt::{self, Debug, Formatter};
use std::future::Future as _;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use js_sys::futures::{JsFuture, spawn_local};
use js_sys::{Reflect, global};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextOptions,
    AudioContextState, AudioScheduledSourceNode, Event,
};

use super::audio_timeline;
use crate::ir::audio::{
    AudioDevices, PlaybackBackend, PlaybackControl, PlaybackError, PlaybackPort,
};
use crate::ir::security::audio::{
    AdmittedSamples, PlaybackObservation, PlaybackRequest, PlaybackState,
    QueueLayout,
};

struct BrowserAudio {
    context: AudioContext,
    layout: QueueLayout,
    pending: RefCell<Option<PendingControl>>,
    queue: RefCell<VecDeque<Chunk>>,
    rate: f32,
    started: Cell<bool>,
}

struct PendingControl {
    destination: PlaybackState,
    future: JsFuture,
}

struct Chunk {
    buffer: AudioBuffer,
    bytes: u32,
    source: Option<ScheduledSource>,
}

struct ScheduledSource {
    callback: Closure<dyn FnMut(Event)>,
    end: f64,
    ended: Rc<Cell<bool>>,
    node: AudioBufferSourceNode,
}

impl Debug for BrowserAudio {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserAudio")
            .finish_non_exhaustive()
    }
}

impl BrowserAudio {
    fn next_start(
        &self,
        queue: &VecDeque<Chunk>,
    ) -> Result<f64, PlaybackError> {
        let now = self.context.current_time();
        let spec = self.layout.spec();
        let lead = f64::from(spec.buffer_frames.get())
            / f64::from(spec.sample_rate.get());
        let tail = queue
            .iter()
            .filter_map(|chunk| chunk.source.as_ref().map(|source| source.end))
            .next_back();
        audio_timeline::next_start(now, lead, tail)
    }

    fn schedule_pending(&self) -> Result<(), PlaybackError> {
        let mut queue = self.queue.borrow_mut();
        queue.retain(|chunk| !chunk.ended());
        let mut next = self.next_start(&queue)?;
        for chunk in queue.iter_mut().filter(|chunk| chunk.source.is_none()) {
            next = chunk.start(&self.context, next)?;
        }
        Ok(())
    }

    fn state(&self) -> PlaybackState {
        let state = self.context.state();
        if state == AudioContextState::Suspended
            || (state == AudioContextState::Running && !self.started.get())
        {
            // Before the first admitted resume no sources are connected.
            PlaybackState::Paused
        } else if state == AudioContextState::Running {
            PlaybackState::Running
        } else {
            PlaybackState::Stopped
        }
    }
}

impl Drop for BrowserAudio {
    fn drop(&mut self) {
        let _cleared = self.clear();
        if let Ok(close) = self.context.close() {
            spawn_local(async move {
                let _closed = close.await;
            });
        }
    }
}

impl PlaybackBackend for BrowserAudio {
    fn clear(&self) -> Result<(), PlaybackError> {
        let mut queue = self.queue.borrow_mut();
        let mut failure = None;
        // One failed cancellation must not prevent clearing other sources.
        // Keep only failed chunks charged so cleanup can be retried.
        queue.retain(|chunk| {
            if let Some(source) = &chunk.source
                && let Err(error) = source.cancel()
            {
                if failure.is_none() {
                    failure = Some(error);
                }
                return true;
            }
            false
        });
        failure.map_or(Ok(()), Err)
    }

    fn observe_playback(&self) -> Result<PlaybackObservation, PlaybackError> {
        let spec = self.layout.spec();
        Ok(PlaybackObservation {
            buffer_bytes: u32::from(spec.buffer_frames.get())
                .saturating_mul(u32::from(spec.channels.get()))
                .saturating_mul(2),
            buffer_frames: spec.buffer_frames.get(),
            channels: spec.channels.get(),
            frequency: if self.context.sample_rate().to_bits()
                == self.rate.to_bits()
            {
                self.layout.frequency()
            } else {
                0
            },
            signed_native_16: true,
            state: self.state(),
        })
    }

    fn output(&self) -> (&str, Option<&str>) {
        ("web-audio", None)
    }

    fn poll_control(&self) -> Result<PlaybackControl, PlaybackError> {
        let mut pending = self.pending.borrow_mut();
        let control = pending.as_mut().ok_or(PlaybackError::InvalidState)?;
        let mut context = Context::from_waker(Waker::noop());
        match Pin::new(&mut control.future).poll(&mut context) {
            Poll::Pending => Ok(PlaybackControl::Pending),
            Poll::Ready(result) => {
                let destination = control.destination;
                drop(pending.take());
                let _fulfilled = result.map_err(|_error| {
                    backend_error("browser audio control rejected")
                })?;
                if destination == PlaybackState::Running {
                    self.started.set(true);
                    if self.state() != PlaybackState::Running {
                        return Err(PlaybackError::InvalidState);
                    }
                    self.schedule_pending()?;
                }
                Ok(PlaybackControl::Complete)
            },
        }
    }

    fn queue_samples(
        &self,
        samples: AdmittedSamples<'_>,
    ) -> Result<(), PlaybackError> {
        let mut chunk = Chunk::new(self, samples.samples())?;
        let mut queue = self.queue.borrow_mut();
        queue.retain(|chunk| !chunk.ended());
        queue
            .try_reserve(1)
            .map_err(|_error| PlaybackError::Allocation)?;
        if self.state() == PlaybackState::Running {
            let _end = chunk.start(&self.context, self.next_start(&queue)?)?;
        }
        queue.push_back(chunk);
        Ok(())
    }

    fn queued_bytes(&self) -> Result<u32, PlaybackError> {
        let mut queue = self.queue.borrow_mut();
        queue.retain(|chunk| !chunk.ended());
        queue.iter().try_fold(0u32, |total, chunk| {
            total
                .checked_add(chunk.bytes)
                .ok_or(PlaybackError::InvalidQueue)
        })
    }

    fn request_control(
        &self,
        next: PlaybackState,
    ) -> Result<PlaybackControl, PlaybackError> {
        if self.pending.borrow().is_some() {
            return Err(PlaybackError::TransitionPending);
        }
        if next == self.state() {
            return Ok(PlaybackControl::Complete);
        }
        let promise = match next {
            PlaybackState::Paused => self.context.suspend(),
            PlaybackState::Running => self.context.resume(),
            PlaybackState::Stopped => return Err(PlaybackError::InvalidState),
        }
        .map_err(|_error| backend_error("browser audio control failed"))?;
        let _previous = self.pending.replace(Some(PendingControl {
            destination: next,
            future: JsFuture::from(promise),
        }));
        Ok(PlaybackControl::Pending)
    }
}

impl Chunk {
    fn ended(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.ended.get())
    }

    fn new(
        audio: &BrowserAudio,
        samples: &[i16],
    ) -> Result<Self, PlaybackError> {
        let channels = usize::from(audio.layout.spec().channels.get());
        let frames = samples
            .len()
            .checked_div(channels)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(PlaybackError::InvalidFrames)?;
        let bytes = samples
            .len()
            .checked_mul(2)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(PlaybackError::InvalidFrames)?;
        let buffer = audio
            .context
            .create_buffer(
                u32::from(audio.layout.spec().channels.get()),
                frames,
                audio.rate,
            )
            .map_err(|_error| backend_error("browser audio buffer failed"))?;
        if buffer.length() != frames
            || buffer.number_of_channels()
                != u32::from(audio.layout.spec().channels.get())
            || buffer.sample_rate().to_bits() != audio.rate.to_bits()
        {
            return Err(PlaybackError::FormatMismatch);
        }
        let mut planar = Vec::new();
        planar
            .try_reserve_exact(
                usize::try_from(frames)
                    .map_err(|_error| PlaybackError::Allocation)?,
            )
            .map_err(|_error| PlaybackError::Allocation)?;
        for channel in 0..channels {
            planar.clear();
            planar.extend(
                samples
                    .iter()
                    .skip(channel)
                    .step_by(channels)
                    .map(|sample| f32::from(*sample) * (1.0f32 / 32_768.0f32)),
            );
            buffer
                .copy_to_channel(
                    &planar,
                    i32::try_from(channel)
                        .map_err(|_error| PlaybackError::FormatMismatch)?,
                )
                .map_err(|_error| backend_error("browser audio copy failed"))?;
        }
        Ok(Self {
            buffer,
            bytes,
            source: None,
        })
    }

    fn start(
        &mut self,
        context: &AudioContext,
        when: f64,
    ) -> Result<f64, PlaybackError> {
        let end = when + self.buffer.duration();
        if !end.is_finite() || end <= when {
            return Err(PlaybackError::InvalidState);
        }
        let node = context
            .create_buffer_source()
            .map_err(|_error| backend_error("browser audio source failed"))?;
        node.set_buffer(Some(&self.buffer));
        let ended = Rc::new(Cell::new(false));
        let observed = Rc::clone(&ended);
        let callback = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            if event.is_trusted() {
                observed.set(true);
            }
        });
        node.add_event_listener_with_callback(
            "ended",
            callback.as_ref().unchecked_ref(),
        )
        .map_err(|_error| backend_error("browser audio listener failed"))?;
        let source = ScheduledSource {
            callback,
            end,
            ended,
            node,
        };
        let _connection = source
            .node
            .connect_with_audio_node(&context.destination())
            .map_err(|_error| backend_error("browser audio connect failed"))?;
        let base: &AudioScheduledSourceNode = source.node.as_ref();
        base.start_with_when(when)
            .map_err(|_error| backend_error("browser audio start failed"))?;
        self.source = Some(source);
        Ok(end)
    }
}

impl Drop for ScheduledSource {
    fn drop(&mut self) {
        let _removed = self.node.remove_event_listener_with_callback(
            "ended",
            self.callback.as_ref().unchecked_ref(),
        );
        let _disconnected = self.node.disconnect();
    }
}

impl ScheduledSource {
    fn cancel(&self) -> Result<(), PlaybackError> {
        let stopped = if self.ended.get() {
            Ok(())
        } else {
            let base: &AudioScheduledSourceNode = self.node.as_ref();
            base.stop()
                .map_err(|_error| backend_error("browser audio stop failed"))
        };
        // Disconnect even when the host refuses to stop the source.
        let disconnected = self
            .node
            .disconnect()
            .map_err(|_error| backend_error("browser audio disconnect failed"));
        stopped.and(disconnected)
    }
}

fn backend_error(message: &str) -> PlaybackError {
    PlaybackError::Backend(message.to_owned())
}

fn create_audio(
    request: PlaybackRequest<'_>,
) -> Result<BrowserAudio, PlaybackError> {
    if request.name().is_some() {
        return Err(PlaybackError::InvalidDevice);
    }
    let layout = request.layout();
    let rate = sample_rate(layout.spec().sample_rate.get())?;
    let options = AudioContextOptions::new();
    options.set_sample_rate(rate);
    let context = AudioContext::new_with_context_options(&options)
        .map_err(|_error| backend_error("browser audio context unavailable"))?;
    let audio = BrowserAudio {
        context,
        layout,
        pending: RefCell::new(None),
        queue: RefCell::new(VecDeque::new()),
        rate,
        started: Cell::new(false),
    };
    if audio.context.sample_rate().to_bits() != rate.to_bits()
        || audio.context.destination().max_channel_count()
            < u32::from(layout.spec().channels.get())
    {
        return Err(PlaybackError::FormatMismatch);
    }
    Ok(audio)
}

pub(in crate::software) fn devices() -> Result<AudioDevices, PlaybackError> {
    let constructor =
        Reflect::get(&global(), &JsValue::from_str("AudioContext"))
            .map_err(|_error| backend_error("browser audio lookup failed"))?;
    if !constructor.is_function() {
        return Err(PlaybackError::Unsupported);
    }
    Ok(AudioDevices {
        driver: "web-audio",
        names: None,
    })
}

pub(in crate::software) fn open_audio(
    request: PlaybackRequest<'_>,
) -> Result<PlaybackPort, PlaybackError> {
    Ok(Box::new(create_audio(request)?))
}

fn sample_rate(value: u32) -> Result<f32, PlaybackError> {
    let high = u16::try_from(value >> 16)
        .map_err(|_error| PlaybackError::InvalidSpec)?;
    let low = u16::try_from(value & u32::from(u16::MAX))
        .map_err(|_error| PlaybackError::InvalidSpec)?;
    let rate = f32::from(high).mul_add(65_536.0f32, f32::from(low));
    if f64::from(rate).to_bits() != f64::from(value).to_bits() {
        return Err(PlaybackError::InvalidSpec);
    }
    Ok(rate)
}
