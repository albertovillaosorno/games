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
//   - Portable operating-system capability contracts.
// - Must-Not:
//   - Access native services or select concrete implementations.
// - Allows:
//   - Inputs: Neutral capability requests and observations.
//   - Outputs: Typed portable values and deterministic admission results.
//   - Side effects: In-memory contract state only.
// - Split-When:
//   - A host capability needs independent responsibility.
// - Merge-When:
//   - Shared host policy already owns the same contract.
// - Summary:
//   - Backend-independent platform framework.
// - Description:
//   - Shares capability semantics across native implementations.
// - Usage:
//   - Implement and consume these contracts through platform adapters.
// - Defaults:
//   - No access to host resources.

//! Portable operating-system capability contracts.

/// Portable playback requests and outcomes.
pub mod audio;
/// Portable window and scene rendering.
pub mod graphics;
/// Portable physical-device bridge.
pub mod hardware;
/// Portable device input sessions.
pub mod input;
/// Clock-independent measurement.
pub mod measurement;
/// Mandatory shared host admission policies.
pub mod security;
/// Portable storage capabilities.
pub mod storage;
/// Bounded bitmap text, menu and scene preparation.
pub mod ui;

#[cfg(test)]
use std::fmt::Debug;

/// Shared RGBA layout and rendering failures.
pub use security::frame;
pub use ui::{menu, scene};

/// Selected service interfaces; concrete implementation paths stay
/// private.
pub use crate::selected::{lan, toolchain};

#[cfg(test)]
pub(crate) fn check<T, U>(
    actual: &T,
    expected: &U,
) -> Result<(), frame::RenderError>
where
    T: Debug + PartialEq<U>,
    U: Debug,
{
    if actual == expected {
        Ok(())
    } else {
        Err(frame::RenderError::Backend(format!(
            "expected {expected:?}, observed {actual:?}"
        )))
    }
}
