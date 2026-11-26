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
//   - Standard Gamepad API acquisition and session-scoped observation.
// - Must-Not:
//   - Change browser focus or own logical action routing.
// - Allows:
//   - Inputs: Explicit canvas, session capacity and browser observations.
//   - Outputs: Bounded controller attachments and timestamped transitions.
//   - Side effects: One removable window disconnection subscription.
// - Split-When:
//   - A separate browser controller capability needs its own resources.
// - Merge-When:
//   - Shared browser input acquisition provides identical observations.
// - Summary:
//   - Poll standard controllers after draining the window event queue.
// - Description:
//   - Publish changes independently of the overflow-prone DOM event queue.
// - Usage:
//   - Retain the session until the input capability is dropped.
// - Defaults:
//   - Ignore unavailable APIs and unmapped or malformed devices.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt::{self, Debug, Formatter};
use std::num::NonZeroUsize;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{
    Element, Event, Gamepad, GamepadButton, GamepadEvent, GamepadMappingType,
    Window,
};

use super::{
    BUTTONS, MAX_DEVICES, MAX_TRANSITIONS, Observation, Registry, State,
};
use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputEvent, InputSample};
use crate::software::web::performance;

pub(in crate::software::web) type Samples = Rc<RefCell<VecDeque<InputSample>>>;

struct Disconnect {
    callback: Closure<dyn FnMut(Event)>,
    window: Window,
}

impl Drop for Disconnect {
    fn drop(&mut self) {
        let _removed = self.window.remove_event_listener_with_callback(
            "gamepaddisconnected",
            self.callback.as_ref().unchecked_ref(),
        );
    }
}

pub(in crate::software::web) struct Gamepads {
    _disconnect: Disconnect,
    element: Element,
    registry: Rc<RefCell<Registry>>,
    samples: Samples,
    scope: u64,
}

impl Debug for Gamepads {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gamepads")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl Gamepads {
    pub(in crate::software::web) fn new(
        element: &Element,
        limit: NonZeroUsize,
        samples: Samples,
        scope: u64,
    ) -> Result<Self, RenderError> {
        let window = element
            .owner_document()
            .and_then(|document| document.default_view())
            .ok_or(RenderError::Unsupported)?;
        let registry = Rc::new(RefCell::new(Registry::new(limit)?));
        samples
            .borrow_mut()
            .try_reserve_exact(limit.get().min(MAX_DEVICES) * MAX_TRANSITIONS)
            .map_err(|_error| RenderError::TooLarge)?;
        let devices = Rc::clone(&registry);
        let callback = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            if event.is_trusted()
                && let Some(gamepad) = event
                    .dyn_ref::<GamepadEvent>()
                    .and_then(GamepadEvent::gamepad)
            {
                devices.borrow_mut().disconnect(gamepad.index());
            }
        });
        window
            .add_event_listener_with_callback(
                "gamepaddisconnected",
                callback.as_ref().unchecked_ref(),
            )
            .map_err(|_error| RenderError::Unsupported)?;
        Ok(Self {
            _disconnect: Disconnect { callback, window },
            element: element.clone(),
            registry,
            samples,
            scope,
        })
    }

    pub(in crate::software::web) fn poll(
        &self,
    ) -> Result<Vec<InputEvent>, RenderError> {
        let mut samples = self.samples.borrow_mut();
        if !samples.is_empty() {
            return Ok(Vec::new());
        }
        let focused = self.element.owner_document().is_some_and(|document| {
            !document.hidden()
                && document.has_focus().unwrap_or(false)
                && document.active_element().as_ref() == Some(&self.element)
        });
        let observed = if focused {
            Some(observations(&self._disconnect.window)?)
        } else {
            None
        };
        let stamp = Some(
            performance::now()
                .map_err(|_error| RenderError::InvalidTimestamp)?,
        );
        self.registry
            .borrow_mut()
            .poll(observed.as_deref(), |edge| {
                samples.push_back(InputSample::new(
                    stamp,
                    edge,
                    Some(self.scope),
                ));
            })
    }

    pub(in crate::software::web) fn remove(
        &self,
        id: u64,
    ) -> Option<HostEvent> {
        self.registry.borrow_mut().remove(id)
    }
}

fn observations(window: &Window) -> Result<Vec<Observation>, RenderError> {
    let Ok(gamepads) = window.navigator().get_gamepads() else {
        return Ok(Vec::new());
    };
    let count = gamepads
        .length()
        .min(u32::try_from(MAX_DEVICES).unwrap_or(0));
    let mut observed = Vec::new();
    observed
        .try_reserve_exact(
            usize::try_from(count).map_err(|_error| RenderError::TooLarge)?,
        )
        .map_err(|_error| RenderError::TooLarge)?;
    for index in 0..count {
        if let Ok(gamepad) = gamepads.get(index).dyn_into::<Gamepad>()
            && gamepad.index() == index
            && gamepad.connected()
            && gamepad.mapping() == GamepadMappingType::Standard
            && let Some(state) = state(&gamepad)
        {
            observed.push((index, state));
        }
    }
    Ok(observed)
}

fn state(gamepad: &Gamepad) -> Option<State> {
    let sticks = gamepad.axes();
    let buttons = gamepad.buttons();
    if sticks.length() < 4 || buttons.length() < 16 {
        return None;
    }
    let mut axes = [0.0f64; 6];
    let mut pressed = [false; BUTTONS.len()];
    for (axis, index) in axes.iter_mut().take(4).zip(0..4) {
        *axis = sticks.get(index).as_f64()?;
    }
    for (axis, index) in axes.iter_mut().skip(4).zip(6..8) {
        *axis = buttons.get(index).dyn_into::<GamepadButton>().ok()?.value();
    }
    for (destination, (index, _)) in pressed.iter_mut().zip(BUTTONS) {
        if index < buttons.length() {
            *destination = buttons
                .get(index)
                .dyn_into::<GamepadButton>()
                .ok()?
                .pressed();
        }
    }
    State::from_values(axes, pressed)
}
