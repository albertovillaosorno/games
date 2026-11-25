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

use std::collections::{BTreeMap, BTreeSet};

/// A platform-neutral input event from a native host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    /// A controller became available to the host.
    Attached {
        /// Stable host-local device identifier.
        device: u64,
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
    /// A controller disconnected and must release any held buttons.
    Detached {
        /// Identifier of the disconnected controller.
        device: u64,
    },
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

/// Bind physical control codes to title-owned logical actions.
#[derive(Clone, Debug)]
pub struct InputMapping<Action> {
    buttons: BTreeMap<u16, Action>,
    keys: BTreeMap<u32, Action>,
    touch: BTreeMap<u16, Action>,
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
    Button(u64, u16),
    Key(u32),
    Touch(u16),
}

/// Shared router that preserves logical state across multiple input devices.
#[derive(Debug)]
pub struct InputRouter<Action: Clone + Ord> {
    connected: BTreeSet<u64>,
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
            buttons: BTreeMap::new(),
            keys: BTreeMap::new(),
            touch: BTreeMap::new(),
        }
    }
}

impl<Action: Clone + Ord> InputRouter<Action> {
    fn detach(&mut self, device: u64) -> Vec<LogicalAction<Action>> {
        let _connected = self.connected.remove(&device);
        let mut changes = Vec::new();
        while let Some(source) = self
            .held
            .range(
                Source::Button(device, u16::MIN)
                    ..=Source::Button(device, u16::MAX),
            )
            .next()
            .map(|(source, _action)| *source)
        {
            if let Some(change) = self.release(source) {
                changes.push(change);
            }
        }
        changes
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
            InputEvent::Button { device, code, down } => {
                if !self.connected.contains(&device) {
                    return Vec::new();
                }
                let mapping = self.mapping.buttons.get(&code).cloned();
                self.transition(Source::Button(device, code), mapping, down)
            },
            InputEvent::Detached { device } => self.detach(device),
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
    use super::{InputEvent, InputMapping, InputRouter, LogicalAction};

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum Action {
        Confirm,
        Jump,
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
}
