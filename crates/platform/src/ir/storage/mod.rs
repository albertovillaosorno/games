// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Portable storage capability composition.
// - Must-Not:
//   - Select native APIs or access host storage directly.
// - Allows:
//   - Inputs: Explicit storage requests.
//   - Outputs: Shared storage observations and typed errors.
//   - Side effects: Explicit backend dispatch only.
// - Split-When:
//   - A capability needs independent admission or lifecycle rules.
// - Merge-When:
//   - A shared storage contract already owns the same responsibility.
// - Summary:
//   - Public storage capability namespace.
// - Description:
//   - Exposes verified canonical bundles and selected storage services.
// - Usage:
//   - Request storage through capability contracts.
// - Defaults:
//   - No implicit host resource access.

//! Storage capability contracts.

/// Verified canonical asset bundles and passive media overlays.
pub mod bundle;

#[cfg(any(test, feature = "test-fixtures"))]
pub use crate::selected::fixtures;
pub use crate::selected::{
    FileReadError, file, preferences, save, snapshot, staging,
};
