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
//   - Browser physical keyboard code and profile admission correspondence.
// - Must-Not:
//   - Interpret keyboard layout, text input, or title actions.
// - Allows:
//   - Inputs: DOM physical codes and shared profile names.
//   - Outputs: Shared physical keyboard identities.
//   - Side effects: None.
// - Split-When:
//   - A second independent keyboard event protocol requires translation.
// - Merge-When:
//   - The event source owns all profile admission and translation.
// - Summary:
//   - Admit only bindings that the browser adapter can translate.
// - Description:
//   - Both directions share the same physical key correspondence.
// - Usage:
//   - Resolve profile names or translate DOM KeyboardEvent.code values.
// - Defaults:
//   - Unmapped codes and unsupported profile names are rejected.

use games::input::physical::KeyboardKey;

const DOM_KEYS: &[(&str, &str)] = &[
    ("Numpad0", "Keypad 0"),
    ("Numpad1", "Keypad 1"),
    ("Numpad2", "Keypad 2"),
    ("Numpad3", "Keypad 3"),
    ("Numpad4", "Keypad 4"),
    ("Numpad5", "Keypad 5"),
    ("Numpad6", "Keypad 6"),
    ("Numpad7", "Keypad 7"),
    ("Numpad8", "Keypad 8"),
    ("Numpad9", "Keypad 9"),
    ("NumpadAdd", "Keypad +"),
    ("NumpadSubtract", "Keypad -"),
    ("NumpadMultiply", "Keypad *"),
    ("NumpadDivide", "Keypad /"),
    ("NumpadDecimal", "Keypad ."),
    ("NumpadComma", "Keypad ,"),
    ("NumpadEnter", "Keypad Enter"),
    ("NumpadEqual", "Keypad ="),
    ("ArrowDown", "Down"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("ArrowUp", "Up"),
    ("Enter", "Return"),
    ("Minus", "-"),
    ("Equal", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Backquote", "`"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("ControlLeft", "Left Ctrl"),
    ("ControlRight", "Right Ctrl"),
    ("ShiftLeft", "Left Shift"),
    ("ShiftRight", "Right Shift"),
    ("AltLeft", "Left Alt"),
    ("AltRight", "Right Alt"),
    ("MetaLeft", "Left GUI"),
    ("MetaRight", "Right GUI"),
    ("ContextMenu", "Application"),
    ("NumLock", "Numlock"),
    ("IntlBackslash", "IntlBackslash"),
    ("Escape", "Escape"),
    ("Backspace", "Backspace"),
    ("Tab", "Tab"),
    ("Space", "Space"),
    ("CapsLock", "CapsLock"),
    ("PrintScreen", "PrintScreen"),
    ("ScrollLock", "ScrollLock"),
    ("Pause", "Pause"),
    ("Insert", "Insert"),
    ("Home", "Home"),
    ("PageUp", "PageUp"),
    ("Delete", "Delete"),
    ("End", "End"),
    ("PageDown", "PageDown"),
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("F13", "F13"),
    ("F14", "F14"),
    ("F15", "F15"),
    ("F16", "F16"),
    ("F17", "F17"),
    ("F18", "F18"),
    ("F19", "F19"),
    ("F20", "F20"),
    ("F21", "F21"),
    ("F22", "F22"),
    ("F23", "F23"),
    ("F24", "F24"),
];

const fn letter(name: &str) -> bool {
    matches!(name.as_bytes(), [value] if value.is_ascii_uppercase())
}

const fn digit(name: &str) -> bool {
    matches!(name.as_bytes(), [value] if value.is_ascii_digit())
}

pub(in crate::software) fn profile_key(name: &str) -> Option<KeyboardKey> {
    let key = KeyboardKey::from_profile_name(name)?;
    let canonical = key.profile_name()?;
    (letter(canonical)
        || digit(canonical)
        || DOM_KEYS.iter().any(|(_, label)| *label == canonical))
    .then_some(key)
}

#[cfg(any(test, feature = "interactive", feature = "test-fixtures"))]
pub(super) fn key_code(code: &str) -> Option<KeyboardKey> {
    let name = code
        .strip_prefix("Key")
        .filter(|name| letter(name))
        .or_else(|| code.strip_prefix("Digit").filter(|name| digit(name)))
        .or_else(|| {
            DOM_KEYS
                .iter()
                .find_map(|&(dom, name)| (dom == code).then_some(name))
        })?;
    KeyboardKey::from_profile_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_keys_round_trip_through_browser_profile_admission() {
        for (index, &(dom, name)) in DOM_KEYS.iter().enumerate() {
            let key = KeyboardKey::from_profile_name(name);
            assert!(key.is_some(), "{name}");
            assert_eq!(profile_key(name), key, "{name}");
            assert_eq!(profile_key(&name.to_ascii_lowercase()), key);
            assert_eq!(key_code(dom), key, "{dom}");
            for &(other_dom, other_name) in DOM_KEYS.iter().skip(index + 1) {
                assert_ne!(dom, other_dom);
                assert_ne!(name, other_name);
            }
        }
    }

    #[test]
    fn alphanumeric_keys_round_trip() {
        for (prefix, characters) in [("Key", 'A'..='Z'), ("Digit", '0'..='9')] {
            for character in characters {
                let name = character.to_string();
                let key = profile_key(&name);
                assert!(key.is_some());
                assert_eq!(key_code(&format!("{prefix}{character}")), key);
            }
        }
    }

    #[test]
    fn unsupported_names_and_unidentified_codes_are_rejected() {
        for name in ["Power", "VolumeUp", "#", "Unidentified", ""] {
            assert_eq!(profile_key(name), None, "{name}");
        }
        for code in ["Keya", "Digit10", "NumpadFoo", "Unidentified", ""] {
            assert_eq!(key_code(code), None, "{code}");
        }
        assert_eq!(
            key_code("IntlBackslash"),
            Some(KeyboardKey::from_code(100))
        );
        assert_eq!(key_code("NumpadComma"), Some(KeyboardKey::from_code(133)));
        assert_ne!(key_code("IntlBackslash"), key_code("Backslash"));
    }
}
