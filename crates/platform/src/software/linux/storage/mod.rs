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
pub mod fixtures;
/// Explicitly unavailable native rejection-test fixtures.
#[cfg(all(
    not(any(target_os = "linux", target_os = "android")),
    any(test, feature = "test-fixtures"),
))]
pub mod fixtures {
    use std::io;
    use std::path::Path;

    /// Whether link and executable fixture operations are implemented.
    pub const AVAILABLE: bool = false;

    /// Reject unavailable fixture link creation.
    /// # Errors
    /// Always returns Unsupported.
    pub fn file_link(_target: &Path, _link: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Reject unavailable fixture permission changes.
    /// # Errors
    /// Always returns Unsupported.
    pub fn owner_executable(_path: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Atomic persistence for portable display and input preferences.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod preferences;

/// Integrity-checked local save slots.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod save;

/// Private, bounded native archive staging.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod staging;

/// Preferences require a verified atomic storage adapter.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub mod preferences {
    /// Whether preference persistence is implemented on the target.
    pub const AVAILABLE: bool = false;
}

/// Saves require a verified atomic storage adapter.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub mod save {
    /// Whether save persistence is implemented on the target.
    pub const AVAILABLE: bool = false;
}

/// Staging requires a verified descriptor-relative storage adapter.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub mod staging {
    /// Whether the target provides private archive staging.
    pub const AVAILABLE: bool = false;
}

#[cfg(not(target_arch = "wasm32"))]
use std::env;
use std::ffi::OsStr;
use std::io;
use std::num::NonZeroU64;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use games_assets::{ContentHash, ManifestLimits};

use crate::ir::storage::bundle::{BundleError, BundleFactory, CanonicalBundle};
use crate::ir::storage::file::{
    FileReadError, SecretSnapshot, SnapshotBackend,
};
use crate::ir::storage::{PathKind, PathObservation};

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
    fn open(
        parent: &OsStr,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<CanonicalBundle, BundleError> {
        bundle::open_bundle(Path::new(parent), digest, limits)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl BundleFactory for () {
    fn open(
        _parent: &OsStr,
        _digest: ContentHash,
        _limits: ManifestLimits,
    ) -> Result<CanonicalBundle, BundleError> {
        Err(BundleError::Io(io::ErrorKind::Unsupported))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(in crate::software) fn executable_path() -> io::Result<PathBuf> {
    env::current_exe()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl PathObservation for () {
    fn path_kind(path: &OsStr) -> io::Result<PathKind> {
        use std::fs;

        let metadata = match fs::symlink_metadata(Path::new(path)) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PathKind::Missing);
            },
            Err(error) => return Err(error),
        };
        Ok(if metadata.is_file() {
            PathKind::RegularFile
        } else if metadata.is_dir() {
            PathKind::Directory
        } else if metadata.file_type().is_symlink() {
            PathKind::SymbolicLink
        } else {
            PathKind::Other
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl PathObservation for () {
    fn path_kind(_path: &OsStr) -> io::Result<PathKind> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::ir::storage::executable_path as observed_executable_path;
    use crate::ir::storage::file::snapshot;

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
