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
//   - SDL keyboard and bounded native controller event admission.
// - Must-Not:
//   - Assign players, define title actions or advance simulation.
// - Allows:
//   - Inputs: SDL events and an explicit native controller capacity.
//   - Outputs: Platform-neutral input transitions and close requests.
//   - Side effects: Opens and closes SDL controller handles.
// - Split-When:
//   - Additional native device families require distinct lifetimes.
// - Merge-When:
//   - Another SDL adapter duplicates controller ownership.
// - Summary:
//   - Translates window and controller events into shared engine input.
// - Description:
//   - Keeps transient discovery indices separate from instance identifiers.
// - Usage:
//   - Open input before polling events and route each returned transition.
// - Defaults:
//   - Ignore repeat keys, unknown controllers and unhandled event types.

//! Native SDL input for the shared action router.

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::num::NonZeroUsize;

use games::input::InputEvent;
pub use sdl2::controller::Button;
use sdl2::controller::GameController;
use sdl2::event::{Event, WindowEvent};
pub use sdl2::keyboard::Scancode;
use sdl2::{GameControllerSubsystem, Sdl};

use super::RenderError;

/// Events a host can forward to its action router or application loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEvent {
    /// A platform-neutral input transition.
    Input(InputEvent),
    /// The application or its window received a close request.
    Quit,
}

/// Owns the opened native controllers for one host window.
pub struct NativeInput {
    controllers: BTreeMap<u64, GameController>,
    limit: NonZeroUsize,
    subsystem: GameControllerSubsystem,
    window: u32,
}

/// Input adapter and initial controller attachments for the shared router.
#[derive(Debug)]
pub struct InputStartup {
    /// Initial attachment events, in discovery order.
    pub attached: Vec<InputEvent>,
    /// Adapter retaining the native controller handles.
    pub input: NativeInput,
}

impl Debug for NativeInput {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInput")
            .field("controllers", &self.controllers.len())
            .field("limit", &self.limit)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl NativeInput {
    fn attach(
        &mut self,
        index: u32,
    ) -> Result<Option<InputEvent>, RenderError> {
        if self.controllers.len() >= self.limit.get()
            || !self.subsystem.is_game_controller(index)
        {
            return Ok(None);
        }
        let controller = self
            .subsystem
            .open(index)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let device = u64::from(controller.instance_id());
        if self.controllers.contains_key(&device) {
            return Ok(None);
        }
        let _old = self.controllers.insert(device, controller);
        Ok(Some(InputEvent::Attached { device }))
    }

    pub(super) fn open(
        sdl: &Sdl,
        window: u32,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        let subsystem = sdl.game_controller().map_err(RenderError::Backend)?;
        let count = subsystem.num_joysticks().map_err(RenderError::Backend)?;
        let mut input = Self {
            controllers: BTreeMap::new(),
            limit,
            subsystem,
            window,
        };
        let mut attached = Vec::new();
        for index in 0..count {
            if input.controllers.len() == limit.get() {
                break;
            }
            if let Some(event) = input.attach(index)? {
                attached.push(event);
            }
        }
        Ok(InputStartup { attached, input })
    }

    /// Translate one polled native event, opening or closing devices as needed.
    ///
    /// Discovery uses the joystick index; subsequent button and removal
    /// events use the admitted controller's stable instance identifier.
    /// Devices above the explicit capacity are not opened.
    ///
    /// # Errors
    /// Reports failure to open a newly reported native controller.
    pub fn translate(
        &mut self,
        event: &Event,
    ) -> Result<Option<HostEvent>, RenderError> {
        if let Event::ControllerDeviceAdded { which, .. } = *event {
            return Ok(self.attach(which)?.map(HostEvent::Input));
        }
        if let Event::ControllerDeviceRemoved { which, .. } = *event {
            let device = u64::from(which);
            return Ok(self.controllers.remove(&device).map(|_controller| {
                HostEvent::Input(InputEvent::Detached { device })
            }));
        }
        let routed = route(event, self.window);
        if let Some(HostEvent::Input(InputEvent::Button { device, .. })) =
            routed
            && !self.controllers.contains_key(&device)
        {
            return Ok(None);
        }
        Ok(routed)
    }
}

/// SDL physical scancode for use in a title's shared key mapping.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "SDL scancodes are nonnegative C ABI enum discriminants"
)]
pub const fn key_code(scancode: Scancode) -> u32 {
    scancode as u32
}

/// SDL controller button code for use in a title's shared button mapping.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "SDL controller button C ABI discriminants are within u16"
)]
pub const fn button_code(button: Button) -> u16 {
    button as u16
}

fn route(event: &Event, window: u32) -> Option<HostEvent> {
    if matches!(event, Event::Quit { .. }) {
        return Some(HostEvent::Quit);
    }
    if let Event::Window { window_id, win_event, .. } = *event
        && window_id == window
    {
        if win_event == WindowEvent::Close {
            return Some(HostEvent::Quit);
        }
        if win_event == WindowEvent::FocusLost {
            return Some(HostEvent::Input(InputEvent::FocusLost));
        }
    }
    if let Event::KeyDown {
        window_id,
        scancode: Some(code),
        repeat: false,
        ..
    } = *event
        && window_id == window
    {
        return Some(HostEvent::Input(InputEvent::Key {
            code: key_code(code),
            down: true,
        }));
    }
    if let Event::KeyUp {
        window_id,
        scancode: Some(code),
        ..
    } = *event
        && window_id == window
    {
        return Some(HostEvent::Input(InputEvent::Key {
            code: key_code(code),
            down: false,
        }));
    }
    let (which, button, down) =
        if let Event::ControllerButtonDown { which, button, .. } = *event {
            (which, button, true)
        } else if let Event::ControllerButtonUp { which, button, .. } = *event {
            (which, button, false)
        } else {
            return None;
        };
    Some(HostEvent::Input(InputEvent::Button {
        device: u64::from(which),
        code: button_code(button),
        down,
    }))
}

#[cfg(test)]
mod tests {
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use sdl2::event::{Event, WindowEvent};
    use sdl2::keyboard::Mod;

    use super::{Button, HostEvent, Scancode, button_code, key_code, route};

    fn key(window_id: u32, repeat: bool) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id,
            keycode: None,
            scancode: Some(Scancode::Space),
            keymod: Mod::NOMOD,
            repeat,
        }
    }

    #[test]
    fn physical_keys_route_only_for_the_selected_window() {
        let mut mapping = InputMapping::new();
        mapping.bind_key(key_code(Scancode::Space), "jump");
        let mut router = InputRouter::new(mapping);
        assert_eq!(route(&key(3, false), 2), None);
        assert_eq!(route(&key(2, true), 2), None);
        let first = route(&key(2, false), 2);
        assert_eq!(
            first,
            Some(HostEvent::Input(InputEvent::Key {
                code: key_code(Scancode::Space),
                down: true,
            }))
        );
        if let Some(HostEvent::Input(event)) = first {
            assert_eq!(router.handle(event), [LogicalAction {
                action: "jump",
                down: true,
            }]);
        }
        let lost = Event::Window {
            timestamp: 0,
            window_id: 2,
            win_event: WindowEvent::FocusLost,
        };
        assert_eq!(route(&lost, 3), None);
        let released = route(&lost, 2);
        assert_eq!(released, Some(HostEvent::Input(InputEvent::FocusLost)));
        if let Some(HostEvent::Input(event)) = released {
            assert_eq!(router.handle(event), [LogicalAction {
                action: "jump",
                down: false,
            }]);
        }
    }

    #[test]
    fn controller_codes_preserve_instance_identity() {
        let down = Event::ControllerButtonDown {
            timestamp: 0,
            which: u32::MAX,
            button: Button::DPadLeft,
        };
        let up = Event::ControllerButtonUp {
            timestamp: 0,
            which: u32::MAX,
            button: Button::DPadLeft,
        };
        assert_eq!(
            route(&down, 1),
            Some(HostEvent::Input(InputEvent::Button {
                device: u64::from(u32::MAX),
                code: button_code(Button::DPadLeft),
                down: true,
            }))
        );
        assert_eq!(
            route(&up, 1),
            Some(HostEvent::Input(InputEvent::Button {
                device: u64::from(u32::MAX),
                code: button_code(Button::DPadLeft),
                down: false,
            }))
        );
    }

    #[test]
    fn close_requests_do_not_become_gameplay_actions() {
        let close = Event::Window {
            timestamp: 0,
            window_id: 4,
            win_event: WindowEvent::Close,
        };
        assert_eq!(route(&close, 3), None);
        assert_eq!(route(&close, 4), Some(HostEvent::Quit));
        assert_eq!(
            route(&Event::Quit { timestamp: 0 }, 3),
            Some(HostEvent::Quit)
        );
    }
}
