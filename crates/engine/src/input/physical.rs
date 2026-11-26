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
//   - Stable physical keyboard, gamepad and pointer identities.
// - Must-Not:
//   - Depend on native device libraries or prescribe title actions.
// - Allows:
//   - Inputs: Physical control identities and version-one profile names.
//   - Outputs: Stable routing codes and validated identities.
//   - Side effects: None.
// - Split-When:
//   - Another control family needs independent translation semantics.
// - Merge-When:
//   - A shared physical input contract covers identical identities.
// - Summary:
//   - Platform-independent control identities for standard input devices.
// - Description:
//   - Preserves existing profile names while separating native event enums.
// - Usage:
//   - Translate native events at the adapter and bind these codes in titles.
// - Defaults:
//   - Unrecognized profile names are rejected.

// Version-one profile labels for common keyboard-page usage ranges.
// Keep other named legacy keys with the platform fallback.
const KEYBOARD_NAMES: &[KeyNameRange] = &[
    (4, &[
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N",
        "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    ]),
    (30, &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]),
    (40, &[
        "Return",
        "Escape",
        "Backspace",
        "Tab",
        "Space",
        "-",
        "=",
        "[",
        "]",
        "\\",
        "#",
        ";",
        "'",
        "`",
        ",",
        ".",
        "/",
        "CapsLock",
    ]),
    (58, &[
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11",
        "F12",
    ]),
    (70, &[
        "PrintScreen",
        "ScrollLock",
        "Pause",
        "Insert",
        "Home",
        "PageUp",
        "Delete",
        "End",
        "PageDown",
        "Right",
        "Left",
        "Down",
        "Up",
        "Numlock",
        "Keypad /",
        "Keypad *",
        "Keypad -",
        "Keypad +",
        "Keypad Enter",
        "Keypad 1",
        "Keypad 2",
        "Keypad 3",
        "Keypad 4",
        "Keypad 5",
        "Keypad 6",
        "Keypad 7",
        "Keypad 8",
        "Keypad 9",
        "Keypad 0",
        "Keypad .",
    ]),
    (100, &["IntlBackslash"]),
    (101, &[
        "Application",
        "Power",
        "Keypad =",
        "F13",
        "F14",
        "F15",
        "F16",
        "F17",
        "F18",
        "F19",
        "F20",
        "F21",
        "F22",
        "F23",
        "F24",
        "Execute",
        "Help",
        "Menu",
        "Select",
        "Stop",
        "Again",
        "Undo",
        "Cut",
        "Copy",
        "Paste",
        "Find",
        "Mute",
        "VolumeUp",
        "VolumeDown",
    ]),
    (133, &["Keypad ,"]),
    (224, &[
        "Left Ctrl",
        "Left Shift",
        "Left Alt",
        "Left GUI",
        "Right Ctrl",
        "Right Shift",
        "Right Alt",
        "Right GUI",
    ]),
];

type KeyNameRange = (u32, &'static [&'static str]);

/// Standard gamepad axes, independent of device-specific enumerations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerAxis {
    /// Horizontal left stick.
    LeftX,
    /// Vertical left stick.
    LeftY,
    /// Horizontal right stick.
    RightX,
    /// Vertical right stick.
    RightY,
    /// Left analog trigger.
    TriggerLeft,
    /// Right analog trigger.
    TriggerRight,
}

impl ControllerAxis {
    /// Stable profile ordering.
    pub const ALL: [Self; 6] = [
        Self::LeftX,
        Self::LeftY,
        Self::RightX,
        Self::RightY,
        Self::TriggerLeft,
        Self::TriggerRight,
    ];

    /// Stable routing code.
    #[must_use]
    pub const fn code(self) -> u16 {
        self.descriptor().0
    }

    const fn descriptor(self) -> (u16, &'static str) {
        match self {
            Self::LeftX => (0, "leftx"),
            Self::LeftY => (1, "lefty"),
            Self::RightX => (2, "rightx"),
            Self::RightY => (3, "righty"),
            Self::TriggerLeft => (4, "lefttrigger"),
            Self::TriggerRight => (5, "righttrigger"),
        }
    }

    /// Resolve a version-one profile name, including legacy sign prefixes.
    #[must_use]
    pub fn from_profile_name(name: &str) -> Option<Self> {
        let unprefixed = name.strip_prefix(['+', '-']).unwrap_or(name);
        Self::ALL
            .into_iter()
            .find(|axis| axis.profile_name().eq_ignore_ascii_case(unprefixed))
    }

    /// Stable version-one profile name.
    #[must_use]
    pub const fn profile_name(self) -> &'static str {
        self.descriptor().1
    }
}

/// Standard gamepad buttons. Face buttons are named by physical position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerButton {
    /// Back or select button.
    Back,
    /// Directional pad down.
    DPadDown,
    /// Directional pad left.
    DPadLeft,
    /// Directional pad right.
    DPadRight,
    /// Directional pad up.
    DPadUp,
    /// Right face button.
    East,
    /// Guide or home button.
    Guide,
    /// Left shoulder button.
    LeftShoulder,
    /// Left stick press.
    LeftStick,
    /// Additional standard auxiliary button.
    Misc1,
    /// Top face button.
    North,
    /// First paddle.
    Paddle1,
    /// Second paddle.
    Paddle2,
    /// Third paddle.
    Paddle3,
    /// Fourth paddle.
    Paddle4,
    /// Right shoulder button.
    RightShoulder,
    /// Right stick press.
    RightStick,
    /// Bottom face button.
    South,
    /// Start button.
    Start,
    /// Touchpad press.
    Touchpad,
    /// Left face button.
    West,
}

impl ControllerButton {
    /// All admitted standard buttons in stable routing order.
    pub const ALL: [Self; 21] = [
        Self::South,
        Self::East,
        Self::West,
        Self::North,
        Self::Back,
        Self::Guide,
        Self::Start,
        Self::LeftStick,
        Self::RightStick,
        Self::LeftShoulder,
        Self::RightShoulder,
        Self::DPadUp,
        Self::DPadDown,
        Self::DPadLeft,
        Self::DPadRight,
        Self::Misc1,
        Self::Paddle1,
        Self::Paddle2,
        Self::Paddle3,
        Self::Paddle4,
        Self::Touchpad,
    ];

    /// Stable routing code.
    #[must_use]
    pub const fn code(self) -> u16 {
        self.descriptor().0
    }

    const fn descriptor(self) -> (u16, &'static str) {
        match self {
            Self::South => (0, "a"),
            Self::East => (1, "b"),
            Self::West => (2, "x"),
            Self::North => (3, "y"),
            Self::Back => (4, "back"),
            Self::Guide => (5, "guide"),
            Self::Start => (6, "start"),
            Self::LeftStick => (7, "leftstick"),
            Self::RightStick => (8, "rightstick"),
            Self::LeftShoulder => (9, "leftshoulder"),
            Self::RightShoulder => (10, "rightshoulder"),
            Self::DPadUp => (11, "dpup"),
            Self::DPadDown => (12, "dpdown"),
            Self::DPadLeft => (13, "dpleft"),
            Self::DPadRight => (14, "dpright"),
            Self::Misc1 => (15, "misc1"),
            Self::Paddle1 => (16, "paddle1"),
            Self::Paddle2 => (17, "paddle2"),
            Self::Paddle3 => (18, "paddle3"),
            Self::Paddle4 => (19, "paddle4"),
            Self::Touchpad => (20, "touchpad"),
        }
    }

    /// Resolve a version-one profile name, case insensitively.
    #[must_use]
    pub fn from_profile_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|button| button.profile_name().eq_ignore_ascii_case(name))
    }

    /// Stable version-one profile name, including legacy face-button labels.
    #[must_use]
    pub const fn profile_name(self) -> &'static str {
        self.descriptor().1
    }
}

/// A physical keyboard routing identity, not a character or layout symbol.
///
/// Standard keys use keyboard-page USB HID usage identifiers. Existing
/// adapter-specific extended codes are reserved for legacy profile migration;
/// another adapter must explicitly translate its device codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardKey(u32);

impl KeyboardKey {
    /// Physical DELETE key.
    pub const DELETE: Self = Self(76);
    /// Physical DOWN key.
    pub const DOWN: Self = Self(81);
    /// Physical ENTER key.
    pub const ENTER: Self = Self(40);
    /// Physical ESCAPE key.
    pub const ESCAPE: Self = Self(41);
    /// Physical F1 key.
    pub const F1: Self = Self(58);
    /// Physical F2 key.
    pub const F2: Self = Self(59);
    /// Physical 1 key on the numeric keypad.
    pub const KP_1: Self = Self(89);
    /// Physical 2 key on the numeric keypad.
    pub const KP_2: Self = Self(90);
    /// Physical 4 key on the numeric keypad.
    pub const KP_4: Self = Self(92);
    /// Physical 5 key on the numeric keypad.
    pub const KP_5: Self = Self(93);
    /// Physical 6 key on the numeric keypad.
    pub const KP_6: Self = Self(94);
    /// Physical 8 key on the numeric keypad.
    pub const KP_8: Self = Self(96);
    /// Physical LEFT key.
    pub const LEFT: Self = Self(80);
    /// Physical A key.
    pub const LETTER_A: Self = Self(4);
    /// Physical B key.
    pub const LETTER_B: Self = Self(5);
    /// Physical C key.
    pub const LETTER_C: Self = Self(6);
    /// Physical D key.
    pub const LETTER_D: Self = Self(7);
    /// Physical E key.
    pub const LETTER_E: Self = Self(8);
    /// Physical F key.
    pub const LETTER_F: Self = Self(9);
    /// Physical G key.
    pub const LETTER_G: Self = Self(10);
    /// Physical H key.
    pub const LETTER_H: Self = Self(11);
    /// Physical I key.
    pub const LETTER_I: Self = Self(12);
    /// Physical J key.
    pub const LETTER_J: Self = Self(13);
    /// Physical K key.
    pub const LETTER_K: Self = Self(14);
    /// Physical L key.
    pub const LETTER_L: Self = Self(15);
    /// Physical M key.
    pub const LETTER_M: Self = Self(16);
    /// Physical N key.
    pub const LETTER_N: Self = Self(17);
    /// Physical O key.
    pub const LETTER_O: Self = Self(18);
    /// Physical P key.
    pub const LETTER_P: Self = Self(19);
    /// Physical Q key.
    pub const LETTER_Q: Self = Self(20);
    /// Physical R key.
    pub const LETTER_R: Self = Self(21);
    /// Physical S key.
    pub const LETTER_S: Self = Self(22);
    /// Physical T key.
    pub const LETTER_T: Self = Self(23);
    /// Physical U key.
    pub const LETTER_U: Self = Self(24);
    /// Physical V key.
    pub const LETTER_V: Self = Self(25);
    /// Physical W key.
    pub const LETTER_W: Self = Self(26);
    /// Physical X key.
    pub const LETTER_X: Self = Self(27);
    /// Physical Y key.
    pub const LETTER_Y: Self = Self(28);
    /// Physical Z key.
    pub const LETTER_Z: Self = Self(29);
    /// Physical 6 key on the main number row.
    pub const NUM_6: Self = Self(35);
    /// Physical 7 key on the main number row.
    pub const NUM_7: Self = Self(36);
    /// Physical 8 key on the main number row.
    pub const NUM_8: Self = Self(37);
    /// Physical RIGHT key.
    pub const RIGHT: Self = Self(79);
    /// Physical SPACE key.
    pub const SPACE: Self = Self(44);
    /// Physical TAB key.
    pub const TAB: Self = Self(43);
    /// Physical UP key.
    pub const UP: Self = Self(82);

    /// Return the stable routing identity.
    #[must_use]
    pub const fn code(self) -> u32 {
        self.0
    }

    /// Admit an explicitly translated keyboard routing identity.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        Self(code)
    }

    /// Resolve a common physical key using its version-one profile name.
    ///
    /// Case is ignored, but whitespace and punctuation remain significant.
    /// Other legacy names may require the originating platform adapter.
    #[must_use]
    pub fn from_profile_name(name: &str) -> Option<Self> {
        KEYBOARD_NAMES.iter().find_map(|(first, names)| {
            let offset = names
                .iter()
                .position(|label| label.eq_ignore_ascii_case(name))?;
            first.checked_add(u32::try_from(offset).ok()?).map(Self)
        })
    }

    /// Stable version-one profile name for a common physical key.
    #[must_use]
    pub fn profile_name(self) -> Option<&'static str> {
        KEYBOARD_NAMES.iter().find_map(|(first, names)| {
            let offset = usize::try_from(self.0.checked_sub(*first)?).ok()?;
            names.get(offset).copied()
        })
    }
}

/// Physical pointer buttons, excluding unknown or synthetic touch buttons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    /// Left button.
    Left,
    /// Middle button.
    Middle,
    /// Right button.
    Right,
    /// First auxiliary button.
    X1,
    /// Second auxiliary button.
    X2,
}

impl PointerButton {
    /// All admitted physical pointer buttons.
    pub const ALL: [Self; 5] =
        [Self::Left, Self::Middle, Self::Right, Self::X1, Self::X2];

    /// Stable routing code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Middle => 2,
            Self::Right => 3,
            Self::X1 => 4,
            Self::X2 => 5,
        }
    }

    /// Resolve a version-one pointer profile name.
    #[must_use]
    pub fn from_profile_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|button| button.profile_name() == name)
    }

    /// Stable version-one pointer profile name.
    #[must_use]
    pub const fn profile_name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
            Self::X1 => "x1",
            Self::X2 => "x2",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerAxis, ControllerButton, KeyboardKey, PointerButton};

    #[test]
    fn axis_codes_and_names_are_stable() {
        for (code, axis) in (0..).zip(ControllerAxis::ALL) {
            assert_eq!(axis.code(), code);
            assert_eq!(
                ControllerAxis::from_profile_name(axis.profile_name()),
                Some(axis)
            );
        }
        assert_eq!(
            ControllerAxis::from_profile_name("-LEFTX"),
            Some(ControllerAxis::LeftX)
        );
        assert_eq!(ControllerAxis::from_profile_name("unknown"), None);
    }

    #[test]
    fn button_codes_and_names_are_stable() {
        for (code, button) in (0..).zip(ControllerButton::ALL) {
            assert_eq!(button.code(), code);
            assert_eq!(
                ControllerButton::from_profile_name(button.profile_name()),
                Some(button)
            );
        }
        assert_eq!(
            ControllerButton::from_profile_name("A"),
            Some(ControllerButton::South)
        );
        assert_eq!(ControllerButton::from_profile_name("unknown"), None);
    }

    #[test]
    fn keyboard_names_round_trip_without_a_platform() {
        for code in (4..=129).chain([133]).chain(224..=231) {
            let key = KeyboardKey::from_code(code);
            assert!(key.profile_name().is_some());
            if let Some(name) = key.profile_name() {
                assert_eq!(KeyboardKey::from_profile_name(name), Some(key));
                assert_eq!(
                    KeyboardKey::from_profile_name(&name.to_ascii_lowercase()),
                    Some(key)
                );
            }
        }
        for (key, name) in [
            (KeyboardKey::LETTER_A, "A"),
            (KeyboardKey::ENTER, "Return"),
            (KeyboardKey::SPACE, "Space"),
            (KeyboardKey::F2, "F2"),
            (KeyboardKey::KP_1, "Keypad 1"),
            (KeyboardKey::LEFT, "Left"),
        ] {
            assert_eq!(key.profile_name(), Some(name));
        }
    }

    #[test]
    fn keyboard_unknown_names_and_codes_are_not_admitted() {
        for code in [0, 3, 130, 223, 232, u32::MAX] {
            assert_eq!(KeyboardKey::from_code(code).profile_name(), None);
        }
        for name in ["", "none", "UNKNOWN", " A", "A ", "A\0"] {
            assert_eq!(KeyboardKey::from_profile_name(name), None);
        }
    }

    #[test]
    fn pointer_names_and_codes_are_stable() {
        assert_eq!(PointerButton::from_profile_name("unknown"), None);
        assert_eq!(PointerButton::Left.code(), 1);
        assert_eq!(PointerButton::Right.code(), 3);
    }
}
