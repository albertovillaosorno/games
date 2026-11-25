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
//   - SDL input admission and event timestamp reconstruction.
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
use std::time::Duration;

use games::input::InputEvent;
use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use sdl2::controller::GameController;
pub use sdl2::controller::{Axis, Button};
use sdl2::event::{Event, WindowEvent};
pub use sdl2::keyboard::Scancode;
use sdl2::{GameControllerSubsystem, Sdl, TimerSubsystem};

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
    origin: u64,
    subsystem: GameControllerSubsystem,
    timer: TimerSubsystem,
    touch_enabled: bool,
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

    /// Discover available controllers in the current joystick-index order.
    ///
    /// Call after routing a removal and draining the native event queue.
    /// Controllers already admitted retain their identity; newly admitted
    /// controllers produce attachment events for the host to route.
    ///
    /// # Errors
    /// Reports failed controller enumeration or device open.
    pub fn discover_available(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
        let mut attached = Vec::new();
        if self.controllers.len() == self.limit.get() {
            return Ok(attached);
        }
        let count = self
            .subsystem
            .num_joysticks()
            .map_err(RenderError::Backend)?;
        for index in 0..count {
            if self.controllers.len() == self.limit.get() {
                break;
            }
            if let Some(event) = self.attach(index)? {
                attached.push(event);
            }
        }
        Ok(attached)
    }

    /// Time since the input session began, using SDL's millisecond clock.
    ///
    /// Use successive elapsed samples to advance a simulation clock.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        Duration::from_millis(self.timer.ticks64().saturating_sub(self.origin))
    }

    /// Enable touch events for a host that owns the only SDL window.
    ///
    /// SDL2 finger events do not expose a window ID through this adapter.
    /// Coordinates are normalized over the entire surface, not the fitted
    /// game viewport. Configure a shared title touch layout separately.
    pub const fn enable_touch(&mut self) {
        self.touch_enabled = true;
    }

    /// Map an SDL event's timestamp onto the input session's elapsed time.
    ///
    /// Events queued before startup belong to time zero. Poll frequently:
    /// SDL timestamps are 32-bit milliseconds, so this reconstruction only
    /// accepts stamps within the preceding half of that counter's range.
    /// Returns None for a future or ambiguous timestamp.
    #[must_use]
    pub fn event_elapsed(&self, event: &Event) -> Option<Duration> {
        event_elapsed(self.origin, self.timer.ticks64(), event.get_timestamp())
    }

    pub(super) fn open(
        sdl: &Sdl,
        window: u32,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        let subsystem = sdl.game_controller().map_err(RenderError::Backend)?;
        let timer = sdl.timer().map_err(RenderError::Backend)?;
        let mut input = Self {
            controllers: BTreeMap::new(),
            limit,
            origin: timer.ticks64(),
            subsystem,
            timer,
            touch_enabled: false,
            window,
        };
        let attached = input.discover_available()?;
        Ok(InputStartup { attached, input })
    }

    /// Translate one polled native event, opening or closing devices as needed.
    ///
    /// Discovery uses the joystick index; subsequent button and removal
    /// events use the admitted controller's stable instance identifier.
    /// Devices above the explicit capacity are not opened. After removal,
    /// the host can call `discover_available` to fill vacant slots.
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
        if self.touch_enabled && event.is_finger() {
            return Ok(route_contact(event));
        }
        let routed = route(event, self.window);
        if let Some(HostEvent::Input(
            InputEvent::Axis { device, .. } | InputEvent::Button { device, .. },
        )) = routed
            && !self.controllers.contains_key(&device)
        {
            return Ok(None);
        }
        Ok(routed)
    }
}

fn event_elapsed(origin: u64, now: u64, stamp: u32) -> Option<Duration> {
    if now < origin {
        return None;
    }
    let low = u32::try_from(now & u64::from(u32::MAX)).ok()?;
    let age = low.wrapping_sub(stamp);
    if age > 0x7fff_ffff {
        return None;
    }
    let absolute = now.checked_sub(u64::from(age))?;
    Some(Duration::from_millis(absolute.saturating_sub(origin)))
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

/// SDL controller axis code for use in a title's shared axis mapping.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "SDL controller axis C ABI discriminants are within u16"
)]
pub const fn axis_code(axis: Axis) -> u16 {
    axis as u16
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

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "finite normalized coordinates are checked before conversion"
)]
fn normalized_coordinate(value: f32) -> Option<u16> {
    if !(0.0..=1.0).contains(&value) {
        return None;
    }
    Some((value * f32::from(u16::MAX)).round() as u16)
}

fn normalized_point(horizontal: f32, vertical: f32) -> Option<TouchPoint> {
    Some(TouchPoint {
        x: normalized_coordinate(horizontal)?,
        y: normalized_coordinate(vertical)?,
    })
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
    if let Event::ControllerAxisMotion { which, axis, value, .. } = *event {
        return Some(HostEvent::Input(InputEvent::Axis {
            code: axis_code(axis),
            device: u64::from(which),
            value,
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

fn route_contact(event: &Event) -> Option<HostEvent> {
    let (device, finger, phase) = if let Event::FingerDown {
        touch_id,
        finger_id,
        x,
        y,
        ..
    } = *event
    {
        (
            touch_id,
            finger_id,
            ContactPhase::Down(normalized_point(x, y)?),
        )
    } else if let Event::FingerMotion {
        touch_id,
        finger_id,
        x,
        y,
        ..
    } = *event
    {
        (
            touch_id,
            finger_id,
            ContactPhase::Motion(normalized_point(x, y)),
        )
    } else if let Event::FingerUp { touch_id, finger_id, .. } = *event {
        (touch_id, finger_id, ContactPhase::Up)
    } else {
        return None;
    };
    Some(HostEvent::Input(InputEvent::Contact {
        id: ContactId { device, finger },
        phase,
    }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use games::input::touch::{ContactId, ContactPhase, TouchPoint};
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use sdl2::event::{Event, WindowEvent};
    use sdl2::keyboard::Mod;

    use super::{
        Axis, Button, HostEvent, Scancode, axis_code, button_code,
        event_elapsed, key_code, route, route_contact,
    };

    #[test]
    fn native_touch_coordinates_are_checked_and_rounded() {
        for (horizontal, vertical, expected) in [
            (0.0, 1.0, Some(TouchPoint { x: 0, y: u16::MAX })),
            (0.5, 0.5, Some(TouchPoint { x: 32_768, y: 32_768 })),
            (-0.1, 0.5, None),
            (1.1, 0.5, None),
            (0.5, f32::NAN, None),
            (f32::INFINITY, 0.5, None),
        ] {
            let event = Event::FingerDown {
                timestamp: 0,
                touch_id: i64::MIN,
                finger_id: i64::MAX,
                x: horizontal,
                y: vertical,
                dx: 0.0,
                dy: 0.0,
                pressure: 1.0,
            };
            assert_eq!(
                route_contact(&event),
                expected.map(|point| HostEvent::Input(InputEvent::Contact {
                    id: ContactId {
                        device: i64::MIN,
                        finger: i64::MAX
                    },
                    phase: ContactPhase::Down(point),
                }))
            );
            assert_eq!(route(&event, 1), None);
        }
    }

    #[test]
    fn invalid_motion_and_release_still_reach_the_contact_router() {
        let motion = Event::FingerMotion {
            timestamp: 0,
            touch_id: 2,
            finger_id: 3,
            x: f32::NAN,
            y: -1.0,
            dx: 0.0,
            dy: 0.0,
            pressure: 1.0,
        };
        let up = Event::FingerUp {
            timestamp: 0,
            touch_id: 2,
            finger_id: 3,
            x: f32::NAN,
            y: -1.0,
            dx: 0.0,
            dy: 0.0,
            pressure: 0.0,
        };
        for (event, phase) in
            [(motion, ContactPhase::Motion(None)), (up, ContactPhase::Up)]
        {
            assert_eq!(
                route_contact(&event),
                Some(HostEvent::Input(InputEvent::Contact {
                    id: ContactId { device: 2, finger: 3 },
                    phase,
                }))
            );
        }
    }

    #[test]
    fn controller_axes_preserve_signed_values_and_instance_identity() {
        for value in [i16::MIN, -1, 0, 1, i16::MAX] {
            let event = Event::ControllerAxisMotion {
                timestamp: 0,
                which: u32::MAX,
                axis: Axis::LeftX,
                value,
            };
            assert_eq!(
                route(&event, 1),
                Some(HostEvent::Input(InputEvent::Axis {
                    code: axis_code(Axis::LeftX),
                    device: u64::from(u32::MAX),
                    value,
                }))
            );
        }
        assert_ne!(axis_code(Axis::LeftX), axis_code(Axis::LeftY));
    }

    #[test]
    fn event_time_reconstructs_wrap_and_pre_start_events() {
        let wrap = u64::from(u32::MAX).saturating_add(1);
        let origin = wrap.saturating_sub(10);
        let now = wrap.saturating_add(8);
        for (stamp, millis) in [
            (u32::MAX, 9),
            (0, 10),
            (8, 18),
            (u32::MAX.saturating_sub(20), 0),
        ] {
            assert_eq!(
                event_elapsed(origin, now, stamp),
                Some(Duration::from_millis(millis))
            );
        }
        assert_eq!(event_elapsed(origin, now, 9), None);
        assert_eq!(event_elapsed(10, 9, 9), None);
        assert_eq!(event_elapsed(0, 3, u32::MAX), None);
    }

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
