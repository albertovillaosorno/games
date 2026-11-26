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
//   - Explicit host scene diagnostic dispatch.
// - Must-Not:
//   - Expose host event handles or access native services.
// - Allows:
//   - Inputs: Explicit diagnostic invocation.
//   - Outputs: Success or typed capability failure.
//   - Side effects: Delegated isolated event fixture operations.
// - Split-When:
//   - Another diagnostic requires independent capabilities.
// - Merge-When:
//   - A shared fixture covers the same input checks.
// - Summary:
//   - Exercise the selected host resident scene.
// - Description:
//   - Keeps host-specific diagnostics behind the graphics boundary.
// - Usage:
//   - Invoke with test-fixtures in an isolated environment.
// - Defaults:
//   - Unsupported on hosts without an scene fixture.

use crate::ir::frame::RenderError;

pub(crate) trait SceneFixturePort {
    fn verify_gpu_scene() -> Result<(), RenderError>;
    fn verify_resident_scene() -> Result<(), RenderError>;
}

/// Verify resident scene rendering and presented geometry.
///
/// # Errors
/// Reports unavailable diagnostics or an observed invariant failure.
pub fn verify_resident_scene() -> Result<(), RenderError> {
    <() as SceneFixturePort>::verify_resident_scene()
}

/// Verify the selected GPU API scene, including actual presentation.
///
/// # Errors
/// Reports unavailable diagnostics or an observed invariant failure.
pub fn verify_gpu_scene() -> Result<(), RenderError> {
    <() as SceneFixturePort>::verify_gpu_scene()
}
