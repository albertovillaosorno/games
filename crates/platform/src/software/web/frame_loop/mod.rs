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
//   - Browser animation-frame registration and asynchronous callback drive.
// - Must-Not:
//   - Advance simulation, busy-wait or substitute a wall clock.
// - Allows:
//   - Inputs: An owned step and terminal callback.
//   - Outputs: Valid host frame timestamps and a single completion outcome.
//   - Side effects: Request one frame at a time and yield to the document.
// - Split-When:
//   - An independent browser event scheduler has different semantics.
// - Merge-When:
//   - A shared browser callback pump covers these same operations.
// - Summary:
//   - One outstanding animation frame with no leaked Rust closure.
// - Description:
//   - Use the host promise resolver as the one-shot frame callback.
// - Usage:
//   - Start a finite or interactive application loop explicitly.
// - Defaults:
//   - Stop on a malformed timestamp, failed step or rejected registration.

use std::time::Duration;

use js_sys::Promise;
use js_sys::futures::spawn_local;
use web_sys::Window;

use crate::ir::frame::RenderError;
use crate::ir::graphics::frame_loop::{FrameComplete, FrameStep};

pub(in crate::software) fn start(
    step: FrameStep,
    complete: FrameComplete,
) -> Result<(), RenderError> {
    let window = web_sys::window().ok_or(RenderError::Unsupported)?;
    let first = request(&window)?;
    spawn_local(async move {
        let result = drive(&window, first, step).await;
        complete(result);
    });
    Ok(())
}

async fn drive(
    window: &Window,
    first: Promise,
    mut step: FrameStep,
) -> Result<(), RenderError> {
    let mut frame = first;
    let mut previous = Duration::ZERO;
    loop {
        let observed = frame
            .await
            .map_err(|_error| failure())?
            .as_f64()
            .ok_or(RenderError::InvalidTimestamp)?;
        let stamp = super::performance::timestamp(observed)
            .map_err(|_error| RenderError::InvalidTimestamp)?;
        if stamp < previous {
            return Err(RenderError::InvalidTimestamp);
        }
        if !step(stamp)? {
            return Ok(());
        }
        previous = stamp;
        frame = request(window)?;
    }
}

fn request(window: &Window) -> Result<Promise, RenderError> {
    let mut result = Err(RenderError::Unsupported);
    let promise = Promise::new(&mut |resolve, _reject| {
        result = window
            .request_animation_frame(&resolve)
            .map(|_request| ())
            .map_err(|_error| failure());
    });
    result?;
    Ok(promise)
}

fn failure() -> RenderError {
    RenderError::Backend("browser animation frame rejected".to_owned())
}
