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
use std::io::{Error, ErrorKind};
use std::num::NonZeroU64;

use games::safety::{SafetyError, admit_save_size};

use super::file::FileReadError;

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

pub(crate) type OpenSave = Result<Box<dyn SaveBackend>, SaveError>;

/// Private selected backend for opening a dedicated save directory.
pub(crate) trait SaveFactory {
    fn open(directory: &OsStr, maximum: NonZeroU64) -> OpenSave;
}

/// A pinned backend verifies complete envelopes and publishes atomically.
pub(crate) trait SaveBackend: Debug + Send + Sync {
    fn read_slot(&self, slot: u32) -> Result<Vec<u8>, SaveError>;

    fn write_slot(
        &self,
        slot: u32,
        payload: &[u8],
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
        let payload = self.backend.read_slot(slot)?;
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
        self.admit(payload)?;
        self.backend.write_slot(slot, payload)
    }
}

#[cfg(test)]
mod tests {
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
        fn read_slot(&self, slot: u32) -> Result<Vec<u8>, SaveError> {
            match slot {
                0 => Ok(Vec::new()),
                1 => Ok(b"four".to_vec()),
                2 => Ok(b"large".to_vec()),
                _ => {
                    Err(SaveError::Read(FileReadError::Io(ErrorKind::NotFound)))
                },
            }
        }

        fn write_slot(
            &self,
            slot: u32,
            payload: &[u8],
        ) -> Result<SaveReceipt, SaveError> {
            if slot == u32::MAX && payload == b"four" {
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
}
