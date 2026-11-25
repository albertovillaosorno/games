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
/// Private staging and atomic publication.
pub mod staging;

use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Result as IoResult};
use std::num::NonZeroUsize;

#[cfg(any(test, feature = "test-fixtures"))]
pub use crate::selected::fixtures;
/// A direct entry in a bounded local directory observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// The observed final entry kind.
    pub kind: PathKind,
    /// A single opaque path component.
    pub name: OsString,
}

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
    fn canonical_path(path: &OsStr) -> IoResult<OsString>;
    fn directory_entries(
        path: &OsStr,
        maximum: NonZeroUsize,
    ) -> IoResult<Vec<DirectoryEntry>>;
    fn path_kind(path: &OsStr) -> IoResult<PathKind>;
}

/// Resolve the canonical local path, explicitly following links.
/// This is an observation only, not a grant of file access.
/// # Errors
/// Returns an unavailable or failed native resolution.
pub fn canonical_path(path: &OsStr) -> IoResult<OsString> {
    <() as PathObservation>::canonical_path(path)
}

/// Observe the direct entries of a directory without following final links.
/// # Errors
/// Reports unavailable observation, excessive entries or invalid names.
pub fn directory_entries(
    path: &OsStr,
    maximum: NonZeroUsize,
) -> IoResult<Vec<DirectoryEntry>> {
    let mut entries =
        <() as PathObservation>::directory_entries(path, maximum)?;
    admit_entries(&entries, maximum)?;
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
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

fn admit_entries(
    entries: &[DirectoryEntry],
    maximum: NonZeroUsize,
) -> IoResult<()> {
    if entries.len() > maximum.get()
        || entries.iter().any(|entry| {
            let name = entry.name.as_encoded_bytes();
            entry.kind == PathKind::Missing
                || matches!(name, b"" | b"." | b"..")
                || name.iter().any(|byte| matches!(byte, b'/' | b'\\' | 0))
        })
    {
        return Err(ErrorKind::InvalidData.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::slice::from_ref;

    use super::*;

    #[test]
    fn directory_admission_rejects_invalid_names_and_counts() {
        let valid = DirectoryEntry {
            kind: PathKind::RegularFile,
            name: OsString::from("file"),
        };
        let maximum = NonZeroUsize::MIN;
        assert!(admit_entries(from_ref(&valid), maximum).is_ok());
        assert!(admit_entries(&[valid.clone(), valid], maximum).is_err());
        for name in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            let entry = DirectoryEntry {
                kind: PathKind::Directory,
                name: OsString::from(name),
            };
            assert!(admit_entries(&[entry], maximum).is_err());
        }
        let missing = DirectoryEntry {
            kind: PathKind::Missing,
            name: OsString::from("missing"),
        };
        assert!(admit_entries(&[missing], maximum).is_err());
    }
}
