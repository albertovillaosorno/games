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
//   - Shared keyboard, gamepad and touch editor for all digital binding
//     families.
// - Description:
//   - Capture presses, not releases; never mutate the running input router.
// - Usage:
//   - Validate and persist proposals before reporting their outcome.
// - Defaults:
//   - First station, first action and keyboard binding, not capturing.

//! Portable next-launch digital binding editor.

mod axis;

use axis::AxisEditor;
pub use axis::{AxisEditorView, AxisField};

use super::{AxisProfile, DigitalBinding, DigitalProfiles};
use crate::input::InputEvent;
use crate::input::physical::{
    ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
};

/// Editable digital source, in menu cycle order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BindingFamily {
    /// Signed controller axis directions and thresholds.
    Axis,
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
            Self::Axis => "CONTROLLER AXIS",
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
            Self::Touch => Self::Axis,
            Self::Axis => Self::Keyboard,
        }
    }
}

/// Last edit result or current capture state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorStatus {
    /// Waiting for the selected source to be assigned.
    Capturing,
    /// The host has been asked to leave; ignore further editor commands.
    Closing,
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
    /// Validate and persist the selected complete axis profile.
    ProposeAxis {
        /// Physical axis to replace.
        axis: ControllerAxis,
        /// Complete staged profile.
        profile: AxisProfile,
    },
    /// Validate and persist the selected seat's touch panel admission.
    ProposeTouchPlayer(bool),
}

/// Menu command, independent of physical binding capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorNavigation {
    /// Cancel capture or close the editor.
    Cancel,
    /// Clear an optional binding.
    Clear,
    /// Begin capture or confirm a pending binding.
    Confirm,
    /// Select the following action or touch slot.
    Down,
    /// Cycle the input family.
    Family,
    /// Select the previous station or touch slot.
    Left,
    /// Select the following station or touch slot.
    Right,
    /// Select the previous action or touch slot.
    Up,
}

impl EditorNavigation {
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
/// A close request is terminal; create a fresh editor on the next entry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlEditor {
    action: usize,
    axes: AxisEditor,
    family: BindingFamily,
    station: usize,
    status: EditorStatus,
    touch_slot: usize,
}

impl ControlEditor {
    /// Record a host-confirmed outcome, never implying successful storage
    /// before the complete profile is admitted and persisted.
    pub const fn acknowledge(&mut self, saved: bool) {
        if matches!(self.status, EditorStatus::Closing) {
            return;
        }
        self.axes.cancel();
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

    /// Current or staged axis values for the axis-specific editor mode.
    #[must_use]
    pub fn axis_view<const PLAYERS: usize, const ACTIONS: usize>(
        self,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> Option<AxisEditorView> {
        if self.family != BindingFamily::Axis {
            return None;
        }
        self.axes.view(profiles.axes.get(self.station)?)
    }

    fn begin_capture<const PLAYERS: usize, const ACTIONS: usize>(
        &mut self,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) {
        if self.family == BindingFamily::Axis {
            if let Some(axes) = profiles.axes.get(self.station)
                && self.axes.begin(axes)
            {
                self.status = EditorStatus::Capturing;
            }
            return;
        }
        if let Some(DigitalBinding::TouchSlot(slot)) = self.current(profiles) {
            self.touch_slot = slot;
        }
        self.status = EditorStatus::Capturing;
    }

    fn capture(&mut self, event: InputEvent, actions: usize) -> EditorEffect {
        let key = pressed_key(event);
        let gamepad = pressed_button(event);
        let navigation = navigation(event);
        // Controller capture must admit every physical button, including
        // the navigation controls. Confirmation is a separate edge.
        if key == Some(KeyboardKey::ESCAPE)
            || (self.family != BindingFamily::Controller
                && navigation == Some(EditorNavigation::Cancel))
        {
            self.axes.cancel();
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
            (BindingFamily::Touch | BindingFamily::Axis, _) => {
                return navigation.map_or(EditorEffect::None, |command| {
                    self.capture_navigation(command, actions)
                });
            },
            _ => None,
        };
        binding.map_or(EditorEffect::None, |proposal| {
            self.status = EditorStatus::Ready;
            EditorEffect::Propose(proposal)
        })
    }

    fn capture_navigation(
        &mut self,
        command: EditorNavigation,
        actions: usize,
    ) -> EditorEffect {
        if command == EditorNavigation::Cancel {
            self.axes.cancel();
            self.status = EditorStatus::Ready;
            return EditorEffect::None;
        }
        if self.family == BindingFamily::Axis {
            let effect = self.axes.capture(command, actions);
            if matches!(effect, EditorEffect::ProposeAxis { .. }) {
                self.status = EditorStatus::Ready;
            }
            return effect;
        }
        if self.family != BindingFamily::Touch {
            return EditorEffect::None;
        }
        match command {
            EditorNavigation::Left | EditorNavigation::Up => {
                self.touch_slot = cycle(self.touch_slot, actions, false);
            },
            EditorNavigation::Right | EditorNavigation::Down => {
                self.touch_slot = cycle(self.touch_slot, actions, true);
            },
            EditorNavigation::Confirm => {
                self.status = EditorStatus::Ready;
                return EditorEffect::Propose(DigitalBinding::TouchSlot(
                    self.touch_slot,
                ));
            },
            EditorNavigation::Cancel
            | EditorNavigation::Clear
            | EditorNavigation::Family => {},
        }
        EditorEffect::None
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
            BindingFamily::Axis => return None,
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
    /// it. Clear toggles the selected seat's next-launch touch panel when
    /// not capturing. Axis mode stages both directions and thresholds;
    /// other families capture the next pressed physical binding.
    pub fn input<const PLAYERS: usize, const ACTIONS: usize>(
        &mut self,
        event: InputEvent,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> EditorEffect {
        if self.status == EditorStatus::Closing
            || (self.current(profiles).is_none()
                && self.axis_view(profiles).is_none())
        {
            return EditorEffect::None;
        }
        if matches!(event, InputEvent::FocusLost | InputEvent::Detached { .. })
        {
            self.axes.cancel();
            self.status = EditorStatus::Ready;
            return EditorEffect::None;
        }
        if self.status == EditorStatus::Capturing {
            return self.capture(event, ACTIONS);
        }
        navigation(event).map_or(EditorEffect::None, |command| {
            self.navigate(command, profiles)
        })
    }

    /// Process an explicit menu command from a title's touch interface.
    ///
    /// Commands never become captured physical keys or controller buttons.
    pub fn navigate<const PLAYERS: usize, const ACTIONS: usize>(
        &mut self,
        command: EditorNavigation,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> EditorEffect {
        if self.current(profiles).is_none()
            && self.axis_view(profiles).is_none()
        {
            return EditorEffect::None;
        }
        match self.status {
            EditorStatus::Closing => return EditorEffect::None,
            EditorStatus::Capturing => {
                return self.capture_navigation(command, ACTIONS);
            },
            EditorStatus::Reviewing(button) => {
                return self.review(button, command);
            },
            EditorStatus::Ready
            | EditorStatus::Rejected
            | EditorStatus::Saved => {},
        }
        match command {
            EditorNavigation::Cancel => {
                self.status = EditorStatus::Closing;
                return EditorEffect::Close;
            },
            EditorNavigation::Left => {
                self.station = cycle(self.station, PLAYERS, false);
            },
            EditorNavigation::Right => {
                self.station = cycle(self.station, PLAYERS, true);
            },
            EditorNavigation::Up => self.select_action(ACTIONS, false),
            EditorNavigation::Down => self.select_action(ACTIONS, true),
            EditorNavigation::Family => self.family = self.family.next(),
            EditorNavigation::Confirm => {
                self.begin_capture(profiles);
                return EditorEffect::None;
            },
            EditorNavigation::Clear => {
                let binding = match self.family {
                    BindingFamily::Keyboard => DigitalBinding::Key(None),
                    BindingFamily::Mouse => DigitalBinding::Mouse(None),
                    BindingFamily::Touch => {
                        return self.toggle_touch_panel(profiles);
                    },
                    BindingFamily::Controller | BindingFamily::Axis => {
                        return EditorEffect::None;
                    },
                };
                return EditorEffect::Propose(binding);
            },
        }
        self.status = EditorStatus::Ready;
        EditorEffect::None
    }

    const fn review(
        &mut self,
        button: ControllerButton,
        command: EditorNavigation,
    ) -> EditorEffect {
        match command {
            EditorNavigation::Confirm => {
                self.status = EditorStatus::Ready;
                EditorEffect::Propose(DigitalBinding::Button(button))
            },
            EditorNavigation::Cancel => {
                self.status = EditorStatus::Ready;
                EditorEffect::None
            },
            EditorNavigation::Clear
            | EditorNavigation::Down
            | EditorNavigation::Family
            | EditorNavigation::Left
            | EditorNavigation::Right
            | EditorNavigation::Up => EditorEffect::None,
        }
    }

    fn select_action(&mut self, actions: usize, forward: bool) {
        if self.family == BindingFamily::Axis {
            self.axes.select(forward);
        } else {
            self.action = cycle(self.action, actions, forward);
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

    fn toggle_touch_panel<const PLAYERS: usize, const ACTIONS: usize>(
        self,
        profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    ) -> EditorEffect {
        profiles
            .touch_players
            .get(self.station)
            .map_or(EditorEffect::None, |enabled| {
                EditorEffect::ProposeTouchPlayer(!enabled)
            })
    }

    /// Pending physical slot when touch capture is active.
    #[must_use]
    pub const fn touch_slot(self) -> usize {
        self.touch_slot
    }
}

fn navigation(event: InputEvent) -> Option<EditorNavigation> {
    pressed_key(event)
        .and_then(EditorNavigation::key)
        .or_else(|| pressed_button(event).and_then(EditorNavigation::button))
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

    #[test]
    fn explicit_touch_commands_can_edit_and_cancel_without_raw_capture() {
        use EditorNavigation::{Down, Family, Right};
        let mut editor = ControlEditor::default();
        for command in [Right, Down, Family, Family, Family] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        assert_eq!((editor.station(), editor.action()), (1, 1));
        assert_eq!(editor.family(), BindingFamily::Touch);
        cancel_touch_proposal(&mut editor);
        verify_touch_proposal(&mut editor);
    }

    fn cancel_touch_proposal(editor: &mut ControlEditor) {
        use EditorNavigation::{Cancel, Confirm, Right};
        assert_eq!(editor.navigate(Confirm, &PROFILE), EditorEffect::None);
        assert_eq!(editor.touch_slot(), 1);
        assert_eq!(editor.navigate(Right, &PROFILE), EditorEffect::None);
        assert_eq!(editor.touch_slot(), 0);
        assert_eq!(editor.navigate(Cancel, &PROFILE), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Ready);
    }

    fn verify_touch_proposal(editor: &mut ControlEditor) {
        use EditorNavigation::{Cancel, Confirm, Right};
        assert_eq!(editor.navigate(Confirm, &PROFILE), EditorEffect::None);
        assert_eq!(editor.navigate(Right, &PROFILE), EditorEffect::None);
        assert_eq!(
            editor.navigate(Confirm, &PROFILE),
            EditorEffect::Propose(DigitalBinding::TouchSlot(0))
        );
        editor.acknowledge(true);
        assert_eq!(editor.navigate(Cancel, &PROFILE), EditorEffect::Close);
    }

    #[test]
    fn touch_clear_proposes_panel_toggle_without_changing_active_profile() {
        use EditorNavigation::{Clear, Family, Right};
        let mut editor = ControlEditor::default();
        for command in [Family, Family, Family] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        assert_eq!(
            editor.navigate(Clear, &PROFILE),
            EditorEffect::ProposeTouchPlayer(false)
        );
        assert!(PROFILE.edited_touch_player(0, false).is_err());
        editor.acknowledge(false);
        assert_eq!(editor.navigate(Right, &PROFILE), EditorEffect::None);
        assert_eq!(
            editor.navigate(Clear, &PROFILE),
            EditorEffect::ProposeTouchPlayer(true)
        );
        let candidate = PROFILE.edited_touch_player(1, true);
        assert_eq!(
            candidate,
            Ok(DigitalProfiles {
                touch_players: [true, true],
                ..PROFILE
            })
        );
        verify_clear_during_touch_capture(&mut editor);
    }

    fn verify_clear_during_touch_capture(editor: &mut ControlEditor) {
        use EditorNavigation::{Clear, Confirm};
        editor.acknowledge(true);
        assert_eq!(editor.navigate(Confirm, &PROFILE), EditorEffect::None);
        assert_eq!(editor.navigate(Clear, &PROFILE), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Capturing);
    }

    #[test]
    fn explicit_cancel_does_not_capture_a_controller_navigation_button() {
        use EditorNavigation::{Cancel, Confirm, Family};
        let mut editor = ControlEditor::default();
        for command in [Family, Family, Confirm] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        assert_eq!(editor.family(), BindingFamily::Controller);
        assert_eq!(editor.status(), EditorStatus::Capturing);
        assert_eq!(editor.navigate(Cancel, &PROFILE), EditorEffect::None);
        assert_eq!(editor.status(), EditorStatus::Ready);
    }

    #[test]
    fn axis_mode_stages_and_commits_both_directions_and_thresholds() {
        use EditorNavigation::{Confirm, Down, Family, Right};
        let mut editor = ControlEditor::default();
        for command in [Family, Family, Family, Family] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        assert_eq!(editor.family(), BindingFamily::Axis);
        assert_eq!(editor.navigate(Right, &PROFILE), EditorEffect::None);
        assert_eq!(editor.navigate(Down, &PROFILE), EditorEffect::None);
        assert_eq!(editor.navigate(Confirm, &PROFILE), EditorEffect::None);
        stage_axis(&mut editor);
    }

    fn stage_axis(editor: &mut ControlEditor) {
        use EditorNavigation::{Confirm, Down, Right};
        for command in [Right, Down, Right, Right, Down, Right] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        let expected = AxisProfile {
            negative: Some(0),
            positive: Some(1),
            press: 17_000,
            ..AxisProfile::UNBOUND
        };
        assert_eq!(
            editor.navigate(Confirm, &PROFILE),
            EditorEffect::ProposeAxis {
                axis: ControllerAxis::LeftY,
                profile: expected,
            }
        );
        assert_eq!(editor.station(), 1);
        let candidate = PROFILE.edited_axis(1, ControllerAxis::LeftY, expected);
        assert!(candidate.is_ok());
        assert_eq!(
            editor.axis_view(&PROFILE).map(|view| view.profile),
            Some(AxisProfile::UNBOUND)
        );
    }

    #[test]
    fn axis_draft_is_discarded_on_focus_loss() {
        use EditorNavigation::{Confirm, Family, Right};
        let mut editor = ControlEditor::default();
        for command in [Family, Family, Family, Family, Confirm, Right] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        assert_eq!(editor.status(), EditorStatus::Capturing);
        assert_eq!(
            editor.input(InputEvent::FocusLost, &PROFILE),
            EditorEffect::None
        );
        assert_eq!(editor.status(), EditorStatus::Ready);
        assert_eq!(
            editor.axis_view(&PROFILE).map(|view| view.profile),
            Some(AxisProfile::UNBOUND)
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

    #[test]
    fn close_request_ignores_queued_editor_commands() {
        let mut keyboard = ControlEditor::default();
        assert_eq!(
            key(&mut keyboard, KeyboardKey::ESCAPE),
            EditorEffect::Close
        );
        assert_eq!(keyboard.status(), EditorStatus::Closing);
        verify_closing(&mut keyboard);

        let mut controller = ControlEditor::default();
        assert_eq!(
            button(&mut controller, ControllerButton::East),
            EditorEffect::Close
        );
        verify_closing(&mut controller);

        let mut touch = ControlEditor::default();
        assert_eq!(
            touch.navigate(EditorNavigation::Cancel, &PROFILE),
            EditorEffect::Close
        );
        verify_closing(&mut touch);
    }

    fn verify_closing(editor: &mut ControlEditor) {
        let closed = *editor;
        for keypress in [
            KeyboardKey::DELETE,
            KeyboardKey::ENTER,
            KeyboardKey::LETTER_A,
            KeyboardKey::ESCAPE,
        ] {
            assert_eq!(key(editor, keypress), EditorEffect::None);
        }
        for buttonpress in [
            ControllerButton::West,
            ControllerButton::South,
            ControllerButton::East,
        ] {
            assert_eq!(button(editor, buttonpress), EditorEffect::None);
        }
        verify_closing_navigation(editor, closed);
    }

    fn verify_closing_navigation(
        editor: &mut ControlEditor,
        closed: ControlEditor,
    ) {
        for command in [
            EditorNavigation::Clear,
            EditorNavigation::Confirm,
            EditorNavigation::Cancel,
            EditorNavigation::Family,
        ] {
            assert_eq!(editor.navigate(command, &PROFILE), EditorEffect::None);
        }
        for event in [InputEvent::FocusLost, InputEvent::Detached { device: 1 }]
        {
            assert_eq!(editor.input(event, &PROFILE), EditorEffect::None);
        }
        editor.acknowledge(true);
        assert_eq!(*editor, closed);
    }
}
