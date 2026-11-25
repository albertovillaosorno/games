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
//   - Native audio device and bounded playback adapter composition.
// - Must-Not:
//   - Interpret source media, simulate gameplay or advance game time.
// - Allows:
//   - Inputs: Verified canonical audio and explicit playback limits.
//   - Outputs: Target-specific playback and queue observations.
//   - Side effects: Explicit native audio device access.
// - Split-When:
//   - Platform-specific playback needs an independent adapter.
// - Merge-When:
//   - Native adapters duplicate audio-device or playback queue ownership.
// - Summary:
//   - Native audio playback independent of simulation and graphics.
// - Description:
//   - Accepts verified canonical audio or already mixed native samples.
// - Usage:
//   - Compose the native adapter in a host application.
// - Defaults:
//   - Open no device or start playback implicitly.

//! Native audio adapters outside the deterministic engine.

/// SDL2-backed Linux signed 16-bit PCM playback.
#[cfg(target_os = "linux")]
pub mod linux;
