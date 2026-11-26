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
//   - An explicitly invoked storage, preference and clock smoke.
// - Must-Not:
//   - Open native or browser APIs, use real saves or interpret game data.
// - Allows:
//   - Inputs: Synthetic payloads in dedicated smoke-only namespaces.
//   - Outputs: Success or a storage, preference or clock invariant failure.
//   - Side effects: Replacement of bounded diagnostic save slots.
// - Split-When:
//   - Another capability scenario needs independent diagnostic setup.
// - Merge-When:
//   - A shared executable covers equivalent capability acceptance.
// - Summary:
//   - Check monotonic observations and save publication and reload.
// - Description:
//   - Exercise the platform's selected clock and persistence implementations.
// - Usage:
//   - Invoke in an isolated test environment, not a user profile.
// - Defaults:
//   - No original assets or gameplay state.

//! Explicit host capability smoke for an isolated diagnostic namespace.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::num::NonZeroU64;

use games::audio::MasterVolume;
use games::presentation::{DisplaySettings, PresentationMode, WindowMode};
use games::safety::SafetyError;
use games_platform::ir::measurement::{elapsed_between, now};
use games_platform::ir::storage::file::FileReadError;
use games_platform::ir::storage::preferences::{
    ControlStore, DisplayStore, PreferenceError, VolumeStore,
};
use games_platform::ir::storage::save::{SaveError, SaveReceipt, SaveStore};

const FIRST: Settings = Settings {
    display: DisplaySettings {
        presentation: PresentationMode::Integer,
        window: WindowMode::Windowed,
    },
    profile: "move-left=A",
    volume: MasterVolume::ThreeQuarters,
};
const INVALID_PREFERENCES: &str = "browser-invalid-preference-smoke";
const MAXIMUM: NonZeroU64 = NonZeroU64::MIN.saturating_add(3);
const NAMESPACE: &str = "browser-storage-smoke";
const PREFERENCES: &str = "browser-preference-smoke";
const PROFILE_MAXIMUM: NonZeroU64 = NonZeroU64::MIN.saturating_add(15);
const SECOND: Settings = Settings {
    display: DisplaySettings {
        presentation: PresentationMode::Fill,
        window: WindowMode::Fullscreen,
    },
    profile: "move-left=\u{e9}",
    volume: MasterVolume::Half,
};
const SLOT: u32 = 0;

struct PreferenceStores {
    control: ControlStore,
    display: DisplayStore,
    volume: VolumeStore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Settings {
    display: DisplaySettings,
    profile: &'static str,
    volume: MasterVolume,
}

impl PreferenceStores {
    fn load(&self) -> Result<Option<Settings>, &'static str> {
        let display = self.display.load().map_err(preference_error)?;
        let volume = self.volume.load().map_err(preference_error)?;
        let profile = self.control.load().map_err(preference_error)?;
        if (display, volume, profile.as_deref()) == (None, None, None) {
            return Ok(None);
        }
        [FIRST, SECOND]
            .into_iter()
            .find(|settings| {
                (display, volume, profile.as_deref())
                    == (
                        Some(settings.display),
                        Some(settings.volume),
                        Some(settings.profile),
                    )
            })
            .map(Some)
            .ok_or("unexpected saved preferences")
    }

    fn open(name: &str) -> Result<Self, &'static str> {
        let location = OsStr::new(name);
        Ok(Self {
            control: ControlStore::open(location, PROFILE_MAXIMUM)
                .map_err(preference_error)?,
            display: DisplayStore::open(location).map_err(preference_error)?,
            volume: VolumeStore::open(location).map_err(preference_error)?,
        })
    }

    fn reject_incompatible(&self) -> Result<(), &'static str> {
        let expected = Err(PreferenceError::InvalidPayload);
        if self.display.load() != expected
            || self.volume.load() != Err(PreferenceError::InvalidPayload)
            || self.control.load() != Err(PreferenceError::InvalidPayload)
        {
            return Err("incompatible preference read accepted");
        }
        for result in [
            self.display.store(FIRST.display),
            self.volume.store(FIRST.volume),
            self.control.store(FIRST.profile),
        ] {
            if result != Err(PreferenceError::InvalidPayload) {
                return Err("incompatible preference replaced");
            }
        }
        Ok(())
    }

    fn store(&self, settings: Settings) -> Result<(), &'static str> {
        check_receipt(self.display.store(settings.display))?;
        check_receipt(self.volume.store(settings.volume))?;
        check_receipt(self.control.store(settings.profile))
    }
}

fn check_receipt(
    receipt: Result<SaveReceipt, PreferenceError>,
) -> Result<(), &'static str> {
    if receipt.map_err(preference_error)?.durability
        != Err(ErrorKind::Unsupported)
    {
        return Err("preference durability mismatch");
    }
    Ok(())
}

fn main() -> Result<(), &'static str> {
    smoke_slots()?;
    smoke_preferences()?;
    smoke_incompatible_preferences()
}

fn open(name: &str) -> Result<SaveStore, &'static str> {
    SaveStore::open(OsStr::new(name), MAXIMUM)
        .map_err(|_error| "storage unavailable")
}

const fn preference_error(_error: PreferenceError) -> &'static str {
    "preference operation failed"
}

fn smoke_incompatible_preferences() -> Result<(), &'static str> {
    let raw = SaveStore::open(
        OsStr::new(INVALID_PREFERENCES),
        NonZeroU64::MIN.saturating_add(63),
    )
    .map_err(|_error| "invalid preference fixture unavailable")?;
    for slot in [0, 1, 2] {
        let _receipt = raw
            .store(slot, b"incompatible-v1")
            .map_err(|_error| "invalid preference fixture failed")?;
    }
    PreferenceStores::open(INVALID_PREFERENCES)?.reject_incompatible()?;
    for slot in [0, 1, 2] {
        if raw.load(slot).as_deref() != Ok(b"incompatible-v1") {
            return Err("invalid preference fixture changed");
        }
    }
    Ok(())
}

fn smoke_preferences() -> Result<(), &'static str> {
    let stores = PreferenceStores::open(PREFERENCES)?;
    let next = match stores.load()? {
        None | Some(SECOND) => FIRST,
        Some(FIRST) => SECOND,
        Some(_) => return Err("unknown preference preset"),
    };
    stores.store(next)?;
    if PreferenceStores::open(PREFERENCES)?.load()? != Some(next) {
        return Err("preference reload mismatch");
    }
    if stores.control.store("") != Err(PreferenceError::InvalidPayload)
        || stores.control.store("0123456789abcdefg")
            != Err(PreferenceError::InvalidPayload)
        || stores.load()? != Some(next)
    {
        return Err("invalid control text altered preferences");
    }
    Ok(())
}

fn smoke_slots() -> Result<(), &'static str> {
    let first = now().map_err(|_error| "clock unavailable or invalid")?;
    let second = now().map_err(|_error| "clock unavailable or invalid")?;
    let _interval =
        elapsed_between(first, second).map_err(|_error| "clock reversed")?;
    let store = open(NAMESPACE)?;
    let separate = open("browser-storage-smoke-other")?;
    let missing = SaveError::Read(FileReadError::Io(ErrorKind::NotFound));
    if separate.load(SLOT) != Err(missing) {
        return Err("namespace isolation failed");
    }
    let replacement: &[u8] = match store.load(SLOT) {
        Err(error) if error == missing => &[0, 1, 254, 255],
        Ok(previous) if previous == [0, 1, 254, 255] => &[255, 0],
        Ok(previous) if previous == [255, 0] => &[0, 1, 254, 255],
        Err(_) | Ok(_) => return Err("invalid existing save"),
    };
    let receipt = store
        .store(SLOT, replacement)
        .map_err(|_error| "publication failed")?;
    if receipt.durability != Err(ErrorKind::Unsupported)
        || store.store(SLOT, b"large")
            != Err(SaveError::Envelope(SafetyError::TooLarge))
    {
        return Err("durability or payload bound mismatch");
    }
    let reopened = open(NAMESPACE)?;
    if reopened.load(SLOT).as_deref() != Ok(replacement)
        || reopened.load(1) != Err(missing)
        || separate.load(SLOT) != Err(missing)
    {
        return Err("published data or slot isolation mismatch");
    }
    Ok(())
}
