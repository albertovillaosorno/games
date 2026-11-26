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
use std::io::ErrorKind;
use std::num::NonZeroU64;

use crate::ir::security::save::admit_payload;
pub use crate::ir::security::save::{PreparedSave, SaveError, VerifiedSave};

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
    /// Load a slot and return its verified payload.
    ///
    /// # Errors
    /// Rejects unsafe storage, missing or malformed slots and excess sizes.
    pub fn load(&self, slot: u32) -> Result<Vec<u8>, SaveError> {
        let payload = self.backend.read_slot(slot)?.into_payload();
        admit_payload(&payload, self.maximum)?;
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
    use games::safety::{
        SafetyError, encode_save, encode_save_header, save_envelope_limit,
    };

    use super::*;
    use crate::ir::security::file::FileReadError;

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
}
