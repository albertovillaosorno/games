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
//   - Native graphics resource and display adapter composition.
// - Must-Not:
//   - Own game simulation, audio, source media or native networking.
// - Allows:
//   - Inputs: Canonical pixels and explicit display configuration.
//   - Outputs: Target-specific presentation capabilities.
//   - Side effects: Explicit native graphics and window access.
// - Split-When:
//   - Platform-specific backends require independent modules.
// - Merge-When:
//   - Other adapters duplicate native rendering resource ownership.
// - Summary:
//   - Native graphics independent of simulation and importer dependencies.
// - Description:
//   - Exposes platform graphics without coupling unrelated host services.
// - Usage:
//   - Compose the native adapter in a host application.
// - Defaults:
//   - Open no display or graphics resources implicitly.

//! Native graphics adapters outside the deterministic engine.

/// SDL2-backed Linux RGBA presentation.
#[cfg(target_os = "linux")]
pub mod linux;
