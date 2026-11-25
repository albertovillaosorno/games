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

//! Private software implementation and current host selection.

mod linux;

/// Explicit absence of the authenticated LAN capability.
#[cfg(any(not(feature = "secure-lan"), target_arch = "wasm32"))]
pub mod lan {
    /// Whether the selected host supports LAN.
    pub const AVAILABLE: bool = false;
}

#[cfg(target_os = "linux")]
pub use linux::graphics;
#[cfg(not(target_os = "linux"))]
pub mod graphics {
    /// Whether the selected host supports graphics.
    pub const AVAILABLE: bool = false;
}
#[cfg(all(feature = "secure-lan", not(target_arch = "wasm32")))]
pub use linux::lan;
#[cfg(not(target_arch = "wasm32"))]
pub use linux::performance;
#[cfg(target_arch = "wasm32")]
pub mod performance {
    /// Whether the selected host supports native measurement.
    pub const AVAILABLE: bool = false;
}
#[cfg(target_os = "linux")]
pub use linux::{audio::NativeAudio, input};
pub use linux::{storage, toolchain};

/// Uninhabited playback handle on targets without a native audio adapter.
#[cfg(not(target_os = "linux"))]
#[derive(Clone, Copy, Debug)]
pub enum NativeAudio {}

/// Input namespace on targets without a native input adapter.
#[cfg(not(target_os = "linux"))]
pub mod input {
    /// Whether the selected host supports native input.
    pub const AVAILABLE: bool = false;
}
