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
//   - Production browser window creation and owned scene transfer.
// - Must-Not:
//   - Block the browser event loop, choose simulation or expose DOM handles.
// - Allows:
//   - Inputs: Portable window requests and admitted scene layouts.
//   - Outputs: Scoped input and independently owned renderer sessions.
//   - Side effects: Attach a document canvas and select a WebGL context.
// - Split-When:
//   - Independent browser window resources need different lifetimes.
// - Merge-When:
//   - Browser scene creation covers the same window capability.
// - Summary:
//   - Select the resident renderer through the ordinary window factory.
// - Description:
//   - Preserve one surface, event scope and context across scene transfer.
// - Usage:
//   - Open, obtain input, configure, then consume the window into a scene.
// - Defaults:
//   - Visible windowed canvas; borrowed blocking callbacks unsupported.

use std::cell::Cell;
use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{Dimensions, PresentationMode, WindowMode};
use web_sys::WebGl2RenderingContext as Gl;

use super::super::dom_error;
use super::super::surface::{self, BrowserSurface, SurfaceKind};
use super::resources::Limits;
use super::{WebGlScene, context, observed_dimensions};
use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::graphics::{
    FrameRun, ResidentScenePort, SceneRun, WindowBackend, WindowPort,
};
use crate::ir::input::InputPort;

thread_local! {
    static NEXT_SCOPE: Cell<u64> = const { Cell::new(1) };
}

struct WebGlWindow {
    gl: Gl,
    limits: Limits,
    surface: BrowserSurface,
}

impl Debug for WebGlWindow {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebGlWindow").finish_non_exhaustive()
    }
}

impl WindowBackend for WebGlWindow {
    fn apply_window_mode(
        &mut self,
        mode: WindowMode,
    ) -> Result<WindowMode, RenderError> {
        match mode {
            WindowMode::Windowed => Ok(WindowMode::Windowed),
            WindowMode::Fullscreen => Err(RenderError::Unsupported),
        }
    }

    fn input(&self, limit: NonZeroUsize) -> Result<InputPort, RenderError> {
        self.surface.input(limit)
    }

    fn into_scene(
        self: Box<Self>,
        layout: FrameLayout,
        mode: PresentationMode,
    ) -> Result<ResidentScenePort, RenderError> {
        Ok(Box::new(WebGlScene::from_window(
            self.surface,
            self.gl,
            self.limits,
            layout,
            mode,
        )?))
    }

    fn renderer_name(&self) -> String {
        "WebGL 2".to_owned()
    }

    fn with_frame(
        &mut self,
        _layout: FrameLayout,
        _mode: PresentationMode,
        _run: &mut FrameRun<'_>,
    ) -> Result<(), RenderError> {
        Err(RenderError::Unsupported)
    }

    fn with_scene(
        &mut self,
        _layout: FrameLayout,
        _mode: PresentationMode,
        _run: &mut SceneRun<'_>,
    ) -> Result<(), RenderError> {
        Err(RenderError::Unsupported)
    }
}

pub(in crate::software) fn open_window(
    title: &str,
    width: NonZeroU32,
    height: NonZeroU32,
    hidden: bool,
) -> Result<WindowPort, RenderError> {
    if hidden {
        return Err(RenderError::Unsupported);
    }
    let document = surface::document()?;
    // Admit real drawable limits before requesting a large backing store.
    let canvas = surface::canvas(
        &document,
        Dimensions::new(NonZeroU32::MIN, NonZeroU32::MIN),
    )?;
    let gl = context(&canvas)?;
    let limits = Limits::observe(&gl)?;
    let _requested = limits.viewport(Dimensions::new(width, height))?;
    canvas
        .set_attribute("aria-label", title)
        .map_err(dom_error)?;
    let scope = NEXT_SCOPE.with(|next| {
        let scope = next.get();
        next.set(scope.checked_add(1).ok_or(RenderError::TooLarge)?);
        Ok(scope)
    })?;
    let capacity = NonZeroUsize::new(1024).ok_or(RenderError::TooLarge)?;
    let surface = BrowserSurface::attach(
        document,
        canvas,
        scope,
        capacity,
        SurfaceKind::Window,
    )?;
    let _observed = observed_dimensions(&gl, limits, &surface)?;
    Ok(Box::new(WebGlWindow { gl, limits, surface }))
}
