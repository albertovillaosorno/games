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

use std::ffi::OsString;
use std::io::Result as IoResult;

#[cfg(any(test, feature = "test-fixtures"))]
pub use crate::selected::fixtures;
pub use crate::selected::{
    FileReadError, file, preferences, save, snapshot, staging,
};

/// Backend observation of the process executable location.
pub(crate) trait ExecutableLocation {
    fn executable_path() -> IoResult<OsString>;
}

/// Observe the running executable's location for explicit local resolution.
///
/// This does not search for assets, inspect their contents or grant trust.
/// # Errors
/// Returns a host error if the location cannot be determined.
pub fn executable_path() -> IoResult<OsString> {
    <() as ExecutableLocation>::executable_path()
}
