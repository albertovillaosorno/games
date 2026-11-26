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
//   - Native storage adapter composition.
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
//   - No operating-system resources opened implicitly.

//! Native storage adapters for verified local assets.

/// Read-only native canonical asset bundle access.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod bundle;

/// Bounded native regular-file snapshots.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod file;

/// Shared native storage fixtures for workspace tests.
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(test, feature = "test-fixtures"),
))]
pub(super) mod fixtures;
#[cfg(all(
    not(any(target_os = "linux", target_os = "android")),
    any(test, feature = "test-fixtures"),
))]
impl crate::ir::storage::fixtures::FixtureFactory for () {
    const AVAILABLE: bool = false;

    fn open_fixture()
    -> std::io::Result<Box<dyn crate::ir::storage::fixtures::FixtureStorage>>
    {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Native integration tests of the portable preference contracts.
#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
mod preferences;

/// Integrity-checked local save slots.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod save;

/// Private, bounded native archive staging.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod staging;

#[cfg(not(target_arch = "wasm32"))]
use std::env;
use std::ffi::{OsStr, OsString};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::fs::{self, Metadata};
use std::io;
use std::num::{NonZeroU64, NonZeroUsize};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::fd::{AsRawFd as _, OwnedFd};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::path::{Component, Path};

#[cfg(any(target_os = "linux", target_os = "android"))]
use file::DIRECTORY_FLAGS;
use games_assets::ContentHash;
#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::fcntl::open;
#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::sys::stat::Mode;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use crate::ir::storage::bundle::BundleError;
use crate::ir::storage::bundle::{BundleFactory, OpenBundle};
use crate::ir::storage::file::{
    FileReadError, SecretSnapshot, SnapshotBackend,
};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use crate::ir::storage::save::SaveError;
use crate::ir::storage::save::{OpenSave, SaveFactory};
use crate::ir::storage::{
    DirectoryEntry, PathKind, PathObservation, PathResolution,
    RelativeComponent,
};

#[cfg(any(target_os = "linux", target_os = "android"))]
impl SaveFactory for () {
    fn open(directory: &OsStr, maximum: NonZeroU64) -> OpenSave {
        Ok(Box::new(save::SaveStore::open(
            Path::new(directory),
            maximum,
        )?))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl SaveFactory for () {
    fn open(_directory: &OsStr, _maximum: NonZeroU64) -> OpenSave {
        Err(SaveError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl SnapshotBackend for () {
    fn read(
        path: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Vec<u8>, FileReadError> {
        file::snapshot(Path::new(path), maximum)
    }

    fn read_secret(
        path: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<SecretSnapshot, FileReadError> {
        file::snapshot_secret(Path::new(path), maximum)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl SnapshotBackend for () {
    fn read(
        _path: &OsStr,
        _maximum: NonZeroU64,
    ) -> Result<Vec<u8>, FileReadError> {
        Err(FileReadError::Io(io::ErrorKind::Unsupported))
    }

    fn read_secret(
        _path: &OsStr,
        _maximum: NonZeroU64,
    ) -> Result<SecretSnapshot, FileReadError> {
        Err(FileReadError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl BundleFactory for () {
    fn open(parent: &OsStr, digest: ContentHash) -> OpenBundle {
        bundle::open_bundle(Path::new(parent), digest)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl BundleFactory for () {
    fn open(_parent: &OsStr, _digest: ContentHash) -> OpenBundle {
        Err(BundleError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(in crate::software) fn executable_path() -> io::Result<PathBuf> {
    env::current_exe()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl PathObservation for () {
    fn canonical_path(path: &OsStr) -> io::Result<OsString> {
        fs::canonicalize(Path::new(path)).map(PathBuf::into_os_string)
    }

    fn directory_entries(
        path: &OsStr,
        maximum: NonZeroUsize,
    ) -> io::Result<Vec<DirectoryEntry>> {
        let directory = open(Path::new(path), DIRECTORY_FLAGS, Mode::empty())?;
        observe_directory(&directory, maximum)
    }

    fn path_kind(path: &OsStr) -> io::Result<PathKind> {
        let metadata = match fs::symlink_metadata(Path::new(path)) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PathKind::Missing);
            },
            Err(error) => return Err(error),
        };
        Ok(entry_kind(&metadata))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl PathObservation for () {
    fn canonical_path(_path: &OsStr) -> io::Result<OsString> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn directory_entries(
        _path: &OsStr,
        _maximum: NonZeroUsize,
    ) -> io::Result<Vec<DirectoryEntry>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn path_kind(_path: &OsStr) -> io::Result<PathKind> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl PathResolution for () {
    fn absolute(path: &OsStr) -> io::Result<bool> {
        Ok(Path::new(path).is_absolute())
    }

    fn equivalent(left: &OsStr, right: &OsStr) -> io::Result<bool> {
        Ok(Path::new(left) == Path::new(right))
    }

    fn extension(path: &OsStr) -> io::Result<Option<&OsStr>> {
        Ok(Path::new(path).extension())
    }

    fn has_prefix(path: &OsStr, prefix: &OsStr) -> io::Result<bool> {
        Ok(Path::new(path).starts_with(prefix))
    }

    fn join(base: &OsStr, child: &OsStr) -> io::Result<OsString> {
        Ok(Path::new(base).join(child).into_os_string())
    }

    fn leaf(path: &OsStr) -> io::Result<&OsStr> {
        Path::new(path)
            .file_name()
            .ok_or_else(|| io::ErrorKind::InvalidInput.into())
    }

    fn parent(path: &OsStr) -> io::Result<&OsStr> {
        Path::new(path)
            .parent()
            .map(Path::as_os_str)
            .ok_or_else(|| io::ErrorKind::InvalidInput.into())
    }

    fn relative_components(
        path: &OsStr,
    ) -> io::Result<Vec<RelativeComponent<'_>>> {
        Path::new(path)
            .components()
            .map(|component| match component {
                Component::CurDir => Ok(RelativeComponent::Current),
                Component::Normal(name) => Ok(RelativeComponent::Name(name)),
                Component::ParentDir => Ok(RelativeComponent::Parent),
                Component::Prefix(_) | Component::RootDir => {
                    Err(io::ErrorKind::InvalidInput.into())
                },
            })
            .collect()
    }

    fn strip_prefix<'path>(
        path: &'path OsStr,
        prefix: &OsStr,
    ) -> io::Result<&'path OsStr> {
        Path::new(path)
            .strip_prefix(prefix)
            .map(Path::as_os_str)
            .map_err(|_error| io::ErrorKind::InvalidInput.into())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl PathResolution for () {
    fn absolute(_path: &OsStr) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn equivalent(_left: &OsStr, _right: &OsStr) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn extension(_path: &OsStr) -> io::Result<Option<&OsStr>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn has_prefix(_path: &OsStr, _prefix: &OsStr) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn join(_base: &OsStr, _child: &OsStr) -> io::Result<OsString> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn leaf(_path: &OsStr) -> io::Result<&OsStr> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn parent(_path: &OsStr) -> io::Result<&OsStr> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn relative_components(
        _path: &OsStr,
    ) -> io::Result<Vec<RelativeComponent<'_>>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn strip_prefix<'path>(
        _path: &'path OsStr,
        _prefix: &OsStr,
    ) -> io::Result<&'path OsStr> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn entry_kind(kind: &Metadata) -> PathKind {
    if kind.is_file() {
        PathKind::RegularFile
    } else if kind.is_dir() {
        PathKind::Directory
    } else if kind.file_type().is_symlink() {
        PathKind::SymbolicLink
    } else {
        PathKind::Other
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn observe_directory(
    directory: &OwnedFd,
    maximum: NonZeroUsize,
) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = Vec::new();
    for result in pinned_directory_entries(directory)? {
        if entries.len() >= maximum.get() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let entry = result?;
        entries.try_reserve(1).map_err(io::Error::other)?;
        entries.push(DirectoryEntry {
            kind: entry_kind(&entry.metadata()?),
            name: entry.file_name(),
        });
    }
    Ok(entries)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn pinned_directory_entries(directory: &OwnedFd) -> io::Result<fs::ReadDir> {
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    fs::read_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::ir::storage::executable_path as observed_executable_path;
    use crate::ir::storage::file::snapshot;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn lexical_paths_preserve_opaque_input_and_relative_components()
    -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt as _;

        use crate::ir::storage::{
            join_path, parent_path, path_extension, relative_components,
            strip_path_prefix,
        };

        let base = OsStr::new("/package");
        let opaque = OsStr::from_bytes(b"file-\xff");
        let joined = join_path(base, opaque)?;
        let valid = joined.as_encoded_bytes() == b"/package/file-\xff"
            && parent_path(&joined)? == base
            && join_path(base, "../assets")? == "/package/../assets"
            && join_path(base, "/other")? == "/other"
            && parent_path(OsStr::new("file"))?.is_empty()
            && path_extension(OsStr::new(".hidden"))?.is_none()
            && path_extension(OsStr::new("a.tar.gz"))?
                == Some(OsStr::new("gz"))
            && relative_components(OsStr::new("./a/../b"))?
                == [
                    RelativeComponent::Current,
                    RelativeComponent::Name(OsStr::new("a")),
                    RelativeComponent::Parent,
                    RelativeComponent::Name(OsStr::new("b")),
                ]
            && relative_components(base).is_err()
            && strip_path_prefix(&joined, base)? == opaque
            && strip_path_prefix(&joined, OsStr::new("/pack")).is_err()
            && parent_path(OsStr::new("/")).is_err_and(|error| {
                error.kind() == io::ErrorKind::InvalidInput
            });
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn lexical_path_comparison_uses_components_not_text_prefixes()
    -> io::Result<()> {
        use crate::ir::storage::{
            path_has_prefix, path_is_absolute, path_leaf, paths_equivalent,
        };
        let root = OsStr::new("/repo/crates");
        let manifest = OsStr::new("/repo/crates/game/Cargo.toml");
        let valid = path_is_absolute(root)?
            && !path_is_absolute(OsStr::new("crates"))?
            && path_has_prefix(manifest, root)?
            && !path_has_prefix(OsStr::new("/repo/crates-other"), root)?
            && paths_equivalent(root, OsStr::new("/repo/./crates/"))?
            && !paths_equivalent(root, OsStr::new("/repo/crates/../crates"))?
            && path_leaf(manifest)? == "Cargo.toml"
            && path_leaf(OsStr::new("/")).is_err();
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn executable_location_matches_running_artifact() -> io::Result<()> {
        let observed = observed_executable_path()?;
        let expected = env::current_exe()?;
        (expected.is_absolute() && observed == expected.into_os_string())
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn directory_observation_preserves_link_kind_and_budget() -> io::Result<()>
    {
        use crate::ir::storage::{canonical_path, directory_entries};
        let root = fixtures::TestRoot::new()?;
        let nested = root.path().join("nested");
        fs::create_dir_all(&nested)?;
        let file = root.path().join("file");
        fs::write(&file, b"data")?;
        let link = root.path().join("link");
        fixtures::file_link(&nested, &link)?;
        let maximum = NonZeroUsize::MIN.saturating_add(2);
        let entries = directory_entries(root.path().as_os_str(), maximum)?;
        let expected = [
            ("file", PathKind::RegularFile),
            ("link", PathKind::SymbolicLink),
            ("nested", PathKind::Directory),
        ];
        let valid = entries.len() == expected.len()
            && entries.iter().zip(expected).all(|(entry, (name, kind))| {
                entry.name == name && entry.kind == kind
            })
            && directory_entries(root.path().as_os_str(), NonZeroUsize::MIN)
                .is_err()
            && directory_entries(link.as_os_str(), maximum).is_err()
            && canonical_path(link.as_os_str())?
                == canonical_path(nested.as_os_str())?;
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn directory_observation_remains_bound_to_open_directory() -> io::Result<()>
    {
        use crate::ir::storage::directory_entries;
        let root = fixtures::TestRoot::new()?;
        let original = root.path().join("original");
        let moved = root.path().join("moved");
        let alternate = root.path().join("alternate");
        fs::create_dir_all(&original)?;
        fs::create_dir_all(&alternate)?;
        fs::write(original.join("expected"), b"data")?;
        fs::write(alternate.join("other"), b"data")?;
        let pinned = open(&original, DIRECTORY_FLAGS, Mode::empty())?;
        fs::rename(&original, &moved)?;
        fixtures::file_link(&alternate, &original)?;
        let maximum = NonZeroUsize::MIN;
        let entries = observe_directory(&pinned, maximum)?;
        (entries
            == [DirectoryEntry {
                kind: PathKind::RegularFile,
                name: OsString::from("expected"),
            }]
            && directory_entries(original.as_os_str(), maximum).is_err())
        .then_some(())
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn path_observation_does_not_follow_final_links() -> io::Result<()> {
        use std::fs;

        use crate::ir::storage::path_kind;

        let root = fixtures::TestRoot::new()?;
        let file = root.path().join("file");
        let missing = root.path().join("missing");
        let link = root.path().join("link");
        fs::write(&file, b"fixture")?;
        fixtures::file_link(&missing, &link)?;
        let valid = path_kind(root.path().as_os_str())? == PathKind::Directory
            && path_kind(file.as_os_str())? == PathKind::RegularFile
            && path_kind(missing.as_os_str())? == PathKind::Missing
            && path_kind(link.as_os_str())? == PathKind::SymbolicLink;
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn snapshots_are_exact_and_bounded() -> Result<(), FileReadError> {
        use std::fs;

        let root = fixtures::TestRoot::new()?;
        let path = root.path().join("file");
        let max = NonZeroU64::new(4).unwrap_or(NonZeroU64::MIN);
        fs::write(&path, b"four")?;
        let exact = snapshot(path.as_os_str(), max)? == b"four";
        fs::write(&path, b"fives")?;
        let oversized =
            snapshot(path.as_os_str(), max) == Err(FileReadError::TooLarge);
        fs::write(&path, b"")?;
        (exact && oversized && snapshot(path.as_os_str(), max)?.is_empty())
            .then_some(())
            .ok_or(FileReadError::Io(io::ErrorKind::InvalidData))
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn unsupported_snapshot_does_not_open_a_path() {
        assert_eq!(
            snapshot(OsStr::new("unavailable"), NonZeroU64::MIN),
            Err(FileReadError::Io(io::ErrorKind::Unsupported))
        );
        assert_eq!(
            crate::ir::storage::file::snapshot_secret(
                OsStr::new("unavailable"),
                NonZeroU64::MIN,
            ),
            Err(FileReadError::Io(io::ErrorKind::Unsupported))
        );
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use games_assets::archive::ArchiveBudget;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use crate::ir::storage::staging::{
    ArchiveFactory, CanonicalFactory, OpenArchive, OpenCanonical,
};
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
use crate::ir::storage::staging::{
    AuthoredFactory, OpenAuthored, StagingError,
};

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl ArchiveFactory for () {
    fn open_archive(_parent: &OsStr) -> OpenArchive {
        Err(StagingError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl CanonicalFactory for () {
    fn open_canonical(
        _parent: &OsStr,
        _budget: ArchiveBudget,
    ) -> OpenCanonical {
        Err(StagingError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
impl AuthoredFactory for () {
    fn open_authored(_parent: &OsStr) -> OpenAuthored {
        Err(StagingError::Io(io::ErrorKind::Unsupported))
    }
}
