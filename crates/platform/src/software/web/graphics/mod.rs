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
//   - Browser canvas scene resources and ordered sprite presentation.
// - Must-Not:
//   - Interpret game actions, asset formats or select presentation geometry.
// - Allows:
//   - Inputs: Admitted atlases, draw commands and scoped browser events.
//   - Outputs: Actual presentation receipts and diagnostic frame pixels.
//   - Side effects: Create canvas resources, draw and subscribe to input.
// - Split-When:
//   - Browser resource ownership differs from scene composition.
// - Merge-When:
//   - A shared browser graphics backend covers these same operations.
// - Summary:
//   - Resident browser GPU scenes with shared geometry admission.
// - Description:
//   - Retains atlases and logical targets across separately driven steps.
// - Usage:
//   - Open a resident window or isolated diagnostic surface.
// - Defaults:
//   - Nearest sampling; windowed only; no claimed GPU acceleration.

#[cfg(feature = "test-fixtures")]
mod canvas;
#[cfg(feature = "test-fixtures")]
mod fixture;
mod surface;
mod webgl;

#[cfg(feature = "test-fixtures")]
pub(in crate::software) use fixture::{
    verify_resident_scene, verify_webgl_scene,
};
use wasm_bindgen::JsValue;
#[cfg(feature = "interactive")]
pub(in crate::software) use webgl::open_window;

use crate::ir::frame::RenderError;

fn dom_error(_error: JsValue) -> RenderError {
    RenderError::Backend("browser canvas operation failed".to_owned())
}
