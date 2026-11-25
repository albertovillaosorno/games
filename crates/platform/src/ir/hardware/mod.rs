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
//   - Neutral host input and application lifecycle event exchange.
// - Must-Not:
//   - Open devices, interpret native handles or select device families.
// - Allows:
//   - Inputs: Normalized input transitions and lifecycle notifications.
//   - Outputs: Events and existing portable feedback contracts.
//   - Side effects: None.
// - Split-When:
//   - A separate device report protocol requires its own contract.
// - Merge-When:
//   - Existing engine input contracts cover equivalent exchange.
// - Summary:
//   - Bridge host device events into the shared action router.
// - Description:
//   - Preserve input semantics without exposing the native event source.
// - Usage:
//   - Translate admitted native events before routing them to an application.
// - Defaults:
//   - No device or feedback capability is assumed.

//! Portable host input and feedback exchange.

pub use games::capability::{
    HapticCommand, HapticEffect, HapticError, HapticSupport, Haptics,
};
pub use games::input::InputEvent;

/// Events delivered by a host to its action router or application loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEvent {
    /// A platform-neutral input transition.
    Input(InputEvent),
    /// The application or its window received a close request.
    Quit,
}
