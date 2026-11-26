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
//   - Scoped diagnostic browser canvases and their input subscriptions.
// - Must-Not:
//   - Select rendering APIs or interpret presentation policy.
// - Allows:
//   - Inputs: Explicit canvas dimensions and a bounded event capacity.
//   - Outputs: Attached canvas lifetime and actual drawable observations.
//   - Side effects: Attach and detach a canvas; register scoped events.
// - Split-When:
//   - Production hosts need distinct surface ownership policy.
// - Merge-When:
//   - A shared browser window covers the same surface operations.
// - Summary:
//   - One canvas attachment and event lifetime for scene backends.
// - Description:
//   - Keeps diagnostic surfaces and subscriptions isolated.
// - Usage:
//   - Allocate a canvas, then attach it after renderer admission.
// - Defaults:
//   - Diagnostic surface only; hidden or empty surfaces are unavailable.

use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::Dimensions;
use wasm_bindgen::JsCast as _;
use web_sys::{Document, HtmlCanvasElement};

use super::dom_error;
use crate::ir::frame::RenderError;
use crate::ir::hardware::InputSample;
use crate::software::web::input::EventSource;

struct Attachment(HtmlCanvasElement);

impl Drop for Attachment {
    fn drop(&mut self) {
        self.0.remove();
    }
}

pub(super) struct BrowserSurface {
    events: EventSource,
    attached: Attachment,
    document: Document,
}

impl BrowserSurface {
    pub(super) fn attach(
        document: Document,
        canvas: HtmlCanvasElement,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let attached = Attachment(canvas);
        attached
            .0
            .set_attribute(
                "style",
                concat!(
                    "position:absolute;left:0;top:0;width:160px;height:160px;",
                    "image-rendering:pixelated;touch-action:none"
                ),
            )
            .map_err(dom_error)?;
        attached
            .0
            .set_attribute("tabindex", "0")
            .map_err(dom_error)?;
        let body = document.body().ok_or(RenderError::Unsupported)?;
        let _node = body.append_child(&attached.0).map_err(dom_error)?;
        let events = EventSource::for_element(&attached.0, scope, capacity)?;
        Ok(Self {
            events,
            attached,
            document,
        })
    }

    pub(super) fn available(&self) -> bool {
        self.attached.0.is_connected() && !self.document.hidden()
    }

    pub(super) fn dimensions(&self) -> Option<Dimensions> {
        if !self.available() {
            return None;
        }
        let canvas = &self.attached.0;
        let rect = canvas.get_bounding_client_rect();
        if !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() <= 0.0
            || rect.height() <= 0.0
        {
            return None;
        }
        NonZeroU32::new(canvas.width())
            .zip(NonZeroU32::new(canvas.height()))
            .map(|(width, height)| Dimensions::new(width, height))
    }

    pub(super) const fn element(&self) -> &HtmlCanvasElement {
        &self.attached.0
    }

    pub(super) const fn document(&self) -> &Document {
        &self.document
    }

    pub(super) fn pop(&self) -> Option<InputSample> {
        self.events.pop()
    }
}

pub(super) fn canvas(
    document: &Document,
    size: Dimensions,
) -> Result<HtmlCanvasElement, RenderError> {
    let canvas = document
        .create_element("canvas")
        .map_err(dom_error)?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| RenderError::Unsupported)?;
    canvas.set_width(size.width.get());
    canvas.set_height(size.height.get());
    Ok(canvas)
}

pub(super) fn document() -> Result<Document, RenderError> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or(RenderError::Unsupported)
}
