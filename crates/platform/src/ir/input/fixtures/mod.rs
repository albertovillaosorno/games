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
//   - Explicit host input diagnostic dispatch.
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
//   - Exercise the selected host event source.
// - Description:
//   - Keeps host-specific diagnostics behind the input boundary.
// - Usage:
//   - Invoke with test-fixtures in an isolated environment.
// - Defaults:
//   - Unsupported on hosts without an event-source fixture.

use crate::ir::frame::RenderError;

pub(crate) trait EventFixturePort {
    fn verify_event_source() -> Result<(), RenderError>;
}

/// Verify scoped host event translation and bounded cancellation.
///
/// # Errors
/// Reports unavailable diagnostics or an observed invariant failure.
pub fn verify_event_source() -> Result<(), RenderError> {
    <() as EventFixturePort>::verify_event_source()
}
