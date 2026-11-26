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
//   - An explicitly invoked storage-capability smoke scenario.
// - Must-Not:
//   - Open native or browser APIs, use real saves or interpret game data.
// - Allows:
//   - Inputs: Synthetic payloads in a dedicated smoke-only namespace.
//   - Outputs: Success or a storage invariant failure.
//   - Side effects: Replacement of a bounded diagnostic save slot.
// - Split-When:
//   - Another storage scenario needs independent diagnostic setup.
// - Merge-When:
//   - A shared executable covers equivalent storage acceptance.
// - Summary:
//   - Check save-slot publication, replacement and reload.
// - Description:
//   - Exercise the platform's selected persistence implementation.
// - Usage:
//   - Invoke in an isolated test environment, not a user profile.
// - Defaults:
//   - No original assets or gameplay state.

//! Explicit storage smoke for an isolated diagnostic namespace.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::num::NonZeroU64;

use games::safety::SafetyError;
use games_platform::ir::storage::file::FileReadError;
use games_platform::ir::storage::save::{SaveError, SaveStore};

const MAXIMUM: NonZeroU64 = NonZeroU64::MIN.saturating_add(3);
const NAMESPACE: &str = "browser-storage-smoke";
const SLOT: u32 = 0;

fn open(name: &str) -> Result<SaveStore, &'static str> {
    SaveStore::open(OsStr::new(name), MAXIMUM)
        .map_err(|_error| "storage unavailable")
}

fn main() -> Result<(), &'static str> {
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
