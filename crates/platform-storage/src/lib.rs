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
pub mod bundle;
/// Bounded native regular-file snapshots.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod file;

/// Shared native storage fixtures for workspace tests.
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(test, feature = "test-fixtures"),
))]
pub mod fixtures;
/// Atomic persistence for portable display and input preferences.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod preferences;

/// Integrity-checked local save slots.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod save;

/// Private, bounded native archive staging.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod staging;

use std::io;
#[cfg(any(test, not(any(target_os = "linux", target_os = "android"))))]
use std::num::NonZeroU64;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::path::Path;

/// Read a bounded snapshot using the host's regular-file adapter.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use file::snapshot;

/// Admission failure for a native file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileReadError {
    /// Opening, inspecting or reading the pinned file failed.
    Io(io::ErrorKind),
    /// The file size or allocation exceeds the supplied limit.
    TooLarge,
    /// The file failed admission or its content metadata changed.
    UnsafeFile,
}

impl From<io::Error> for FileReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
