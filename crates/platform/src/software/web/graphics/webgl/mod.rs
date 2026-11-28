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
//   - WebGL scene rendering and physical drawable observation.
// - Must-Not:
//   - Choose game actions, title geometry or portable presentation policy.
// - Allows:
//   - Inputs: Admitted atlases, ordered draws and display settings.
//   - Outputs: Actual drawable receipts and logical RGBA readback.
//   - Side effects: Submit texture uploads, GPU draws and framebuffer copies.
// - Split-When:
//   - An independent GPU resource or presentation lifetime emerges.
// - Merge-When:
//   - Another browser GPU scene owns the same capability.
// - Summary:
//   - Resident WebGL 2 atlas scene with portable layout admission.
// - Description:
//   - Reuses immutable textures and one framebuffer across frames.
// - Usage:
//   - Drive an owned scene session and inspect the actual drawing buffer.
// - Defaults:
//   - Windowed, nearest sampling, no silent Canvas2D fallback.

mod resources;
#[cfg(feature = "interactive")]
mod window;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
    Viewport, WindowMode,
};
use wasm_bindgen::JsCast as _;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlContextAttributes,
};
#[cfg(feature = "interactive")]
pub(in crate::software) use window::open_window;

use self::resources::{
    Limits, Program, Target, Texture, checked, dimensions, signed,
};
use super::dom_error;
#[cfg(feature = "test-fixtures")]
use super::surface;
use super::surface::BrowserSurface;
#[cfg(feature = "test-fixtures")]
use super::surface::SurfaceKind;
use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::graphics::scene_renderer::SceneBackend;
use crate::ir::hardware::InputSample;
use crate::ir::security::scene::{AdmittedAtlas, AdmittedDraws};

pub(super) struct WebGlScene {
    atlases: Vec<Texture>,
    target: Target,
    program: Program,
    gl: Gl,
    limits: Limits,
    layout: FrameLayout,
    mode: PresentationMode,
    surface: BrowserSurface,
}

impl Debug for WebGlScene {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebGlScene")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl WebGlScene {
    #[cfg(feature = "test-fixtures")]
    pub(super) fn new(
        layout: FrameLayout,
        size: Dimensions,
        scope: u64,
        capacity: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let document = surface::document()?;
        let canvas = surface::canvas(&document, size)?;
        let gl = context(&canvas)?;
        let limits = Limits::observe(&gl)?;
        let _viewport = limits.viewport(size)?;
        let target = Target::new(&gl, limits, layout)?;
        let program = Program::new(&gl)?;
        let surface = BrowserSurface::attach(
            document,
            canvas,
            scope,
            capacity,
            SurfaceKind::Diagnostic,
        )?;
        let scene = Self {
            atlases: Vec::new(),
            target,
            program,
            gl,
            limits,
            layout,
            mode: PresentationMode::AspectFit,
            surface,
        };
        let observed = scene.surface_dimensions()?;
        if observed.is_some_and(|actual| actual != size) {
            return Err(RenderError::InvalidDimensions);
        }
        Ok(scene)
    }

    #[cfg(feature = "test-fixtures")]
    pub(super) const fn canvas(&self) -> &HtmlCanvasElement {
        self.surface.element()
    }

    #[cfg(feature = "test-fixtures")]
    pub(super) const fn context(&self) -> &Gl {
        &self.gl
    }

    #[cfg(feature = "interactive")]
    fn from_window(
        surface: BrowserSurface,
        gl: Gl,
        limits: Limits,
        layout: FrameLayout,
        mode: PresentationMode,
    ) -> Result<Self, RenderError> {
        let target = Target::new(&gl, limits, layout)?;
        let program = Program::new(&gl)?;
        Ok(Self {
            atlases: Vec::new(),
            target,
            program,
            gl,
            limits,
            layout,
            mode,
            surface,
        })
    }

    fn render(
        &self,
        background: [u8; 3],
        sprites: AdmittedDraws<'_>,
    ) -> Result<(), RenderError> {
        let gl = &self.gl;
        let size = self.limits.viewport(dimensions(self.layout))?;
        let [width, height] = size;
        self.target.bind(Gl::FRAMEBUFFER);
        gl.viewport(0, 0, width, height);
        gl.disable(Gl::DITHER);
        gl.disable(Gl::SCISSOR_TEST);
        gl.enable(Gl::BLEND);
        gl.blend_func_separate(
            Gl::SRC_ALPHA,
            Gl::ONE_MINUS_SRC_ALPHA,
            Gl::ONE,
            Gl::ONE_MINUS_SRC_ALPHA,
        );
        clear(gl, background);
        self.program.activate(size);
        let mut bound = None;
        for sprite in sprites.as_slice() {
            if bound != Some(sprite.atlas) {
                self.atlases
                    .get(sprite.atlas.0)
                    .ok_or(RenderError::InvalidSprite)?
                    .bind();
                bound = Some(sprite.atlas);
            }
            self.program.draw(*sprite)?;
        }
        checked(gl)
    }

    fn copy(&self, layout: PresentationLayout) -> Result<(), RenderError> {
        let gl = &self.gl;
        gl.disable(Gl::BLEND);
        gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, None);
        clear(gl, [0, 0, 0]);
        self.target.bind(Gl::READ_FRAMEBUFFER);
        for region in layout.regions().into_iter().flatten() {
            let [sx0, sy0, sx1, sy1] = edges(region.source, layout.design)?;
            let [dx0, dy0, dx1, dy1] =
                edges(region.destination, layout.surface)?;
            gl.blit_framebuffer(
                sx0,
                sy0,
                sx1,
                sy1,
                dx0,
                dy0,
                dx1,
                dy1,
                Gl::COLOR_BUFFER_BIT,
                Gl::NEAREST,
            );
        }
        gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
        checked(gl)
    }
}

impl SceneBackend for WebGlScene {
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<WindowMode, RenderError> {
        if settings.window != WindowMode::Windowed {
            return Err(RenderError::Unsupported);
        }
        checked(&self.gl)?;
        self.mode = settings.presentation;
        Ok(WindowMode::Windowed)
    }

    fn drawable_available(&self) -> bool {
        self.surface.available() && !self.gl.is_context_lost()
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
        checked(&self.gl)?;
        let texture = Texture::new(
            &self.gl,
            self.limits,
            frame.layout(),
            Some(frame.pixels()),
        )?;
        self.atlases.push(texture);
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
        checked(&self.gl)?;
        let Some(size) = self.surface_dimensions()? else {
            return Ok(None);
        };
        let layout = self.layout.presentation_with_protected_bottom(
            self.mode,
            size,
            protected_bottom,
        )?;
        self.render(background, sprites)?;
        self.copy(layout)?;
        if self.surface_dimensions()? != Some(size) {
            return Ok(None);
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
        if self.layout != layout {
            return Err(RenderError::InvalidDimensions);
        }
        self.target.bind(Gl::READ_FRAMEBUFFER);
        let pixels = read_rgba(&self.gl, layout);
        self.gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, None);
        pixels
    }

    fn replace_atlas(
        &mut self,
        upload: AdmittedAtlas<'_>,
    ) -> Result<(), RenderError> {
        let frame = upload.frame();
        self.atlases
            .get(upload.id().0)
            .ok_or(RenderError::InvalidSprite)?
            .replace(frame.layout(), frame.pixels())
    }

    fn resize_target(
        &mut self,
        layout: FrameLayout,
    ) -> Result<(), RenderError> {
        checked(&self.gl)?;
        let target = Target::new(&self.gl, self.limits, layout)?;
        self.target = target;
        self.layout = layout;
        Ok(())
    }

    fn surface_dimensions(&self) -> Result<Option<Dimensions>, RenderError> {
        observed_dimensions(&self.gl, self.limits, &self.surface)
    }
}

fn context(canvas: &HtmlCanvasElement) -> Result<Gl, RenderError> {
    let options = WebGlContextAttributes::new();
    options.set_alpha(false);
    options.set_antialias(false);
    options.set_depth(false);
    options.set_stencil(false);
    let gl = canvas
        .get_context_with_context_options("webgl2", options.as_ref())
        .map_err(dom_error)?
        .ok_or(RenderError::Unsupported)?
        .dyn_into::<Gl>()
        .map_err(|_| RenderError::Unsupported)?;
    let observed = gl
        .get_context_attributes()
        .ok_or(RenderError::Unsupported)?;
    if observed.get_alpha() != Some(false)
        || observed.get_antialias() != Some(false)
    {
        return Err(RenderError::Unsupported);
    }
    checked(&gl)?;
    Ok(gl)
}

fn clear(gl: &Gl, color: [u8; 3]) {
    let [red, green, blue] = color.map(|channel| f32::from(channel) / 255.0);
    gl.clear_color(red, green, blue, 1.0);
    gl.clear(Gl::COLOR_BUFFER_BIT);
}

fn edges(
    viewport: Viewport,
    size: Dimensions,
) -> Result<[i32; 4], RenderError> {
    let right = viewport
        .x
        .checked_add(viewport.width)
        .filter(|right| *right <= size.width.get())
        .ok_or(RenderError::InvalidDimensions)?;
    let bottom = viewport
        .y
        .checked_add(viewport.height)
        .ok_or(RenderError::InvalidDimensions)?;
    let low = size
        .height
        .get()
        .checked_sub(bottom)
        .ok_or(RenderError::InvalidDimensions)?;
    let high = size
        .height
        .get()
        .checked_sub(viewport.y)
        .ok_or(RenderError::InvalidDimensions)?;
    Ok([
        signed(viewport.x)?,
        signed(low)?,
        signed(right)?,
        signed(high)?,
    ])
}

pub(super) fn read_rgba(
    gl: &Gl,
    layout: FrameLayout,
) -> Result<Vec<u8>, RenderError> {
    checked(gl)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(layout.bytes())
        .map_err(|_| RenderError::TooLarge)?;
    pixels.resize(layout.bytes(), 0);
    gl.read_pixels_with_opt_u8_array(
        0,
        0,
        signed(layout.width().get())?,
        signed(layout.height().get())?,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(&mut pixels),
    )
    .map_err(dom_error)?;
    checked(gl)?;
    // WebGL readPixels starts at the bottom; the shared frame starts at top.
    let pitch = layout.pitch();
    let middle = (layout.bytes() / pitch / 2) * pitch;
    let (upper, lower) = pixels.split_at_mut(middle);
    for (first, last) in upper
        .chunks_exact_mut(pitch)
        .zip(lower.rchunks_exact_mut(pitch))
    {
        first.swap_with_slice(last);
    }
    Ok(pixels)
}

fn observed_dimensions(
    gl: &Gl,
    limits: Limits,
    surface: &BrowserSurface,
) -> Result<Option<Dimensions>, RenderError> {
    checked(gl)?;
    let Some(size) = surface.dimensions()? else {
        return Ok(None);
    };
    let [width, height] = limits.viewport(size)?;
    // Bound the default drawing buffer independently of logical atlas bytes.
    // A drawable can be smaller than the CSS extent on a limited device.
    let max_bytes = NonZeroUsize::new(64 * 1024 * 1024)
        .ok_or(RenderError::InvalidDimensions)?;
    let _layout = FrameLayout::new(size.width, size.height, max_bytes)?;
    let canvas = surface.element();
    let width_changed = canvas.width() != size.width.get();
    let height_changed = canvas.height() != size.height.get();
    if width_changed && size.width.get() < canvas.width() {
        canvas.set_width(size.width.get());
        if height_changed {
            canvas.set_height(size.height.get());
        }
    } else {
        if height_changed {
            canvas.set_height(size.height.get());
        }
        if width_changed {
            canvas.set_width(size.width.get());
        }
    }
    checked(gl)?;
    if gl.drawing_buffer_width() != width
        || gl.drawing_buffer_height() != height
    {
        return Err(RenderError::InvalidDimensions);
    }
    Ok(Some(size))
}
