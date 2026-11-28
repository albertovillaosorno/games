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
//   - Private host service selection and software adapter composition.
// - Must-Not:
//   - Own game simulation or source media interpretation.
// - Allows:
//   - Inputs: Shared engine configuration and host-provided resources.
//   - Outputs: Native platform capabilities.
//   - Side effects: Explicit host operating-system access.
// - Split-When:
//   - A platform adapter needs an independent responsibility.
// - Merge-When:
//   - Multiple adapters implement the same platform boundary.
// - Summary:
//   - Platform integration outside the transport-neutral engine.
// - Description:
//   - Exposes native storage adapters behind target-specific module boundaries.
// - Usage:
//   - Compose adapters with the engine in a host application.
// - Defaults:
//   - No host resources are accessed by module selection.

//! Linux-kernel storage services and shared SDL2 presentation.

#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
pub(super) mod audio;
#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
pub(super) mod graphics;
#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
pub(super) mod input;
#[cfg(all(feature = "secure-lan", not(target_arch = "wasm32")))]
mod lan;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod performance;
#[cfg(target_os = "linux")]
mod process;
pub(super) mod storage;
#[cfg(target_os = "linux")]
mod toolchain;

#[cfg(all(
    test,
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
use std::sync::Mutex;

/// Serialize test initialization of SDL across native capabilities.
#[cfg(all(
    test,
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
static NATIVE_SMOKE_LOCK: Mutex<()> = Mutex::new(());
