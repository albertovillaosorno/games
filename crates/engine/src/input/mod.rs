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
//   - Shared logical input routing and hot-plug release semantics.
// - Must-Not:
//   - Read operating-system devices or prescribe per-title controls.
// - Allows:
//   - Inputs: Platform-neutral key, gamepad and touch events.
//   - Outputs: Aggregated logical action transitions.
//   - Side effects: Tracks connected devices and held inputs in memory.
// - Split-When:
//   - Native operating-system input adapters require separate ownership.
// - Merge-When:
//   - Another shared input router covers identical semantics.
// - Summary:
//   - Configurable keyboard, controller and touch input routing.
// - Description:
//   - Releases held actions on unplug without releasing other sources.
// - Usage:
//   - Configure mappings per title, then process platform events.
// - Defaults:
//   - Disconnected controllers do not produce logical actions.
//
//! Platform-independent input mapping with robust device disconnects.

/// Normalized touch geometry and bounded contact lifetimes.
pub mod touch;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use self::touch::{
    ContactId, ContactPhase, ContactUpdate, Contacts, TouchLayout,
};

/// A platform-neutral input event from a native host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    /// A controller became available to the host.
    Attached {
        /// Stable host-local device identifier.
        device: u64,
    },
    /// A controller axis changed its signed native value.
    Axis {
        /// Controller-specific axis code.
        code: u16,
        /// Host-local controller identifier.
        device: u64,
        /// Signed axis value, without host dead-zone filtering.
        value: i16,
    },
    /// A gamepad button changed state.
    Button {
        /// Host-local controller identifier.
        device: u64,
        /// Controller-specific button code.
        code: u16,
        /// Whether the button is pressed.
        down: bool,
    },
    /// A normalized touch contact changed position or lifetime.
    Contact {
        /// Stable identity of this device and finger.
        id: ContactId,
        /// Press, motion or release of the contact.
        phase: ContactPhase,
    },
    /// A controller disconnected and must release all held controls.
    Detached {
        /// Identifier of the disconnected controller.
        device: u64,
    },
    /// The host window lost keyboard and touch focus.
    FocusLost,
    /// A keyboard scancode changed state.
    Key {
        /// Physical keyboard scancode.
        code: u32,
        /// Whether the key is pressed.
        down: bool,
    },
    /// A configured title-specific touch zone changed state.
    Touch {
        /// Title-defined touch zone identifier.
        zone: u16,
        /// Whether the touch zone is pressed.
        down: bool,
    },
}

/// Optional directional actions and hysteresis for a signed controller axis.
#[derive(Clone, Debug)]
pub struct AxisBinding<Action> {
    negative: Option<Action>,
    positive: Option<Action>,
    press: u16,
    release: u16,
}

impl<Action> AxisBinding<Action> {
    /// Configure independent negative and positive actions.
    ///
    /// A direction presses at the inclusive press magnitude and releases
    /// at or below the release magnitude. Both directions use the same
    /// thresholds. Invalid thresholds are rejected; zero is neutral.
    #[must_use]
    pub fn new(
        negative: Option<Action>,
        positive: Option<Action>,
        press: u16,
        release: u16,
    ) -> Option<Self> {
        if press > 32_767 || release >= press {
            return None;
        }
        Some(Self {
            negative,
            positive,
            press,
            release,
        })
    }
}

/// Bind physical control codes to title-owned logical actions.
#[derive(Clone, Debug)]
pub struct InputMapping<Action> {
    axes: BTreeMap<u16, AxisBinding<Action>>,
    buttons: BTreeMap<u16, Action>,
    keys: BTreeMap<u32, Action>,
    touch: BTreeMap<u16, Action>,
    touch_layout: Option<TouchLayout>,
}

/// A logical action changed its aggregate pressed state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalAction<Action> {
    /// Title-defined action identifier.
    pub action: Action,
    /// `true` when the first source pressed, `false` on the last release.
    pub down: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Source {
    Axis(u64, u16, AxisDirection),
    Button(u64, u16),
    Contact(ContactId),
    Key(u32),
    Touch(u16),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AxisDirection {
    Negative,
    Positive,
}

#[derive(Debug)]
struct ControllerMapping<Action> {
    axes: BTreeMap<u16, AxisBinding<Action>>,
    buttons: BTreeMap<u16, Action>,
}

/// Shared router that preserves logical state across multiple input devices.
#[derive(Debug)]
pub struct InputRouter<Action: Clone + Ord> {
    connected: BTreeSet<u64>,
    contacts: Contacts,
    controller_maps: BTreeMap<u64, ControllerMapping<Action>>,
    held: BTreeMap<Source, Action>,
    mapping: InputMapping<Action>,
    pressed: BTreeMap<Action, u32>,
}

impl<Action> Default for InputMapping<Action> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Action> InputMapping<Action> {
    /// Bind an axis to optional negative and positive logical actions.
    pub fn bind_axis(&mut self, code: u16, binding: AxisBinding<Action>) {
        let _old = self.axes.insert(code, binding);
    }

    /// Assign a controller button to a logical action.
    pub fn bind_button(&mut self, code: u16, action: Action) {
        let _old = self.buttons.insert(code, action);
    }

    /// Assign a keyboard scancode to a logical action.
    pub fn bind_key(&mut self, code: u32, action: Action) {
        let _old = self.keys.insert(code, action);
    }

    /// Assign a per-title touch zone to a logical action.
    pub fn bind_touch(&mut self, zone: u16, action: Action) {
        let _old = self.touch.insert(zone, action);
    }

    /// Create a mapping with no implicit bindings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            axes: BTreeMap::new(),
            buttons: BTreeMap::new(),
            keys: BTreeMap::new(),
            touch: BTreeMap::new(),
            touch_layout: None,
        }
    }

    /// Enable bounded touch-contact routing with title-selected zones.
    pub fn set_touch_layout(&mut self, layout: TouchLayout) {
        self.touch_layout = Some(layout);
    }
}

impl<Action: Clone + Ord> InputRouter<Action> {
    fn axis(
        &mut self,
        device: u64,
        code: u16,
        value: i16,
    ) -> Vec<LogicalAction<Action>> {
        if !self.connected.contains(&device) {
            return Vec::new();
        }
        let candidate = self
            .controller_maps
            .get(&device)
            .map_or(&self.mapping.axes, |mapping| &mapping.axes)
            .get(&code)
            .cloned();
        let Some(binding) = candidate else {
            return Vec::new();
        };
        let negative = Source::Axis(device, code, AxisDirection::Negative);
        let positive = Source::Axis(device, code, AxisDirection::Positive);
        let (active, other, action) = if value < 0 {
            (negative, positive, binding.negative)
        } else {
            (positive, negative, binding.positive)
        };
        let magnitude = value.unsigned_abs();
        let down = if self.held.contains_key(&active) {
            magnitude > binding.release
        } else {
            magnitude >= binding.press
        };
        // Press before releasing the opposite direction so the same
        // logical action bound on both sides never flickers on reversal.
        let mut changes = self.transition(active, action, down);
        changes.extend(self.release(other));
        changes
    }

    fn contact(
        &mut self,
        id: ContactId,
        phase: ContactPhase,
    ) -> Vec<LogicalAction<Action>> {
        let Some(layout) = &self.mapping.touch_layout else {
            return Vec::new();
        };
        let ContactUpdate::Zone(zone) = self.contacts.update(layout, id, phase)
        else {
            return Vec::new();
        };
        let candidate = zone.and_then(|number| self.mapping.touch.get(&number));
        let source = Source::Contact(id);
        if self.held.get(&source) == candidate {
            return Vec::new();
        }
        let selected = candidate.cloned();
        let mut changes: Vec<_> = self.release(source).into_iter().collect();
        changes.extend(selected.and_then(|action| self.press(source, action)));
        changes
    }

    fn detach(&mut self, device: u64) -> Vec<LogicalAction<Action>> {
        let _connected = self.connected.remove(&device);
        let _mapping = self.controller_maps.remove(&device);
        self.release_controller(device)
    }

    fn focus_lost(&mut self) -> Vec<LogicalAction<Action>> {
        self.contacts.clear();
        self.release_range(
            Source::Contact(ContactId {
                device: i64::MIN,
                finger: i64::MIN,
            })..=Source::Touch(u16::MAX),
        )
    }

    /// Process an event and return only changed logical action states.
    ///
    /// Multiple held physical sources for one action are aggregated, so
    /// disconnecting one controller cannot release another held control.
    pub fn handle(&mut self, event: InputEvent) -> Vec<LogicalAction<Action>> {
        match event {
            InputEvent::Attached { device } => {
                let _connected = self.connected.insert(device);
                Vec::new()
            },
            InputEvent::Axis { code, device, value } => {
                self.axis(device, code, value)
            },
            InputEvent::Button { device, code, down } => {
                if !self.connected.contains(&device) {
                    return Vec::new();
                }
                let mapping = self
                    .controller_maps
                    .get(&device)
                    .map_or(&self.mapping.buttons, |mapping| &mapping.buttons)
                    .get(&code)
                    .cloned();
                self.transition(Source::Button(device, code), mapping, down)
            },
            InputEvent::Contact { id, phase } => self.contact(id, phase),
            InputEvent::Detached { device } => self.detach(device),
            InputEvent::FocusLost => self.focus_lost(),
            InputEvent::Key { code, down } => {
                let mapping = self.mapping.keys.get(&code).cloned();
                self.transition(Source::Key(code), mapping, down)
            },
            InputEvent::Touch { zone, down } => {
                let mapping = self.mapping.touch.get(&zone).cloned();
                self.transition(Source::Touch(zone), mapping, down)
            },
        }
    }

    /// Create an input router with title-configured logical actions.
    #[must_use]
    pub const fn new(mapping: InputMapping<Action>) -> Self {
        Self {
            connected: BTreeSet::new(),
            contacts: Contacts::new(),
            controller_maps: BTreeMap::new(),
            held: BTreeMap::new(),
            mapping,
            pressed: BTreeMap::new(),
        }
    }

    fn press(
        &mut self,
        source: Source,
        action: Action,
    ) -> Option<LogicalAction<Action>> {
        if self.held.contains_key(&source) {
            return None;
        }
        let _previous = self.held.insert(source, action.clone());
        let count = self.pressed.entry(action.clone()).or_default();
        *count = count.saturating_add(1);
        (*count == 1).then_some(LogicalAction { action, down: true })
    }

    fn release(&mut self, source: Source) -> Option<LogicalAction<Action>> {
        let action = self.held.remove(&source)?;
        let count = self.pressed.get_mut(&action)?;
        *count = count.saturating_sub(1);
        if *count != 0 {
            return None;
        }
        let _count = self.pressed.remove(&action);
        Some(LogicalAction { action, down: false })
    }

    fn release_controller(
        &mut self,
        device: u64,
    ) -> Vec<LogicalAction<Action>> {
        let mut changes = self.release_range(
            Source::Axis(device, u16::MIN, AxisDirection::Negative)
                ..=Source::Axis(device, u16::MAX, AxisDirection::Positive),
        );
        changes.extend(self.release_range(
            Source::Button(device, u16::MIN)..=Source::Button(device, u16::MAX),
        ));
        changes
    }

    fn release_range(
        &mut self,
        range: RangeInclusive<Source>,
    ) -> Vec<LogicalAction<Action>> {
        let mut changes = Vec::new();
        while let Some(source) = self
            .held
            .range(range.clone())
            .next()
            .map(|(source, _action)| *source)
        {
            if let Some(change) = self.release(source) {
                changes.push(change);
            }
        }
        changes
    }

    /// Assign an attached controller exclusive button and axis mappings.
    ///
    /// Existing holds are released before reassignment. Unmapped axes and
    /// buttons never fall back to the shared mapping.
    /// Returns None if the device is not attached.
    #[must_use]
    pub fn set_controller_controls(
        &mut self,
        device: u64,
        buttons: BTreeMap<u16, Action>,
        axes: BTreeMap<u16, AxisBinding<Action>>,
    ) -> Option<Vec<LogicalAction<Action>>> {
        if !self.connected.contains(&device) {
            return None;
        }
        let changes = self.release_controller(device);
        let mapping = ControllerMapping { axes, buttons };
        let _previous = self.controller_maps.insert(device, mapping);
        Some(changes)
    }

    /// Assign an attached controller an exclusive, player-specific mapping.
    ///
    /// This button-only mapping disables axes. Unmapped buttons never
    /// fall back to the shared mapping. Existing holds are released before
    /// reassignment; a detached device must be assigned again.
    /// Returns None if the device is not attached.
    #[must_use]
    pub fn set_controller_mapping(
        &mut self,
        device: u64,
        buttons: BTreeMap<u16, Action>,
    ) -> Option<Vec<LogicalAction<Action>>> {
        self.set_controller_controls(device, buttons, BTreeMap::new())
    }

    fn transition(
        &mut self,
        source: Source,
        mapping: Option<Action>,
        down: bool,
    ) -> Vec<LogicalAction<Action>> {
        let transition = if down {
            mapping.and_then(|action| self.press(source, action))
        } else {
            self.release(source)
        };
        transition.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        AxisBinding, InputEvent, InputMapping, InputRouter, LogicalAction,
    };

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum Action {
        Confirm,
        Jump,
    }

    type Step<'actions> = (InputEvent, &'actions [LogicalAction<&'static str>]);

    const fn axis_event(device: u64, value: i16) -> InputEvent {
        InputEvent::Axis {
            code: u16::MAX,
            device,
            value,
        }
    }

    const fn change(action: &str, down: bool) -> LogicalAction<&str> {
        LogicalAction { action, down }
    }

    fn verify_steps(
        router: &mut InputRouter<&'static str>,
        steps: &[Step<'_>],
    ) -> Result<(), &'static str> {
        for (event, expected) in steps {
            if router.handle(*event) != *expected {
                return Err("unexpected action transition");
            }
        }
        Ok(())
    }

    #[test]
    fn axis_hysteresis_and_direct_reversal() -> Result<(), &'static str> {
        let mut map = InputMapping::new();
        let binding =
            AxisBinding::new(Some("left"), Some("right"), 16_000, 8000)
                .ok_or("invalid test binding")?;
        map.bind_axis(u16::MAX, binding);
        let mut router = InputRouter::new(map);
        verify_steps(&mut router, &[
            (axis_event(7, i16::MIN), &[]),
            (InputEvent::Attached { device: 7 }, &[]),
            (axis_event(7, -15_999), &[]),
            (axis_event(7, -16_000), &[change("left", true)]),
            (InputEvent::FocusLost, &[]),
            (axis_event(7, -8001), &[]),
            (axis_event(7, -8000), &[change("left", false)]),
            (axis_event(7, i16::MIN), &[change("left", true)]),
            (axis_event(7, i16::MAX), &[
                change("right", true),
                change("left", false),
            ]),
            (axis_event(7, 8001), &[]),
            (axis_event(7, 8000), &[change("right", false)]),
            (axis_event(7, 0), &[]),
        ])
    }

    #[test]
    fn invalid_axis_thresholds_are_rejected() {
        assert!(AxisBinding::<&str>::new(None, None, 0, 0).is_none());
        assert!(AxisBinding::<&str>::new(None, None, 32_768, 0).is_none());
        assert!(AxisBinding::<&str>::new(None, None, 8000, 8000).is_none());
    }

    #[test]
    fn axes_and_buttons_aggregate_without_reversal_flicker()
    -> Result<(), &'static str> {
        let mut map = InputMapping::new();
        let binding = AxisBinding::new(Some("move"), Some("move"), 10, 5)
            .ok_or("invalid test binding")?;
        map.bind_axis(u16::MAX, binding);
        map.bind_button(0, "move");
        let mut router = InputRouter::new(map);
        verify_steps(&mut router, &[
            (InputEvent::Attached { device: u64::MIN }, &[]),
            (InputEvent::Attached { device: u64::MAX }, &[]),
            (axis_event(u64::MIN, -10), &[change("move", true)]),
            (axis_event(u64::MIN, 10), &[]),
            (axis_event(u64::MAX, 10), &[]),
            (
                InputEvent::Button {
                    device: u64::MAX,
                    code: 0,
                    down: true,
                },
                &[],
            ),
            (InputEvent::Detached { device: u64::MIN }, &[]),
            (axis_event(u64::MAX, 0), &[]),
            (InputEvent::Detached { device: u64::MAX }, &[change(
                "move", false,
            )]),
            (axis_event(u64::MAX, 10), &[]),
        ])
    }

    #[test]
    fn axis_remapping_releases_holds_without_shared_fallback()
    -> Result<(), &'static str> {
        let mut map = InputMapping::new();
        let shared = AxisBinding::new(None, Some("shared"), 10, 5)
            .ok_or("invalid shared binding")?;
        map.bind_axis(u16::MAX, shared);
        let mut router = InputRouter::new(map);
        let assigned = AxisBinding::new(None, Some("assigned"), 10, 5)
            .ok_or("invalid assigned binding")?;
        let axes = BTreeMap::from([(u16::MAX, assigned)]);
        if router
            .set_controller_controls(2, BTreeMap::new(), axes.clone())
            .is_some()
        {
            return Err("detached mapping accepted");
        }
        verify_steps(&mut router, &[
            (InputEvent::Attached { device: 2 }, &[]),
            (axis_event(2, 10), &[change("shared", true)]),
        ])?;
        if router.set_controller_controls(2, BTreeMap::new(), axes)
            != Some(vec![change("shared", false)])
        {
            return Err("shared axis hold not released");
        }
        verify_steps(&mut router, &[(axis_event(2, 10), &[change(
            "assigned", true,
        )])])?;
        if router.set_controller_mapping(2, BTreeMap::new())
            != Some(vec![change("assigned", false)])
        {
            return Err("assigned axis hold not released");
        }
        verify_steps(&mut router, &[
            (axis_event(2, 10), &[]),
            (InputEvent::Detached { device: 2 }, &[]),
            (InputEvent::Attached { device: 2 }, &[]),
            (axis_event(2, 10), &[change("shared", true)]),
            (axis_event(2, -10), &[change("shared", false)]),
            (axis_event(2, -20), &[]),
        ])
    }

    #[test]
    fn key_and_gamepad_share_one_logical_hold() {
        let mut map = InputMapping::new();
        map.bind_key(42, Action::Jump);
        map.bind_button(7, Action::Jump);
        let mut router = InputRouter::new(map);
        let down = LogicalAction {
            action: Action::Jump,
            down: true,
        };
        let up = LogicalAction {
            action: Action::Jump,
            down: false,
        };
        assert_eq!(router.handle(InputEvent::Key { code: 42, down: true }), [
            down
        ]);
        assert_eq!(router.handle(InputEvent::Attached { device: 5 }), []);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 5,
                code: 7,
                down: true,
            }),
            []
        );
        assert_eq!(router.handle(InputEvent::Key { code: 42, down: false }), [
        ]);
        assert_eq!(router.handle(InputEvent::Detached { device: 5 }), [up]);
    }

    #[test]
    fn hotplug_releases_only_disconnected_controller() {
        let mut map = InputMapping::new();
        map.bind_button(1, Action::Confirm);
        let mut router = InputRouter::new(map);
        let pressed = LogicalAction {
            action: Action::Confirm,
            down: true,
        };
        let released = LogicalAction {
            action: Action::Confirm,
            down: false,
        };
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 1,
                code: 1,
                down: true,
            }),
            []
        );
        assert_eq!(router.handle(InputEvent::Attached { device: 1 }), []);
        assert_eq!(router.handle(InputEvent::Attached { device: 2 }), []);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 1,
                code: 1,
                down: true,
            }),
            [pressed]
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 2,
                code: 1,
                down: true,
            }),
            []
        );
        assert_eq!(router.handle(InputEvent::Detached { device: 1 }), []);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 2,
                code: 1,
                down: false,
            }),
            [released]
        );
    }

    #[test]
    fn title_specific_touch_zones_remain_independent() {
        let mut first_map = InputMapping::new();
        first_map.bind_touch(8, Action::Jump);
        let mut second_map = InputMapping::new();
        second_map.bind_touch(8, Action::Confirm);
        let mut first = InputRouter::new(first_map);
        let mut second = InputRouter::new(second_map);
        assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: true }), [
            LogicalAction {
                action: Action::Jump,
                down: true
            }
        ]);
        assert_eq!(second.handle(InputEvent::Touch { zone: 8, down: true }), [
            LogicalAction {
                action: Action::Confirm,
                down: true
            }
        ]);
        assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: true }), []);
        assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: false }), [
            LogicalAction {
                action: Action::Jump,
                down: false
            }
        ]);
    }

    #[test]
    fn detach_releases_only_its_held_buttons_in_control_order() {
        let mut map = InputMapping::new();
        for code in [0u16, 1, 2, u16::MAX] {
            map.bind_button(code, code);
        }
        let mut router = InputRouter::new(map);
        for device in [u64::MIN, 5, u64::MAX] {
            drop(router.handle(InputEvent::Attached { device }));
        }
        for code in [u16::MAX, 2, 1, 0] {
            assert_eq!(
                router.handle(InputEvent::Button {
                    device: 5,
                    code,
                    down: true,
                }),
                [LogicalAction { action: code, down: true }]
            );
        }
        for (device, code) in [(u64::MIN, 1), (u64::MAX, u16::MAX)] {
            assert_eq!(
                router.handle(InputEvent::Button { device, code, down: true }),
                []
            );
        }
        assert_eq!(router.handle(InputEvent::Detached { device: 5 }), [
            LogicalAction { action: 0, down: false },
            LogicalAction { action: 2, down: false },
        ]);
        assert_eq!(router.handle(InputEvent::Detached { device: u64::MIN }), [
            LogicalAction { action: 1, down: false }
        ]);
        assert_eq!(router.handle(InputEvent::Detached { device: u64::MAX }), [
            LogicalAction {
                action: u16::MAX,
                down: false
            }
        ]);
    }

    fn assign_and_press(router: &mut InputRouter<u16>, player: u16) {
        let device = u64::from(player);
        assert_eq!(router.handle(InputEvent::Attached { device }), []);
        assert_eq!(
            router
                .set_controller_mapping(device, BTreeMap::from([(1, player)])),
            Some(Vec::new())
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device,
                code: 1,
                down: true
            }),
            [LogicalAction {
                action: player,
                down: true
            }]
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device,
                code: 2,
                down: true,
            }),
            []
        );
    }

    #[test]
    fn four_controllers_have_independent_player_actions() {
        let mut mapping = InputMapping::new();
        mapping.bind_button(2, 99u16);
        let mut router = InputRouter::new(mapping);
        assert_eq!(router.set_controller_mapping(1, BTreeMap::new()), None);
        for player in 1u16..=4 {
            assign_and_press(&mut router, player);
        }
        for player in (1u16..=4).rev() {
            assert_eq!(
                router.handle(InputEvent::Detached {
                    device: u64::from(player)
                }),
                [LogicalAction {
                    action: player,
                    down: false
                }]
            );
        }
        assert_eq!(router.handle(InputEvent::Attached { device: 1 }), []);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 1,
                code: 1,
                down: true,
            }),
            []
        );
    }

    #[test]
    fn reassignment_releases_old_player_without_affecting_other_sources() {
        let mut mapping = InputMapping::new();
        mapping.bind_key(10, 1u16);
        mapping.bind_button(1, 1);
        mapping.bind_button(2, 2);
        let mut router = InputRouter::new(mapping);
        assert_eq!(router.handle(InputEvent::Attached { device: 7 }), []);
        assert_eq!(router.handle(InputEvent::Key { code: 10, down: true }), [
            LogicalAction { action: 1, down: true },
        ]);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 7,
                code: 1,
                down: true,
            }),
            []
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 7,
                code: 2,
                down: true,
            }),
            [LogicalAction { action: 2, down: true }]
        );
        assert_eq!(
            router.set_controller_mapping(7, BTreeMap::from([(1, 3)])),
            Some(vec![LogicalAction { action: 2, down: false }])
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 7,
                code: 1,
                down: false,
            }),
            []
        );
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 7,
                code: 1,
                down: true,
            }),
            [LogicalAction { action: 3, down: true }]
        );
        assert_eq!(router.handle(InputEvent::Key { code: 10, down: false }), [
            LogicalAction { action: 1, down: false },
        ]);
    }

    #[test]
    fn focus_loss_releases_keys_and_touch_but_not_controllers() {
        let mut mapping = InputMapping::new();
        mapping.bind_key(1, 1u16);
        mapping.bind_touch(1, 2);
        mapping.bind_key(2, 3);
        mapping.bind_button(1, 1);
        let mut router = InputRouter::new(mapping);
        for event in [
            InputEvent::Attached { device: 1 },
            InputEvent::Key { code: 1, down: true },
            InputEvent::Touch { zone: 1, down: true },
            InputEvent::Key { code: 2, down: true },
            InputEvent::Button {
                device: 1,
                code: 1,
                down: true,
            },
        ] {
            drop(router.handle(event));
        }
        assert_eq!(router.handle(InputEvent::FocusLost), [
            LogicalAction { action: 3, down: false },
            LogicalAction { action: 2, down: false },
        ]);
        assert_eq!(router.handle(InputEvent::FocusLost), []);
        assert_eq!(router.handle(InputEvent::Detached { device: 1 }), [
            LogicalAction { action: 1, down: false },
        ]);
    }
}
