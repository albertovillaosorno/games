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
//   - Portable event-driven frame scheduling capability.
// - Must-Not:
//   - Advance simulation or claim display scanout and latency.
// - Allows:
//   - Inputs: Owned frame step and terminal completion callbacks.
//   - Outputs: Comparable host timestamps and one completion outcome.
//   - Side effects: Delegated asynchronous host callback registration.
// - Split-When:
//   - Independent callback families require different timing semantics.
// - Merge-When:
//   - A shared event loop exposes equivalent frame scheduling.
// - Summary:
//   - Yield to the host between bounded application steps.
// - Description:
//   - Keeps callbacks and their state alive without a blocking loop.
// - Usage:
//   - Move an owned scene and runtime state into the step callback.
// - Defaults:
//   - Unavailable on hosts without an event-driven frame scheduler.

//! Event-driven host frame scheduling.

use std::time::Duration;

use crate::ir::frame::RenderError;

pub(crate) type FrameStep =
    Box<dyn FnMut(Duration) -> Result<bool, RenderError>>;
pub(crate) type FrameComplete = Box<dyn FnOnce(Result<(), RenderError>)>;

pub(crate) trait FrameLoopPort {
    fn start(
        step: FrameStep,
        complete: FrameComplete,
    ) -> Result<(), RenderError>;
}

/// Schedule one owned application step per host frame callback.
///
/// The step receives a monotonic host timestamp. Returning false stops the
/// loop successfully. A step or scheduling failure stops it with an error.
/// Completion is called once after successful initial registration, and
/// never if registration itself fails. The step is dropped before completion.
/// No application simulation rate or display scanout is implied.
///
/// # Errors
/// Reports unavailable scheduling or initial callback registration failure.
pub fn start<Step, Complete>(
    step: Step,
    complete: Complete,
) -> Result<(), RenderError>
where
    Step: FnMut(Duration) -> Result<bool, RenderError> + 'static,
    Complete: FnOnce(Result<(), RenderError>) + 'static,
{
    <() as FrameLoopPort>::start(Box::new(step), Box::new(complete))
}
