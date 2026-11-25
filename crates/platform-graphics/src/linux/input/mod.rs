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

use games::capability::{
    HapticCommand, HapticEffect, HapticError, HapticSupport, Haptics,
};
use games::input::InputEvent;
use games::input::physical::{
    ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
};
use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use games::presentation::PresentationLayout;
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Scancode;
use sdl2::mouse::MouseButton;
use sdl2::video::Window;
use sdl2::{GameControllerSubsystem, Sdl, TimerSubsystem};

use super::sprites::SceneSession;
use crate::frame::RenderError;

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
    feedback_allowed: bool,
    limit: NonZeroUsize,
    origin: u64,
    rediscover: bool,
    subsystem: GameControllerSubsystem,
    timer: TimerSubsystem,
    touch_enabled: bool,
    touch_geometry: TouchGeometry,
    window: u32,
}

#[derive(Debug, Default)]
struct TouchGeometry {
    cutoff: Option<Duration>,
    layout: Option<PresentationLayout>,
}

impl TouchGeometry {
    fn accepts(&self, stamp: Option<Duration>) -> bool {
        self.cutoff
            .is_none_or(|cutoff| stamp.is_some_and(|elapsed| elapsed >= cutoff))
    }

    fn observe(
        &mut self,
        layout: Option<PresentationLayout>,
        elapsed: Duration,
    ) -> bool {
        if self.layout == layout {
            return false;
        }
        let invalidated = self.layout.is_some();
        self.layout = layout;
        if invalidated || self.cutoff.is_some() {
            self.cutoff = Some(elapsed);
        }
        invalidated
    }
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
    /// Discover controllers after a native poll returns no event.
    ///
    /// A device change in an earlier limited batch remains pending until the
    /// caller has reached a no-event poll. Re-enumeration is
    /// unnecessary unless device availability may have changed.
    /// # Errors
    /// Reports failed controller enumeration or device open.
    pub fn discover_after_drain(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
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
    pub fn discover_available(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
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
        window: &Window,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        let subsystem = sdl.game_controller().map_err(RenderError::Backend)?;
        let timer = sdl.timer().map_err(RenderError::Backend)?;
        let mut input = Self {
            controllers: BTreeMap::new(),
            feedback_allowed: window.has_input_focus()
                && !window.is_minimized(),
            limit,
            origin: timer.ticks64(),
            rediscover: false,
            subsystem,
            timer,
            touch_enabled: false,
            touch_geometry: TouchGeometry::default(),
            window: window.id(),
        };
        let attached = input.discover_available()?;
        Ok(InputStartup { attached, input })
    }

    fn stop_all_feedback(&mut self) {
        for controller in self.controllers.values_mut() {
            stop_feedback(controller);
        }
    }

    /// Reconcile touch input with the scene before polling native events.
    ///
    /// A changed or lost presented transform invalidates held contacts.
    /// Route `TouchCancelled` at the returned elapsed time before later
    /// native transitions.
    /// Once invalidated, queued finger events older than the observation
    /// are ignored rather than projected onto a different presentation.
    /// # Errors
    /// Reports a failed drawable query.
    pub fn synchronize_touch(
        &mut self,
        scene: &SceneSession<'_, '_>,
    ) -> Result<Option<Duration>, RenderError> {
        if !self.touch_enabled {
            return Ok(None);
        }
        let layout = scene.visible_layout()?;
        let elapsed = self.elapsed();
        Ok(self
            .touch_geometry
            .observe(layout, elapsed)
            .then_some(elapsed))
    }

    /// Translate one polled native event, recording device changes.
    ///
    /// Add notifications are discovery hints, not stable device identities.
    /// Drain queued removals before enumerating current joystick indices.
    /// Button, remap and removal events use admitted instance identifiers.
    /// Only devices within the explicit capacity may be opened.
    #[must_use]
    pub fn translate(&mut self, event: &Event) -> Option<HostEvent> {
        if matches!(event, Event::ControllerDeviceAdded { .. }) {
            self.rediscover |= self.controllers.len() < self.limit.get();
            return None;
        }
        if let Event::ControllerDeviceRemoved { which, .. } = *event {
            let device = u64::from(which);
            let removed = self.controllers.remove(&device);
            self.rediscover |= self.controllers.len() < self.limit.get();
            return removed.map(|mut controller| {
                stop_feedback(&mut controller);
                HostEvent::Input(InputEvent::Detached { device })
            });
        }
        if self.touch_enabled && event.is_finger() {
            if !self.touch_geometry.accepts(self.event_elapsed(event)) {
                return None;
            }
            return route_contact(event);
        }
        let routed = route(event, self.window);
        if matches!(
            routed,
            Some(HostEvent::Quit | HostEvent::Input(InputEvent::FocusLost))
        ) {
            self.feedback_allowed = false;
            self.stop_all_feedback();
        } else if matches!(
            event,
            Event::Window {
                window_id,
                win_event: WindowEvent::FocusGained,
                ..
            } if *window_id == self.window
        ) {
            self.feedback_allowed = true;
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
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        self.stop_all_feedback();
    }
}

impl Haptics for NativeInput {
    fn submit(&mut self, command: HapticCommand) -> Result<(), HapticError> {
        let controller = self
            .controllers
            .get_mut(&command.device)
            .ok_or(HapticError::Disconnected)?;
        deliver_feedback(
            controller_support(controller),
            self.feedback_allowed,
            command.effect,
            |low, high, milliseconds| {
                controller
                    .set_rumble(low, high, milliseconds)
                    .map_err(|_error| HapticError::Unavailable)
            },
        )
    }

    fn support(&self, device: u64) -> Option<HapticSupport> {
        self.controllers.get(&device).and_then(controller_support)
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
    support: Option<HapticSupport>,
    allowed: bool,
    effect: HapticEffect,
    deliver: impl FnOnce(u16, u16, u32) -> Result<(), HapticError>,
) -> Result<(), HapticError> {
    let admitted = match support {
        None => return Err(HapticError::Disconnected),
        Some(HapticSupport::Unsupported) => {
            return Err(HapticError::Unsupported);
        },
        Some(HapticSupport::SingleMotor) => effect.single_motor(),
        Some(HapticSupport::DualMotor) => effect,
    };
    match admitted {
        HapticEffect::Stop => deliver(0, 0, 0),
        HapticEffect::Rumble { low, high, milliseconds } if allowed => {
            deliver(low, high, NonZeroU32::from(milliseconds).get())
        },
        HapticEffect::Rumble { .. } => Err(HapticError::Unavailable),
    }
}

fn stop_feedback(controller: &mut GameController) {
    if controller.attached() && controller.has_rumble() {
        // Loss of the native device can race with cleanup. Effects have
        // finite duration even if a stop fails or the device disappears.
        let _result = controller.set_rumble(0, 0, 0);
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

/// Resolve a legacy keyboard profile name at the native boundary.
#[must_use]
pub fn key_from_name(name: &str) -> Option<KeyboardKey> {
    Scancode::from_name(name).map(|key| KeyboardKey::from_code(key_code(key)))
}

/// Native presentation name for a translated physical key.
#[must_use]
pub fn key_name(key: KeyboardKey) -> &'static str {
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
        if matches!(
            win_event,
            WindowEvent::FocusLost
                | WindowEvent::Hidden
                | WindowEvent::Minimized
        ) {
            return Some(HostEvent::Input(InputEvent::FocusLost));
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
    use std::collections::BTreeMap;
    use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::capability::{HapticEffect, HapticError, HapticSupport};
    use games::input::physical::{
        ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
    };
    use games::input::touch::{ContactId, ContactPhase, TouchPoint};
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use games::presentation::{Dimensions, PresentationMode};
    use sdl2::event::{Event, WindowEvent};
    use sdl2::keyboard::Mod;
    use sdl2::mouse::MouseButton;

    use super::{
        Axis, Button, HostEvent, Scancode, TouchGeometry, admit_discovery,
        axis_code, button_code, deliver_feedback, event_elapsed, key_from_name,
        key_name, mouse_code, open_candidate, route, route_contact,
    };
    use crate::frame::RenderError;

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
    fn feedback_delivers_only_bounded_supported_foreground_effects() {
        let mut deliveries = Vec::new();
        let mut send = |support, allowed, effect| {
            deliver_feedback(support, allowed, effect, |low, high, time| {
                deliveries.push((low, high, time));
                Ok(())
            })
        };
        let effect = HapticEffect::Rumble {
            low: u16::MAX,
            high: 7,
            milliseconds: NonZeroU16::MAX,
        };
        let dual = Some(HapticSupport::DualMotor);
        let single = Some(HapticSupport::SingleMotor);
        assert_eq!(send(None, true, effect), Err(HapticError::Disconnected));
        assert_eq!(
            send(Some(HapticSupport::Unsupported), true, effect),
            Err(HapticError::Unsupported),
        );
        assert_eq!(send(dual, false, effect), Err(HapticError::Unavailable));
        assert_eq!(send(dual, true, effect), Ok(()));
        assert_eq!(send(single, true, effect), Ok(()));
        assert_eq!(send(dual, false, HapticEffect::Stop), Ok(()));
        assert_eq!(deliveries, [
            (u16::MAX, 7, 65_535),
            (u16::MAX, u16::MAX, 65_535),
            (0, 0, 0)
        ],);
        assert_eq!(
            deliver_feedback(dual, true, effect, |_, _, _| {
                Err(HapticError::Unavailable)
            }),
            Err(HapticError::Unavailable),
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
    fn touch_geometry_cancels_once_and_rejects_stale_samples() {
        let surface = Dimensions::new(
            NonZeroU32::new(96).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(48).unwrap_or(NonZeroU32::MIN),
        );
        let design = Dimensions::new(NonZeroU32::MIN, NonZeroU32::MIN);
        let first = Some(PresentationMode::AspectFit.layout(surface, design));
        let second = Some(PresentationMode::Stretch.layout(surface, design));
        let mut geometry = TouchGeometry::default();
        let at = Duration::from_millis;
        for (layout, tick, cancelled, accepted) in [
            (None, 1, false, [true; 4]),
            (first, 2, false, [true; 4]),
            (first, 3, false, [true; 4]),
            (second, 4, true, [false, false, true, true]),
            (second, 5, false, [false, false, true, true]),
            (None, 6, true, [false, false, false, true]),
            (None, 7, false, [false, false, false, true]),
            (first, 8, false, [false, false, false, true]),
        ] {
            assert_eq!(geometry.observe(layout, at(tick)), cancelled);
            let stamps = [None, Some(at(1)), Some(at(4)), Some(at(8))];
            assert_eq!(stamps.map(|stamp| geometry.accepts(stamp)), accepted);
        }
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
    }
}
