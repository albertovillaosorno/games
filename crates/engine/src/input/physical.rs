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
//   - Stable physical gamepad and pointer identities and profile names.
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
//   - Platform-independent control identities for standard gamepads and mice.
// - Description:
//   - Preserves existing profile names while separating native event enums.
// - Usage:
//   - Translate native events at the adapter and bind these codes in titles.
// - Defaults:
//   - Unrecognized profile names are rejected.

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
        match name {
            "left" => Some(Self::Left),
            "middle" => Some(Self::Middle),
            "right" => Some(Self::Right),
            "x1" => Some(Self::X1),
            "x2" => Some(Self::X2),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerAxis, ControllerButton, PointerButton};

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
    fn pointer_names_and_codes_are_stable() {
        assert_eq!(PointerButton::from_profile_name("unknown"), None);
        assert_eq!(PointerButton::Left.code(), 1);
        assert_eq!(PointerButton::Right.code(), 3);
    }
}
