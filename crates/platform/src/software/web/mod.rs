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
//   - Browser adapter composition.
// - Must-Not:
//   - Own game simulation or source media interpretation.
// - Allows:
//   - Inputs: Shared engine configuration and host-provided resources.
//   - Outputs: Browser platform capabilities.
//   - Side effects: Explicit browser capability access.
// - Split-When:
//   - A platform adapter needs an independent responsibility.
// - Merge-When:
//   - Multiple adapters implement the same platform boundary.
// - Summary:
//   - Platform integration outside the transport-neutral engine.
// - Description:
//   - Exposes origin-local storage, output and timing behind a private
//     boundary.
// - Usage:
//   - Compose adapters with the engine in a host application.
// - Defaults:
//   - No operating-system resources opened implicitly.

//! Browser implementations behind the portable capability interface.

#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    feature = "interactive"
))]
pub(super) mod audio;

#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    feature = "test-fixtures"
))]
pub(super) mod graphics;
#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    feature = "test-fixtures"
))]
pub(super) mod input;
pub(super) mod performance;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod process;
mod storage;
