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
//   - Scoped browser canvases and their input subscriptions.
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
//   - Hidden or empty surfaces are unavailable.

use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::Dimensions;
use wasm_bindgen::JsCast as _;
use web_sys::{Document, HtmlCanvasElement};

use super::dom_error;
use crate::ir::frame::RenderError;
use crate::ir::hardware::InputSample;
use crate::software::web::input::EventSource;

#[derive(Clone, Copy)]
pub(super) enum SurfaceKind {
    #[cfg(feature = "test-fixtures")]
    Diagnostic,
    #[cfg(feature = "interactive")]
    Window,
}

impl SurfaceKind {
    const fn style(self) -> &'static str {
        match self {
            #[cfg(feature = "test-fixtures")]
            Self::Diagnostic => concat!(
                "position:absolute;left:0;top:0;width:160px;height:160px;",
                "image-rendering:pixelated;touch-action:none"
            ),
            #[cfg(feature = "interactive")]
            Self::Window => concat!(
                "position:fixed;inset:0;width:100%;height:100%;",
                "display:block;image-rendering:pixelated;touch-action:none"
            ),
        }
    }
}

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
    kind: SurfaceKind,
}

impl BrowserSurface {
    pub(super) fn attach(
        document: Document,
        canvas: HtmlCanvasElement,
        scope: u64,
        capacity: NonZeroUsize,
        kind: SurfaceKind,
    ) -> Result<Self, RenderError> {
        let attached = Attachment(canvas);
        attached
            .0
            .set_attribute("style", kind.style())
            .map_err(dom_error)?;
        attached
            .0
            .set_attribute("tabindex", "0")
            .map_err(dom_error)?;
        let body = document.body().ok_or(RenderError::Unsupported)?;
        let _node = body.append_child(&attached.0).map_err(dom_error)?;
        let events = EventSource::for_element(&attached.0, scope, capacity)?;
        #[cfg(feature = "interactive")]
        if matches!(kind, SurfaceKind::Window) {
            attached.0.focus().map_err(dom_error)?;
        }
        Ok(Self {
            events,
            attached,
            document,
            kind,
        })
    }

    pub(super) fn available(&self) -> bool {
        self.attached.0.is_connected() && !self.document.hidden()
    }

    pub(super) fn dimensions(&self) -> Result<Option<Dimensions>, RenderError> {
        if !self.available() {
            return Ok(None);
        }
        let canvas = &self.attached.0;
        let rect = canvas.get_bounding_client_rect();
        if !rect.width().is_finite() || !rect.height().is_finite() {
            return Err(RenderError::InvalidDimensions);
        }
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Ok(None);
        }
        match self.kind {
            #[cfg(feature = "test-fixtures")]
            SurfaceKind::Diagnostic => Ok(NonZeroU32::new(canvas.width())
                .zip(NonZeroU32::new(canvas.height()))
                .map(|(width, height)| Dimensions::new(width, height))),
            #[cfg(feature = "interactive")]
            SurfaceKind::Window => {
                let ratio = self
                    .document
                    .default_view()
                    .ok_or(RenderError::Unsupported)?
                    .device_pixel_ratio();
                let width = physical_extent(rect.width(), ratio)?;
                let height = physical_extent(rect.height(), ratio)?;
                Ok(Some(Dimensions::new(width, height)))
            },
        }
    }

    pub(super) const fn element(&self) -> &HtmlCanvasElement {
        &self.attached.0
    }

    #[cfg(feature = "test-fixtures")]
    pub(super) const fn document(&self) -> &Document {
        &self.document
    }

    #[cfg(feature = "interactive")]
    pub(super) fn input(
        &self,
        limit: NonZeroUsize,
    ) -> Result<crate::ir::input::InputPort, RenderError> {
        self.events.input(&self.attached.0, limit)
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

#[cfg(feature = "interactive")]
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positive finite device pixels are admitted before conversion"
)]
fn physical_extent(css: f64, ratio: f64) -> Result<NonZeroU32, RenderError> {
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(RenderError::InvalidDimensions);
    }
    let pixels = (css * ratio).round().max(1.0);
    if !(1.0..=f64::from(u32::MAX)).contains(&pixels) {
        return Err(RenderError::InvalidDimensions);
    }
    NonZeroU32::new(pixels as u32).ok_or(RenderError::InvalidDimensions)
}
