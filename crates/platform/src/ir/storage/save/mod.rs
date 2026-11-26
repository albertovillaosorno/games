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
//   - Portable numeric save-slot requests, admission and receipts.
// - Must-Not:
//   - Open host paths, expose descriptors or interpret title payloads.
// - Allows:
//   - Inputs: Explicit directory value, budget, slot and payload.
//   - Outputs: Verified payloads, durability receipts and typed failures.
//   - Side effects: Explicit selected backend operations only.
// - Split-When:
//   - Another storage capability needs independent lifecycle semantics.
// - Merge-When:
//   - A shared contract already owns numeric save-slot persistence.
// - Summary:
//   - Isolate bounded atomic save persistence behind a portable port.
// - Description:
//   - Keep envelope limits consistent before and after backend dispatch.
// - Usage:
//   - Open a dedicated directory and read or replace a numeric slot.
// - Defaults:
//   - Reject unsupported storage, malformed envelopes and excess sizes.

//! Portable save-slot capability.

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{Error, ErrorKind, Read};
use std::num::NonZeroU64;

use games::safety::{
    SAVE_HEADER, SafetyError, admit_save_size, encode_save_header,
    save_envelope_limit, validate_save_header,
};

use super::file::FileReadError;
use crate::ir::security::file::{admit_size, read_exact_snapshot};

/// Save admission or pre-publication failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveError {
    /// The save envelope is invalid or exceeds its payload limit.
    Envelope(SafetyError),
    /// A native storage operation failed before publication.
    Io(ErrorKind),
    /// Bounded file admission rejected the requested slot.
    Read(FileReadError),
    /// The directory or destination is linked or otherwise untrusted.
    UnsafeStorage,
}

impl From<Error> for SaveError {
    fn from(error: Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<SafetyError> for SaveError {
    fn from(error: SafetyError) -> Self {
        Self::Envelope(error)
    }
}

impl From<FileReadError> for SaveError {
    fn from(error: FileReadError) -> Self {
        Self::Read(error)
    }
}

/// Save publication succeeded; directory sync can fail after publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveReceipt {
    /// Error here means the new save is visible but crash durability is
    /// uncertain. It does not mean the old save was preserved.
    pub durability: Result<(), ErrorKind>,
}

/// One encoded, admitted save with a borrowed, unchanged payload.
///
/// The separate header avoids allocating a second copy of the payload.
#[derive(Debug)]
pub struct PreparedSave<'payload> {
    header: [u8; SAVE_HEADER],
    payload: &'payload [u8],
    size: u64,
}

impl<'payload> PreparedSave<'payload> {
    /// Exact encoded byte count, including the fixed header.
    #[must_use]
    pub const fn encoded_size(&self) -> u64 {
        self.size
    }

    /// Canonical fixed-width encoded header.
    #[must_use]
    pub const fn header(&self) -> &[u8; SAVE_HEADER] {
        &self.header
    }

    /// Prepare the complete envelope before dispatching a slot write.
    ///
    /// # Errors
    /// Rejects payloads exceeding the requested or format limit.
    pub fn new(
        payload: &'payload [u8],
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        let header = encode_save_header(payload, maximum)?;
        let size = header
            .len()
            .checked_add(payload.len())
            .and_then(|length| u64::try_from(length).ok())
            .ok_or(SafetyError::TooLarge)?;
        Ok(Self { header, payload, size })
    }

    /// Exact borrowed bytes following the encoded header.
    #[must_use]
    pub const fn payload(&self) -> &'payload [u8] {
        self.payload
    }
}

/// A complete save payload whose header and checksum have been verified.
#[derive(Debug)]
pub struct VerifiedSave(Vec<u8>);

impl VerifiedSave {
    /// Transfer the verified payload without copying it.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> {
        self.0
    }

    /// Verify one encoded save from an independently sized stream.
    ///
    /// # Errors
    /// Rejects oversized, truncated, extended or corrupt envelopes.
    pub fn read(
        input: &mut dyn Read,
        size: u64,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        admit_size(size, save_envelope_limit(maximum))?;
        let header_size = u64::try_from(SAVE_HEADER).unwrap_or(u64::MAX);
        let length = size
            .checked_sub(header_size)
            .ok_or(SafetyError::InvalidSave)?;
        let mut bytes = [0; SAVE_HEADER];
        input.read_exact(&mut bytes).map_err(FileReadError::from)?;
        let header = validate_save_header(&bytes, length, maximum)?;
        let mut payload = Vec::new();
        read_exact_snapshot(input, length, &mut payload)?;
        header.verify(&payload)?;
        Ok(Self(payload))
    }
}

pub(crate) type OpenSave = Result<Box<dyn SaveBackend>, SaveError>;

/// Private selected backend for opening a dedicated save directory.
pub(crate) trait SaveFactory {
    fn open(directory: &OsStr, maximum: NonZeroU64) -> OpenSave;
}

/// A pinned backend reads admitted envelopes and publishes atomically.
pub(crate) trait SaveBackend: Debug + Send + Sync {
    fn read_slot(&self, slot: u32) -> Result<VerifiedSave, SaveError>;

    fn write_slot(
        &self,
        slot: u32,
        save: PreparedSave<'_>,
    ) -> Result<SaveReceipt, SaveError>;
}

/// Dedicated, bounded numeric save slots.
///
/// Backends must reject untrusted storage and malformed or incomplete
/// envelopes. Concurrent writes have last-publication-wins semantics.
/// A failure after publication is reported separately from rejection.
#[derive(Debug)]
pub struct SaveStore {
    backend: Box<dyn SaveBackend>,
    maximum: NonZeroU64,
}

impl SaveStore {
    fn admit(&self, payload: &[u8]) -> Result<(), SaveError> {
        let size = u64::try_from(payload.len())
            .map_err(|_error| SafetyError::TooLarge)?;
        let _length = admit_save_size(size, self.maximum)?;
        Ok(())
    }

    /// Load a slot and return its verified payload.
    ///
    /// # Errors
    /// Rejects unsafe storage, missing or malformed slots and excess sizes.
    pub fn load(&self, slot: u32) -> Result<Vec<u8>, SaveError> {
        let payload = self.backend.read_slot(slot)?.into_payload();
        self.admit(&payload)?;
        Ok(payload)
    }

    /// Open an explicitly selected, trusted save directory.
    ///
    /// # Errors
    /// Rejects unsupported storage and unsafe directory values.
    pub fn open(
        directory: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        Self::open_with::<()>(directory, maximum)
    }

    fn open_with<Factory>(
        directory: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError>
    where
        Factory: SaveFactory,
    {
        Ok(Self {
            backend: Factory::open(directory, maximum)?,
            maximum,
        })
    }

    /// Atomically replace a numeric slot with a bounded payload.
    ///
    /// # Errors
    /// Rejects excess sizes before dispatch and pre-publication failures.
    /// The returned receipt distinguishes a post-publication sync failure.
    pub fn store(
        &self,
        slot: u32,
        payload: &[u8],
    ) -> Result<SaveReceipt, SaveError> {
        self.backend
            .write_slot(slot, PreparedSave::new(payload, self.maximum)?)
    }
}

#[cfg(test)]
mod tests {
    use games::safety::encode_save;

    use super::*;

    #[derive(Debug)]
    struct Fake;

    impl SaveFactory for Fake {
        fn open(directory: &OsStr, maximum: NonZeroU64) -> OpenSave {
            if directory == OsStr::new("saves") && maximum.get() == 4 {
                Ok(Box::new(Self))
            } else {
                Err(SaveError::UnsafeStorage)
            }
        }
    }

    impl SaveBackend for Fake {
        fn read_slot(&self, slot: u32) -> Result<VerifiedSave, SaveError> {
            let maximum = NonZeroU64::MIN.saturating_add(4);
            let payload: &[u8] = match slot {
                0 => b"",
                1 | 4 => b"four",
                2 => b"large",
                _ => {
                    return Err(SaveError::Read(FileReadError::Io(
                        ErrorKind::NotFound,
                    )));
                },
            };
            let mut bytes = encode_save(payload, maximum)?;
            if slot == 4 {
                let last = bytes.last_mut().ok_or(SafetyError::InvalidSave)?;
                *last = 0;
            }
            let size = u64::try_from(bytes.len())
                .map_err(|_error| SafetyError::TooLarge)?;
            VerifiedSave::read(&mut bytes.as_slice(), size, maximum)
        }

        fn write_slot(
            &self,
            slot: u32,
            save: PreparedSave<'_>,
        ) -> Result<SaveReceipt, SaveError> {
            let maximum = NonZeroU64::MIN.saturating_add(3);
            if slot == u32::MAX
                && save.payload() == b"four"
                && *save.header() == encode_save_header(b"four", maximum)?
                && save.encoded_size() == save_envelope_limit(maximum)
            {
                Ok(SaveReceipt {
                    durability: Err(ErrorKind::Other),
                })
            } else {
                Err(SaveError::UnsafeStorage)
            }
        }
    }

    #[test]
    fn portable_save_dispatch_preserves_admission_and_receipts()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN.saturating_add(3);
        let store = SaveStore::open_with::<Fake>(OsStr::new("saves"), maximum)?;
        let valid = store.load(0)?.is_empty()
            && store.load(1)? == b"four"
            && store.load(2) == Err(SafetyError::TooLarge.into())
            && store.load(3)
                == Err(SaveError::Read(FileReadError::Io(ErrorKind::NotFound)))
            && store.load(4) == Err(SafetyError::InvalidSave.into())
            && store.store(0, b"large") == Err(SafetyError::TooLarge.into())
            && store.store(0, b"four") == Err(SaveError::UnsafeStorage)
            && store.store(u32::MAX, b"four")?.durability
                == Err(ErrorKind::Other)
            && matches!(
                SaveStore::open_with::<Fake>(OsStr::new("unsafe"), maximum),
                Err(SaveError::UnsafeStorage)
            );
        valid
            .then_some(())
            .ok_or(SaveError::Io(ErrorKind::InvalidData))
    }

    #[test]
    fn portable_stream_admission_rejects_size_before_reading()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN;
        let limit = save_envelope_limit(maximum);
        let mut input = b"unchanged".as_slice();
        let result =
            VerifiedSave::read(&mut input, limit.saturating_add(1), maximum);
        if result.err() != Some(FileReadError::TooLarge.into())
            || input != b"unchanged"
        {
            return Err(SaveError::UnsafeStorage);
        }

        let header = encode_save_header(b"", maximum)?;
        let mut stream = header.as_slice();
        let malformed = VerifiedSave::read(&mut stream, limit, maximum);
        (malformed.err() == Some(SafetyError::InvalidSave.into()))
            .then_some(())
            .ok_or(SaveError::UnsafeStorage)
    }

    #[test]
    fn portable_stream_admission_rejects_truncation_and_trailing_bytes()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN;
        let bytes = encode_save(b"x", maximum)?;
        let size = save_envelope_limit(maximum);
        let mut trailing = bytes.clone();
        trailing.push(0);
        let truncated = bytes
            .get(..bytes.len().saturating_sub(1))
            .ok_or(SafetyError::InvalidSave)?;
        let cases = [
            (truncated, FileReadError::Io(ErrorKind::UnexpectedEof)),
            (trailing.as_slice(), FileReadError::TooLarge),
        ];
        for (mut input, error) in cases {
            let observed = VerifiedSave::read(&mut input, size, maximum);
            if observed.err() != Some(error.into()) {
                return Err(SaveError::UnsafeStorage);
            }
        }
        Ok(())
    }
}
