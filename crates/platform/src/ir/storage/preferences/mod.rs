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
//   - Portable persistence of shared display, audio and input preferences.
// - Must-Not:
//   - Select a title's settings location or change its display.
// - Allows:
//   - Inputs: Explicit trusted location and portable settings.
//   - Outputs: Verified preferences or publication receipt.
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
//   - Open a dedicated settings location and load or store preferences.
// - Defaults:
//   - Missing preferences return None; malformed data is rejected.

//! Portable preference persistence.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::num::NonZeroU64;

use games::audio::{AudioSource, MasterVolume, SourceSettings};
use games::presentation::DisplaySettings;

use super::file::FileReadError;
use super::save::{SaveError, SaveReceipt, SaveStore};
use crate::ir::security::preferences::{
    InvalidPreference, control_payload_limit, decode_control,
    decode_source_volume, decode_volume, encode_control, encode_source_volume,
    encode_volume,
};

const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(63);
const DISPLAY_SLOT: u32 = 0;
const CONTROL_SLOT: u32 = 1;
const VOLUME_SLOT: u32 = 2;
const EFFECTS_SLOT: u32 = 3;
const MUSIC_SLOT: u32 = 4;

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

impl From<InvalidPreference> for PreferenceError {
    fn from(_error: InvalidPreference) -> Self {
        Self::InvalidPayload
    }
}

/// Display preferences in a dedicated, caller-selected save location.
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

    /// Open a dedicated location as described by [`SaveStore::open`].
    ///
    /// # Errors
    /// Rejects unavailable or unsafe storage locations.
    pub fn open(location: &OsStr) -> Result<Self, PreferenceError> {
        Ok(Self {
            store: SaveStore::open(location, MAX_BYTES)?,
        })
    }

    /// Atomically replace the saved preferences.
    ///
    /// # Errors
    /// Rejects unsafe storage, malformed existing preferences or failures
    /// before publication. The receipt reports crash durability separately.
    /// The caller must dedicate the location to preferences; this check
    /// is not a compare-and-swap against other writers.
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

/// Independent audio volume preferences in the dedicated settings location.
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
            .map(|bytes| decode_volume(&bytes).map_err(PreferenceError::from))
            .transpose()
    }

    /// Read a source override without replacing its catalogue default.
    /// # Errors
    /// Rejects unsafe storage and incompatible source preferences.
    pub fn load_source(
        &self,
        source: AudioSource,
    ) -> Result<Option<MasterVolume>, PreferenceError> {
        load_optional(&self.store, source_slot(source))?
            .map(|bytes| {
                decode_source_volume(&bytes, source)
                    .map_err(PreferenceError::from)
            })
            .transpose()
    }

    /// Open a dedicated location as described by [`SaveStore::open`].
    /// # Errors
    /// Rejects unavailable or unsafe storage locations.
    pub fn open(location: &OsStr) -> Result<Self, PreferenceError> {
        Ok(Self {
            store: SaveStore::open(location, MAX_BYTES)?,
        })
    }

    /// Overlay saved preferences on the currently admitted source defaults.
    /// # Errors
    /// Rejects invalid saved levels for an admitted source.
    pub fn resolve_sources(
        &self,
        defaults: SourceSettings,
    ) -> Result<SourceSettings, PreferenceError> {
        let resolve = |source, default| {
            self.load_source(source)
                .map(|stored| stored.unwrap_or(default))
        };
        Ok(SourceSettings {
            effects: defaults
                .effects
                .map(|level| resolve(AudioSource::Effects, level))
                .transpose()?,
            music: defaults
                .music
                .map(|level| resolve(AudioSource::Music, level))
                .transpose()?,
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
        let payload = encode_volume(volume);
        Ok(self.store.store(VOLUME_SLOT, payload.as_bytes())?)
    }

    /// Atomically replace only the selected source override.
    /// # Errors
    /// Rejects unsafe storage, incompatible existing data or save failure.
    pub fn store_source(
        &self,
        source: AudioSource,
        volume: MasterVolume,
    ) -> Result<SaveReceipt, PreferenceError> {
        let _existing = self.load_source(source)?;
        let payload = encode_source_volume(source, volume);
        Ok(self.store.store(source_slot(source), payload.as_bytes())?)
    }
}

/// Input profile preferences in a dedicated settings location.
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
                decode_control(&bytes, self.maximum)
                    .map(str::to_owned)
                    .map_err(PreferenceError::from)
            })
            .transpose()
    }

    /// Open a settings location with a caller-selected byte bound.
    ///
    /// # Errors
    /// Rejects unsafe locations and an overflowing payload bound.
    pub fn open(
        location: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Self, PreferenceError> {
        let bound = control_payload_limit(maximum)?;
        Ok(Self {
            maximum,
            store: SaveStore::open(location, bound)?,
        })
    }

    /// Atomically replace the input profile with already validated text.
    ///
    /// # Errors
    /// Rejects empty or oversized input, unsafe storage, an incompatible
    /// existing payload or publication failure.
    pub fn store(&self, profile: &str) -> Result<SaveReceipt, PreferenceError> {
        let payload = encode_control(profile, self.maximum)?;
        let _existing = self.load()?;
        Ok(self.store.store(CONTROL_SLOT, payload.as_bytes())?)
    }
}

/// Required assurance for an already published preference value.
///
/// This never converts a rejected write into a successful publication.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PublicationPolicy {
    /// Accept successful publication when durability is unsupported.
    BestEffort,
    /// Require the backend to confirm crash durability.
    #[default]
    Durable,
}

impl PublicationPolicy {
    /// Verify the publication receipt against the caller's requirements.
    ///
    /// # Errors
    /// Reports a durability failure unless this policy explicitly admits
    /// an unsupported durability guarantee.
    pub const fn confirm(self, receipt: SaveReceipt) -> Result<(), ErrorKind> {
        match (self, receipt.durability) {
            (Self::BestEffort, Err(ErrorKind::Unsupported)) => Ok(()),
            (_, durability) => durability,
        }
    }
}

const fn source_slot(source: AudioSource) -> u32 {
    match source {
        AudioSource::Effects => EFFECTS_SLOT,
        AudioSource::Music => MUSIC_SLOT,
    }
}

fn load_optional(
    store: &SaveStore,
    slot: u32,
) -> Result<SlotPayload, PreferenceError> {
    match store.load(slot) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(SaveError::Read(FileReadError::Io(ErrorKind::NotFound))) => {
            Ok(None)
        },
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_publication_distinguishes_durability() {
        let durable = PublicationPolicy::Durable;
        let best_effort = PublicationPolicy::BestEffort;
        let confirmed = SaveReceipt { durability: Ok(()) };
        let unsupported = SaveReceipt {
            durability: Err(ErrorKind::Unsupported),
        };
        assert_eq!(durable.confirm(confirmed), Ok(()));
        assert_eq!(best_effort.confirm(confirmed), Ok(()));
        assert_eq!(durable.confirm(unsupported), Err(ErrorKind::Unsupported));
        assert_eq!(best_effort.confirm(unsupported), Ok(()));
        for error in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            let receipt = SaveReceipt { durability: Err(error) };
            assert_eq!(durable.confirm(receipt), Err(error));
            assert_eq!(best_effort.confirm(receipt), Err(error));
        }
    }
}
