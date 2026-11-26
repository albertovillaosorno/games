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
//   - Explicit host scene diagnostic executable.
// - Must-Not:
//   - Open native APIs, interpret game data or modify user settings.
// - Allows:
//   - Inputs: Explicit host diagnostic invocation.
//   - Outputs: Success or portable scene diagnostic failure.
//   - Side effects: Delegated isolated scene diagnostic.
// - Split-When:
//   - Other smoke scenarios need independent resources.
// - Merge-When:
//   - A shared executable covers the same acceptance checks.
// - Summary:
//   - Exercise host scene rendering without game assets.
// - Description:
//   - Delegates through the portable graphics fixture contract.
// - Usage:
//   - Invoke in an isolated browser context.
// - Defaults:
//   - No original assets or gameplay state.

//! Isolated browser resident scene diagnostic.

use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::fixtures::verify_resident_scene;

fn main() -> Result<(), RenderError> {
    verify_resident_scene()
}
