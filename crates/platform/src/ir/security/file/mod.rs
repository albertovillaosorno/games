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
//   - Shared file, directory and snapshot resource admission.
// - Must-Not:
//   - Open host paths or interpret file contents.
// - Allows:
//   - Inputs: Observed counts, explicit limits, names and borrowed streams.
//   - Outputs: Admitted directory entries and exact snapshot lengths.
//   - Side effects: Caller-owned stream reads and bounded allocations.
// - Split-When:
//   - Another storage service has independent resource admission.
// - Merge-When:
//   - A shared bounded-resource contract enforces identical invariants.
// - Summary:
//   - Host-independent size admission before native content access.
// - Description:
//   - Separates wire/storage bounds from addressable buffer capacity.
// - Usage:
//   - Admit observed metadata before opening contents; check allocation size.
// - Defaults:
//   - Reject excessive and unrepresentable sizes.

//! Shared snapshot admission before native content access.

use std::io::{Error, ErrorKind, Read, Result as IoResult};
use std::num::{NonZeroU64, NonZeroUsize};

use zeroize::Zeroizing;

use crate::ir::storage::{DirectoryEntry, PathKind};

/// Admission failure for a bounded regular-file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileReadError {
    /// Opening, inspecting or reading the pinned file failed.
    Io(ErrorKind),
    /// The file size or allocation exceeds the supplied limit.
    TooLarge,
    /// The file failed admission or its content metadata changed.
    UnsafeFile,
}

impl From<Error> for FileReadError {
    fn from(error: Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Admit an independently observed byte count against an explicit limit.
///
/// Streaming readers may admit storage larger than a single allocation.
///
/// # Errors
/// Rejects any observed size greater than the permitted maximum.
pub const fn admit_size(
    observed: u64,
    maximum: u64,
) -> Result<(), FileReadError> {
    if observed > maximum {
        Err(FileReadError::TooLarge)
    } else {
        Ok(())
    }
}

/// Admit a bounded byte count for one memory-resident buffer.
///
/// This check is independent of actual allocation availability.
///
/// # Errors
/// Rejects a count outside the maximum addressable byte-buffer length.
pub fn allocation_length(size: u64) -> Result<usize, FileReadError> {
    let length =
        usize::try_from(size).map_err(|_error| FileReadError::TooLarge)?;
    if isize::try_from(length).is_err() {
        return Err(FileReadError::TooLarge);
    }
    Ok(length)
}

/// Admit an observed memory buffer against its requested byte budget.
pub(crate) fn admit_length(
    length: usize,
    maximum: NonZeroU64,
) -> Result<(), FileReadError> {
    let observed =
        u64::try_from(length).map_err(|_error| FileReadError::TooLarge)?;
    admit_size(observed, maximum.get())
}

/// Read exactly the independently admitted length without excess allocation.
/// The one-byte overrun probe is zeroized for private snapshots as well.
pub(crate) fn read_exact_snapshot(
    input: &mut dyn Read,
    size: u64,
    bytes: &mut Vec<u8>,
) -> Result<(), FileReadError> {
    let length = allocation_length(size)?;
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| FileReadError::TooLarge)?;
    bytes.resize(length, 0);
    input.read_exact(bytes)?;
    let mut extra = Zeroizing::new([0u8; 1]);
    match input.read_exact(&mut *extra) {
        Ok(()) => Err(FileReadError::TooLarge),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Admit unique direct names and deterministic order without path resolution.
pub(crate) fn admit_directory_entries(
    entries: &mut [DirectoryEntry],
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
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    if entries
        .iter()
        .zip(entries.iter().skip(1))
        .any(|(left, right)| left.name == right.name)
    {
        return Err(ErrorKind::InvalidData.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::slice::from_mut;

    use super::*;

    #[test]
    fn file_size_admission_preserves_empty_and_exact_limits() {
        assert_eq!(admit_size(0, 0), Ok(()));
        assert_eq!(admit_size(4, 4), Ok(()));
        assert_eq!(admit_size(5, 4), Err(FileReadError::TooLarge));
        assert_eq!(admit_size(u64::MAX, u64::MAX), Ok(()));
    }

    #[test]
    fn allocation_admission_is_distinct_from_streaming_size()
    -> Result<(), String> {
        let maximum =
            u64::try_from(isize::MAX).map_err(|error| error.to_string())?;
        let excessive = maximum
            .checked_add(1)
            .ok_or_else(|| String::from("unrepresentable test size"))?;
        if allocation_length(0) != Ok(0)
            || allocation_length(4) != Ok(4)
            || allocation_length(maximum).ok() != usize::try_from(maximum).ok()
            || allocation_length(excessive) != Err(FileReadError::TooLarge)
            || admit_size(excessive, excessive).is_err()
        {
            return Err(String::from("inconsistent buffer capacity admission"));
        }
        Ok(())
    }

    #[test]
    fn exact_snapshot_admission_rejects_short_and_extended_streams()
    -> Result<(), FileReadError> {
        for (mut input, length, expected) in [
            (b"".as_slice(), 0, Ok(())),
            (b"data".as_slice(), 4, Ok(())),
            (
                b"dat".as_slice(),
                4,
                Err(FileReadError::Io(ErrorKind::UnexpectedEof)),
            ),
            (b"extra".as_slice(), 4, Err(FileReadError::TooLarge)),
            (b"extra".as_slice(), 0, Err(FileReadError::TooLarge)),
        ] {
            let mut bytes = Vec::new();
            let original = input;
            let actual = read_exact_snapshot(&mut input, length, &mut bytes);
            if actual != expected || (actual.is_ok() && bytes != original) {
                return Err(FileReadError::UnsafeFile);
            }
        }
        let mut input = b"untouched".as_slice();
        let mut bytes = Vec::new();
        if read_exact_snapshot(&mut input, u64::MAX, &mut bytes)
            != Err(FileReadError::TooLarge)
            || !bytes.is_empty()
            || input != b"untouched"
        {
            return Err(FileReadError::UnsafeFile);
        }
        Ok(())
    }
    #[test]
    fn directory_admission_rejects_invalid_names_and_counts() {
        let mut valid = DirectoryEntry {
            kind: PathKind::RegularFile,
            name: OsString::from("file"),
        };
        let maximum = NonZeroUsize::MIN;
        assert!(admit_directory_entries(from_mut(&mut valid), maximum).is_ok());
        assert!(
            admit_directory_entries(
                &mut [valid.clone(), valid.clone()],
                maximum
            )
            .is_err()
        );
        let two = maximum.saturating_add(1);
        let mut distinct = [
            DirectoryEntry {
                kind: PathKind::Directory,
                name: OsString::from("z"),
            },
            valid.clone(),
        ];
        assert!(admit_directory_entries(&mut distinct, two).is_ok());
        assert_eq!(distinct.first(), Some(&valid));
        let duplicate = DirectoryEntry {
            kind: PathKind::Directory,
            name: valid.name.clone(),
        };
        assert!(admit_directory_entries(&mut [valid, duplicate], two).is_err());
        for name in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            let entry = DirectoryEntry {
                kind: PathKind::Directory,
                name: OsString::from(name),
            };
            assert!(admit_directory_entries(&mut [entry], maximum).is_err());
        }
        let missing = DirectoryEntry {
            kind: PathKind::Missing,
            name: OsString::from("missing"),
        };
        assert!(admit_directory_entries(&mut [missing], maximum).is_err());
    }
}
