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
/// Explicitly enabled private test storage.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixtures;
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

use crate::ir::security::file::admit_directory_entries;

/// A direct entry in a bounded local directory observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// The observed final entry kind.
    pub kind: PathKind,
    /// A single opaque path component.
    pub name: OsString,
}

/// Backend observation of the application artifact and private data root.
pub(crate) trait ExecutableLocation {
    fn application_data_path() -> IoResult<OsString>;

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
        access: DirectoryAccess,
    ) -> IoResult<Vec<DirectoryEntry>>;
    fn path_kind(path: &OsStr) -> IoResult<PathKind>;
}

pub(crate) enum DirectoryAccess {
    Observed,
    OwnerControlled,
}

/// A lexical component of a relative local path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelativeComponent<'path> {
    /// An explicitly selected current directory.
    Current,
    /// An ordinary local name, without interpreting its encoding.
    Name(&'path OsStr),
    /// An explicitly selected parent directory.
    Parent,
}

/// Local path syntax, without access to the filesystem.
pub(crate) trait PathResolution {
    fn absolute(path: &OsStr) -> IoResult<bool>;
    fn equivalent(left: &OsStr, right: &OsStr) -> IoResult<bool>;
    fn extension(path: &OsStr) -> IoResult<Option<&OsStr>>;
    fn has_prefix(path: &OsStr, prefix: &OsStr) -> IoResult<bool>;
    fn join(base: &OsStr, child: &OsStr) -> IoResult<OsString>;
    fn leaf(path: &OsStr) -> IoResult<&OsStr>;
    fn parent(path: &OsStr) -> IoResult<&OsStr>;
    fn relative_components(
        path: &OsStr,
    ) -> IoResult<Vec<RelativeComponent<'_>>>;
    fn strip_prefix<'path>(
        path: &'path OsStr,
        prefix: &OsStr,
    ) -> IoResult<&'path OsStr>;
}

/// Resolve the canonical local path, explicitly following links.
/// This is an observation only, not a grant of file access.
/// # Errors
/// Returns an unavailable or failed native resolution.
pub fn canonical_path(path: &OsStr) -> IoResult<OsString> {
    <() as PathObservation>::canonical_path(path)
}

/// Observe the canonical location of an existing or future directory.
///
/// Resolve the deepest existing ancestor, then append the missing names.
/// Existing links are followed. This is a preflight observation, not a
/// guarantee of containment if ancestors change before publication.
/// # Errors
/// Rejects relative paths, invalid components, non-directory ancestors,
/// unresolved links and unavailable native path observations.
pub fn canonical_future_directory(path: &OsStr) -> IoResult<OsString> {
    if !path_is_absolute(path)? {
        return Err(ErrorKind::InvalidInput.into());
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while path_kind(ancestor)? == PathKind::Missing {
        suffix.push(path_leaf(ancestor)?);
        ancestor = parent_path(ancestor)?;
    }
    let mut resolved = canonical_path(ancestor)?;
    if path_kind(&resolved)? != PathKind::Directory {
        return Err(ErrorKind::NotADirectory.into());
    }
    for child in suffix.into_iter().rev() {
        resolved = join_path(&resolved, child)?;
    }
    Ok(resolved)
}

/// Observe the direct entries of a directory without following final links.
/// Membership and metadata may change during observation.
/// # Errors
/// Reports unavailable observation, excess entries or invalid or duplicate
/// names.
pub fn directory_entries(
    path: &OsStr,
    maximum: NonZeroUsize,
) -> IoResult<Vec<DirectoryEntry>> {
    entries_with(path, maximum, DirectoryAccess::Observed)
}

/// Observe a directory controlled by the current owner.
///
/// Backends must reject directories writable by other users and check
/// the pinned directory's access policy before and after enumeration.
/// As with other snapshots, ancestors must remain trusted.
/// # Errors
/// Reports unavailable or unsafe directories and invalid entry observations.
pub fn trusted_directory_entries(
    path: &OsStr,
    maximum: NonZeroUsize,
) -> IoResult<Vec<DirectoryEntry>> {
    entries_with(path, maximum, DirectoryAccess::OwnerControlled)
}

fn entries_with(
    path: &OsStr,
    maximum: NonZeroUsize,
    access: DirectoryAccess,
) -> IoResult<Vec<DirectoryEntry>> {
    let mut entries =
        <() as PathObservation>::directory_entries(path, maximum, access)?;
    admit_directory_entries(&mut entries, maximum)?;
    Ok(entries)
}

/// Observe an admitted app-private data directory supplied at launch.
///
/// This is distinct from the installed artifact directory and does not
/// discover media or grant access to its contents.
/// # Errors
/// Reports unsupported application data or a host observation error.
pub fn application_data_path() -> IoResult<OsString> {
    <() as ExecutableLocation>::application_data_path()
}

/// Observe the running executable's location for explicit local resolution.
///
/// This does not search for assets, inspect their contents or grant trust.
/// # Errors
/// Returns a host error if the location cannot be determined.
pub fn executable_path() -> IoResult<OsString> {
    <() as ExecutableLocation>::executable_path()
}

/// Interpret a child using the selected host's local path syntax.
///
/// This is lexical only: an absolute child replaces the base and parent
/// components are not resolved. The result does not grant file access.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn join_path<Child>(base: &OsStr, child: &Child) -> IoResult<OsString>
where
    Child: AsRef<OsStr> + ?Sized,
{
    <() as PathResolution>::join(base, child.as_ref())
}

/// Identify the lexical parent of a local path without accessing storage.
///
/// A single relative filename has an empty parent. A root has no parent.
/// # Errors
/// Reports unavailable interpretation or a path with no parent.
pub fn parent_path(path: &OsStr) -> IoResult<&OsStr> {
    <() as PathResolution>::parent(path)
}

/// Compare local paths by lexical component equality, not file identity.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn paths_equivalent(left: &OsStr, right: &OsStr) -> IoResult<bool> {
    <() as PathResolution>::equivalent(left, right)
}

/// Test whether a path has the given lexical component prefix.
///
/// This is not a containment or access guarantee: no links or parent
/// components are resolved.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn path_has_prefix(path: &OsStr, prefix: &OsStr) -> IoResult<bool> {
    <() as PathResolution>::has_prefix(path, prefix)
}

/// Test whether either local path is a lexical prefix of the other.
///
/// Resolve directory locations first when comparing trusted linked roots.
/// This is not a file identity or access guarantee.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn paths_overlap(left: &OsStr, right: &OsStr) -> IoResult<bool> {
    Ok(path_has_prefix(left, right)? || path_has_prefix(right, left)?)
}

/// Identify an absolute local path using host syntax.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn path_is_absolute(path: &OsStr) -> IoResult<bool> {
    <() as PathResolution>::absolute(path)
}

/// Identify the final filename component without accessing storage.
/// # Errors
/// Reports unavailable interpretation or a path without a filename.
pub fn path_leaf(path: &OsStr) -> IoResult<&OsStr> {
    <() as PathResolution>::leaf(path)
}

/// Identify a local filename extension without accessing storage.
/// # Errors
/// Reports unavailable local path interpretation.
pub fn path_extension(path: &OsStr) -> IoResult<Option<&OsStr>> {
    <() as PathResolution>::extension(path)
}

/// List lexical relative path components, preserving parent observations.
/// # Errors
/// Rejects absolute paths and unavailable local interpretation.
pub fn relative_components(
    path: &OsStr,
) -> IoResult<Vec<RelativeComponent<'_>>> {
    <() as PathResolution>::relative_components(path)
}

/// Strip a lexical component prefix without following links.
/// # Errors
/// Reports unavailable interpretation or a mismatched prefix.
pub fn strip_path_prefix<'path>(
    path: &'path OsStr,
    prefix: &OsStr,
) -> IoResult<&'path OsStr> {
    <() as PathResolution>::strip_prefix(path, prefix)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_directories_resolve_existing_ancestors_without_creation()
    -> IoResult<()> {
        if !fixtures::AVAILABLE {
            return Ok(());
        }
        let root = fixtures::TestRoot::new()?;
        root.create_directories("existing")?;
        let existing = join_path(root.location(), "existing")?;
        root.link(&existing, "alias")?;
        let alias = join_path(root.location(), "alias")?;
        let missing = join_path(&alias, "new/nested")?;
        let resolved = join_path(&canonical_path(&existing)?, "new/nested")?;
        let valid = canonical_future_directory(&alias)?
            == canonical_path(&existing)?
            && canonical_future_directory(&missing)? == resolved
            && path_kind(&resolved)? == PathKind::Missing
            && paths_overlap(&resolved, &canonical_path(&existing)?)?
            && paths_overlap(&canonical_path(&existing)?, &resolved)?
            && paths_overlap(&resolved, &resolved)?
            && !paths_overlap(
                &resolved,
                &join_path(&canonical_path(&existing)?, "new/nested-other")?,
            )?;
        valid
            .then_some(())
            .ok_or_else(|| ErrorKind::InvalidData.into())
    }

    #[test]
    fn future_directories_reject_non_directory_and_unresolved_ancestors()
    -> IoResult<()> {
        if !fixtures::AVAILABLE {
            return Ok(());
        }
        let root = fixtures::TestRoot::new()?;
        root.write_file("file", b"data")?;
        let missing = join_path(root.location(), "missing")?;
        root.link(&missing, "dangling")?;
        let valid = ["file", "file/child", "dangling", "dangling/child"]
            .into_iter()
            .map(|name| join_path(root.location(), name))
            .collect::<IoResult<Vec<_>>>()?
            .iter()
            .all(|path| canonical_future_directory(path).is_err())
            && canonical_future_directory(OsStr::new("relative")).is_err();
        valid
            .then_some(())
            .ok_or_else(|| ErrorKind::InvalidData.into())
    }
}
