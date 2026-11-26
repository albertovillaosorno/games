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
//   - Mouse button transitions from browser pointer observations.
// - Must-Not:
//   - Bind game actions or translate emulated touch as physical mouse.
// - Allows:
//   - Inputs: Scoped mouse pointer events and capture observations.
//   - Outputs: At most five portable mouse button transitions per event.
//   - Side effects: Request capture and suppress handled defaults.
// - Split-When:
//   - Additional pointing devices require different physical identities.
// - Merge-When:
//   - One physical input adapter supplies equivalent mouse observations.
// - Summary:
//   - Preserve native button identities and chording on a browser surface.
// - Description:
//   - Release tracked mouse buttons on cancellation or lost capture.
// - Usage:
//   - Share one state across the owning surface's pointer listeners.
// - Defaults:
//   - Ignore unsupported buttons, ordinary motion and unadmitted chords.

use games::input::physical::PointerButton;
use web_sys::{Element, PointerEvent, PointerEventInit};

use super::{EventSource, next_event, pointer, verify};
use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputEvent, InputTransition};
use crate::ir::input::NativeInput;

// The DOM button index and buttons bitmask are different for middle/right.
const BUTTONS: [(PointerButton, u16); 5] = [
    (PointerButton::Left, 1),
    (PointerButton::Middle, 4),
    (PointerButton::Right, 2),
    (PointerButton::X1, 8),
    (PointerButton::X2, 16),
];

pub(super) type Transitions = [Option<InputTransition>; BUTTONS.len()];

#[derive(Default)]
pub(super) struct MouseButtons {
    held: u16,
}

impl MouseButtons {
    pub(super) fn clear(&mut self) {
        self.held = 0;
    }

    pub(super) fn transition(
        &mut self,
        element: &Element,
        kind: &str,
        event: &PointerEvent,
    ) -> Transitions {
        match kind {
            "pointerdown" | "pointermove" | "pointerup" => {
                let change = self.button_change(element, kind, event);
                if change.is_some() {
                    event.prevent_default();
                }
                [change, None, None, None, None]
            },
            "pointerleave"
                if element.has_pointer_capture(event.pointer_id()) =>
            {
                [None; BUTTONS.len()]
            },
            "pointercancel" | "lostpointercapture" | "pointerleave" => {
                let held = std::mem::take(&mut self.held);
                BUTTONS.map(|(button, mask)| {
                    (held & mask != 0).then(|| change(button, false))
                })
            },
            _ => [None; BUTTONS.len()],
        }
    }

    fn button_change(
        &mut self,
        element: &Element,
        kind: &str,
        event: &PointerEvent,
    ) -> Option<InputTransition> {
        let index = usize::try_from(event.button()).ok()?;
        let &(button, mask) = BUTTONS.get(index)?;
        let down = event.buttons() & mask != 0;
        if (kind == "pointerdown" && !down) || (kind == "pointerup" && down) {
            return None;
        }
        if down {
            if kind == "pointerdown" {
                let _point = pointer::point(element, event)?;
                if !pointer::capture(element, event) {
                    return None;
                }
            } else if self.held == 0
                || (event.is_trusted()
                    && !element.has_pointer_capture(event.pointer_id()))
            {
                return None;
            }
            self.held |= mask;
        } else {
            if self.held & mask == 0 {
                return None;
            }
            self.held &= !mask;
        }
        Some(change(button, down))
    }
}

fn event(button: PointerButton, down: bool) -> HostEvent {
    HostEvent::Input(InputEvent::Mouse {
        code: button.code(),
        down,
    })
}

fn change(button: PointerButton, down: bool) -> InputTransition {
    InputTransition::Event(event(button, down))
}

fn send(
    element: &Element,
    kind: &str,
    button: i16,
    buttons: u16,
) -> Result<bool, RenderError> {
    let init = PointerEventInit::new();
    init.set_pointer_type("mouse");
    init.set_pointer_id(1);
    init.set_client_x(110);
    init.set_client_y(55);
    init.set_button(button);
    init.set_buttons(buttons);
    init.set_cancelable(true);
    pointer::dispatch(element, kind, &init)
}

pub(super) fn verify_source(
    source: &EventSource,
    element: &Element,
    input: &mut NativeInput,
) -> Result<(), RenderError> {
    for (index, (button, mask)) in BUTTONS.into_iter().enumerate() {
        let index = i16::try_from(index).map_err(|_| RenderError::TooLarge)?;
        for (kind, buttons, down) in
            [("pointerdown", mask, true), ("pointerup", 0, false)]
        {
            verify(!send(element, kind, index, buttons)?, "mouse default")?;
            verify(
                next_event(source, input)? == Some(event(button, down)),
                "mouse physical identity",
            )?;
        }
    }
    for (kind, index, mask, button, down) in [
        ("pointerdown", 0, 1, PointerButton::Left, true),
        ("pointermove", 2, 3, PointerButton::Right, true),
        ("pointermove", 0, 2, PointerButton::Left, false),
        ("pointerup", 2, 0, PointerButton::Right, false),
    ] {
        verify(!send(element, kind, index, mask)?, "chord default")?;
        verify(
            next_event(source, input)? == Some(event(button, down)),
            "mouse chord transition",
        )?;
    }
    for (kind, index, mask) in [
        ("pointermove", -1, 0),
        ("pointerdown", 5, 32),
        ("pointermove", 2, 2),
    ] {
        verify(
            send(element, kind, index, mask)?
                && next_event(source, input)?.is_none(),
            "unadmitted mouse observation",
        )?;
    }
    verify_cancellation(source, element, input)?;
    let _sent = send(element, "pointerdown", 0, 1)?;
    verify(
        next_event(source, input)? == Some(event(PointerButton::Left, true)),
        "mouse blur setup",
    )?;
    let blur = web_sys::Event::new("blur").map_err(super::dom_error)?;
    let _sent = element.dispatch_event(&blur).map_err(super::dom_error)?;
    verify(
        next_event(source, input)?
            == Some(HostEvent::Input(InputEvent::FocusLost)),
        "mouse blur cancellation",
    )?;
    let _sent = send(element, "lostpointercapture", -1, 0)?;
    verify(next_event(source, input)?.is_none(), "stale mouse capture")
}

fn verify_cancellation(
    source: &EventSource,
    element: &Element,
    input: &mut NativeInput,
) -> Result<(), RenderError> {
    for kind in ["pointercancel", "lostpointercapture", "pointerleave"] {
        let _sent = send(element, "pointerdown", 0, 1)?;
        let _sent = send(element, "pointermove", 1, 5)?;
        for button in [PointerButton::Left, PointerButton::Middle] {
            verify(
                next_event(source, input)? == Some(event(button, true)),
                "mouse cancellation setup",
            )?;
        }
        let _sent = send(element, kind, -1, 0)?;
        for button in [PointerButton::Left, PointerButton::Middle] {
            verify(
                next_event(source, input)? == Some(event(button, false)),
                "mouse cancellation release",
            )?;
        }
        verify(next_event(source, input)?.is_none(), "extra mouse release")?;
    }
    Ok(())
}
