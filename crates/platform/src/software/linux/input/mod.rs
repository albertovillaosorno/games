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
//   - SDL input admission, timestamps and controller feedback.
// - Must-Not:
//   - Assign players, define title actions or advance simulation.
// - Allows:
//   - Inputs: SDL events, presented geometry and native controller capacity.
//   - Outputs: Platform-neutral input and feedback results.
//   - Side effects: Opens controllers and submits bounded rumble.
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
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

use games::capability::{HapticEffect, HapticError, HapticSupport};
use games::input::InputEvent;
use games::input::physical::{
    ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
};
use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Scancode;
use sdl2::mouse::MouseButton;
use sdl2::sensor::SensorType;
use sdl2::video::Window;
use sdl2::{GameControllerSubsystem, Sdl, TimerSubsystem};

use crate::ir::frame::RenderError;
use crate::ir::hardware::sensor::{MotionError, MotionSensor};
use crate::ir::hardware::{HostEvent, InputSample, InputTransition};
use crate::ir::input::{InputBackend, InputPort};
use crate::ir::security::haptics::FeedbackRequest;

/// Owns the opened native controllers for one host window.
struct LinuxInput {
    controllers: BTreeMap<u64, GameController>,
    initial_feedback_allowed: bool,
    limit: NonZeroUsize,
    rediscover: bool,
    subsystem: GameControllerSubsystem,
    timer: TimerSubsystem,
    window: u32,
}

impl Debug for LinuxInput {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxInput")
            .field("controllers", &self.controllers.len())
            .field("limit", &self.limit)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl InputBackend for LinuxInput {
    fn clock(&self) -> Result<Duration, RenderError> {
        Ok(Duration::from_millis(self.timer.ticks64()))
    }

    /// Discover controllers after a native poll returns no event.
    ///
    /// A device change in an earlier limited batch remains pending until the
    /// caller has reached a no-event poll. Re-enumeration is
    /// unnecessary unless device availability may have changed.
    /// # Errors
    /// Reports failed controller enumeration or device open.
    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError> {
        if self.rediscover {
            self.discover_available()
        } else {
            Ok(Vec::new())
        }
    }

    /// Discover available controllers in the current joystick-index order.
    ///
    /// Call after device changes and draining the native event queue.
    /// Controllers already admitted retain their identity; newly admitted
    /// controllers produce attachment events for the host to route. A device
    /// that disappears during opening schedules another post-drain discovery.
    ///
    /// # Errors
    /// Reports failed enumeration or opening a still-available device. On
    /// failure, no new controllers or partial attachment batch are admitted.
    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError> {
        if self.controllers.len() == self.limit.get() {
            self.rediscover = false;
            return Ok(Vec::new());
        }
        let count = self
            .subsystem
            .num_joysticks()
            .map_err(RenderError::Backend)?;
        let subsystem = &self.subsystem;
        let mut retry = false;
        let attached = admit_discovery(
            &mut self.controllers,
            self.limit,
            count,
            |index| discover_controller(subsystem, index, &mut retry),
        )?;
        self.rediscover = retry && self.controllers.len() < self.limit.get();
        Ok(attached)
    }

    fn feedback_support(&self, device: u64) -> Option<HapticSupport> {
        self.controllers.get(&device).and_then(controller_support)
    }

    fn initial_feedback_allowed(&self) -> bool {
        self.initial_feedback_allowed
    }

    fn read_motion(
        &mut self,
        device: u64,
        sensor: MotionSensor,
    ) -> Result<[f32; 3], MotionError> {
        let controller = self
            .controllers
            .get(&device)
            .filter(|controller| controller.attached())
            .ok_or(MotionError::Disconnected)?;
        let native = sensor_type(sensor);
        if !controller.has_sensor(native) {
            return Err(MotionError::Unsupported);
        }
        controller
            .sensor_set_enabled(native, true)
            .map_err(|_error| MotionError::Unavailable)?;
        let mut values = [0.0; 3];
        let observation = controller.sensor_get_data(native, &mut values);
        controller
            .sensor_set_enabled(native, false)
            .map_err(|_error| MotionError::Unavailable)?;
        observation.map_err(|_error| MotionError::Unavailable)?;
        Ok(values)
    }

    fn submit_feedback(
        &mut self,
        request: FeedbackRequest,
    ) -> Result<(), HapticError> {
        let command = request.command();
        let controller = self
            .controllers
            .get_mut(&command.device)
            .ok_or(HapticError::Disconnected)?;
        deliver_feedback(command.effect, |low, high, milliseconds| {
            controller
                .set_rumble(low, high, milliseconds)
                .map_err(|_error| HapticError::Unavailable)
        })
    }

    /// Translate one polled native event, recording device changes.
    ///
    /// Add notifications are discovery hints, not stable device identities.
    /// Drain queued removals before enumerating current joystick indices.
    /// Button, remap and removal events use admitted instance identifiers.
    /// Only devices within the explicit capacity may be opened.
    fn translate(&mut self, event: &InputSample) -> Option<HostEvent> {
        if !event.matches_window(u64::from(self.window)) {
            return None;
        }
        let routed = match event.transition() {
            InputTransition::DiscoveryHint => {
                self.rediscover |= self.controllers.len() < self.limit.get();
                return None;
            },
            InputTransition::Disconnected(device) => {
                let removed = self.controllers.remove(&device);
                self.rediscover |= self.controllers.len() < self.limit.get();
                return removed.map(|mut controller| {
                    stop_feedback(&mut controller);
                    HostEvent::Input(InputEvent::Detached { device })
                });
            },
            InputTransition::FocusGained | InputTransition::Ignored => {
                return None;
            },
            InputTransition::Event(routed) => Some(routed),
        };
        if matches!(
            routed,
            Some(
                HostEvent::Quit
                    | HostEvent::Input(
                        InputEvent::ApplicationSuspended
                            | InputEvent::FocusLost
                    )
            )
        ) {
            self.stop_all_feedback();
        }
        if let Some(HostEvent::Input(
            InputEvent::Axis { device, .. }
            | InputEvent::Button { device, .. }
            | InputEvent::ControllerRemapped { device },
        )) = routed
            && !self.controllers.contains_key(&device)
        {
            return None;
        }
        routed
    }

    fn window_scope(&self) -> u64 {
        u64::from(self.window)
    }
}

impl LinuxInput {
    fn open(
        sdl: &Sdl,
        window: &Window,
        limit: NonZeroUsize,
    ) -> Result<InputPort, RenderError> {
        let subsystem = sdl.game_controller().map_err(RenderError::Backend)?;
        let timer = sdl.timer().map_err(RenderError::Backend)?;
        let input = Self {
            controllers: BTreeMap::new(),
            initial_feedback_allowed: window.has_input_focus()
                && !window.is_minimized(),
            limit,
            rediscover: false,
            subsystem,
            timer,
            window: window.id(),
        };
        Ok(Box::new(input))
    }

    fn stop_all_feedback(&mut self) {
        for controller in self.controllers.values_mut() {
            stop_feedback(controller);
        }
    }
}

impl Drop for LinuxInput {
    fn drop(&mut self) {
        self.stop_all_feedback();
    }
}

type Discovered<Handle> = Result<Option<(u64, Handle)>, RenderError>;

fn controller_support(controller: &GameController) -> Option<HapticSupport> {
    if !controller.attached() {
        return None;
    }
    // SDL reports standard two-channel rumble support, not the physical
    // actuator count. The backend may combine the channels itself.
    Some(if controller.has_rumble() {
        HapticSupport::DualMotor
    } else {
        HapticSupport::Unsupported
    })
}

fn deliver_feedback(
    effect: HapticEffect,
    deliver: impl FnOnce(u16, u16, u32) -> Result<(), HapticError>,
) -> Result<(), HapticError> {
    match effect {
        HapticEffect::Stop => deliver(0, 0, 0),
        HapticEffect::Rumble { low, high, milliseconds } => {
            deliver(low, high, NonZeroU32::from(milliseconds).get())
        },
    }
}

const fn sensor_type(sensor: MotionSensor) -> SensorType {
    match sensor {
        MotionSensor::Accelerometer => SensorType::Accelerometer,
        MotionSensor::Gyroscope => SensorType::Gyroscope,
        MotionSensor::LeftAccelerometer => SensorType::LeftAccelerometer,
        MotionSensor::LeftGyroscope => SensorType::LeftGyroscope,
        MotionSensor::RightAccelerometer => SensorType::RightAccelerometer,
        MotionSensor::RightGyroscope => SensorType::RightGyroscope,
    }
}

fn stop_feedback(controller: &mut GameController) {
    if controller.attached() && controller.has_rumble() {
        // Loss of the native device can race with cleanup. Effects have
        // finite duration even if a stop fails or the device disappears.
        let _result = controller.set_rumble(0, 0, 0);
    }
    for sensor in MotionSensor::ALL {
        let native = sensor_type(sensor);
        if controller.sensor_enabled(native) {
            let _stopped = controller.sensor_set_enabled(native, false);
        }
    }
}

fn discover_controller(
    subsystem: &GameControllerSubsystem,
    index: u32,
    retry: &mut bool,
) -> Discovered<GameController> {
    let Some(controller) = open_candidate(
        || subsystem.is_game_controller(index),
        || {
            subsystem
                .open(index)
                .map_err(|error| RenderError::Backend(error.to_string()))
        },
        retry,
    )?
    else {
        return Ok(None);
    };
    if !controller.attached() {
        *retry = true;
        return Ok(None);
    }
    let device = u64::from(controller.instance_id());
    Ok(Some((device, controller)))
}

// SDL locks each enumeration operation separately. An index can disappear

// after its mapping was checked; do not treat that observed race as a fatal
// backend error or interpret the old index as a stable device identity.
fn open_candidate<Handle>(
    mut available: impl FnMut() -> bool,
    open: impl FnOnce() -> Result<Handle, RenderError>,
    retry: &mut bool,
) -> Result<Option<Handle>, RenderError> {
    if !available() {
        return Ok(None);
    }
    match open() {
        Ok(handle) => Ok(Some(handle)),
        Err(_) if !available() => {
            *retry = true;
            Ok(None)
        },
        Err(error) => Err(error),
    }
}

// Stage handles in discovery order. A later backend error must not retain
// an earlier handle whose attachment notification could not reach the host.
fn admit_discovery<Handle, Candidate>(
    controllers: &mut BTreeMap<u64, Handle>,
    limit: NonZeroUsize,
    count: u32,
    mut candidate: Candidate,
) -> Result<Vec<InputEvent>, RenderError>
where
    Candidate: FnMut(u32) -> Discovered<Handle>,
{
    let mut staged = BTreeMap::new();
    let mut attached = Vec::new();
    for index in 0..count {
        if controllers.len().saturating_add(staged.len()) >= limit.get() {
            break;
        }
        if let Some((device, handle)) = candidate(index)?
            && !controllers.contains_key(&device)
            && !staged.contains_key(&device)
        {
            let _old = staged.insert(device, handle);
            attached.push(InputEvent::Attached { device });
        }
    }
    controllers.append(&mut staged);
    Ok(attached)
}

fn absolute_event_time(now: u64, stamp: u32) -> Option<Duration> {
    let low = u32::try_from(now & u64::from(u32::MAX)).ok()?;
    let age = low.wrapping_sub(stamp);
    if age > 0x7fff_ffff {
        return None;
    }
    let absolute = now.checked_sub(u64::from(age))?;
    Some(Duration::from_millis(absolute))
}

/// Resolve a legacy keyboard profile name at the native boundary.
#[must_use]
pub(in crate::software) fn key_from_name(name: &str) -> Option<KeyboardKey> {
    Scancode::from_name(name).map(|key| KeyboardKey::from_code(key_code(key)))
}

/// Native presentation name for a translated physical key.
#[must_use]
pub(in crate::software) fn key_name(key: KeyboardKey) -> &'static str {
    i32::try_from(key.code())
        .ok()
        .and_then(Scancode::from_i32)
        .map_or("UNKNOWN", Scancode::name)
}

/// Translate a native keyboard identity for the shared router.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "SDL scancodes are nonnegative C ABI enum discriminants"
)]
const fn key_code(scancode: Scancode) -> u32 {
    scancode as u32
}

const fn axis_code(axis: Axis) -> u16 {
    match axis {
        Axis::LeftX => ControllerAxis::LeftX,
        Axis::LeftY => ControllerAxis::LeftY,
        Axis::RightX => ControllerAxis::RightX,
        Axis::RightY => ControllerAxis::RightY,
        Axis::TriggerLeft => ControllerAxis::TriggerLeft,
        Axis::TriggerRight => ControllerAxis::TriggerRight,
    }
    .code()
}

const fn button_code(button: Button) -> u16 {
    match button {
        Button::A => ControllerButton::South,
        Button::B => ControllerButton::East,
        Button::X => ControllerButton::West,
        Button::Y => ControllerButton::North,
        Button::Back => ControllerButton::Back,
        Button::Guide => ControllerButton::Guide,
        Button::Start => ControllerButton::Start,
        Button::LeftStick => ControllerButton::LeftStick,
        Button::RightStick => ControllerButton::RightStick,
        Button::LeftShoulder => ControllerButton::LeftShoulder,
        Button::RightShoulder => ControllerButton::RightShoulder,
        Button::DPadUp => ControllerButton::DPadUp,
        Button::DPadDown => ControllerButton::DPadDown,
        Button::DPadLeft => ControllerButton::DPadLeft,
        Button::DPadRight => ControllerButton::DPadRight,
        Button::Misc1 => ControllerButton::Misc1,
        Button::Paddle1 => ControllerButton::Paddle1,
        Button::Paddle2 => ControllerButton::Paddle2,
        Button::Paddle3 => ControllerButton::Paddle3,
        Button::Paddle4 => ControllerButton::Paddle4,
        Button::Touchpad => ControllerButton::Touchpad,
    }
    .code()
}

const fn mouse_code(button: MouseButton) -> Option<u8> {
    Some(
        match button {
            MouseButton::Left => PointerButton::Left,
            MouseButton::Middle => PointerButton::Middle,
            MouseButton::Right => PointerButton::Right,
            MouseButton::X1 => PointerButton::X1,
            MouseButton::X2 => PointerButton::X2,
            MouseButton::Unknown => return None,
        }
        .code(),
    )
}

pub(super) fn sample(event: &Event, window: u32, now: u64) -> InputSample {
    let transition = if matches!(event, Event::ControllerDeviceAdded { .. }) {
        InputTransition::DiscoveryHint
    } else if let Event::ControllerDeviceRemoved { which, .. } = *event {
        InputTransition::Disconnected(u64::from(which))
    } else if let Event::Window {
        window_id,
        win_event: WindowEvent::FocusGained,
        ..
    } = *event
        && window_id == window
    {
        InputTransition::FocusGained
    } else {
        route_contact(event)
            .or_else(|| route(event, window))
            .map_or(InputTransition::Ignored, InputTransition::Event)
    };
    let scope = event.get_window_id().map(u64::from);
    InputSample::new(
        absolute_event_time(now, event.get_timestamp()),
        transition,
        scope,
    )
}

const fn route_application(event: &Event) -> Option<HostEvent> {
    if matches!(event, Event::Quit { .. } | Event::AppTerminating { .. }) {
        return Some(HostEvent::Quit);
    }
    // Either background boundary can replace window focus loss. Repeated
    // releases are idempotent. Foreground alone does not grant focus.
    if matches!(
        event,
        Event::AppWillEnterBackground { .. }
            | Event::AppDidEnterBackground { .. }
    ) {
        return Some(HostEvent::Input(InputEvent::ApplicationSuspended));
    }
    None
}

fn route(event: &Event, window: u32) -> Option<HostEvent> {
    if let Some(lifecycle) = route_application(event) {
        return Some(lifecycle);
    }
    if let Event::Window { window_id, win_event, .. } = *event
        && window_id == window
    {
        if win_event == WindowEvent::Close {
            return Some(HostEvent::Quit);
        }
        if matches!(
            win_event,
            WindowEvent::FocusLost
                | WindowEvent::Hidden
                | WindowEvent::Minimized
        ) {
            return Some(HostEvent::Input(InputEvent::FocusLost));
        }
        // Capture is not guaranteed across native SDL backends. A pointer
        // leaving the window may never deliver its eventual button release.
        if win_event == WindowEvent::Leave {
            return Some(HostEvent::Input(InputEvent::MouseCancelled));
        }
        if matches!(
            win_event,
            WindowEvent::Resized(..)
                | WindowEvent::SizeChanged(..)
                | WindowEvent::DisplayChanged(..)
        ) {
            return Some(HostEvent::Input(InputEvent::TouchCancelled));
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
    route_mouse(event, window).or_else(|| route_controller(event))
}

fn route_mouse(event: &Event, window: u32) -> Option<HostEvent> {
    let (window_id, which, button, down) = if let Event::MouseButtonDown {
        window_id,
        which,
        mouse_btn,
        ..
    } = *event
    {
        (window_id, which, mouse_btn, true)
    } else if let Event::MouseButtonUp {
        window_id,
        which,
        mouse_btn,
        ..
    } = *event
    {
        (window_id, which, mouse_btn, false)
    } else {
        return None;
    };
    // SDL_TOUCH_MOUSEID is not a physical mouse. Avoid double routing.
    if window_id != window || which == u32::MAX {
        return None;
    }
    Some(HostEvent::Input(InputEvent::Mouse {
        code: mouse_code(button)?,
        down,
    }))
}

fn route_controller(event: &Event) -> Option<HostEvent> {
    if let Event::ControllerDeviceRemapped { which, .. } = *event {
        return Some(HostEvent::Input(InputEvent::ControllerRemapped {
            device: u64::from(which),
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
            ContactPhase::Down(TouchPoint::from_normalized(
                f64::from(x),
                f64::from(y),
            )?),
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
            ContactPhase::Motion(TouchPoint::from_normalized(
                f64::from(x),
                f64::from(y),
            )),
        )
    } else if let Event::FingerUp { touch_id, finger_id, .. } = *event {
        (touch_id, finger_id, ContactPhase::Up)
    } else {
        return None;
    };
    // SDL_MOUSE_TOUCHID denotes a mouse-generated contact, not a touch
    // device. Keep physical mouse bindings independent of touch controls.
    if device == -1 {
        return None;
    }
    Some(HostEvent::Input(InputEvent::Contact {
        id: ContactId { device, finger },
        phase,
    }))
}

pub(super) fn open_input(
    sdl: &Sdl,
    window: &Window,
    limit: NonZeroUsize,
) -> Result<InputPort, RenderError> {
    LinuxInput::open(sdl, window, limit)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::{NonZeroU16, NonZeroUsize};
    use std::time::Duration;

    use games::capability::{HapticEffect, HapticError};
    use games::input::physical::{
        ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
    };
    use games::input::touch::{ContactId, ContactPhase, TouchPoint};
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use sdl2::event::{Event, WindowEvent};
    use sdl2::keyboard::Mod;
    use sdl2::mouse::MouseButton;

    use super::{
        Axis, Button, HostEvent, Scancode, absolute_event_time,
        admit_discovery, axis_code, button_code, deliver_feedback, key_code,
        key_from_name, key_name, mouse_code, open_candidate, route,
        route_contact, sample,
    };
    use crate::ir::frame::RenderError;
    use crate::ir::hardware::InputTransition;
    use crate::ir::input::{
        key_from_name as portable_key_from_name, key_name as portable_key_name,
    };

    #[test]
    fn keyboard_profile_names_preserve_routing_identities() {
        for (name, expected) in [
            ("A", KeyboardKey::LETTER_A),
            ("Space", KeyboardKey::SPACE),
            ("Return", KeyboardKey::ENTER),
            ("F2", KeyboardKey::F2),
            ("6", KeyboardKey::NUM_6),
            ("Keypad 1", KeyboardKey::KP_1),
            ("Left", KeyboardKey::LEFT),
        ] {
            assert_eq!(key_from_name(name), Some(expected));
            assert_eq!(key_name(expected), name);
        }
        assert_eq!(key_from_name("not-a-key"), None);
        assert_eq!(key_name(KeyboardKey::from_code(u32::MAX)), "UNKNOWN");
    }

    #[test]
    fn common_portable_keyboard_names_match_existing_native_profiles() {
        for code in (4..=99).chain(101..=129).chain(224..=231) {
            let key = KeyboardKey::from_code(code);
            assert_eq!(key.profile_name(), Some(key_name(key)));
            if let Some(name) = key.profile_name() {
                assert_eq!(key_from_name(name), Some(key));
                assert_eq!(portable_key_from_name(name), Some(key));
                assert_eq!(portable_key_name(key), name);
            }
        }
    }

    #[test]
    fn extended_portable_keyboard_names_match_native_codes() {
        let extended = KeyboardKey::from_code(133);
        assert_eq!(extended.profile_name(), Some("Keypad ,"));
        assert_eq!(portable_key_from_name("Keypad ,"), Some(extended));
        assert_eq!(portable_key_name(extended), "Keypad ,");
        let international = KeyboardKey::from_code(100);
        assert_eq!(key_code(Scancode::NonUsBackslash), international.code());
        assert_eq!(
            portable_key_from_name("IntlBackslash"),
            Some(international)
        );
        assert_eq!(portable_key_name(international), "IntlBackslash");
    }

    #[test]
    fn native_control_names_translate_to_shared_codes() {
        for axis in ControllerAxis::ALL {
            assert_eq!(
                Axis::from_string(axis.profile_name()).map(axis_code),
                Some(axis.code())
            );
        }
        for button in ControllerButton::ALL {
            assert_eq!(
                Button::from_string(button.profile_name()).map(button_code),
                Some(button.code())
            );
        }
        for (native, shared) in [
            (MouseButton::Left, PointerButton::Left),
            (MouseButton::Middle, PointerButton::Middle),
            (MouseButton::Right, PointerButton::Right),
            (MouseButton::X1, PointerButton::X1),
            (MouseButton::X2, PointerButton::X2),
        ] {
            assert_eq!(mouse_code(native), Some(shared.code()));
        }
        assert_eq!(mouse_code(MouseButton::Unknown), None);
    }

    #[test]
    fn feedback_encodes_bounded_effects_and_stops() {
        let mut deliveries = Vec::new();
        let mut send = |effect| {
            deliver_feedback(effect, |low, high, time| {
                deliveries.push((low, high, time));
                Ok(())
            })
        };
        let effect = HapticEffect::Rumble {
            low: u16::MAX,
            high: 7,
            milliseconds: NonZeroU16::MAX,
        };
        assert_eq!(send(effect), Ok(()));
        assert_eq!(send(effect.single_motor()), Ok(()));
        assert_eq!(send(HapticEffect::Stop), Ok(()));
        assert_eq!(deliveries, [
            (u16::MAX, 7, 65_535),
            (u16::MAX, u16::MAX, 65_535),
            (0, 0, 0)
        ]);
        assert_eq!(
            deliver_feedback(effect, |_, _, _| {
                Err(HapticError::Unavailable)
            }),
            Err(HapticError::Unavailable)
        );
    }

    #[test]
    fn mouse_edges_only_route_for_physical_buttons_in_own_window() {
        let pressed = |window_id, which, mouse_btn| Event::MouseButtonDown {
            timestamp: 0,
            window_id,
            which,
            mouse_btn,
            clicks: 1,
            x: 10i32,
            y: 20i32,
        };
        let input = |down| {
            Some(HostEvent::Input(InputEvent::Mouse {
                code: PointerButton::Left.code(),
                down,
            }))
        };
        assert_eq!(route(&pressed(3, 0, MouseButton::Left), 3), input(true));
        assert_eq!(route(&pressed(4, 0, MouseButton::Left), 3), None);
        assert_eq!(route(&pressed(3, u32::MAX, MouseButton::Left), 3), None);
        assert_eq!(route(&pressed(3, 0, MouseButton::Unknown), 3), None);
        let released = Event::MouseButtonUp {
            timestamp: 1,
            window_id: 3,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 10i32,
            y: 20i32,
        };
        assert_eq!(route(&released, 3), input(false));
    }

    #[test]
    fn mouse_leave_cancels_only_the_owning_window() {
        let leave = Event::Window {
            timestamp: 0,
            window_id: 2,
            win_event: WindowEvent::Leave,
        };
        assert_eq!(route(&leave, 3), None);
        assert_eq!(
            route(&leave, 2),
            Some(HostEvent::Input(InputEvent::MouseCancelled))
        );
        let enter = Event::Window {
            timestamp: 1,
            window_id: 2,
            win_event: WindowEvent::Enter,
        };
        assert_eq!(route(&enter, 2), None);
    }

    #[test]
    fn discovery_distinguishes_a_vanished_index_from_a_backend_error() {
        let failure = || RenderError::Backend("open failed".to_owned());
        let mut retry = false;
        let mut checks = [true, false].into_iter();
        let vanished = open_candidate::<()>(
            || checks.next().unwrap_or(false),
            || Err(failure()),
            &mut retry,
        );
        assert!(matches!(vanished, Ok(None)));
        assert!(retry);
        retry = false;
        let present =
            open_candidate::<()>(|| true, || Err(failure()), &mut retry);
        assert!(matches!(present, Err(RenderError::Backend(_))));
        assert!(!retry);
        let unopened =
            open_candidate::<()>(|| false, || Err(failure()), &mut retry);
        assert!(matches!(unopened, Ok(None)));
        assert!(!retry);
    }

    #[test]
    fn discovery_failure_does_not_lose_attachment_notifications() {
        let mut controllers = BTreeMap::from([(10, "old")]);
        let limit = NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN);
        let failed = admit_discovery(&mut controllers, limit, 2, |index| {
            if index == 0 {
                Ok(Some((20, "new")))
            } else {
                Err(RenderError::Backend("open failed".to_owned()))
            }
        });
        assert!(matches!(failed, Err(RenderError::Backend(_))));
        assert_eq!(controllers, BTreeMap::from([(10, "old")]));
        let mut visited = Vec::new();
        let attached = admit_discovery(&mut controllers, limit, 5, |index| {
            visited.push(index);
            Ok(match index {
                0 => Some((10, "duplicate")),
                1 => Some((30, "first")),
                2 => Some((30, "duplicate")),
                _ => Some((20, "second")),
            })
        });
        assert!(matches!(attached, Ok(events) if events == [
            InputEvent::Attached { device: 30 },
            InputEvent::Attached { device: 20 },
        ]));
        assert_eq!(visited, [0, 1, 2, 3]);
        assert_eq!(
            controllers,
            BTreeMap::from([(10, "old"), (20, "second"), (30, "first"),])
        );
    }

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
    fn mouse_generated_contacts_are_not_admitted() {
        for event in [
            Event::FingerDown {
                timestamp: 1,
                touch_id: -1,
                finger_id: 0,
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: 0.0,
                pressure: 1.0,
            },
            Event::FingerMotion {
                timestamp: 2,
                touch_id: -1,
                finger_id: 0,
                x: 0.6,
                y: 0.5,
                dx: 0.1,
                dy: 0.0,
                pressure: 1.0,
            },
            Event::FingerUp {
                timestamp: 3,
                touch_id: -1,
                finger_id: 0,
                x: 0.6,
                y: 0.5,
                dx: 0.0,
                dy: 0.0,
                pressure: 0.0,
            },
        ] {
            assert_eq!(
                sample(&event, 1, 3).transition(),
                InputTransition::Ignored
            );
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
                    code: ControllerAxis::LeftX.code(),
                    device: u64::from(u32::MAX),
                    value,
                }))
            );
        }
        assert_ne!(axis_code(Axis::LeftX), axis_code(Axis::LeftY));
    }

    #[test]
    fn event_time_reconstructs_unambiguous_absolute_observations() {
        let wrap = u64::from(u32::MAX).saturating_add(1);
        let now = wrap.saturating_add(8);
        for (stamp, millis) in [
            (u32::MAX, wrap.saturating_sub(1)),
            (0, wrap),
            (8, now),
            (u32::MAX.saturating_sub(20), wrap.saturating_sub(21)),
        ] {
            assert_eq!(
                absolute_event_time(now, stamp),
                Some(Duration::from_millis(millis))
            );
        }
        assert_eq!(absolute_event_time(now, 9), None);
        assert_eq!(absolute_event_time(3, u32::MAX), None);
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
    fn observations_retain_the_actual_window_scope() {
        let foreign = sample(&key(3, false), 2, 0);
        assert!(!foreign.matches_window(2));
        assert!(foreign.matches_window(3));
        assert_eq!(foreign.transition(), InputTransition::Ignored);
        let local = sample(&key(2, false), 2, 0);
        assert!(local.matches_window(2));
        assert!(!local.matches_window(3));
        let quit = sample(&Event::Quit { timestamp: 0 }, 2, 0);
        assert!(quit.matches_window(2));
        assert!(quit.matches_window(3));
        assert_eq!(quit.transition(), InputTransition::Event(HostEvent::Quit));
    }

    #[test]
    fn physical_keys_route_only_for_the_selected_window() {
        let mut mapping = InputMapping::new();
        mapping.bind_key(KeyboardKey::SPACE.code(), "jump");
        let mut router = InputRouter::new(mapping);
        assert_eq!(route(&key(3, false), 2), None);
        assert_eq!(route(&key(2, true), 2), None);
        let first = route(&key(2, false), 2);
        assert_eq!(
            first,
            Some(HostEvent::Input(InputEvent::Key {
                code: KeyboardKey::SPACE.code(),
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
    fn resize_and_display_changes_cancel_only_selected_window_touch() {
        for kind in [
            WindowEvent::Resized(800, 600),
            WindowEvent::SizeChanged(800, 600),
            WindowEvent::DisplayChanged(1),
        ] {
            let event = Event::Window {
                timestamp: 0,
                window_id: 2,
                win_event: kind,
            };
            assert_eq!(route(&event, 3), None);
            assert_eq!(
                route(&event, 2),
                Some(HostEvent::Input(InputEvent::TouchCancelled))
            );
        }
    }

    #[test]
    fn visibility_loss_releases_only_the_selected_window() {
        for kind in [WindowEvent::Hidden, WindowEvent::Minimized] {
            let event = Event::Window {
                timestamp: 0,
                window_id: 2,
                win_event: kind,
            };
            assert_eq!(route(&event, 3), None);
            assert_eq!(
                route(&event, 2),
                Some(HostEvent::Input(InputEvent::FocusLost))
            );
        }
        for kind in [WindowEvent::Shown, WindowEvent::Restored] {
            assert_eq!(
                route(
                    &Event::Window {
                        timestamp: 0,
                        window_id: 2,
                        win_event: kind,
                    },
                    2
                ),
                None
            );
        }
    }

    #[test]
    fn application_background_releases_held_input_globally() {
        let mut mapping = InputMapping::new();
        mapping.bind_key(KeyboardKey::SPACE.code(), "jump");
        let mut router = InputRouter::new(mapping);
        assert_eq!(
            router.handle(InputEvent::Key {
                code: KeyboardKey::SPACE.code(),
                down: true,
            }),
            [LogicalAction {
                action: "jump",
                down: true,
            }]
        );
        for (event, expected) in [
            (
                Event::AppWillEnterBackground { timestamp: 1 },
                &[LogicalAction {
                    action: "jump",
                    down: false,
                }][..],
            ),
            (Event::AppDidEnterBackground { timestamp: 2 }, &[][..]),
        ] {
            let observation = sample(&event, 2, 2);
            assert!(observation.matches_window(2));
            assert!(observation.matches_window(3));
            assert_eq!(
                observation.transition(),
                InputTransition::Event(HostEvent::Input(
                    InputEvent::ApplicationSuspended
                ))
            );
            assert_eq!(
                router.handle(InputEvent::ApplicationSuspended),
                expected
            );
        }
    }

    #[test]
    fn application_foreground_does_not_grant_window_focus() {
        for event in [
            Event::AppWillEnterForeground { timestamp: 1 },
            Event::AppDidEnterForeground { timestamp: 2 },
            Event::AppLowMemory { timestamp: 3 },
        ] {
            assert_eq!(
                sample(&event, 2, 3).transition(),
                InputTransition::Ignored
            );
        }
        let focus = Event::Window {
            timestamp: 4,
            window_id: 2,
            win_event: WindowEvent::FocusGained,
        };
        assert_eq!(
            sample(&focus, 2, 4).transition(),
            InputTransition::FocusGained
        );
        assert_eq!(sample(&focus, 3, 4).transition(), InputTransition::Ignored);
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
                code: ControllerButton::DPadLeft.code(),
                down: true,
            }))
        );
        assert_eq!(
            route(&up, 1),
            Some(HostEvent::Input(InputEvent::Button {
                device: u64::from(u32::MAX),
                code: ControllerButton::DPadLeft.code(),
                down: false,
            }))
        );
    }

    #[test]
    fn native_remap_identifies_the_existing_controller() {
        let remapped = Event::ControllerDeviceRemapped {
            timestamp: 0,
            which: u32::MAX,
        };
        assert_eq!(
            route(&remapped, 1),
            Some(HostEvent::Input(InputEvent::ControllerRemapped {
                device: u64::from(u32::MAX),
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
        let terminate = Event::AppTerminating { timestamp: 1 };
        let observation = sample(&terminate, 3, 1);
        assert_eq!(
            observation.transition(),
            InputTransition::Event(HostEvent::Quit)
        );
        assert!(observation.matches_window(3));
        assert!(observation.matches_window(4));
    }
}
