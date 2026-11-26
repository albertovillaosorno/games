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
//   - Selection and capture for next-launch digital control editing.
// - Must-Not:
//   - Assign devices, admit title policies, persist profiles or draw menus.
// - Allows:
//   - Inputs: Portable physical events and the current admitted profile.
//   - Outputs: Selected binding proposals, cancellation and close requests.
//   - Side effects: Deterministic in-memory editor transitions only.
// - Split-When:
//   - Analogue editing needs an independent calibration workflow.
// - Merge-When:
//   - Another editor duplicates digital binding selection or capture.
// - Summary:
//   - Shared keyboard and gamepad editor for all digital binding families.
// - Description:
//   - Capture presses, not releases; never mutate the running input router.
// - Usage:
//   - Validate and persist proposals before reporting their outcome.
// - Defaults:
//   - First station, first action and keyboard binding, not capturing.

//! Portable next-launch digital binding editor.

use super::{DigitalBinding, DigitalProfiles};
use crate::input::InputEvent;
use crate::input::physical::{ControllerButton, KeyboardKey, PointerButton};

/// Editable digital source, in menu cycle order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BindingFamily {
    /// Standard controller button.
    Controller,
    /// Physical keyboard key.
    #[default]
    Keyboard,
    /// Physical pointer button.
    Mouse,
    /// Title-provided physical touch slot.
    Touch,
}

impl BindingFamily {
    /// Human-readable source label for a control menu.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "KEYBOARD",
            Self::Mouse => "MOUSE",
            Self::Controller => "CONTROLLER",
            Self::Touch => "TOUCH SLOT",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Keyboard => Self::Mouse,
            Self::Mouse => Self::Controller,
            Self::Controller => Self::Touch,
            Self::Touch => Self::Keyboard,
        }
    }
}

/// Last edit result or current capture state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorStatus {
    /// Waiting for the selected source to be assigned.
    Capturing,
    /// No capture is active.
    #[default]
    Ready,
    /// The candidate was not admitted or saved.
    Rejected,
    /// A captured controller button awaits confirmation or cancellation.
    Reviewing(ControllerButton),
    /// The host confirmed the next-launch profile was saved.
    Saved,
}

/// Request for the host to apply an edit or close the menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorEffect {
    /// Exit the editor without changing active bindings.
    Close,
    /// No external effect.
    None,
    /// Validate and persist this selected binding.
    Propose(DigitalBinding),
}

/// Navigation uses physical identities independently of gameplay bindings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Navigation {
    Cancel,
    Clear,
    Confirm,
    Down,
    Family,
    Left,
    Right,
    Up,
}

impl Navigation {
    const fn button(button: ControllerButton) -> Option<Self> {
        match button {
            ControllerButton::East
            | ControllerButton::Back
            | ControllerButton::Start => Some(Self::Cancel),
            ControllerButton::West => Some(Self::Clear),
            ControllerButton::South => Some(Self::Confirm),
            ControllerButton::DPadDown => Some(Self::Down),
            ControllerButton::RightShoulder => Some(Self::Family),
            ControllerButton::DPadLeft => Some(Self::Left),
            ControllerButton::DPadRight => Some(Self::Right),
            ControllerButton::DPadUp => Some(Self::Up),
            ControllerButton::Guide
            | ControllerButton::LeftShoulder
            | ControllerButton::LeftStick
            | ControllerButton::Misc1
            | ControllerButton::North
            | ControllerButton::Paddle1
            | ControllerButton::Paddle2
            | ControllerButton::Paddle3
            | ControllerButton::Paddle4
            | ControllerButton::RightStick
            | ControllerButton::Touchpad => None,
        }
    }

    const fn key(key: KeyboardKey) -> Option<Self> {
        match key {
            KeyboardKey::ESCAPE => Some(Self::Cancel),
            KeyboardKey::DELETE => Some(Self::Clear),
            KeyboardKey::ENTER => Some(Self::Confirm),
            KeyboardKey::DOWN => Some(Self::Down),
            KeyboardKey::TAB => Some(Self::Family),
            KeyboardKey::LEFT => Some(Self::Left),
            KeyboardKey::RIGHT => Some(Self::Right),
            KeyboardKey::UP => Some(Self::Up),
            _ => None,
        }
    }
}

/// Shared editor selection, independent of title names and storage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlEditor {
    action: usize,
    family: BindingFamily,
    station: usize,
    status: EditorStatus,
    touch_slot: usize,
}

impl ControlEditor {
    /// Record a host-confirmed outcome, never implying successful storage
    /// before the complete profile is admitted and persisted.
    pub const fn acknowledge(&mut self, saved: bool) {
        self.status = if saved {
            EditorStatus::Saved
        } else {
            EditorStatus::Rejected
        };
    }

    /// Selected logical action index.
    #[must_use]
    pub const fn action(self) -> usize {
        self.action
    }

    fn capture(&mut self, event: InputEvent, actions: usize) -> EditorEffect {
        let key = pressed_key(event);
        let gamepad = pressed_button(event);
        let navigation = navigation(event);
        // Controller capture must admit every physical button, including
        // the navigation controls. Confirmation is a separate edge.
        if key == Some(KeyboardKey::ESCAPE)
            || (self.family != BindingFamily::Controller
                && navigation == Some(Navigation::Cancel))
        {
            self.status = EditorStatus::Ready;
            return EditorEffect::None;
        }
        let binding = match (self.family, event) {
            (BindingFamily::Keyboard, InputEvent::Key { down: true, .. }) => {
                key.map(|pressed| DigitalBinding::Key(Some(pressed)))
            },
            (BindingFamily::Mouse, InputEvent::Mouse { code, down: true }) => {
                PointerButton::ALL
                    .into_iter()
                    .find(|button| button.code() == code)
                    .map(|button| DigitalBinding::Mouse(Some(button)))
            },
            (
                BindingFamily::Controller,
                InputEvent::Button { down: true, .. },
            ) => {
                if let Some(candidate) = gamepad {
                    self.status = EditorStatus::Reviewing(candidate);
                }
                None
            },
            (BindingFamily::Touch, _) => {
                match navigation {
                    Some(Navigation::Left | Navigation::Up) => {
                        self.touch_slot =
                            cycle(self.touch_slot, actions, false);
                    },
                    Some(Navigation::Right | Navigation::Down) => {
                        self.touch_slot = cycle(self.touch_slot, actions, true);
                    },
                    Some(Navigation::Confirm) => {
                        self.status = EditorStatus::Ready;
                        return EditorEffect::Propose(
                            DigitalBinding::TouchSlot(self.touch_slot),
                        );
                    },
                    _ => {},
                }
                None
            },
            _ => None,
        };
        binding.map_or(EditorEffect::None, |proposal| {
            self.status = EditorStatus::Ready;
            EditorEffect::Propose(proposal)
        })
    }

    /// Resolve the current admitted binding at the editor selection.
    #[must_use]
    pub fn current<const PLAYERS: usize, const ACTIONS: usize>(
        self,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> Option<DigitalBinding> {
        let station = self.station;
        let action = self.action;
        Some(match self.family {
            BindingFamily::Keyboard => {
                DigitalBinding::Key(*profiles.keys.get(station)?.get(action)?)
            },
            BindingFamily::Mouse => DigitalBinding::Mouse(
                *profiles.mouse.get(station)?.get(action)?,
            ),
            BindingFamily::Controller => DigitalBinding::Button(
                *profiles.buttons.get(station)?.get(action)?,
            ),
            BindingFamily::Touch => DigitalBinding::TouchSlot(
                *profiles.touch_slots.get(station)?.get(action)?,
            ),
        })
    }

    /// Selected source family.
    #[must_use]
    pub const fn family(self) -> BindingFamily {
        self.family
    }

    /// Translate a physical event into an optional host edit request.
    ///
    /// Arrows select station and action, Tab cycles family, Enter starts
    /// capture, Delete clears optional sources, and Escape cancels or exits.
    /// A gamepad uses the D-pad, right shoulder, south and west face
    /// buttons; the east face, back or start button cancels.
    /// Confirm a captured controller button before saving it.
    /// During touch capture, arrows select a physical slot and Enter applies
    /// it. Other families capture the next pressed physical binding.
    pub fn input<const PLAYERS: usize, const ACTIONS: usize>(
        &mut self,
        event: InputEvent,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> EditorEffect {
        if self.current(profiles).is_none() {
            return EditorEffect::None;
        }
        if matches!(event, InputEvent::FocusLost | InputEvent::Detached { .. })
        {
            self.status = EditorStatus::Ready;
            return EditorEffect::None;
        }
        let command = navigation(event);
        match self.status {
            EditorStatus::Capturing => return self.capture(event, ACTIONS),
            EditorStatus::Reviewing(button) => {
                return self.review(button, command);
            },
            EditorStatus::Ready
            | EditorStatus::Rejected
            | EditorStatus::Saved => {},
        }
        match command {
            Some(Navigation::Cancel) => return EditorEffect::Close,
            Some(Navigation::Left) => {
                self.station = cycle(self.station, PLAYERS, false);
            },
            Some(Navigation::Right) => {
                self.station = cycle(self.station, PLAYERS, true);
            },
            Some(Navigation::Up) => {
                self.action = cycle(self.action, ACTIONS, false);
            },
            Some(Navigation::Down) => {
                self.action = cycle(self.action, ACTIONS, true);
            },
            Some(Navigation::Family) => self.family = self.family.next(),
            Some(Navigation::Confirm) => {
                if let Some(DigitalBinding::TouchSlot(slot)) =
                    self.current(profiles)
                {
                    self.touch_slot = slot;
                }
                self.status = EditorStatus::Capturing;
                return EditorEffect::None;
            },
            Some(Navigation::Clear) => {
                let binding = match self.family {
                    BindingFamily::Keyboard => DigitalBinding::Key(None),
                    BindingFamily::Mouse => DigitalBinding::Mouse(None),
                    BindingFamily::Controller | BindingFamily::Touch => {
                        return EditorEffect::None;
                    },
                };
                return EditorEffect::Propose(binding);
            },
            _ => return EditorEffect::None,
        }
        self.status = EditorStatus::Ready;
        EditorEffect::None
    }

    const fn review(
        &mut self,
        button: ControllerButton,
        command: Option<Navigation>,
    ) -> EditorEffect {
        match command {
            Some(Navigation::Confirm) => {
                self.status = EditorStatus::Ready;
                EditorEffect::Propose(DigitalBinding::Button(button))
            },
            Some(Navigation::Cancel) => {
                self.status = EditorStatus::Ready;
                EditorEffect::None
            },
            _ => EditorEffect::None,
        }
    }

    /// Selected station index.
    #[must_use]
    pub const fn station(self) -> usize {
        self.station
    }

    /// Current capture or save status.
    #[must_use]
    pub const fn status(self) -> EditorStatus {
        self.status
    }

    /// Pending physical slot when touch capture is active.
    #[must_use]
    pub const fn touch_slot(self) -> usize {
        self.touch_slot
    }
}

fn navigation(event: InputEvent) -> Option<Navigation> {
    pressed_key(event)
        .and_then(Navigation::key)
        .or_else(|| pressed_button(event).and_then(Navigation::button))
}

fn pressed_button(event: InputEvent) -> Option<ControllerButton> {
    let InputEvent::Button { code, down: true, .. } = event else {
        return None;
    };
    ControllerButton::ALL
        .into_iter()
        .find(|button| button.code() == code)
}

const fn pressed_key(event: InputEvent) -> Option<KeyboardKey> {
    if let InputEvent::Key { code, down: true } = event {
        Some(KeyboardKey::from_code(code))
    } else {
        None
    }
}

fn cycle(index: usize, count: usize, forward: bool) -> usize {
    let offset = if forward {
        1
    } else {
        count.saturating_sub(1)
    };
    index
        .saturating_add(offset)
        .checked_rem(count)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::profile::AxisProfile;

    const PROFILE: DigitalProfiles<2, 2> = DigitalProfiles {
        axes: [[AxisProfile::UNBOUND; 6]; 2],
        buttons: [[ControllerButton::South, ControllerButton::East]; 2],
        keys: [[Some(KeyboardKey::LETTER_A), None], [None; 2]],
        mouse: [[None; 2]; 2],
        touch_players: [true, false],
        touch_slots: [[0, 1]; 2],
    };

    fn key(editor: &mut ControlEditor, key: KeyboardKey) -> EditorEffect {
        editor.input(
            InputEvent::Key {
                code: key.code(),
                down: true,
            },
            &PROFILE,
        )
    }

    #[test]
    fn selection_capture_and_rejection_do_not_change_admitted_profile() {
        let mut editor = ControlEditor::default();
        assert_eq!(key(&mut editor, KeyboardKey::LEFT), EditorEffect::None);
        assert_eq!(key(&mut editor, KeyboardKey::UP), EditorEffect::None);
        assert_eq!((editor.station(), editor.action()), (1, 1));
        assert_eq!(key(&mut editor, KeyboardKey::ENTER), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Capturing);
        reject_duplicate(&mut editor);
    }

    fn reject_duplicate(editor: &mut ControlEditor) {
        assert_eq!(
            editor.input(
                InputEvent::Key {
                    code: KeyboardKey::LETTER_A.code(),
                    down: false,
                },
                &PROFILE
            ),
            EditorEffect::None
        );
        let binding = DigitalBinding::Key(Some(KeyboardKey::LETTER_A));
        assert_eq!(
            key(editor, KeyboardKey::LETTER_A),
            EditorEffect::Propose(binding)
        );
        assert!(PROFILE.edited(1, 1, binding).is_err());
        editor.acknowledge(false);
        assert_eq!(editor.status(), EditorStatus::Rejected);
        assert_eq!(editor.current(&PROFILE), Some(DigitalBinding::Key(None)));
        assert_eq!(key(editor, KeyboardKey::ESCAPE), EditorEffect::Close);
    }

    #[test]
    fn family_capture_cancellation_and_touch_slot_selection() {
        let mut editor = ControlEditor::default();
        assert_eq!(key(&mut editor, KeyboardKey::TAB), EditorEffect::None);
        assert_eq!(editor.family(), BindingFamily::Mouse);
        assert_eq!(key(&mut editor, KeyboardKey::ENTER), EditorEffect::None);
        assert_eq!(
            editor.input(
                InputEvent::Mouse {
                    code: PointerButton::Right.code(),
                    down: true,
                },
                &PROFILE
            ),
            EditorEffect::Propose(DigitalBinding::Mouse(Some(
                PointerButton::Right
            )))
        );
        editor.acknowledge(true);
        assert_eq!(editor.status(), EditorStatus::Saved);
        capture_controller(&mut editor);
        capture_touch(&mut editor);
    }

    fn capture_controller(editor: &mut ControlEditor) {
        assert_eq!(key(editor, KeyboardKey::TAB), EditorEffect::None);
        assert_eq!(key(editor, KeyboardKey::ENTER), EditorEffect::None);
        assert_eq!(
            editor.input(
                InputEvent::Button {
                    code: ControllerButton::North.code(),
                    device: 1,
                    down: true,
                },
                &PROFILE
            ),
            EditorEffect::None
        );
        assert_eq!(
            editor.status(),
            EditorStatus::Reviewing(ControllerButton::North)
        );
        assert_eq!(
            key(editor, KeyboardKey::ENTER),
            EditorEffect::Propose(DigitalBinding::Button(
                ControllerButton::North
            ))
        );
    }

    fn button(
        editor: &mut ControlEditor,
        button: ControllerButton,
    ) -> EditorEffect {
        editor.input(
            InputEvent::Button {
                code: button.code(),
                device: 1,
                down: true,
            },
            &PROFILE,
        )
    }

    #[test]
    fn controller_navigation_can_cancel_or_bind_its_own_buttons() {
        use ControllerButton::{DPadDown, DPadRight, RightShoulder};
        let mut editor = ControlEditor::default();
        for action in [DPadRight, DPadDown, RightShoulder, RightShoulder] {
            assert_eq!(button(&mut editor, action), EditorEffect::None);
        }
        assert_eq!((editor.station(), editor.action()), (1, 1));
        assert_eq!(editor.family(), BindingFamily::Controller);
        cancel_controller_capture(&mut editor);
        confirm_controller_capture(&mut editor);
    }

    fn cancel_controller_capture(editor: &mut ControlEditor) {
        use ControllerButton::{East, South};
        for action in [South, East] {
            assert_eq!(button(editor, action), EditorEffect::None);
        }
        assert_eq!(editor.status(), EditorStatus::Reviewing(East));
        assert_eq!(button(editor, East), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Ready);
        assert_eq!(
            editor.current(&PROFILE),
            Some(DigitalBinding::Button(East))
        );
    }

    fn confirm_controller_capture(editor: &mut ControlEditor) {
        use ControllerButton::{East, RightShoulder, South};
        for action in [South, RightShoulder] {
            assert_eq!(button(editor, action), EditorEffect::None);
        }
        assert_eq!(
            button(editor, South),
            EditorEffect::Propose(DigitalBinding::Button(RightShoulder))
        );
        editor.acknowledge(true);
        assert_eq!(button(editor, East), EditorEffect::Close);
    }

    fn capture_touch(editor: &mut ControlEditor) {
        assert_eq!(key(editor, KeyboardKey::TAB), EditorEffect::None);
        assert_eq!(key(editor, KeyboardKey::ENTER), EditorEffect::None);
        assert_eq!(key(editor, KeyboardKey::LEFT), EditorEffect::None);
        assert_eq!(editor.touch_slot(), 1);
        assert_eq!(key(editor, KeyboardKey::ESCAPE), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Ready);
        apply_touch(editor);
    }

    fn apply_touch(editor: &mut ControlEditor) {
        assert_eq!(key(editor, KeyboardKey::ENTER), EditorEffect::None);
        assert_eq!(editor.touch_slot(), 0);
        assert_eq!(key(editor, KeyboardKey::RIGHT), EditorEffect::None);
        assert_eq!(
            key(editor, KeyboardKey::ENTER),
            EditorEffect::Propose(DigitalBinding::TouchSlot(1))
        );
    }
}
