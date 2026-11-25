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
pub mod file;

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

/// Safe file admission is unavailable until the target has a backend.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub mod file {
    use std::io::ErrorKind;
    use std::num::NonZeroU64;
    use std::path::Path;

    pub use super::snapshot;
    pub use crate::ir::security::file::FileReadError;

    /// Reject secret reads before touching any host resource.
    /// # Errors
    /// Always reports unsupported storage admission.
    pub const fn snapshot_secret(
        _path: &Path,
        _maximum: NonZeroU64,
    ) -> Result<Vec<u8>, FileReadError> {
        Err(FileReadError::Io(ErrorKind::Unsupported))
    }
}

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
#[cfg(any(test, not(any(target_os = "linux", target_os = "android"))))]
use std::num::NonZeroU64;
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// Read a bounded snapshot using the host's regular-file adapter.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use file::snapshot;
use games_assets::{ContentHash, ManifestLimits};

/// Shared portable snapshot admission failure.
pub use crate::ir::security::file::FileReadError;
use crate::ir::storage::bundle::{BundleError, BundleFactory, CanonicalBundle};

/// Reject a snapshot request when safe regular-file admission is unavailable.
///
/// No path lookup or content access is attempted.
/// # Errors
/// Always returns an unsupported host error.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub const fn snapshot(
    _path: &Path,
    _maximum: NonZeroU64,
) -> Result<Vec<u8>, FileReadError> {
    Err(FileReadError::Io(io::ErrorKind::Unsupported))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::ir::storage::executable_path as observed_executable_path;

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
    fn snapshots_are_exact_and_bounded() -> Result<(), FileReadError> {
        use std::fs;

        let root = fixtures::TestRoot::new()?;
        let path = root.path().join("file");
        let max = NonZeroU64::new(4).unwrap_or(NonZeroU64::MIN);
        fs::write(&path, b"four")?;
        let exact = snapshot(&path, max)? == b"four";
        fs::write(&path, b"fives")?;
        let oversized = snapshot(&path, max) == Err(FileReadError::TooLarge);
        fs::write(&path, b"")?;
        (exact && oversized && snapshot(&path, max)?.is_empty())
            .then_some(())
            .ok_or(FileReadError::Io(io::ErrorKind::InvalidData))
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn unsupported_snapshot_does_not_open_a_path() {
        assert_eq!(
            snapshot(Path::new("unavailable"), NonZeroU64::MIN),
            Err(FileReadError::Io(io::ErrorKind::Unsupported))
        );
    }
}
