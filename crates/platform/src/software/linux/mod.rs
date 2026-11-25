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

//! Linux-kernel storage services and Linux SDL presentation.

#[cfg(all(feature = "interactive", target_os = "linux"))]
pub(super) mod audio;
#[cfg(all(feature = "interactive", target_os = "linux"))]
pub(super) mod graphics;
#[cfg(all(feature = "interactive", target_os = "linux"))]
pub(super) mod input;
#[cfg(all(feature = "secure-lan", not(target_arch = "wasm32")))]
pub mod lan;
#[cfg(not(target_arch = "wasm32"))]
pub mod performance;
pub(super) mod storage;
pub mod toolchain;

#[cfg(all(test, feature = "interactive", target_os = "linux"))]
use std::sync::Mutex;

/// Serialize test initialization of SDL across native capabilities.
#[cfg(all(test, feature = "interactive", target_os = "linux"))]
static NATIVE_SMOKE_LOCK: Mutex<()> = Mutex::new(());
