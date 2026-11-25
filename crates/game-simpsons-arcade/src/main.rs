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
//   - No implicit source or asset discovery.
// - Usage:
//   - Provide the local canonical art bundle identities.
// - Defaults:
//   - Fail if unsupported or supplied art is invalid.

//! Executable entry point for the native movement preview.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::env::args_os;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use games_simpsons_arcade::native;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn main() -> Result<(), String> {
    native::run(args_os().skip(1).collect())
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn main() -> Result<(), String> {
    Err(String::from("native preview currently requires GNU/Linux"))
}
