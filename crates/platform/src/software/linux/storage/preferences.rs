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
//   - Native integration tests for shared display, audio, input and locale
//     settings.
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

use games::audio::{AudioSource, MasterVolume};
use games::presentation::{DisplaySettings, LocaleTag};

use crate::ir::storage::preferences::{
    ControlStore, DisplayStore, LocaleStore, PreferenceError, VolumeStore,
};
use crate::ir::storage::save::{SaveError, SaveStore};

const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(63);
const DISPLAY_SLOT: u32 = 0;
const CONTROL_SLOT: u32 = 1;
const VOLUME_SLOT: u32 = 2;
const EFFECTS_SLOT: u32 = 3;
const LOCALE_SLOT: u32 = 5;

#[cfg(test)]
mod tests {
    use std::fs;

    use games::presentation::{PresentationMode, WindowMode};

    use super::super::fixtures::TestRoot;
    use super::*;
    #[test]
    fn locale_preferences_are_canonical_and_independent()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let location = root.path.as_os_str();
        let locale = LocaleStore::open(location)?;
        let display = DisplayStore::open(location)?;
        let volume = VolumeStore::open(location)?;
        require(locale.load()?.is_none())?;
        let settings = DisplaySettings::default();
        require(display.store(settings)?.durability.is_ok())?;
        require(volume.store(MasterVolume::Half)?.durability.is_ok())?;
        let selected = LocaleTag::parse("ES-mx")
            .map_err(|_error| PreferenceError::InvalidPayload)?;
        require(locale.store(&selected)?.durability.is_ok())?;
        let reopened = LocaleStore::open(location)?;
        require(reopened.load()? == Some(selected.clone()))?;
        require(display.load()? == Some(settings))?;
        require(volume.load()? == Some(MasterVolume::Half))?;
        let raw = SaveStore::open(location, MAX_BYTES)?;
        require(raw.load(LOCALE_SLOT)? == b"locale-v1\nes-MX")?;
        let wrong = b"master-volume-v1\n50";
        let _receipt = raw.store(LOCALE_SLOT, wrong)?;
        require(reopened.load() == Err(PreferenceError::InvalidPayload))?;
        require(reopened.store(&selected).is_err())?;
        require(raw.load(LOCALE_SLOT)? == wrong)?;
        require(volume.load()? == Some(MasterVolume::Half))
    }

    fn require(condition: bool) -> Result<(), PreferenceError> {
        condition
            .then_some(())
            .ok_or(PreferenceError::InvalidPayload)
    }

    #[test]
    fn volume_preferences_are_independent_and_validated()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let display = DisplayStore::open(root.path.as_os_str())?;
        let controls = ControlStore::open(root.path.as_os_str(), MAX_BYTES)?;
        let volume = VolumeStore::open(root.path.as_os_str())?;
        require(volume.load()?.is_none())?;
        let settings = DisplaySettings::default();
        require(display.store(settings)?.durability.is_ok())?;
        let profile = "version=1\n";
        require(controls.store(profile)?.durability.is_ok())?;
        for level in MasterVolume::ALL {
            require(volume.store(level)?.durability.is_ok())?;
            require(
                VolumeStore::open(root.path.as_os_str())?.load()?
                    == Some(level),
            )?;
        }
        require(display.load()? == Some(settings))?;
        require(controls.load()?.as_deref() == Some(profile))?;
        let raw = SaveStore::open(root.path.as_os_str(), MAX_BYTES)?;
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
    fn source_overrides_are_independent_of_master_and_each_other()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let volume = VolumeStore::open(root.path.as_os_str())?;
        let effects = AudioSource::Effects;
        let music = AudioSource::Music;
        require(volume.load_source(effects)?.is_none())?;
        require(volume.load_source(music)?.is_none())?;
        require(volume.store(MasterVolume::Half)?.durability.is_ok())?;
        require(
            volume
                .store_source(effects, MasterVolume::Quarter)?
                .durability
                .is_ok(),
        )?;
        require(volume.load_source(music)?.is_none())?;
        require(
            volume
                .store_source(music, MasterVolume::Muted)?
                .durability
                .is_ok(),
        )?;
        let reopened = VolumeStore::open(root.path.as_os_str())?;
        require(reopened.load()? == Some(MasterVolume::Half))?;
        require(reopened.load_source(effects)? == Some(MasterVolume::Quarter))?;
        require(reopened.load_source(music)? == Some(MasterVolume::Muted))?;
        let raw = SaveStore::open(root.path.as_os_str(), MAX_BYTES)?;
        let wrong = b"music-volume-v1\n50";
        let _receipt = raw.store(EFFECTS_SLOT, wrong)?;
        require(
            reopened.load_source(effects)
                == Err(PreferenceError::InvalidPayload),
        )?;
        require(reopened.store_source(effects, MasterVolume::Full).is_err())?;
        require(raw.load(EFFECTS_SLOT)? == wrong)?;
        require(reopened.load_source(music)? == Some(MasterVolume::Muted))
    }

    #[test]
    fn source_resolution_preserves_defaults_and_capability_absence()
    -> Result<(), PreferenceError> {
        use games::audio::SourceSettings;
        let root = TestRoot::new().map_err(SaveError::from)?;
        let store = VolumeStore::open(root.path.as_os_str())?;
        let defaults = SourceSettings {
            effects: Some(MasterVolume::Half),
            music: Some(MasterVolume::Quarter),
        };
        require(store.resolve_sources(defaults)? == defaults)?;
        let _receipt =
            store.store_source(AudioSource::Music, MasterVolume::Muted)?;
        let effects_only = SourceSettings { music: None, ..defaults };
        require(store.resolve_sources(effects_only)? == effects_only)?;
        require(
            store.resolve_sources(defaults)?
                == SourceSettings {
                    music: Some(MasterVolume::Muted),
                    ..defaults
                },
        )
    }

    #[test]
    fn control_preferences_are_independent_and_bounded()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let display = DisplayStore::open(root.path.as_os_str())?;
        let maximum = NonZeroU64::MIN.saturating_add(31);
        let controls = ControlStore::open(root.path.as_os_str(), maximum)?;
        let settings = DisplaySettings {
            presentation: PresentationMode::Stretch,
            window: WindowMode::Fullscreen,
        };
        require(controls.load()?.is_none())?;
        require(display.store(settings)?.durability.is_ok())?;
        let first = "version=1\nkey 1 right=Q\n";
        require(controls.store(first)?.durability.is_ok())?;
        drop(controls);
        let reopened = ControlStore::open(root.path.as_os_str(), maximum)?;
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
        let raw = SaveStore::open(root.path.as_os_str(), MAX_BYTES)?;
        let controls = ControlStore::open(root.path.as_os_str(), MAX_BYTES)?;
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
        let store = DisplayStore::open(root.path.as_os_str())?;
        require(store.load()?.is_none())?;
        require(store.store(first)?.durability.is_ok())?;
        drop(store);
        let reopened = DisplayStore::open(root.path.as_os_str())?;
        require(reopened.load()? == Some(first))?;
        let second = DisplaySettings::default();
        require(reopened.store(second)?.durability.is_ok())?;
        require(
            DisplayStore::open(root.path.as_os_str())?.load()? == Some(second),
        )?;
        Ok(())
    }

    #[test]
    fn refuses_other_save_payloads_without_overwriting_them()
    -> Result<(), PreferenceError> {
        let root = TestRoot::new().map_err(SaveError::from)?;
        let raw = SaveStore::open(root.path.as_os_str(), MAX_BYTES)?;
        let _receipt = raw.store(DISPLAY_SLOT, b"title-progress")?;
        require(
            DisplayStore::open(root.path.as_os_str())?.load()
                == Err(PreferenceError::InvalidPayload),
        )?;
        require(
            DisplayStore::open(root.path.as_os_str())?
                .store(DisplaySettings::default())
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
        let store = DisplayStore::open(root.path.as_os_str())?;
        require(matches!(
            store.store(DisplaySettings::default()),
            Err(PreferenceError::Storage(SaveError::Envelope(_)))
        ))?;
        require(fs::read(path).map_err(SaveError::from)? == corrupted)
    }
}
