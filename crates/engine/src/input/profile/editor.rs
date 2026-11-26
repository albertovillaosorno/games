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
//   - Shared keyboard-driven editor for all digital binding families.
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

    fn capture(
        &mut self,
        event: InputEvent,
        key: Option<KeyboardKey>,
        actions: usize,
    ) -> EditorEffect {
        if key == Some(KeyboardKey::ESCAPE) {
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
                InputEvent::Button { code, down: true, .. },
            ) => ControllerButton::ALL
                .into_iter()
                .find(|button| button.code() == code)
                .map(DigitalBinding::Button),
            (BindingFamily::Touch, _) => {
                match key {
                    Some(KeyboardKey::LEFT | KeyboardKey::UP) => {
                        self.touch_slot =
                            cycle(self.touch_slot, actions, false);
                    },
                    Some(KeyboardKey::RIGHT | KeyboardKey::DOWN) => {
                        self.touch_slot = cycle(self.touch_slot, actions, true);
                    },
                    Some(KeyboardKey::ENTER) => {
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
        if event == InputEvent::FocusLost {
            self.status = EditorStatus::Ready;
            return EditorEffect::None;
        }
        let key = pressed_key(event);
        if self.status == EditorStatus::Capturing {
            return self.capture(event, key, ACTIONS);
        }
        match key {
            Some(KeyboardKey::ESCAPE) => return EditorEffect::Close,
            Some(KeyboardKey::LEFT) => {
                self.station = cycle(self.station, PLAYERS, false);
            },
            Some(KeyboardKey::RIGHT) => {
                self.station = cycle(self.station, PLAYERS, true);
            },
            Some(KeyboardKey::UP) => {
                self.action = cycle(self.action, ACTIONS, false);
            },
            Some(KeyboardKey::DOWN) => {
                self.action = cycle(self.action, ACTIONS, true);
            },
            Some(KeyboardKey::TAB) => self.family = self.family.next(),
            Some(KeyboardKey::ENTER) => {
                if let Some(DigitalBinding::TouchSlot(slot)) =
                    self.current(profiles)
                {
                    self.touch_slot = slot;
                }
                self.status = EditorStatus::Capturing;
                return EditorEffect::None;
            },
            Some(KeyboardKey::DELETE) => {
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
            EditorEffect::Propose(DigitalBinding::Button(
                ControllerButton::North
            ))
        );
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
