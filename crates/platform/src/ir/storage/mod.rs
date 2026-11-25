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
/// Bounded ordinary and private file snapshots.
pub mod file;
/// Configured persistent asset and executable locations.
pub mod layout;
/// Portable display, audio and input preferences.
pub mod preferences;
/// Bounded integrity-checked numeric save slots.
pub mod save;

use std::ffi::{OsStr, OsString};
use std::io::Result as IoResult;

#[cfg(any(test, feature = "test-fixtures"))]
pub use crate::selected::fixtures;
pub use crate::selected::staging;

/// Backend observation of the process executable location.
pub(crate) trait ExecutableLocation {
    fn executable_path() -> IoResult<OsString>;
}

/// Observed kind of a local entry, without following its final link.
///
/// This is not a verified descriptor or permission to access its contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathKind {
    /// An existing directory.
    Directory,
    /// No entry exists at the observed path.
    Missing,
    /// An existing special entry, such as a device or socket.
    Other,
    /// An existing regular file.
    RegularFile,
    /// A symbolic link, including one with a missing target.
    SymbolicLink,
}

/// Native metadata observation without exposing a host metadata object.
pub(crate) trait PathObservation {
    fn path_kind(path: &OsStr) -> IoResult<PathKind>;
}

/// Observe the running executable's location for explicit local resolution.
///
/// This does not search for assets, inspect their contents or grant trust.
/// # Errors
/// Returns a host error if the location cannot be determined.
pub fn executable_path() -> IoResult<OsString> {
    <() as ExecutableLocation>::executable_path()
}

/// Observe the final entry kind for preflight diagnostics.
///
/// Use descriptor-based admission for subsequent reads or publication:
/// the observation is not a promise that the entry will remain unchanged.
///
/// # Errors
/// Returns metadata or unavailable capability errors.
pub fn path_kind(path: &OsStr) -> IoResult<PathKind> {
    <() as PathObservation>::path_kind(path)
}
