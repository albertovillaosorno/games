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
//   - Isolated browser scene and input acceptance observations.
// - Must-Not:
//   - Open original game assets or claim actual gameplay acceptance.
// - Allows:
//   - Inputs: Procedural atlas and diagnostic canvas events.
//   - Outputs: Verified logical and drawable pixels and geometry receipts.
//   - Side effects: Create and remove one scoped diagnostic surface.
// - Split-When:
//   - Independent browser capabilities require separate fixtures.
// - Merge-When:
//   - A shared renderer fixture covers identical acceptance observations.
// - Summary:
//   - Exercise independently owned browser scene resources across steps.
// - Description:
//   - Checks presentation geometry against the actual drawable and input.
// - Usage:
//   - Invoke in an isolated visible browser document.
// - Defaults:
//   - Synthetic pixels and input only.

use std::num::{NonZeroU32, NonZeroUsize};

use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use games::presentation::{
    Dimensions, DisplaySettings, PresentationMode, WindowMode,
};
use web_sys::{HtmlCanvasElement, PointerEvent, PointerEventInit};

use super::canvas::CanvasScene;
use super::dom_error;
use super::webgl::{WebGlScene, read_rgba};
use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::graphics::scene_renderer::{SceneBackend, SceneSession};
use crate::ir::hardware::{HostEvent, InputEvent};
use crate::ir::input::{InputStartup, NativeInput};
use crate::ir::scene::{EventBatch, SceneLimits, SpriteDraw, SpriteRect};
use crate::software::web::input;

const SCOPE: u64 = 11;
const BACKGROUND: [u8; 3] = [10, 20, 30];

pub(in crate::software) fn verify_resident_scene() -> Result<(), RenderError> {
    let base = layout(4, 2)?;
    let surface = Dimensions::new(nonzero(8)?, nonzero(8)?);
    let backend = CanvasScene::new(
        base,
        surface,
        SCOPE,
        NonZeroUsize::new(8).ok_or(RenderError::InvalidLength)?,
    )?;
    let canvas = backend.visible.canvas.clone();
    let context = backend.visible.context.clone();
    verify_scene(Box::new(backend), &canvas, |surface| {
        Ok(context
            .get_image_data(
                0.0,
                0.0,
                f64::from(surface.width().get()),
                f64::from(surface.height().get()),
            )
            .map_err(dom_error)?
            .data()
            .0)
    })
}

pub(in crate::software) fn verify_webgl_scene() -> Result<(), RenderError> {
    let base = layout(4, 2)?;
    let surface = Dimensions::new(nonzero(8)?, nonzero(8)?);
    let backend = WebGlScene::new(
        base,
        surface,
        SCOPE,
        NonZeroUsize::new(8).ok_or(RenderError::InvalidLength)?,
    )?;
    let canvas = backend.canvas().clone();
    let context = backend.context().clone();
    verify_scene(Box::new(backend), &canvas, |surface| {
        read_rgba(&context, surface)
    })
}

fn verify_scene<Read>(
    backend: Box<dyn SceneBackend>,
    canvas: &HtmlCanvasElement,
    read: Read,
) -> Result<(), RenderError>
where
    Read: Fn(FrameLayout) -> Result<Vec<u8>, RenderError>,
{
    let base = layout(4, 2)?;
    let surface = Dimensions::new(nonzero(8)?, nonzero(8)?);
    let limits = SceneLimits {
        atlas_bytes: NonZeroUsize::new(16).ok_or(RenderError::InvalidLength)?,
        atlases: NonZeroUsize::MIN,
        sprites: NonZeroUsize::new(3).ok_or(RenderError::InvalidLength)?,
    };
    let mut scene = SceneSession::resident(
        backend,
        base,
        limits,
        PresentationMode::AspectFit,
    )?;
    let atlas = scene.load_atlas(layout(2, 2)?, &[
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0, 0, 255, 128,
    ])?;
    let sprite = SpriteDraw {
        atlas,
        source: rectangle(0, 0, 2, 1)?,
        destination: rectangle(0, 0, 2, 1)?,
        flip_horizontal: false,
        flip_vertical: false,
    };
    let sprites = [
        sprite,
        SpriteDraw {
            destination: rectangle(2, 0, 2, 1)?,
            flip_horizontal: true,
            ..sprite
        },
        SpriteDraw {
            source: rectangle(0, 1, 2, 1)?,
            destination: rectangle(-1, 1, 2, 1)?,
            ..sprite
        },
    ];
    verify(
        scene.visible_layout()?.is_none(),
        "initial browser scene geometry",
    )?;
    verify(
        scene.present(BACKGROUND, &sprites)?,
        "browser scene presentation",
    )?;
    let receipt = base.presentation(PresentationMode::AspectFit, surface)?;
    verify(
        scene.visible_layout()? == Some(receipt),
        "browser scene receipt",
    )?;
    verify_pixels(&scene.readback()?)?;
    let surface_layout = layout(8, 8)?;
    verify_surface(&read(surface_layout)?, surface_layout)?;
    verify_order(&mut scene, sprite)?;
    verify(
        scene.present(BACKGROUND, &sprites)?,
        "restore contact scene",
    )?;
    let mut input =
        InputStartup::new(input::browser_input(SCOPE), NonZeroUsize::MIN)?
            .input;
    input.enable_touch();
    verify_contact(&mut scene, &mut input, canvas)?;
    verify_resize(&mut scene, &mut input, canvas, &sprites)?;
    verify_protected(&mut scene, canvas, &sprites, &read)?;
    drop(scene);
    verify(!canvas.is_connected(), "resident browser surface cleanup")
}

fn verify_pixels(pixels: &[u8]) -> Result<(), RenderError> {
    let upper = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255,
    ];
    verify(
        pixels.get(..16) == Some(upper.as_slice()),
        "ordered flipped sprites",
    )?;
    let blend = pixels.get(16..20).ok_or(RenderError::InvalidLength)?;
    verify(
        blend
            .iter()
            .zip([5, 10, 143, 255])
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1),
        "clipped alpha sprite",
    )?;
    let background = [10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255];
    verify(
        pixels.get(20..) == Some(background.as_slice()),
        "opaque logical background",
    )
}

fn verify_surface(
    pixels: &[u8],
    surface: FrameLayout,
) -> Result<(), RenderError> {
    for (position, expected) in [
        ([0, 0], [0, 0, 0, 255]),
        ([0, 2], [255, 0, 0, 255]),
        ([2, 2], [0, 255, 0, 255]),
        ([6, 2], [255, 0, 0, 255]),
        ([0, 7], [0, 0, 0, 255]),
    ] {
        verify(
            pixel(pixels, surface, position)? == expected,
            "actual drawable letterbox pixels",
        )?;
    }
    Ok(())
}

fn verify_contact(
    scene: &mut SceneSession<'_, '_>,
    input: &mut NativeInput,
    canvas: &HtmlCanvasElement,
) -> Result<(), RenderError> {
    let _cancel = input.synchronize_touch(|| scene.visible_layout())?;
    let init = PointerEventInit::new();
    init.set_pointer_type("touch");
    init.set_pointer_id(3);
    init.set_client_x(80);
    init.set_client_y(80);
    init.set_cancelable(true);
    let event = PointerEvent::new_with_event_init_dict("pointerdown", &init)
        .map_err(dom_error)?;
    verify(
        !canvas.dispatch_event(&event).map_err(dom_error)?,
        "resident pointer default",
    )?;
    let expected = HostEvent::Input(InputEvent::Contact {
        id: ContactId { device: 0, finger: 3 },
        phase: ContactPhase::Down(TouchPoint { x: 32_768, y: 32_768 }),
    });
    let batch =
        scene.dispatch_events(NonZeroUsize::MIN, 0, |view, sample| {
            verify(view.visible_layout()?.is_some(), "event scene receipt")?;
            verify(
                input.event_elapsed(sample).is_some(),
                "resident event clock",
            )?;
            verify(
                input.translate(sample) == Some(expected),
                "resident pointer admission",
            )?;
            Ok(true)
        })?;
    verify(batch == EventBatch::Limited, "resident event batch")
}

fn verify_resize(
    scene: &mut SceneSession<'_, '_>,
    input: &mut NativeInput,
    canvas: &HtmlCanvasElement,
    sprites: &[SpriteDraw],
) -> Result<(), RenderError> {
    canvas.set_width(12);
    canvas.set_height(4);
    verify(
        scene.visible_layout()?.is_none(),
        "stale browser drawable receipt",
    )?;
    verify(
        input
            .synchronize_touch(|| scene.visible_layout())?
            .is_some(),
        "resize cancels touch",
    )?;
    scene.apply_display_settings(DisplaySettings {
        presentation: PresentationMode::Adaptive,
        window: WindowMode::Windowed,
    })?;
    let maximum = layout(6, 2)?;
    verify(
        scene.fit_adaptive_width(maximum)? == maximum,
        "browser adaptive target",
    )?;
    verify(
        scene.present(BACKGROUND, sprites)?,
        "resident atlas after resize",
    )?;
    verify(
        scene.visible_layout()?.is_some_and(|receipt| {
            receipt.design.width == maximum.width()
                && receipt.surface.width.get() == 12
        }),
        "renewed actual drawable receipt",
    )?;
    verify_contact(scene, input, canvas)?;
    verify(
        scene.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Fill,
            window: WindowMode::Fullscreen,
        }) == Err(RenderError::Unsupported),
        "unverified fullscreen rejected",
    )
}

fn layout(width: u32, height: u32) -> Result<FrameLayout, RenderError> {
    FrameLayout::new(nonzero(width)?, nonzero(height)?, NonZeroUsize::MAX)
}
fn rectangle(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<SpriteRect, RenderError> {
    Ok(SpriteRect {
        x,
        y,
        width: nonzero(width)?,
        height: nonzero(height)?,
    })
}
fn nonzero(value: u32) -> Result<NonZeroU32, RenderError> {
    NonZeroU32::new(value).ok_or(RenderError::InvalidDimensions)
}
fn verify(condition: bool, message: &str) -> Result<(), RenderError> {
    if condition {
        Ok(())
    } else {
        Err(RenderError::Backend(message.to_owned()))
    }
}

fn pixel(
    pixels: &[u8],
    layout: FrameLayout,
    [x, y]: [u32; 2],
) -> Result<&[u8], RenderError> {
    if x >= layout.width().get() || y >= layout.height().get() {
        return Err(RenderError::InvalidDimensions);
    }
    let x = usize::try_from(x).map_err(|_| RenderError::InvalidDimensions)?;
    let y = usize::try_from(y).map_err(|_| RenderError::InvalidDimensions)?;
    let start = y * layout.pitch() + x * 4;
    pixels
        .get(start..start + 4)
        .ok_or(RenderError::InvalidLength)
}

fn verify_order(
    scene: &mut SceneSession<'_, '_>,
    sprite: SpriteDraw,
) -> Result<(), RenderError> {
    let draws = [
        SpriteDraw {
            destination: rectangle(0, 0, 4, 2)?,
            ..sprite
        },
        SpriteDraw {
            source: rectangle(1, 0, 1, 2)?,
            destination: rectangle(1, 0, 2, 2)?,
            flip_vertical: true,
            ..sprite
        },
        SpriteDraw {
            source: rectangle(0, 1, 1, 1)?,
            destination: rectangle(0, 0, 4, 2)?,
            ..sprite
        },
    ];
    verify(scene.present(BACKGROUND, &draws)?, "ordered scene")?;
    let pixels = scene.readback()?;
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let expected = [
        red,
        [127, 0, 128, 255],
        [0, 127, 128, 255],
        green,
        red,
        green,
        green,
        green,
    ];
    verify(
        pixels
            .iter()
            .zip(expected.into_iter().flatten())
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1),
        "ordered scaling, vertical flip and alpha",
    )
}

fn verify_protected<Read>(
    scene: &mut SceneSession<'_, '_>,
    canvas: &HtmlCanvasElement,
    sprites: &[SpriteDraw],
    read: &Read,
) -> Result<(), RenderError>
where
    Read: Fn(FrameLayout) -> Result<Vec<u8>, RenderError>,
{
    canvas.set_width(8);
    canvas.set_height(8);
    scene.apply_display_settings(DisplaySettings {
        presentation: PresentationMode::Fill,
        window: WindowMode::Windowed,
    })?;
    let base = layout(4, 2)?;
    verify(
        scene.fit_adaptive_width(layout(6, 2)?)? == base,
        "restored logical target",
    )?;
    scene.protect_bottom(Some(NonZeroU32::MIN))?;
    verify(scene.present(BACKGROUND, sprites)?, "protected fill")?;
    let size = Dimensions::new(nonzero(8)?, nonzero(8)?);
    let expected = base.presentation_with_protected_bottom(
        PresentationMode::Fill,
        size,
        Some(NonZeroU32::MIN),
    )?;
    verify(
        expected.protected.is_some()
            && scene.visible_layout()? == Some(expected),
        "protected two-region receipt",
    )?;
    let surface = layout(8, 8)?;
    let pixels = read(surface)?;
    for (point, expected) in [
        ([0, 0], [0, 255, 0, 255]),
        ([7, 3], [0, 255, 0, 255]),
        ([0, 4], [0, 0, 0, 255]),
        ([0, 7], [0, 0, 0, 255]),
        ([7, 5], [10, 20, 30, 255]),
    ] {
        verify(
            pixel(&pixels, surface, point)? == expected,
            "actual protected fill pixels",
        )?;
    }
    let blue = pixel(&pixels, surface, [0, 5])?;
    verify(
        blue.iter()
            .zip([5, 10, 143, 255])
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1),
        "uncropped protected region",
    )
}
