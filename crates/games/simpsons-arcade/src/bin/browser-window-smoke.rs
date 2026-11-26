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
//   - Explicit production window ownership and geometry smoke.
// - Must-Not:
//   - Open native APIs or depend on synthetic host adapters.
// - Allows:
//   - Inputs: Visible window and procedural atlas.
//   - Outputs: Verified resident presentation or typed failure.
//   - Side effects: Delegated window, input and scene operations.
// - Split-When:
//   - Separate capabilities require independent smoke lifetimes.
// - Merge-When:
//   - A production playable smoke exercises the same window operations.
// - Summary:
//   - Exercise the ordinary window factory without original assets.
// - Description:
//   - Present twice across independent owned scene calls.
// - Usage:
//   - Invoke in a visible browser document.
// - Defaults:
//   - Windowed, procedural pixels and no persistent changes.

//! Production resident window smoke.

use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{DisplaySettings, PresentationMode, WindowMode};
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::NativeWindow;
use games_platform::ir::scene::{SceneLimits, SpriteDraw, SpriteRect};

fn main() -> Result<(), RenderError> {
    let two = NonZeroU32::new(2).ok_or(RenderError::InvalidDimensions)?;
    let sixteen = NonZeroUsize::new(16).ok_or(RenderError::InvalidLength)?;
    let layout = FrameLayout::new(two, two, sixteen)?;
    let limits = SceneLimits {
        atlas_bytes: sixteen,
        atlases: NonZeroUsize::MIN,
        sprites: NonZeroUsize::MIN,
    };
    let mut window =
        NativeWindow::open("Resident window smoke", two, two, false)?;
    window.apply_display_settings(DisplaySettings {
        window: WindowMode::Windowed,
        presentation: PresentationMode::AspectFit,
    })?;
    let mut input = window.input(NonZeroUsize::MIN)?.input;
    input.enable_touch();
    let mut scene = window.into_scene(layout, limits)?;
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let atlas = scene.load_atlas(layout, &pixels)?;
    let rect = SpriteRect {
        x: 0,
        y: 0,
        width: two,
        height: two,
    };
    let draw = SpriteDraw {
        atlas,
        source: rect,
        destination: rect,
        flip_horizontal: false,
        flip_vertical: false,
    };
    for mode in [PresentationMode::AspectFit, PresentationMode::Stretch] {
        scene.apply_display_settings(DisplaySettings {
            window: WindowMode::Windowed,
            presentation: mode,
        })?;
        if !scene.present([0; 3], &[draw])? {
            return Err(RenderError::InvalidDimensions);
        }
        let receipt = scene
            .visible_layout()?
            .ok_or(RenderError::InvalidDimensions)?;
        if receipt != layout.presentation(mode, receipt.surface)? {
            return Err(RenderError::InvalidDimensions);
        }
        let _cancelled = input.synchronize_touch(|| scene.visible_layout())?;
        if scene.readback()? != pixels {
            return Err(RenderError::InvalidLength);
        }
    }
    Ok(())
}
