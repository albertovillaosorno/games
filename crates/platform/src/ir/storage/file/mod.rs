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
//   - Portable bounded regular-file snapshot dispatch and result admission.
// - Must-Not:
//   - Open native descriptors, inspect native permissions or decode files.
// - Allows:
//   - Inputs: Explicit path values and nonzero byte budgets.
//   - Outputs: Bounded ordinary or zeroizing secret bytes, typed failures.
//   - Side effects: Explicit backend dispatch only.
// - Split-When:
//   - A different storage operation needs independent admission.
// - Merge-When:
//   - Another contract owns identical snapshot semantics.
// - Summary:
//   - Admit regular-file snapshots without exposing native storage types.
// - Description:
//   - Preserve private credential handling across every backend.
// - Usage:
//   - Supply an explicit path value and a caller-selected byte budget.
// - Defaults:
//   - Reject unsupported storage and oversized backend results.

//! Portable bounded local file snapshots.

use std::ffi::OsStr;
use std::num::NonZeroU64;

use zeroize::Zeroizing;

pub use crate::ir::security::file::FileReadError;
use crate::ir::security::file::admit_size;

/// Owned credential bytes with zeroizing drop and redacted debug output.
pub type SecretSnapshot = Zeroizing<Vec<u8>>;

pub(crate) trait SnapshotBackend {
    fn read(
        path: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Vec<u8>, FileReadError>;

    fn read_secret(
        path: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<SecretSnapshot, FileReadError>;
}

fn admit_length(
    length: usize,
    maximum: NonZeroU64,
) -> Result<(), FileReadError> {
    let observed =
        u64::try_from(length).map_err(|_error| FileReadError::TooLarge)?;
    admit_size(observed, maximum.get())
}

/// Read an explicitly selected bounded regular-file snapshot.
///
/// Backends must admit the final component without following a symlink
/// or opening a special file, and reject observable content changes.
/// Ancestors must be trusted; the snapshot does not authenticate contents.
///
/// # Errors
/// Rejects unsupported storage, unsafe files and exceeded byte budgets.
pub fn snapshot(
    path: &OsStr,
    maximum: NonZeroU64,
) -> Result<Vec<u8>, FileReadError> {
    snapshot_with::<()>(path, maximum)
}

fn snapshot_with<Backend>(
    path: &OsStr,
    maximum: NonZeroU64,
) -> Result<Vec<u8>, FileReadError>
where
    Backend: SnapshotBackend,
{
    let bytes = Backend::read(path, maximum)?;
    admit_length(bytes.len(), maximum)?;
    Ok(bytes)
}

/// Read an owner-only bounded credential or private configuration file.
///
/// The backend must reject untrusted ownership, permissions, extra links
/// and observable content changes. Borrow the returned zeroizing buffer
/// during parsing rather than copying private bytes.
///
/// # Errors
/// Rejects unsupported storage, unsafe ownership or exceeded budgets.
pub fn snapshot_secret(
    path: &OsStr,
    maximum: NonZeroU64,
) -> Result<SecretSnapshot, FileReadError> {
    secret_with::<()>(path, maximum)
}

fn secret_with<Backend>(
    path: &OsStr,
    maximum: NonZeroU64,
) -> Result<SecretSnapshot, FileReadError>
where
    Backend: SnapshotBackend,
{
    let bytes = Backend::read_secret(path, maximum)?;
    admit_length(bytes.len(), maximum)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    struct Fake;

    impl SnapshotBackend for Fake {
        fn read(
            path: &OsStr,
            _maximum: NonZeroU64,
        ) -> Result<Vec<u8>, FileReadError> {
            if path == OsStr::new("empty") {
                Ok(Vec::new())
            } else if path == OsStr::new("missing") {
                Err(FileReadError::Io(ErrorKind::NotFound))
            } else {
                Ok(b"test".to_vec())
            }
        }

        fn read_secret(
            path: &OsStr,
            maximum: NonZeroU64,
        ) -> Result<SecretSnapshot, FileReadError> {
            if path == OsStr::new("untrusted") {
                return Err(FileReadError::UnsafeFile);
            }
            Self::read(path, maximum).map(Zeroizing::new)
        }
    }

    #[test]
    fn portable_file_results_respect_budget_and_errors()
    -> Result<(), FileReadError> {
        let exact = NonZeroU64::MIN.saturating_add(3);
        let valid = snapshot_with::<Fake>(OsStr::new("file"), exact)?
            == b"test"
            && snapshot_with::<Fake>(OsStr::new("empty"), exact)?.is_empty()
            && snapshot_with::<Fake>(OsStr::new("file"), NonZeroU64::MIN)
                == Err(FileReadError::TooLarge)
            && snapshot_with::<Fake>(OsStr::new("missing"), exact)
                == Err(FileReadError::Io(ErrorKind::NotFound));
        valid
            .then_some(())
            .ok_or(FileReadError::Io(ErrorKind::InvalidData))
    }

    #[test]
    fn portable_secrets_respect_budget_and_redaction()
    -> Result<(), FileReadError> {
        let exact = NonZeroU64::MIN.saturating_add(3);
        let secret = secret_with::<Fake>(OsStr::new("file"), exact)?;
        let valid = secret.as_slice() == b"test"
            && !format!("{secret:?}").contains("test")
            && secret_with::<Fake>(OsStr::new("file"), NonZeroU64::MIN)
                == Err(FileReadError::TooLarge)
            && secret_with::<Fake>(OsStr::new("untrusted"), exact)
                == Err(FileReadError::UnsafeFile);
        valid
            .then_some(())
            .ok_or(FileReadError::Io(ErrorKind::InvalidData))
    }
}
