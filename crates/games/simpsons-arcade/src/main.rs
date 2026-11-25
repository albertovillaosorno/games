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
//   - Executable entry point for the local native title prototype.
// - Must-Not:
//   - Decode original media or duplicate simulation.
// - Allows:
//   - Inputs: Explicit canonical bundle paths and launch mode.
//   - Outputs: Native preview or a launch error.
//   - Side effects: Delegated native window creation.
// - Split-When:
//   - More native launch targets need separate adapters.
// - Merge-When:
//   - Entry points duplicate title host composition.
// - Summary:
//   - Launch the title's bounded native preview.
// - Description:
//   - Optional executable-adjacent pinned local launch file; no media search.
// - Usage:
//   - Provide the local canonical art bundle identities.
// - Defaults:
//   - Fail if unsupported or supplied art is invalid.

//! Executable entry point for the native movement preview.

use games_platform::ir::process::arguments;
use games_simpsons_arcade::runtime;

fn main() -> Result<(), String> {
    runtime::run(arguments().map_err(|error| error.to_string())?)
}
