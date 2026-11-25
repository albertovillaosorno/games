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
//   - Shared engine library boundary.
// - Must-Not:
//   - Own individual title logic or original media adapters.
// - Allows:
//   - Inputs: Future shared engine configuration and domain state.
//   - Outputs: Reusable engine capabilities.
//   - Side effects: None at bootstrap.
// - Split-When:
//   - A capability needs an independent public contract.
// - Merge-When:
//   - Separate modules own the same reusable mechanism.
// - Summary:
//   - Shared cross-platform engine library.
// - Description:
//   - Accumulates capabilities through playable vertical slices.
// - Usage:
//   - Import the games crate.
// - Defaults:
//   - No title-specific behaviour or runtime dependencies.
//

//! Shared engine boundary. Capabilities are added with playable vertical
//! slices.
/// Bounded deterministic PCM playback and stereo mixing.
pub mod audio;
/// Configurable keyboard, gamepad and touch mapping with hot-plug recovery.
pub mod input;
/// Offline-first local sessions and opt-in self-hosted LAN access policy.
pub mod network;
/// Bounded frame-time, latency, memory and binary-size measurement summaries.
pub mod performance;
/// Responsive layout, localization and frame-pacing policy.
pub mod presentation;
/// Public-action recording and replay for deterministic fixed simulations.
pub mod replay;
/// Integrity checks for untrusted local save data.
pub mod safety;
/// Fixed-step simulation time shared across titles and platforms.
pub mod timing;
