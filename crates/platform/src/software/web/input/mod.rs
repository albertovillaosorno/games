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
//   - Scoped browser keyboard and focus event subscription.
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
//   - Ignore unknown keys and autorepeat; no gamepad support.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::time::Duration;

use games::input::physical::KeyboardKey;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::{Event, EventTarget, KeyboardEvent, KeyboardEventInit};

use crate::ir::frame::RenderError;
use crate::ir::hardware::sensor::{MotionError, MotionSensor};
use crate::ir::hardware::{
    HapticError, HapticSupport, HostEvent, InputEvent, InputSample,
    InputTransition,
};
use crate::ir::input::{InputBackend, InputStartup, NativeInput};
use crate::ir::security::haptics::FeedbackRequest;

const SCOPE: u64 = 7;

#[derive(Debug)]
struct BrowserInput {
    scope: u64,
}

struct EventQueue {
    capacity: NonZeroUsize,
    overflow: bool,
    samples: VecDeque<InputSample>,
}

struct EventSource {
    _listeners: Vec<Listener>,
    queue: Rc<RefCell<EventQueue>>,
}

struct Listener {
    callback: Closure<dyn FnMut(Event)>,
    kind: &'static str,
    target: EventTarget,
}

impl InputBackend for BrowserInput {
    fn clock(&self) -> Result<Duration, RenderError> {
        super::performance::now().map_err(|_| RenderError::InvalidTimestamp)
    }

    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError> {
        Ok(Vec::new())
    }

    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError> {
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
        let sample = self.samples.pop_front();
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
    ) {
        if self.overflow {
            return;
        }
        if self.samples.len() == self.capacity.get() {
            self.samples.clear();
            self.overflow = true;
            self.samples.push_back(InputSample::new(
                stamp,
                focus_lost(),
                Some(scope),
            ));
        } else {
            self.samples.push_back(InputSample::new(
                stamp,
                transition,
                Some(scope),
            ));
        }
    }
}

impl EventSource {
    fn new(
        target: &EventTarget,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let queue = Rc::new(RefCell::new(EventQueue::new(capacity)?));
        let mut listeners = Vec::new();
        for kind in ["keydown", "keyup", "focus", "blur"] {
            let sink = Rc::clone(&queue);
            let callback = Closure::<dyn FnMut(Event)>::new(move |event| {
                if let Some(transition) = transition(kind, &event) {
                    let stamp =
                        super::performance::timestamp(event.time_stamp()).ok();
                    sink.borrow_mut().push(stamp, transition, scope);
                }
            });
            target
                .add_event_listener_with_callback(
                    kind,
                    callback.as_ref().unchecked_ref(),
                )
                .map_err(dom_error)?;
            listeners.push(Listener {
                callback,
                kind,
                target: target.clone(),
            });
        }
        Ok(Self {
            _listeners: listeners,
            queue,
        })
    }

    fn pop(&self) -> Option<InputSample> {
        self.queue.borrow_mut().pop()
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

fn focus_lost() -> InputTransition {
    InputTransition::Event(HostEvent::Input(InputEvent::FocusLost))
}

fn key_code(code: &str) -> Option<KeyboardKey> {
    if let Some(letter) = code.strip_prefix("Key") {
        if letter.len() == 1 && letter.as_bytes().first()?.is_ascii_uppercase()
        {
            return KeyboardKey::from_profile_name(letter);
        }
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        if digit.len() == 1 && digit.as_bytes().first()?.is_ascii_digit() {
            return KeyboardKey::from_profile_name(digit);
        }
    }
    if let Some(digit) = code.strip_prefix("Numpad") {
        let name = match digit {
            "0" => "Keypad 0",
            "1" => "Keypad 1",
            "2" => "Keypad 2",
            "3" => "Keypad 3",
            "4" => "Keypad 4",
            "5" => "Keypad 5",
            "6" => "Keypad 6",
            "7" => "Keypad 7",
            "8" => "Keypad 8",
            "9" => "Keypad 9",
            "Add" => "Keypad +",
            "Subtract" => "Keypad -",
            "Multiply" => "Keypad *",
            "Divide" => "Keypad /",
            "Decimal" => "Keypad .",
            "Enter" => "Keypad Enter",
            "Equal" => "Keypad =",
            _ => return None,
        };
        return KeyboardKey::from_profile_name(name);
    }
    let name = match code {
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "Enter" => "Return",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Backquote" => "`",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "ControlLeft" => "Left Ctrl",
        "ControlRight" => "Right Ctrl",
        "ShiftLeft" => "Left Shift",
        "ShiftRight" => "Right Shift",
        "AltLeft" => "Left Alt",
        "AltRight" => "Right Alt",
        "MetaLeft" => "Left GUI",
        "MetaRight" => "Right GUI",
        "ContextMenu" => "Application",
        "NumLock" => "Numlock",
        "IntlBackslash" => return Some(KeyboardKey::from_code(100)),
        "Escape" | "Backspace" | "Tab" | "Space" | "CapsLock"
        | "PrintScreen" | "ScrollLock" | "Pause" | "Insert" | "Home"
        | "PageUp" | "Delete" | "End" | "PageDown" | "F1" | "F2" | "F3"
        | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
        | "F13" | "F14" | "F15" | "F16" | "F17" | "F18" | "F19" | "F20"
        | "F21" | "F22" | "F23" | "F24" => code,
        _ => return None,
    };
    KeyboardKey::from_profile_name(name)
}

fn next_key(
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

fn verify(condition: bool, message: &str) -> Result<(), RenderError> {
    if condition {
        Ok(())
    } else {
        Err(RenderError::Backend(message.to_owned()))
    }
}

pub(in crate::software) fn verify_event_source() -> Result<(), RenderError> {
    let target = EventTarget::new().map_err(dom_error)?;
    let cap = NonZeroUsize::MIN.saturating_add(1);
    let source = EventSource::new(&target, SCOPE, cap)?;
    let mut input = InputStartup::new(
        Box::new(BrowserInput { scope: SCOPE }),
        NonZeroUsize::MIN,
    )?
    .input;
    for (code, expected) in [
        ("KeyA", KeyboardKey::LETTER_A),
        ("ArrowLeft", KeyboardKey::LEFT),
        ("Numpad4", KeyboardKey::KP_4),
        ("Digit6", KeyboardKey::NUM_6),
        ("Space", KeyboardKey::SPACE),
        ("Enter", KeyboardKey::ENTER),
    ] {
        verify(!send_key(&target, "keydown", code, false)?, "key default")?;
        verify(!send_key(&target, "keydown", code, true)?, "repeat default")?;
        verify(!send_key(&target, "keyup", code, false)?, "up default")?;
        for down in [true, false] {
            verify(
                next_key(&source, &mut input)?
                    == Some(HostEvent::Input(InputEvent::Key {
                        code: expected.code(),
                        down,
                    })),
                "key translation",
            )?;
        }
        verify(next_key(&source, &mut input)?.is_none(), "unexpected key")?;
    }
    verify(
        send_key(&target, "keydown", "Unidentified", false)?,
        "unknown",
    )?;
    verify(
        next_key(&source, &mut input)?.is_none(),
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
        next_key(&source, &mut input)?
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
    verify(next_key(&source, &mut input)?.is_none(), "overflow drain")?;
    let _sent = send_key(&target, "keyup", "KeyA", false)?;
    verify(
        next_key(&source, &mut input)?
            == Some(HostEvent::Input(InputEvent::Key {
                code: KeyboardKey::LETTER_A.code(),
                down: false,
            })),
        "recovery",
    )?;
    drop(source);
    verify(
        send_key(&target, "keydown", "KeyA", false)?,
        "listener removed",
    )
}
