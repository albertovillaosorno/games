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
//   - Native persistence of shared display preferences.
// - Must-Not:
//   - Select a title's settings directory or change its display.
// - Allows:
//   - Inputs: Dedicated trusted directory and portable settings.
//   - Outputs: Verified preferences or durable publication receipt.
//   - Side effects: Atomic local save publication through SaveStore.
// - Split-When:
//   - A platform requires a different persistence mechanism.
// - Merge-When:
//   - Other shared preferences use the same persistence contract.
// - Summary:
//   - Store display preferences separately from title progress and assets.
// - Description:
//   - Use a bounded versioned payload in an existing atomic save envelope.
// - Usage:
//   - Open a dedicated settings directory and load or store preferences.
// - Defaults:
//   - Missing preferences return None; malformed data is rejected.

use std::io;
use std::num::NonZeroU64;
use std::path::Path;

use games::presentation::DisplaySettings;

use crate::file::FileReadError;
use crate::save::{SaveError, SaveReceipt, SaveStore};

const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(63);
const SLOT: u32 = 0;

/// Display preference admission or publication error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayStoreError {
    /// The portable preferences payload is invalid.
    InvalidPayload,
    /// The underlying atomic store rejected the operation.
    Storage(SaveError),
}

impl From<SaveError> for DisplayStoreError {
    fn from(error: SaveError) -> Self {
        Self::Storage(error)
    }
}

/// Native display preferences in a dedicated, caller-selected directory.
#[derive(Debug)]
pub struct DisplayStore {
    store: SaveStore,
}

impl DisplayStore {
    /// Read saved display preferences, or None on first use.
    ///
    /// # Errors
    /// Rejects unsafe storage and malformed or incompatible preferences.
    pub fn load(&self) -> Result<Option<DisplaySettings>, DisplayStoreError> {
        match self.store.load(SLOT) {
            Ok(bytes) => DisplaySettings::decode(&bytes)
                .map(Some)
                .map_err(|_error| DisplayStoreError::InvalidPayload),
            Err(SaveError::Read(FileReadError::Io(
                io::ErrorKind::NotFound,
            ))) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Pin an existing dedicated settings directory.
    ///
    /// # Errors
    /// Rejects missing, linked, non-directory or writable-by-others paths.
    pub fn open(directory: &Path) -> Result<Self, DisplayStoreError> {
        Ok(Self {
            store: SaveStore::open(directory, MAX_BYTES)?,
        })
    }

    /// Atomically replace the saved preferences.
    ///
    /// # Errors
    /// Reports unsafe storage or failures before publication. The receipt
    /// reports whether the published file's directory sync succeeded.
    pub fn store(
        &self,
        settings: DisplaySettings,
    ) -> Result<SaveReceipt, DisplayStoreError> {
        Ok(self.store.store(SLOT, settings.encode().as_bytes())?)
    }
}

#[cfg(test)]
mod tests {
    use games::presentation::{PresentationMode, WindowMode};

    use super::*;
    use crate::bundle::TestRoot;
    fn require(condition: bool) -> Result<(), DisplayStoreError> {
        condition
            .then_some(())
            .ok_or(DisplayStoreError::InvalidPayload)
    }

    #[test]
    fn absent_preferences_and_replacements_survive_reopen()
    -> Result<(), DisplayStoreError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let first = DisplaySettings {
            presentation: PresentationMode::Stretch,
            window: WindowMode::Fullscreen,
        };
        let store = DisplayStore::open(&root.path)?;
        require(store.load()?.is_none())?;
        require(store.store(first)?.durability.is_ok())?;
        drop(store);
        let reopened = DisplayStore::open(&root.path)?;
        require(reopened.load()? == Some(first))?;
        let second = DisplaySettings::default();
        require(reopened.store(second)?.durability.is_ok())?;
        require(DisplayStore::open(&root.path)?.load()? == Some(second))?;
        Ok(())
    }

    #[test]
    fn refuses_other_save_payloads_without_overwriting_them()
    -> Result<(), DisplayStoreError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let raw = SaveStore::open(&root.path, MAX_BYTES)?;
        let _receipt = raw.store(SLOT, b"title-progress")?;
        require(
            DisplayStore::open(&root.path)?.load()
                == Err(DisplayStoreError::InvalidPayload),
        )?;
        require(raw.load(SLOT)? == b"title-progress")?;
        Ok(())
    }
}
