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
//   - Browser touch observations scoped to one presented element.
// - Must-Not:
//   - Own logical actions, presentation transforms or contact budgets.
// - Allows:
//   - Inputs: DOM pointer events and the element's client rectangle.
//   - Outputs: Normalized portable contact transitions.
//   - Side effects: Suppress touch defaults and request pointer capture.
// - Split-When:
//   - Different pointer devices require independent lifecycle handling.
// - Merge-When:
//   - The host provides a single equivalent contact observation source.
// - Summary:
//   - Admit CSS-relative contact positions without assuming backing pixels.
// - Description:
//   - Cancel a contact on release, cancellation or capture loss.
// - Usage:
//   - Attach to the same element as the owning input and scene session.
// - Defaults:
//   - Ignore other pointer types and presses outside the client rectangle.

use std::num::{NonZeroU32, NonZeroUsize};

use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use games::presentation::{Dimensions, PresentationMode};
use wasm_bindgen::JsCast as _;
use web_sys::{Element, Event, PointerEvent, PointerEventInit};

use super::{BrowserInput, EventSource, SCOPE, dom_error, next_event, verify};
use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputEvent, InputTransition};
use crate::ir::input::InputStartup;

fn contact(id: i32, phase: ContactPhase) -> HostEvent {
    HostEvent::Input(InputEvent::Contact {
        id: ContactId {
            device: 0,
            finger: i64::from(id),
        },
        phase,
    })
}

fn point(element: &Element, event: &PointerEvent) -> Option<TouchPoint> {
    let rect = element.get_bounding_client_rect();
    let width = rect.width();
    let height = rect.height();
    if !(width.is_finite()
        && height.is_finite()
        && width > 0.0f64
        && height > 0.0f64)
    {
        return None;
    }
    TouchPoint::from_normalized(
        (f64::from(event.client_x()) - rect.left()) / width,
        (f64::from(event.client_y()) - rect.top()) / height,
    )
}

fn transition(
    element: &Element,
    kind: &str,
    event: &Event,
) -> Option<InputTransition> {
    let pointer = event.dyn_ref::<PointerEvent>()?;
    if pointer.pointer_type() != "touch" {
        return None;
    }
    let id = pointer.pointer_id();
    let phase = match kind {
        "pointerdown" => {
            let position = point(element, pointer)?;
            if event.is_trusted()
                && !element.has_pointer_capture(id)
                && element.set_pointer_capture(id).is_err()
            {
                return None;
            }
            ContactPhase::Down(position)
        },
        "pointermove" => ContactPhase::Motion(point(element, pointer)),
        "pointerleave" if element.has_pointer_capture(id) => return None,
        "pointerup" | "pointercancel" | "lostpointercapture"
        | "pointerleave" => ContactPhase::Up,
        _ => return None,
    };
    event.prevent_default();
    Some(InputTransition::Event(contact(id, phase)))
}

fn subscribe(
    source: &mut EventSource,
    element: &Element,
) -> Result<(), RenderError> {
    for kind in [
        "pointerdown",
        "pointermove",
        "pointerup",
        "pointercancel",
        "lostpointercapture",
        "pointerleave",
    ] {
        let canvas = element.clone();
        source.listen(element, kind, move |event| {
            transition(&canvas, kind, event)
        })?;
    }
    Ok(())
}

// The diagnostic owns a real, attached CSS-sized surface independently of
// the game or user document. Removal is guaranteed on normal error returns.
struct Surface(Element);

impl Drop for Surface {
    fn drop(&mut self) {
        self.0.remove();
    }
}

impl Surface {
    fn new() -> Result<Self, RenderError> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(RenderError::Unsupported)?;
        let canvas = document.create_element("canvas").map_err(dom_error)?;
        canvas.set_attribute("width", "360").map_err(dom_error)?;
        canvas.set_attribute("height", "180").map_err(dom_error)?;
        canvas
            .set_attribute(
                "style",
                concat!(
                    "position:absolute;left:20px;top:10px;",
                    "width:180px;height:90px;touch-action:none"
                ),
            )
            .map_err(dom_error)?;
        let body = document.body().ok_or(RenderError::Unsupported)?;
        let _node = body.append_child(&canvas).map_err(dom_error)?;
        Ok(Self(canvas))
    }
}

fn send_pointer(
    element: &Element,
    kind: &str,
    pointer_type: &str,
    id: i32,
    position: [i32; 2],
) -> Result<bool, RenderError> {
    let init = PointerEventInit::new();
    init.set_pointer_type(pointer_type);
    init.set_pointer_id(id);
    init.set_client_x(position[0]);
    init.set_client_y(position[1]);
    init.set_cancelable(true);
    let event = PointerEvent::new_with_event_init_dict(kind, &init)
        .map_err(dom_error)?;
    element.dispatch_event(&event).map_err(dom_error)
}

pub(super) fn verify_touch_source() -> Result<(), RenderError> {
    let surface = Surface::new()?;
    let canvas = &surface.0;
    let capacity = NonZeroUsize::MIN.saturating_add(7);
    let mut source = EventSource::new(canvas, SCOPE, capacity)?;
    subscribe(&mut source, canvas)?;
    let mut input = InputStartup::new(
        Box::new(BrowserInput { scope: SCOPE }),
        NonZeroUsize::MIN,
    )?
    .input;
    input.enable_touch();
    let size = Dimensions::new(
        NonZeroU32::new(360).ok_or(RenderError::InvalidLength)?,
        NonZeroU32::new(180).ok_or(RenderError::InvalidLength)?,
    );
    let layout = PresentationMode::AspectFit.layout(size, size);
    verify(
        input.synchronize_touch(|| Ok(Some(layout)))?.is_none(),
        "initial browser touch geometry",
    )?;
    let first = TouchPoint { x: 0, y: 0 };
    let center = TouchPoint { x: 32_768, y: 32_768 };
    let last = TouchPoint { x: u16::MAX, y: u16::MAX };
    for (kind, id, position, phase) in [
        ("pointerdown", 3, [20, 10], ContactPhase::Down(first)),
        ("pointerdown", 7, [200, 100], ContactPhase::Down(last)),
        (
            "pointermove",
            3,
            [110, 55],
            ContactPhase::Motion(Some(center)),
        ),
        ("pointermove", 7, [201, 100], ContactPhase::Motion(None)),
        ("pointercancel", 3, [-1, -1], ContactPhase::Up),
        ("pointerup", 7, [-1, -1], ContactPhase::Up),
        ("pointerdown", 3, [110, 55], ContactPhase::Down(center)),
        ("lostpointercapture", 3, [-1, -1], ContactPhase::Up),
        ("pointerdown", 3, [20, 10], ContactPhase::Down(first)),
        ("pointerleave", 3, [-1, -1], ContactPhase::Up),
    ] {
        verify(
            !send_pointer(canvas, kind, "touch", id, position)?,
            "touch default",
        )?;
        verify(
            next_event(&source, &mut input)? == Some(contact(id, phase)),
            "CSS-relative contact transition",
        )?;
    }
    verify(
        send_pointer(canvas, "pointerdown", "touch", 3, [19, 10])?
            && next_event(&source, &mut input)?.is_none(),
        "out-of-surface touch press",
    )?;
    verify(
        send_pointer(canvas, "pointerdown", "mouse", 3, [110, 55])?
            && next_event(&source, &mut input)?.is_none(),
        "foreign pointer kind",
    )?;
    // A sibling surface never contributes to the scoped source.
    let sibling = Surface::new()?;
    verify(
        send_pointer(&sibling.0, "pointerdown", "touch", 3, [110, 55])?
            && next_event(&source, &mut input)?.is_none(),
        "foreign touch surface",
    )?;
    drop(source);
    verify(
        send_pointer(canvas, "pointerdown", "touch", 3, [110, 55])?,
        "touch listeners removed",
    )
}
