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
//   - Browser canvas scene resources and ordered sprite presentation.
// - Must-Not:
//   - Interpret game actions, asset formats or select presentation geometry.
// - Allows:
//   - Inputs: Admitted atlases, draw commands and scoped browser events.
//   - Outputs: Actual presentation receipts and diagnostic frame pixels.
//   - Side effects: Create canvas resources, draw and subscribe to input.
// - Split-When:
//   - Browser resource ownership differs from scene composition.
// - Merge-When:
//   - A shared browser graphics backend covers these same operations.
// - Summary:
//   - Resident Canvas2D scene diagnostic with shared geometry admission.
// - Description:
//   - Retains atlases and logical targets across separately driven steps.
// - Usage:
//   - Open an isolated diagnostic surface and drive the portable session.
// - Defaults:
//   - Nearest sampling; windowed only; no claimed GPU acceleration.

mod fixture;
mod surface;
mod webgl;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

pub(in crate::software) use fixture::{
    verify_resident_scene, verify_webgl_scene,
};
use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
    Viewport, WindowMode,
};
use js_sys::{Array, Function, Reflect};
use wasm_bindgen::{Clamped, JsCast as _, JsValue};
use web_sys::{
    CanvasRenderingContext2d, Document, HtmlCanvasElement, ImageData,
};

use self::surface::BrowserSurface;
use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::graphics::scene_renderer::SceneBackend;
use crate::ir::hardware::InputSample;
use crate::ir::scene::{SpriteDraw, SpriteRect};
use crate::ir::security::scene::{AdmittedAtlas, AdmittedDraws};

struct Target {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    draw_image: Function,
}

impl Target {
    fn new(document: &Document, size: Dimensions) -> Result<Self, RenderError> {
        let canvas = surface::canvas(document, size)?;
        let context = canvas
            .get_context("2d")
            .map_err(dom_error)?
            .ok_or(RenderError::Unsupported)?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| RenderError::Unsupported)?;
        let draw_image =
            Reflect::get(&context, &JsValue::from_str("drawImage"))
                .map_err(dom_error)?
                .dyn_into::<Function>()
                .map_err(|_| RenderError::Unsupported)?;
        context.set_image_smoothing_enabled(false);
        // Ask for one pixel to exercise the actual allocation before commit.
        let _pixel = context
            .get_image_data(0.0, 0.0, 1.0, 1.0)
            .map_err(dom_error)?;
        Ok(Self {
            canvas,
            context,
            draw_image,
        })
    }

    fn clear(&self, color: [u8; 3]) {
        let [red, green, blue] = color;
        self.context
            .set_fill_style_str(&format!("rgb({red},{green},{blue})"));
        self.context.fill_rect(
            0.0,
            0.0,
            f64::from(self.canvas.width()),
            f64::from(self.canvas.height()),
        );
        self.context.set_image_smoothing_enabled(false);
    }

    fn copy(
        &self,
        source: &HtmlCanvasElement,
        from: [f64; 4],
        to: [f64; 4],
    ) -> Result<(), RenderError> {
        let [sx, sy, sw, sh] = from;
        let [dx, dy, dw, dh] = to;
        let args = Array::new();
        let _length = args.push(source.as_ref());
        for coordinate in [sx, sy, sw, sh, dx, dy, dw, dh] {
            let _length = args.push(&JsValue::from_f64(coordinate));
        }
        let _result = self
            .draw_image
            .apply(&self.context, &args)
            .map_err(dom_error)?;
        Ok(())
    }

    fn sprite(
        &self,
        atlas: &HtmlCanvasElement,
        sprite: SpriteDraw,
    ) -> Result<(), RenderError> {
        let from = sprite_rectangle(sprite.source);
        let to = sprite_rectangle(sprite.destination);
        if !sprite.flip_horizontal && !sprite.flip_vertical {
            return self.copy(atlas, from, to);
        }
        let [x, y, width, height] = to;
        let (x, horizontal) = if sprite.flip_horizontal {
            (x + width, -1.0)
        } else {
            (x, 1.0)
        };
        let (y, vertical) = if sprite.flip_vertical {
            (y + height, -1.0)
        } else {
            (y, 1.0)
        };
        self.context.save();
        let result = (|| {
            self.context.translate(x, y).map_err(dom_error)?;
            self.context
                .scale(horizontal, vertical)
                .map_err(dom_error)?;
            self.copy(atlas, from, [0.0, 0.0, width, height])
        })();
        self.context.restore();
        result
    }

    fn read(&self, width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
        Ok(self
            .context
            .get_image_data(0.0, 0.0, f64::from(width), f64::from(height))
            .map_err(dom_error)?
            .data()
            .0)
    }
}

struct CanvasScene {
    atlases: Vec<HtmlCanvasElement>,
    layout: FrameLayout,
    logical: Target,
    mode: PresentationMode,
    visible: Target,
    surface: BrowserSurface,
}

impl Debug for CanvasScene {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasScene")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl CanvasScene {
    fn new(
        layout: FrameLayout,
        size: Dimensions,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let document = surface::document()?;
        let logical = Target::new(&document, dimensions(layout))?;
        let visible = Target::new(&document, size)?;
        let surface = BrowserSurface::attach(
            document,
            visible.canvas.clone(),
            scope,
            capacity,
        )?;
        Ok(Self {
            atlases: Vec::new(),
            layout,
            logical,
            mode: PresentationMode::AspectFit,
            visible,
            surface,
        })
    }
}

impl SceneBackend for CanvasScene {
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<WindowMode, RenderError> {
        if settings.window != WindowMode::Windowed {
            return Err(RenderError::Unsupported);
        }
        self.mode = settings.presentation;
        Ok(WindowMode::Windowed)
    }

    fn drawable_available(&self) -> bool {
        self.surface.available()
    }

    fn layout(&self) -> FrameLayout {
        self.layout
    }

    fn load_atlas(
        &mut self,
        upload: AdmittedAtlas<'_>,
    ) -> Result<(), RenderError> {
        if upload.id().0 != self.atlases.len() {
            return Err(RenderError::InvalidSprite);
        }
        self.atlases
            .try_reserve(1)
            .map_err(|_| RenderError::TooLarge)?;
        let frame = upload.frame();
        let size = dimensions(frame.layout());
        let atlas = Target::new(self.surface.document(), size)?;
        let data = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(frame.pixels()),
            size.width.get(),
            size.height.get(),
        )
        .map_err(dom_error)?;
        atlas
            .context
            .put_image_data(&data, 0.0, 0.0)
            .map_err(dom_error)?;
        self.atlases.push(atlas.canvas);
        Ok(())
    }

    fn poll_event(&mut self, _wait_ms: u32) -> Option<InputSample> {
        self.surface.pop()
    }

    fn present(
        &mut self,
        background: [u8; 3],
        sprites: AdmittedDraws<'_>,
        protected_bottom: Option<NonZeroU32>,
    ) -> Result<Option<PresentationLayout>, RenderError> {
        let Some(size) = self.surface_dimensions()? else {
            return Ok(None);
        };
        let layout = self.layout.presentation_with_protected_bottom(
            self.mode,
            size,
            protected_bottom,
        )?;
        self.logical.clear(background);
        for &sprite in sprites.as_slice() {
            let atlas = self
                .atlases
                .get(sprite.atlas.0)
                .ok_or(RenderError::InvalidSprite)?;
            self.logical.sprite(atlas, sprite)?;
        }
        self.visible.clear([0; 3]);
        for region in layout.regions().into_iter().flatten() {
            self.visible.copy(
                &self.logical.canvas,
                viewport(region.source),
                viewport(region.destination),
            )?;
        }
        Ok(Some(layout))
    }

    fn presentation_mode(&self) -> PresentationMode {
        self.mode
    }

    fn readback(
        &mut self,
        layout: FrameLayout,
    ) -> Result<Vec<u8>, RenderError> {
        self.logical
            .read(layout.width().get(), layout.height().get())
    }

    fn resize_target(
        &mut self,
        layout: FrameLayout,
    ) -> Result<(), RenderError> {
        let target = Target::new(self.surface.document(), dimensions(layout))?;
        self.logical = target;
        self.layout = layout;
        Ok(())
    }

    fn surface_dimensions(&self) -> Result<Option<Dimensions>, RenderError> {
        Ok(self.surface.dimensions())
    }
}

fn dimensions(layout: FrameLayout) -> Dimensions {
    Dimensions::new(layout.width(), layout.height())
}

fn sprite_rectangle(rect: SpriteRect) -> [f64; 4] {
    [
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.width.get()),
        f64::from(rect.height.get()),
    ]
}

fn viewport(rect: Viewport) -> [f64; 4] {
    [
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.width),
        f64::from(rect.height),
    ]
}

fn dom_error(_error: JsValue) -> RenderError {
    RenderError::Backend("browser canvas operation failed".to_owned())
}
