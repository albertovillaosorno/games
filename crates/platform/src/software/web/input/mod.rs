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
//   - Scoped browser keyboard, pointer and focus event subscription.
// - Must-Not:
//   - Own game actions, rendering or controller discovery policy.
// - Allows:
//   - Inputs: DOM events on an explicitly supplied target.
//   - Outputs: Bounded portable input samples.
//   - Side effects: Register and remove scoped DOM listeners.
// - Split-When:
//   - Other device types need independent browser resources.
// - Merge-When:
//   - A shared browser event source owns the same subscriptions.
// - Summary:
//   - Translate physical key codes without using layout-dependent labels.
// - Description:
//   - Overflow cancels held input rather than losing arbitrary releases.
// - Usage:
//   - Subscribe to a host target, drain samples and route through input IR.
// - Defaults:
//   - Ignore unknown keys, autorepeat and nonstandard controllers.

mod mouse;
mod pointer;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::time::Duration;

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
use games::input::physical::ControllerAxis;
#[cfg(feature = "test-fixtures")]
use games::input::physical::KeyboardKey;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
#[cfg(feature = "test-fixtures")]
use web_sys::KeyboardEventInit;
use web_sys::{Event, EventTarget, KeyboardEvent};

#[cfg(feature = "interactive")]
use super::gamepad::{
    ControllerEvents,
    browser::{Events, Gamepads},
};
use super::keyboard::key_code;
use crate::ir::frame::RenderError;
use crate::ir::hardware::sensor::{MotionError, MotionSensor};
use crate::ir::hardware::{
    HapticError, HapticSupport, HostEvent, InputEvent, InputSample,
    InputTransition,
};
use crate::ir::input::InputBackend;
#[cfg(feature = "test-fixtures")]
use crate::ir::input::{InputStartup, NativeInput};
use crate::ir::security::haptics::FeedbackRequest;

#[cfg(feature = "test-fixtures")]
const SCOPE: u64 = 7;

#[derive(Debug)]
struct BrowserInput {
    #[cfg(feature = "interactive")]
    gamepads: Option<Gamepads>,
    scope: u64,
}

struct EventQueue {
    capacity: NonZeroUsize,
    overflow: bool,
    samples: VecDeque<(u64, InputSample)>,
}

pub(super) struct EventSource {
    #[cfg(feature = "interactive")]
    controller_events: Events,
    listeners: Vec<Listener>,
    mouse: Rc<mouse::MouseButtons>,
    queue: Rc<RefCell<EventQueue>>,
    scope: u64,
}

struct Listener {
    callback: Closure<dyn FnMut(Event)>,
    kind: &'static str,
    target: EventTarget,
}

#[cfg(feature = "test-fixtures")]
pub(super) fn browser_input(scope: u64) -> crate::ir::input::InputPort {
    Box::new(BrowserInput {
        #[cfg(feature = "interactive")]
        gamepads: None,
        scope,
    })
}

impl InputBackend for BrowserInput {
    fn clock(&self) -> Result<Duration, RenderError> {
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)
    }

    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError> {
        self.discover_available()
    }

    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError> {
        #[cfg(feature = "interactive")]
        if let Some(gamepads) = &self.gamepads {
            return gamepads.poll();
        }
        Ok(Vec::new())
    }

    fn feedback_support(&self, _device: u64) -> Option<HapticSupport> {
        None
    }

    fn initial_feedback_allowed(&self) -> bool {
        false
    }

    fn read_motion(
        &mut self,
        _device: u64,
        _sensor: MotionSensor,
    ) -> Result<[f32; 3], MotionError> {
        Err(MotionError::Unsupported)
    }

    fn submit_feedback(
        &mut self,
        _request: FeedbackRequest,
    ) -> Result<(), HapticError> {
        Err(HapticError::Unsupported)
    }

    fn translate(&mut self, sample: &InputSample) -> Option<HostEvent> {
        #[cfg(feature = "interactive")]
        if let InputTransition::Disconnected(device) = sample.transition() {
            return self.gamepads.as_ref()?.remove(device);
        }
        if let InputTransition::Event(event) = sample.transition() {
            Some(event)
        } else {
            None
        }
    }

    fn window_scope(&self) -> u64 {
        self.scope
    }
}

impl EventQueue {
    fn new(capacity: NonZeroUsize) -> Result<Self, RenderError> {
        let mut samples = VecDeque::new();
        samples
            .try_reserve_exact(capacity.get())
            .map_err(|_| RenderError::TooLarge)?;
        Ok(Self {
            capacity,
            overflow: false,
            samples,
        })
    }

    fn pop(&mut self) -> Option<InputSample> {
        let sample = self.samples.pop_front().map(|(_order, sample)| sample);
        if self.samples.is_empty() {
            self.overflow = false;
        }
        sample
    }

    fn push(
        &mut self,
        stamp: Option<Duration>,
        transition: InputTransition,
        scope: u64,
        order: u64,
    ) {
        let sample = (order, InputSample::new(stamp, transition, Some(scope)));
        if self.overflow {
            // Ordinary overflow withdraws window input, but must not
            // swallow a subsequent application-wide suspension.
            if transition == application_suspended() {
                self.samples.clear();
                self.samples.push_back(sample);
            }
            return;
        }
        if self.samples.len() == self.capacity.get() {
            // Keep the latest real suspension and its acquisition order.
            // It already cancels everything lost by discarding DOM input.
            let suspension = if transition == application_suspended() {
                Some(sample)
            } else {
                self.samples.iter().rev().copied().find(|(_, queued)| {
                    queued.transition() == application_suspended()
                })
            };
            self.samples.clear();
            self.overflow = true;
            self.samples.push_back(suspension.unwrap_or((
                order,
                InputSample::new(stamp, focus_lost(), Some(scope)),
            )));
        } else {
            self.samples.push_back(sample);
        }
    }
}

impl EventSource {
    fn new(
        target: &EventTarget,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let mut source = Self {
            #[cfg(feature = "interactive")]
            controller_events: Rc::new(RefCell::new(ControllerEvents::new(
                scope,
            ))),
            listeners: Vec::new(),
            mouse: Rc::new(mouse::MouseButtons::default()),
            queue: Rc::new(RefCell::new(EventQueue::new(capacity)?)),
            scope,
        };
        for kind in ["keydown", "keyup", "focus", "blur"] {
            source
                .listen(target, kind, move |event| transition(kind, event))?;
        }
        Ok(source)
    }

    fn listen<Transitions: IntoIterator<Item = InputTransition>>(
        &mut self,
        target: &EventTarget,
        kind: &'static str,
        mut translate: impl FnMut(&Event) -> Transitions + 'static,
    ) -> Result<(), RenderError> {
        let sink = Rc::clone(&self.queue);
        let mouse = Rc::clone(&self.mouse);
        let scope = self.scope;
        #[cfg(feature = "interactive")]
        let controllers = Rc::clone(&self.controller_events);
        let callback = Closure::<dyn FnMut(Event)>::new(move |event| {
            let mut transitions = translate(&event).into_iter();
            let Some(first) = transitions.next() else {
                return;
            };
            let stamp = super::performance::timestamp(event.time_stamp()).ok();
            let mut queue = sink.borrow_mut();
            for transition in std::iter::once(first).chain(transitions) {
                #[cfg(feature = "interactive")]
                let order = {
                    let mut controllers = controllers.borrow_mut();
                    if transition == focus_lost()
                        || transition == application_suspended()
                    {
                        controllers.cancel(stamp);
                    }
                    controllers.next_order()
                };
                #[cfg(not(feature = "interactive"))]
                let order = 0;
                queue.push(stamp, transition, scope, order);
            }
            // Overflow cancels logical holds. Discard adapter holds too,
            // including presses observed while the queue rejects events.
            if queue.overflow {
                mouse.clear();
            }
        });
        target
            .add_event_listener_with_callback(
                kind,
                callback.as_ref().unchecked_ref(),
            )
            .map_err(dom_error)?;
        self.listeners.push(Listener {
            callback,
            kind,
            target: target.clone(),
        });
        Ok(())
    }

    pub(super) fn for_element(
        element: &web_sys::Element,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let mut source = Self::new(element, scope, capacity)?;
        pointer::subscribe(&mut source, element)?;
        Ok(source)
    }

    #[cfg(feature = "interactive")]
    pub(super) fn input(
        &self,
        element: &web_sys::Element,
        limit: NonZeroUsize,
    ) -> Result<crate::ir::input::InputPort, RenderError> {
        let gamepads =
            Gamepads::new(element, limit, Rc::clone(&self.controller_events))?;
        Ok(Box::new(BrowserInput {
            gamepads: Some(gamepads),
            scope: self.scope,
        }))
    }

    pub(super) fn pop(&self) -> Option<InputSample> {
        let mut queue = self.queue.borrow_mut();
        #[cfg(feature = "interactive")]
        {
            let mut controllers = self.controller_events.borrow_mut();
            if controllers.front_order().is_some_and(|order| {
                queue
                    .samples
                    .front()
                    .is_none_or(|(dom_order, _sample)| order <= *dom_order)
            }) {
                return controllers.pop();
            }
        }
        queue.pop()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _result = self.target.remove_event_listener_with_callback(
            self.kind,
            self.callback.as_ref().unchecked_ref(),
        );
    }
}

fn dom_error(_error: JsValue) -> RenderError {
    RenderError::Backend("browser event operation failed".to_owned())
}

fn application_suspended() -> InputTransition {
    InputTransition::Event(HostEvent::Input(InputEvent::ApplicationSuspended))
}

fn focus_lost() -> InputTransition {
    InputTransition::Event(HostEvent::Input(InputEvent::FocusLost))
}

#[cfg(feature = "test-fixtures")]
fn next_event(
    source: &EventSource,
    input: &mut NativeInput,
) -> Result<Option<HostEvent>, RenderError> {
    let Some(sample) = source.pop() else {
        return Ok(None);
    };
    verify(sample.matches_window(SCOPE), "sample scope")?;
    verify(!sample.matches_window(8), "foreign scope")?;
    verify(input.event_elapsed(&sample).is_some(), "event clock")?;
    Ok(input.translate(&sample))
}

#[cfg(feature = "test-fixtures")]
fn key_event(
    kind: &str,
    code: &str,
    repeat: bool,
) -> Result<KeyboardEvent, RenderError> {
    let init = KeyboardEventInit::new();
    init.set_code(code);
    init.set_key("different-layout-label");
    init.set_repeat(repeat);
    init.set_cancelable(true);
    KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init)
        .map_err(dom_error)
}

#[cfg(feature = "test-fixtures")]
fn send_key(
    target: &EventTarget,
    kind: &str,
    code: &str,
    repeat: bool,
) -> Result<bool, RenderError> {
    let event = key_event(kind, code, repeat)?;
    target.dispatch_event(&event).map_err(dom_error)
}

fn transition(kind: &str, event: &Event) -> Option<InputTransition> {
    match kind {
        "focus" => Some(InputTransition::FocusGained),
        "blur" => Some(focus_lost()),
        "keydown" | "keyup" => {
            let key = event.dyn_ref::<KeyboardEvent>()?;
            let code = key_code(&key.code())?.code();
            event.prevent_default();
            let down = kind == "keydown";
            if down && key.repeat() {
                return None;
            }
            Some(InputTransition::Event(HostEvent::Input(InputEvent::Key {
                code,
                down,
            })))
        },
        _ => None,
    }
}

#[cfg(feature = "test-fixtures")]
fn verify(condition: bool, message: &str) -> Result<(), RenderError> {
    if condition {
        Ok(())
    } else {
        Err(RenderError::Backend(message.to_owned()))
    }
}

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
fn controller_edge(value: i16) -> InputTransition {
    InputTransition::Event(HostEvent::Input(InputEvent::Axis {
        code: ControllerAxis::LeftX.code(),
        device: 1,
        value,
    }))
}

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
fn verify_suspension_order() -> Result<(), RenderError> {
    let target = EventTarget::new().map_err(dom_error)?;
    let mut source = EventSource::new(&target, SCOPE, NonZeroUsize::MIN)?;
    source.listen(&target, "fixture-suspend", |_| {
        Some(application_suspended())
    })?;
    let stamp =
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)?;
    verify(
        source.controller_events.borrow_mut().fixture_hold(stamp)?
            == [InputEvent::Attached { device: 1 }],
        "suspension fixture attachment",
    )?;
    verify(
        source.pop().is_some_and(|sample| {
            sample.transition() == controller_edge(i16::MAX)
        }),
        "suspension fixture controller hold",
    )?;
    let event = Event::new("fixture-suspend").map_err(dom_error)?;
    let _sent = target.dispatch_event(&event).map_err(dom_error)?;
    for expected in [controller_edge(0), application_suspended()] {
        verify(
            source
                .pop()
                .is_some_and(|sample| sample.transition() == expected),
            "ordered application suspension",
        )?;
    }
    let _duplicate = target.dispatch_event(&event).map_err(dom_error)?;
    verify(
        source.pop().is_some_and(|sample| {
            sample.transition() == application_suspended()
        }) && source.pop().is_none(),
        "duplicate suspension must not duplicate neutral controller edges",
    )?;
    verify_suspension_overflow(&source, &target)
}

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
fn verify_suspension_overflow(
    source: &EventSource,
    target: &EventTarget,
) -> Result<(), RenderError> {
    // Suspension can fill the queue, trigger overflow, or arrive after it.
    for keys_before in 0..=2 {
        let stamp = super::performance::now()
            .map_err(|_| RenderError::InvalidTimestamp)?;
        let attached =
            source.controller_events.borrow_mut().fixture_hold(stamp)?;
        verify(
            attached.is_empty(),
            "suspension retains controller identity",
        )?;
        verify(
            source.pop()
                == Some(InputSample::new(
                    Some(stamp),
                    controller_edge(i16::MAX),
                    Some(SCOPE),
                )),
            "controller reassertion before suspension overflow",
        )?;
        for _ in 0..keys_before {
            let _sent = send_key(target, "keydown", "KeyA", false)?;
        }
        let event = Event::new("fixture-suspend").map_err(dom_error)?;
        let stamp = super::performance::timestamp(event.time_stamp()).ok();
        let _sent = target.dispatch_event(&event).map_err(dom_error)?;
        // Force overflow if the suspension was the first observation.
        let _sent = send_key(target, "keydown", "KeyA", false)?;
        for expected in [controller_edge(0), application_suspended()] {
            verify(
                source.pop()
                    == Some(InputSample::new(stamp, expected, Some(SCOPE))),
                "suspension overflow preserves real timestamp and order",
            )?;
        }
        verify(source.pop().is_none(), "suspension overflow drained")?;
    }
    let _sent = send_key(target, "keyup", "KeyA", false)?;
    verify(
        source.pop().is_some_and(|sample| {
            sample.transition()
                == InputTransition::Event(HostEvent::Input(InputEvent::Key {
                    code: KeyboardKey::LETTER_A.code(),
                    down: false,
                }))
        }) && source.pop().is_none(),
        "input resumes after suspension overflow",
    )
}

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
fn verify_controller_lifecycle(
    source: &EventSource,
    target: &EventTarget,
) -> Result<(), RenderError> {
    let stamp =
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)?;
    verify(
        source.controller_events.borrow_mut().fixture_hold(stamp)?
            == [InputEvent::Attached { device: 1 }],
        "controller attachment",
    )?;
    verify(
        source.pop()
            == Some(InputSample::new(
                Some(stamp),
                controller_edge(i16::MAX),
                Some(SCOPE),
            )),
        "controller hold",
    )?;
    for _ in 0..4 {
        let _sent = send_key(target, "keydown", "KeyA", false)?;
    }
    verify(
        source
            .pop()
            .is_some_and(|event| event.transition() == focus_lost())
            && source.pop().is_none(),
        "DOM overflow does not release controller",
    )?;
    let blur = Event::new("blur").map_err(dom_error)?;
    let stamp = super::performance::timestamp(blur.time_stamp()).ok();
    let _sent = target.dispatch_event(&blur).map_err(dom_error)?;
    let focus = Event::new("focus").map_err(dom_error)?;
    let _sent = target.dispatch_event(&focus).map_err(dom_error)?;
    verify(
        source.pop()
            == Some(InputSample::new(stamp, controller_edge(0), Some(SCOPE))),
        "blur and refocus between controller polls release the hold",
    )?;
    verify(
        source
            .pop()
            .is_some_and(|event| event.transition() == focus_lost())
            && source.pop().is_some_and(|event| {
                event.transition() == InputTransition::FocusGained
            })
            && source.pop().is_none(),
        "ordered lifecycle and no duplicate release",
    )?;
    let stamp =
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)?;
    let attached = source.controller_events.borrow_mut().fixture_hold(stamp)?;
    verify(
        attached.is_empty()
            && source.pop()
                == Some(InputSample::new(
                    Some(stamp),
                    controller_edge(i16::MAX),
                    Some(SCOPE),
                ))
            && source.pop().is_none(),
        "refocus snapshot reasserts the same controller identity",
    )?;
    verify_controller_handoff(source, target)
}

#[cfg(all(feature = "interactive", feature = "test-fixtures"))]
fn verify_controller_handoff(
    source: &EventSource,
    target: &EventTarget,
) -> Result<(), RenderError> {
    use games::input::{AxisBinding, InputMapping, InputRouter, LogicalAction};

    let key = KeyboardKey::LETTER_A.code();
    let mut mapping = InputMapping::new();
    mapping.bind_key(key, "move");
    mapping.bind_axis(
        ControllerAxis::LeftX.code(),
        AxisBinding::new(None, Some("move"), 16_000, 8000)
            .ok_or(RenderError::InvalidLength)?,
    );
    let mut router = InputRouter::new(mapping);
    let _attached = router.handle(InputEvent::Attached { device: 1 });
    verify(
        router.handle(InputEvent::Axis {
            device: 1,
            code: ControllerAxis::LeftX.code(),
            value: i16::MAX,
        }) == [LogicalAction {
            action: "move",
            down: true,
        }],
        "controller handoff initial hold",
    )?;
    let _sent = send_key(target, "keydown", "KeyA", false)?;
    let blur = Event::new("blur").map_err(dom_error)?;
    let _sent = target.dispatch_event(&blur).map_err(dom_error)?;
    // The key was pressed before blur, while the controller was held.
    // Delivering the neutral controller edge first would spuriously
    // release and re-press the aggregate action.
    let mut actions = Vec::new();
    for expected in [
        InputTransition::Event(HostEvent::Input(InputEvent::Key {
            code: key,
            down: true,
        })),
        controller_edge(0),
        focus_lost(),
    ] {
        let sample = source.pop().ok_or(RenderError::InvalidLength)?;
        verify(sample.transition() == expected, "cross-source order")?;
        if let InputTransition::Event(HostEvent::Input(event)) =
            sample.transition()
        {
            actions.extend(router.handle(event));
        }
    }
    verify(
        actions
            == [LogicalAction {
                action: "move",
                down: false,
            }]
            && source.pop().is_none(),
        "handoff releases only on focus loss",
    )?;

    // DOM overflow must not erase either the older polled edge or the
    // later controller cancellation, even if the actual blur is dropped.
    let stamp =
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)?;
    let _attached =
        source.controller_events.borrow_mut().fixture_hold(stamp)?;
    for _ in 0..4 {
        let _sent = send_key(target, "keydown", "KeyA", false)?;
    }
    let blur = Event::new("blur").map_err(dom_error)?;
    let _sent = target.dispatch_event(&blur).map_err(dom_error)?;
    for expected in
        [controller_edge(i16::MAX), focus_lost(), controller_edge(0)]
    {
        verify(
            source
                .pop()
                .is_some_and(|sample| sample.transition() == expected),
            "independent overflow order",
        )?;
    }
    verify(source.pop().is_none(), "independent overflow drained")
}

#[cfg(feature = "test-fixtures")]
pub(in crate::software) fn verify_event_source() -> Result<(), RenderError> {
    let target = EventTarget::new().map_err(dom_error)?;
    let cap = NonZeroUsize::MIN.saturating_add(1);
    let source = EventSource::new(&target, SCOPE, cap)?;
    let mut input =
        InputStartup::new(browser_input(SCOPE), NonZeroUsize::MIN)?.input;
    for (code, expected) in [
        ("KeyA", KeyboardKey::LETTER_A),
        ("ArrowLeft", KeyboardKey::LEFT),
        ("Numpad4", KeyboardKey::KP_4),
        ("Digit6", KeyboardKey::NUM_6),
        ("Space", KeyboardKey::SPACE),
        ("Enter", KeyboardKey::ENTER),
        ("IntlBackslash", KeyboardKey::from_code(100)),
        ("NumpadComma", KeyboardKey::from_code(133)),
    ] {
        verify(!send_key(&target, "keydown", code, false)?, "key default")?;
        verify(!send_key(&target, "keydown", code, true)?, "repeat default")?;
        verify(!send_key(&target, "keyup", code, false)?, "up default")?;
        for down in [true, false] {
            verify(
                next_event(&source, &mut input)?
                    == Some(HostEvent::Input(InputEvent::Key {
                        code: expected.code(),
                        down,
                    })),
                "key translation",
            )?;
        }
        verify(next_event(&source, &mut input)?.is_none(), "unexpected key")?;
    }
    verify(
        send_key(&target, "keydown", "Unidentified", false)?,
        "unknown",
    )?;
    verify(
        next_event(&source, &mut input)?.is_none(),
        "unknown event routed",
    )?;
    let focus = Event::new("focus").map_err(dom_error)?;
    let _sent = target.dispatch_event(&focus).map_err(dom_error)?;
    let gained = source.pop().ok_or(RenderError::InvalidLength)?;
    verify(gained.transition() == InputTransition::FocusGained, "focus")?;
    verify(input.translate(&gained).is_none(), "focus routing")?;
    let blur = Event::new("blur").map_err(dom_error)?;
    let _sent = target.dispatch_event(&blur).map_err(dom_error)?;
    verify(
        next_event(&source, &mut input)?
            == Some(HostEvent::Input(InputEvent::FocusLost)),
        "blur",
    )?;
    let mut expected_stamp = None;
    for index in 0..4 {
        let event = key_event("keydown", "KeyA", false)?;
        if index == cap.get() {
            expected_stamp =
                super::performance::timestamp(event.time_stamp()).ok();
        }
        let _sent = target.dispatch_event(&event).map_err(dom_error)?;
    }
    let overflow = source.pop().ok_or(RenderError::InvalidLength)?;
    verify(input.event_elapsed(&overflow).is_some(), "overflow clock")?;
    verify(
        overflow.elapsed_since(Duration::ZERO) == expected_stamp,
        "overflow triggering timestamp",
    )?;
    verify(
        input.translate(&overflow)
            == Some(HostEvent::Input(InputEvent::FocusLost)),
        "overflow",
    )?;
    verify(next_event(&source, &mut input)?.is_none(), "overflow drain")?;
    let _sent = send_key(&target, "keyup", "KeyA", false)?;
    verify(
        next_event(&source, &mut input)?
            == Some(HostEvent::Input(InputEvent::Key {
                code: KeyboardKey::LETTER_A.code(),
                down: false,
            })),
        "recovery",
    )?;
    #[cfg(feature = "interactive")]
    verify_controller_lifecycle(&source, &target)?;
    #[cfg(feature = "interactive")]
    verify_suspension_order()?;
    drop(source);
    verify(
        send_key(&target, "keydown", "KeyA", false)?,
        "listener removed",
    )?;
    pointer::verify_touch_source()
}
