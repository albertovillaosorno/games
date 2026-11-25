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
//   - Native persistence of shared display, audio and input preferences.
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
//   - Store preferences separately from title progress and assets.
// - Description:
//   - Use a bounded versioned payload in an existing atomic save envelope.
// - Usage:
//   - Open a dedicated settings directory and load or store preferences.
// - Defaults:
//   - Missing preferences return None; malformed data is rejected.

use std::num::NonZeroU64;
use std::path::Path;
use std::{io, str};

use games::audio::MasterVolume;
use games::presentation::DisplaySettings;

use crate::file::FileReadError;
use crate::save::{SaveError, SaveReceipt, SaveStore};

const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(63);
const DISPLAY_SLOT: u32 = 0;
const CONTROL_SLOT: u32 = 1;
const VOLUME_SLOT: u32 = 2;
const VOLUME_PREFIX: &str = "master-volume-v1\n";
const CONTROL_PREFIX: &str = "input-profile-v1\n";

/// Display preference admission or publication error.
pub type DisplayStoreError = PreferenceError;
type SlotPayload = Option<Vec<u8>>;

/// Preference admission or publication error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferenceError {
    /// The portable preferences payload is invalid.
    InvalidPayload,
    /// The underlying atomic store rejected the operation.
    Storage(SaveError),
}

impl From<SaveError> for PreferenceError {
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
    pub fn load(&self) -> Result<Option<DisplaySettings>, PreferenceError> {
        load_optional(&self.store, DISPLAY_SLOT)?
            .map(|bytes| {
                DisplaySettings::decode(&bytes)
                    .map_err(|_error| PreferenceError::InvalidPayload)
            })
            .transpose()
    }

    /// Pin an existing dedicated settings directory.
    ///
    /// # Errors
    /// Rejects missing, linked, non-directory or writable-by-others paths.
    pub fn open(directory: &Path) -> Result<Self, PreferenceError> {
        Ok(Self {
            store: SaveStore::open(directory, MAX_BYTES)?,
        })
    }

    /// Atomically replace the saved preferences.
    ///
    /// # Errors
    /// Rejects unsafe storage, malformed existing preferences or failures
    /// before publication. The receipt reports directory sync status.
    /// The caller must dedicate the directory to preferences; this check
    /// is not a compare-and-swap against other same-user writers.
    pub fn store(
        &self,
        settings: DisplaySettings,
    ) -> Result<SaveReceipt, PreferenceError> {
        let _existing = self.load()?;
        Ok(self
            .store
            .store(DISPLAY_SLOT, settings.encode().as_bytes())?)
    }
}

/// Master audio volume in the same dedicated preferences directory.
#[derive(Debug)]
pub struct VolumeStore {
    store: SaveStore,
}

impl VolumeStore {
    /// Load saved volume, or None when no preference has been saved.
    /// # Errors
    /// Rejects unsafe storage and malformed or incompatible preferences.
    pub fn load(&self) -> Result<Option<MasterVolume>, PreferenceError> {
        load_optional(&self.store, VOLUME_SLOT)?
            .map(|bytes| {
                let text = str::from_utf8(&bytes)
                    .map_err(|_error| PreferenceError::InvalidPayload)?;
                let value = text
                    .strip_prefix(VOLUME_PREFIX)
                    .ok_or(PreferenceError::InvalidPayload)?;
                MasterVolume::parse(value)
                    .map_err(|_error| PreferenceError::InvalidPayload)
            })
            .transpose()
    }

    /// Pin an existing dedicated settings directory.
    /// # Errors
    /// Rejects missing, linked, non-directory or writable-by-others paths.
    pub fn open(directory: &Path) -> Result<Self, PreferenceError> {
        Ok(Self {
            store: SaveStore::open(directory, MAX_BYTES)?,
        })
    }

    /// Atomically replace saved volume.
    /// # Errors
    /// Rejects unsafe storage, incompatible existing data or save failure.
    pub fn store(
        &self,
        volume: MasterVolume,
    ) -> Result<SaveReceipt, PreferenceError> {
        let _existing = self.load()?;
        let payload = format!("{VOLUME_PREFIX}{}", volume.percent());
        Ok(self.store.store(VOLUME_SLOT, payload.as_bytes())?)
    }
}

/// Native input profile preferences in a dedicated settings directory.
///
/// The caller validates the profile's title-specific bindings before use.
#[derive(Debug)]
pub struct ControlStore {
    maximum: NonZeroU64,
    store: SaveStore,
}

impl ControlStore {
    /// Read a saved UTF-8 profile, or None when no profile has been saved.
    ///
    /// # Errors
    /// Rejects unsafe storage, other payload types and malformed UTF-8.
    pub fn load(&self) -> Result<Option<String>, PreferenceError> {
        load_optional(&self.store, CONTROL_SLOT)?
            .map(|bytes| {
                let text = str::from_utf8(&bytes)
                    .map_err(|_error| PreferenceError::InvalidPayload)?;
                text.strip_prefix(CONTROL_PREFIX)
                    .filter(|profile| !profile.is_empty())
                    .map(str::to_owned)
                    .ok_or(PreferenceError::InvalidPayload)
            })
            .transpose()
    }

    /// Pin an existing settings directory with a caller-selected byte bound.
    ///
    /// # Errors
    /// Rejects unsafe directories and an overflowing payload bound.
    pub fn open(
        directory: &Path,
        maximum: NonZeroU64,
    ) -> Result<Self, PreferenceError> {
        let prefix = u64::try_from(CONTROL_PREFIX.len())
            .map_err(|_error| PreferenceError::InvalidPayload)?;
        let bound = maximum
            .get()
            .checked_add(prefix)
            .and_then(NonZeroU64::new)
            .ok_or(PreferenceError::InvalidPayload)?;
        Ok(Self {
            maximum,
            store: SaveStore::open(directory, bound)?,
        })
    }

    /// Atomically replace the input profile with already validated text.
    ///
    /// # Errors
    /// Rejects empty or oversized input, unsafe storage, an incompatible
    /// existing payload or publication failure.
    pub fn store(&self, profile: &str) -> Result<SaveReceipt, PreferenceError> {
        if profile.is_empty()
            || u64::try_from(profile.len())
                .map_or(true, |size| size > self.maximum.get())
        {
            return Err(PreferenceError::InvalidPayload);
        }
        let _existing = self.load()?;
        let payload = format!("{CONTROL_PREFIX}{profile}");
        Ok(self.store.store(CONTROL_SLOT, payload.as_bytes())?)
    }
}

fn load_optional(
    store: &SaveStore,
    slot: u32,
) -> Result<SlotPayload, PreferenceError> {
    match store.load(slot) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(SaveError::Read(FileReadError::Io(io::ErrorKind::NotFound))) => {
            Ok(None)
        },
        Err(error) => Err(error.into()),
    }
}
#[cfg(test)]
mod tests {
    use std::fs;

    use games::presentation::{PresentationMode, WindowMode};

    use super::*;
    use crate::fixtures::TestRoot;
    fn require(condition: bool) -> Result<(), PreferenceError> {
        condition
            .then_some(())
            .ok_or(PreferenceError::InvalidPayload)
    }

    #[test]
    fn volume_preferences_are_independent_and_validated()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let display = DisplayStore::open(&root.path)?;
        let controls = ControlStore::open(&root.path, MAX_BYTES)?;
        let volume = VolumeStore::open(&root.path)?;
        require(volume.load()?.is_none())?;
        let settings = DisplaySettings::default();
        require(display.store(settings)?.durability.is_ok())?;
        let profile = "version=1\n";
        require(controls.store(profile)?.durability.is_ok())?;
        for level in MasterVolume::ALL {
            require(volume.store(level)?.durability.is_ok())?;
            require(VolumeStore::open(&root.path)?.load()? == Some(level))?;
        }
        require(display.load()? == Some(settings))?;
        require(controls.load()?.as_deref() == Some(profile))?;
        let raw = SaveStore::open(&root.path, MAX_BYTES)?;
        for payload in [
            b"title-progress".as_slice(),
            b"master-volume-v1\n101",
            b"master-volume-v1\n50\n",
        ] {
            let _receipt = raw.store(VOLUME_SLOT, payload)?;
            require(volume.load() == Err(PreferenceError::InvalidPayload))?;
            require(volume.store(MasterVolume::Full).is_err())?;
            require(raw.load(VOLUME_SLOT)? == payload)?;
        }
        Ok(())
    }

    #[test]
    fn control_preferences_are_independent_and_bounded()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let display = DisplayStore::open(&root.path)?;
        let maximum = NonZeroU64::MIN.saturating_add(31);
        let controls = ControlStore::open(&root.path, maximum)?;
        let settings = DisplaySettings {
            presentation: PresentationMode::Stretch,
            window: WindowMode::Fullscreen,
        };
        require(controls.load()?.is_none())?;
        require(display.store(settings)?.durability.is_ok())?;
        let first = "version=1\nkey 1 right=Q\n";
        require(controls.store(first)?.durability.is_ok())?;
        drop(controls);
        let reopened = ControlStore::open(&root.path, maximum)?;
        require(reopened.load()?.as_deref() == Some(first))?;
        require(reopened.store("").is_err())?;
        require(reopened.store(&"a".repeat(33)).is_err())?;
        require(reopened.load()?.as_deref() == Some(first))?;
        require(reopened.store("version=1\n")?.durability.is_ok())?;
        require(reopened.load()?.as_deref() == Some("version=1\n"))?;
        require(display.load()? == Some(settings))
    }

    #[test]
    fn control_preferences_preserve_other_payloads()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let raw = SaveStore::open(&root.path, MAX_BYTES)?;
        let controls = ControlStore::open(&root.path, MAX_BYTES)?;
        for payload in [
            b"title-progress".as_slice(),
            b"input-profile-v1\n",
            b"input-profile-v1\n\xff",
        ] {
            let _receipt = raw.store(CONTROL_SLOT, payload)?;
            require(controls.load() == Err(PreferenceError::InvalidPayload))?;
            require(controls.store("version=1\n").is_err())?;
            require(raw.load(CONTROL_SLOT)? == payload)?;
        }
        Ok(())
    }

    #[test]
    fn absent_preferences_and_replacements_survive_reopen()
    -> Result<(), PreferenceError> {
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
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let raw = SaveStore::open(&root.path, MAX_BYTES)?;
        let _receipt = raw.store(DISPLAY_SLOT, b"title-progress")?;
        require(
            DisplayStore::open(&root.path)?.load()
                == Err(PreferenceError::InvalidPayload),
        )?;
        require(
            DisplayStore::open(&root.path)?.store(DisplaySettings::default())
                == Err(PreferenceError::InvalidPayload),
        )?;
        require(raw.load(DISPLAY_SLOT)? == b"title-progress")?;
        Ok(())
    }

    #[test]
    fn refuses_corrupt_envelopes_without_repairing_them()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let path = root.path.join("slot-0.gms");
        let corrupted = b"invalid envelope";
        fs::write(&path, corrupted).map_err(SaveError::from)?;
        let store = DisplayStore::open(&root.path)?;
        require(matches!(
            store.store(DisplaySettings::default()),
            Err(PreferenceError::Storage(SaveError::Envelope(_)))
        ))?;
        require(fs::read(path).map_err(SaveError::from)? == corrupted)
    }
}
