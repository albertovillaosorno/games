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
//   - Origin-local browser save slots and their bounded text encoding.
// - Must-Not:
//   - Interpret payloads or claim crash-durable browser publication.
// - Allows:
//   - Inputs: Explicit namespace, byte budget and prepared save envelopes.
//   - Outputs: Verified saves, publication receipts and typed failures.
//   - Side effects: Explicit origin-local storage operations only.
// - Split-When:
//   - Another browser persistence backend requires independent admission.
// - Merge-When:
//   - A shared browser backend provides identical persistence guarantees.
// - Summary:
//   - Publish each canonical save envelope as one origin-local value.
// - Description:
//   - Admit a namespace and bound the text before transferring its contents.
// - Usage:
//   - Open through the portable numeric save-slot capability.
// - Defaults:
//   - Reject unavailable storage, malformed values and excessive lengths.

use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::str::from_utf8;

use games::safety::{SafetyError, save_envelope_limit};

use crate::ir::security::file::{admit_size, allocation_length};
use crate::ir::storage::save::{PreparedSave, SaveError, VerifiedSave};

const KEY_PREFIX: &str = "games-save-v1:";
const MAX_NAMESPACE_BYTES: usize = 64;

fn namespace(value: &OsStr) -> Result<&str, SaveError> {
    let text = value.to_str().ok_or(SaveError::UnsafeStorage)?;
    if text.len() > MAX_NAMESPACE_BYTES
        || !text
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(SaveError::UnsafeStorage);
    }
    Ok(text)
}

fn key(namespace: &str, slot: u32) -> String {
    format!("{KEY_PREFIX}{namespace}:{slot}")
}

fn text_limit(maximum: NonZeroU64) -> u64 {
    save_envelope_limit(maximum).saturating_mul(2)
}

fn encode(save: &PreparedSave<'_>) -> Result<String, SaveError> {
    use std::fmt::Write as _;

    let length = allocation_length(save.encoded_size().saturating_mul(2))?;
    let mut text = String::new();
    text.try_reserve_exact(length)
        .map_err(|_error| SafetyError::TooLarge)?;
    for byte in save.header().iter().chain(save.payload()) {
        write!(&mut text, "{byte:02x}")
            .map_err(|_error| SafetyError::InvalidSave)?;
    }
    Ok(text)
}

fn decode(text: &str, maximum: NonZeroU64) -> Result<VerifiedSave, SaveError> {
    let length =
        u64::try_from(text.len()).map_err(|_error| SafetyError::TooLarge)?;
    admit_size(length, text_limit(maximum))?;
    let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(SafetyError::InvalidSave.into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(pairs.len())
        .map_err(|_error| SafetyError::TooLarge)?;
    for pair in pairs {
        if !pair
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(SafetyError::InvalidSave.into());
        }
        let hex = from_utf8(pair).map_err(|_error| SafetyError::InvalidSave)?;
        bytes.push(
            u8::from_str_radix(hex, 16)
                .map_err(|_error| SafetyError::InvalidSave)?,
        );
    }
    VerifiedSave::from_encoded(bytes, maximum)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod browser {
    use std::ffi::OsStr;
    use std::io::ErrorKind;
    use std::num::NonZeroU64;

    use games::safety::SafetyError;
    use js_sys::{JsString, Reflect};
    use wasm_bindgen::{JsCast as _, JsValue};
    use web_sys::Storage;

    use super::{decode, encode, key, namespace, text_limit};
    use crate::ir::security::file::{
        FileReadError, admit_size, allocation_length,
    };
    use crate::ir::storage::save::{
        OpenSave, PreparedSave, SaveBackend, SaveError, SaveFactory,
        SaveReceipt, VerifiedSave,
    };

    #[derive(Debug)]
    struct BrowserSave {
        maximum: NonZeroU64,
        namespace: String,
    }

    fn storage() -> Result<Storage, SaveError> {
        web_sys::window()
            .ok_or(SaveError::Io(ErrorKind::Unsupported))?
            .local_storage()
            .map_err(|_error| SaveError::Io(ErrorKind::PermissionDenied))?
            .ok_or(SaveError::Io(ErrorKind::Unsupported))
    }

    impl SaveFactory for () {
        fn open(directory: &OsStr, maximum: NonZeroU64) -> OpenSave {
            let namespace = namespace(directory)?.to_owned();
            let _available = storage()?;
            Ok(Box::new(BrowserSave { maximum, namespace }))
        }
    }

    impl SaveBackend for BrowserSave {
        fn read_slot(&self, slot: u32) -> Result<VerifiedSave, SaveError> {
            let storage = storage()?;
            // Read the raw Storage named property before converting UTF-16
            // into a Rust string. The fixed key cannot alias a built-in name.
            let key = JsValue::from_str(&key(&self.namespace, slot));
            let raw = Reflect::get(storage.as_ref(), &key)
                .map_err(|_error| SaveError::Io(ErrorKind::PermissionDenied))?;
            if raw.is_undefined() {
                return Err(FileReadError::Io(ErrorKind::NotFound).into());
            }
            let text =
                raw.dyn_ref::<JsString>().ok_or(SafetyError::InvalidSave)?;
            admit_size(u64::from(text.length()), text_limit(self.maximum))?;
            let _capacity =
                allocation_length(u64::from(text.length()).saturating_mul(3))?;
            let owned = raw.as_string().ok_or(SafetyError::InvalidSave)?;
            decode(&owned, self.maximum)
        }

        fn write_slot(
            &self,
            slot: u32,
            save: PreparedSave<'_>,
        ) -> Result<SaveReceipt, SaveError> {
            let text = encode(&save)?;
            storage()?
                .set_item(&key(&self.namespace, slot), &text)
                .map_err(|_error| SaveError::Io(ErrorKind::Other))?;
            Ok(SaveReceipt {
                durability: Err(ErrorKind::Unsupported),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_and_slots_are_distinct() -> Result<(), SaveError> {
        let name = namespace(OsStr::new("simpsons-arcade"))?;
        if key(name, 1) == key(name, 11)
            || key(name, 1) == key("simpsons-arcade1", 1)
        {
            return Err(SaveError::UnsafeStorage);
        }
        for invalid in
            ["", ".", "..", "-name", "a/b", "a:b", "a b", "\u{1f600}"]
        {
            if namespace(OsStr::new(invalid)) != Err(SaveError::UnsafeStorage) {
                return Err(SaveError::UnsafeStorage);
            }
        }
        (namespace(OsStr::new(&"a".repeat(65)))
            == Err(SaveError::UnsafeStorage))
        .then_some(())
        .ok_or(SaveError::UnsafeStorage)
    }

    #[test]
    fn text_round_trips_canonical_envelopes_and_rejects_damage()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN.saturating_add(3);
        for payload in [b"".as_slice(), &[0, 1, 254, 255]] {
            let text = encode(&PreparedSave::new(payload, maximum)?)?;
            if decode(&text, maximum)?.into_payload() != payload
                || decode(&text.to_ascii_uppercase(), maximum).is_ok()
                || decode(&format!("{text}0"), maximum).is_ok()
                || decode(&format!("{text}00"), maximum).is_ok()
            {
                return Err(SaveError::UnsafeStorage);
            }
            let mut corrupt = text;
            corrupt.replace_range(..2, "ff");
            if decode(&corrupt, maximum).is_ok() {
                return Err(SaveError::UnsafeStorage);
            }
        }
        (decode(&"0".repeat(1000), maximum).is_err()
            && decode("\u{e9}\u{e9}", maximum).is_err())
        .then_some(())
        .ok_or(SaveError::UnsafeStorage)
    }
}
