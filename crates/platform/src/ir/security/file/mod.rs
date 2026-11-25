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
//   - Shared file-size and snapshot allocation admission.
// - Must-Not:
//   - Open a path, read a descriptor or interpret file contents.
// - Allows:
//   - Inputs: Observed byte counts, explicit limits and error values.
//   - Outputs: Admitted lengths or typed snapshot rejections.
//   - Side effects: None.
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

use std::io::{Error, ErrorKind};

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

#[cfg(test)]
mod tests {
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
}
