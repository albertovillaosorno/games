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
//   - Bounded SDL2 streaming textures and accelerated window presentation.
// - Must-Not:
//   - Advance simulation, interpret source media or infer frame latency.
// - Allows:
//   - Inputs: Caller-owned RGBA frames and explicit dimension/byte limits.
//   - Outputs: Native events and aspect-preserving presented frames.
//   - Side effects: Explicit window, renderer and texture creation.
// - Split-When:
//   - Native input mapping or sprite composition needs separate ownership.
// - Merge-When:
//   - Native adapters duplicate texture upload or viewport policy.
// - Summary:
//   - Nearest-neighbor native presentation with a persistent GPU texture.
// - Description:
//   - Uses the shared viewport and preserves the caller's simulation clock.
// - Usage:
//   - Open a window and run a frame session with a bounded logical extent.
// - Defaults:
//   - Require acceleration; do not request vertical synchronization.

//! Accelerated SDL2 RGBA presentation for Linux hosts.

/// Bounded GPU-resident sprite atlases and ordered scene composition.
mod scene_renderer;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{
    Dimensions, PresentationLayout, PresentationMode, Viewport, WindowMode,
};
#[cfg(test)]
use sdl2::event::Event;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::{Rect, max_int_value};
use sdl2::render::{
    BlendMode, Canvas, RenderTarget, ScaleMode, Texture, TextureCreator,
    WindowCanvas,
};
use sdl2::video::FullscreenType;
use sdl2::{EventPump, Sdl, TimerSubsystem};

use self::scene_renderer::{LinuxScene, SceneTextures};
use super::input::{open_input, sample};
use crate::ir::frame::{AdmittedRgba, FrameLayout, RenderError};
use crate::ir::graphics::{
    FrameBackend, FrameRun, SceneRun, WindowBackend, WindowPort,
};
use crate::ir::hardware::InputSample;
use crate::ir::input::InputPort;

// SDL_RENDERER_ACCELERATED in the public SDL2 renderer flags.
const ACCELERATED_FLAG: u32 = 2;
// SDL_WINDOW_HIDDEN and SDL_WINDOW_MINIMIZED in public SDL2 window flags.
const HIDDEN_FLAG: u32 = 8;
const MINIMIZED_FLAG: u32 = 64;

/// A native window with an explicitly accelerated renderer.
struct LinuxWindow {
    canvas: WindowCanvas,
    diagnostic_hidden: bool,
    sdl: Sdl,
}

impl Debug for LinuxWindow {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxWindow").finish_non_exhaustive()
    }
}

impl WindowBackend for LinuxWindow {
    /// Apply portable display settings before starting a render session.
    /// Fullscreen uses the desktop mode, not a resolution change.
    /// # Errors
    /// Reports a native window mode transition failure.
    fn apply_window_mode(
        &mut self,
        mode: WindowMode,
    ) -> Result<WindowMode, RenderError> {
        apply_window_mode(&mut self.canvas, mode)
    }

    /// Open native input and return initial controller attachment events.
    ///
    /// Feed the initial events to the shared router before polling new
    /// events. The adapter owns at most the requested controller count.
    ///
    /// # Errors
    /// Reports unavailable controller enumeration or failed device opens.
    fn input(&self, limit: NonZeroUsize) -> Result<InputPort, RenderError> {
        open_input(&self.sdl, self.canvas.window(), limit)
    }

    /// Owned name reported by the selected native rendering driver.
    fn renderer_name(&self) -> String {
        self.canvas.info().name.to_owned()
    }

    /// Reuse one texture and event pump for the caller's presentation loop.
    ///
    /// The texture never escapes its owning canvas, so a frame session
    /// cannot accidentally present another renderer's texture.
    ///
    /// # Errors
    /// Rejects unavailable textures, event pumps or callback errors.
    fn with_frame(
        &mut self,
        layout: FrameLayout,
        mode: PresentationMode,
        run: &mut FrameRun<'_>,
    ) -> Result<(), RenderError> {
        let creator = self.canvas.texture_creator();
        let texture = create_texture(&creator, layout)?;
        let events = self.sdl.event_pump().map_err(RenderError::Backend)?;
        let mut backend = LinuxFrame {
            canvas: &mut self.canvas,
            diagnostic_hidden: self.diagnostic_hidden,
            events,
            presentation: FramePresentation { layout, mode },
            texture,
            timer: self.sdl.timer().map_err(RenderError::Backend)?,
        };
        run(&mut backend)
    }

    /// Run an ordered sprite scene without uploading a complete CPU frame.
    ///
    /// The logical render target and bounded atlases belong to this window.
    ///
    /// # Errors
    /// Rejects unavailable render targets, event pumps or callback errors.
    fn with_scene(
        &mut self,
        layout: FrameLayout,
        mode: PresentationMode,
        run: &mut SceneRun<'_>,
    ) -> Result<(), RenderError> {
        let creator = self.canvas.texture_creator();
        let events = self.sdl.event_pump().map_err(RenderError::Backend)?;
        let textures = SceneTextures::new(&creator, layout, mode)?;
        let mut backend = LinuxScene::new(
            (&mut self.canvas, self.diagnostic_hidden),
            events,
            textures,
            self.sdl.timer().map_err(RenderError::Backend)?,
        )?;
        run(&mut backend)
    }
}

impl LinuxWindow {
    /// Open a resizable window without requesting vertical synchronization.
    ///
    /// A hidden window is useful for explicit native smoke tests. No
    /// software fallback is selected implicitly.
    ///
    /// # Errors
    /// Rejects invalid dimensions or unavailable native acceleration.
    fn open(
        title: &str,
        width: NonZeroU32,
        height: NonZeroU32,
        hidden: bool,
    ) -> Result<Self, RenderError> {
        validate_dimensions(width, height)?;
        let sdl = sdl2::init().map_err(RenderError::Backend)?;
        let video = sdl.video().map_err(RenderError::Backend)?;
        let mut builder = video.window(title, width.get(), height.get());
        let _builder = builder.resizable().allow_highdpi();
        if hidden {
            let _hidden_builder = builder.hidden();
        }
        let window = builder
            .build()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let canvas = window
            .into_canvas()
            .accelerated()
            .build()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        if canvas.info().flags & ACCELERATED_FLAG == 0 {
            return Err(RenderError::Backend(String::from(
                "SDL did not select an accelerated renderer",
            )));
        }
        Ok(Self {
            canvas,
            diagnostic_hidden: hidden,
            sdl,
        })
    }
}

#[cfg(test)]
impl LinuxWindow {
    fn sample(&self, event: &Event) -> Result<InputSample, RenderError> {
        let timer = self.sdl.timer().map_err(RenderError::Backend)?;
        Ok(sample(event, self.canvas.window().id(), timer.ticks64()))
    }
}

/// A persistent frame texture and native event pump owned by one window.
struct LinuxFrame<'canvas, 'texture> {
    canvas: &'canvas mut WindowCanvas,
    diagnostic_hidden: bool,
    events: EventPump,
    presentation: FramePresentation,
    texture: Texture<'texture>,
    timer: TimerSubsystem,
}

impl Debug for LinuxFrame<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxFrame")
            .field("layout", &self.presentation.layout)
            .finish_non_exhaustive()
    }
}

impl FrameBackend for LinuxFrame<'_, '_> {
    fn layout(&self) -> FrameLayout {
        self.presentation.layout
    }

    /// Poll one window-scoped portable input observation.
    fn poll_event(&mut self) -> Option<InputSample> {
        self.events.poll_event().map(|event| {
            sample(&event, self.canvas.window().id(), self.timer.ticks64())
        })
    }

    /// Upload and present a complete logical frame using nearest sampling.
    ///
    /// Returns false for an unavailable drawable rather than stretching,
    /// allocating a replacement texture or advancing simulation.
    ///
    /// # Errors
    /// Rejects inconsistent frame geometry or failed native operations.
    fn present(
        &mut self,
        frame: AdmittedRgba<'_>,
    ) -> Result<bool, RenderError> {
        if frame.layout() != self.presentation.layout {
            return Err(RenderError::InvalidDimensions);
        }
        if !drawable_allowed(
            self.canvas.window().window_flags(),
            self.diagnostic_hidden,
        ) {
            return Ok(false);
        }
        let drawn = draw_frame(
            self.canvas,
            &mut self.texture,
            self.presentation,
            frame,
        )?;
        if drawn {
            self.canvas.present();
        }
        Ok(drawn)
    }

    fn presentation_mode(&self) -> PresentationMode {
        self.presentation.mode
    }
}

#[derive(Clone, Copy, Debug)]
struct FramePresentation {
    layout: FrameLayout,
    mode: PresentationMode,
}

fn apply_window_mode(
    canvas: &mut WindowCanvas,
    mode: WindowMode,
) -> Result<WindowMode, RenderError> {
    let fullscreen = match mode {
        WindowMode::Fullscreen => FullscreenType::Desktop,
        WindowMode::Windowed => FullscreenType::Off,
    };
    if canvas.window().fullscreen_state() != fullscreen {
        canvas
            .window_mut()
            .set_fullscreen(fullscreen)
            .map_err(RenderError::Backend)?;
    }
    match canvas.window().fullscreen_state() {
        FullscreenType::Desktop => Ok(WindowMode::Fullscreen),
        FullscreenType::Off => Ok(WindowMode::Windowed),
        FullscreenType::True => Err(RenderError::InvalidWindowMode),
    }
}

fn copy_to_viewport<Target>(
    canvas: &mut Canvas<Target>,
    texture: &Texture<'_>,
    layout: PresentationLayout,
) -> Result<(), RenderError>
where
    Target: RenderTarget,
{
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    for region in layout.regions().into_iter().flatten() {
        canvas
            .copy(
                texture,
                native_rect(region.source)?,
                native_rect(region.destination)?,
            )
            .map_err(RenderError::Backend)?;
    }
    Ok(())
}

fn create_texture<Context>(
    creator: &TextureCreator<Context>,
    layout: FrameLayout,
) -> Result<Texture<'_>, RenderError> {
    let mut texture = creator
        .create_texture_streaming(
            PixelFormatEnum::RGBA32,
            layout.width().get(),
            layout.height().get(),
        )
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    texture.set_blend_mode(BlendMode::NONE);
    texture.set_scale_mode(ScaleMode::Nearest);
    Ok(texture)
}

fn draw_frame<Target>(
    canvas: &mut Canvas<Target>,
    texture: &mut Texture<'_>,
    presentation: FramePresentation,
    frame: AdmittedRgba<'_>,
) -> Result<bool, RenderError>
where
    Target: RenderTarget,
{
    let FramePresentation { layout, mode } = presentation;
    if frame.layout() != layout {
        return Err(RenderError::InvalidDimensions);
    }
    let Some(drawable) = drawable_viewport(canvas, layout, mode, None)? else {
        return Ok(false);
    };
    texture
        .update(None, frame.pixels(), layout.pitch())
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    copy_to_viewport(canvas, texture, drawable)?;
    Ok(true)
}

// Explicit hidden diagnostics may render; an ordinary hidden or minimized
// window is not an available presentation surface.
const fn drawable_allowed(flags: u32, diagnostic_hidden: bool) -> bool {
    let minimized = flags & MINIMIZED_FLAG != 0;
    let hidden = flags & HIDDEN_FLAG != 0;
    !minimized && (diagnostic_hidden || !hidden)
}

fn drawable_viewport<Target>(
    canvas: &Canvas<Target>,
    layout: FrameLayout,
    mode: PresentationMode,
    protected_bottom: Option<NonZeroU32>,
) -> Result<Option<PresentationLayout>, RenderError>
where
    Target: RenderTarget,
{
    output_dimensions(canvas)?
        .map(|surface| {
            layout.presentation_with_protected_bottom(
                mode,
                surface,
                protected_bottom,
            )
        })
        .transpose()
}

fn native_rect(viewport: Viewport) -> Result<Rect, RenderError> {
    let left = i32::try_from(viewport.x)
        .map_err(|_error| RenderError::InvalidDimensions)?;
    let top = i32::try_from(viewport.y)
        .map_err(|_error| RenderError::InvalidDimensions)?;
    Ok(Rect::new(left, top, viewport.width, viewport.height))
}

fn output_dimensions<Target>(
    canvas: &Canvas<Target>,
) -> Result<Option<Dimensions>, RenderError>
where
    Target: RenderTarget,
{
    let (width, height) = canvas.output_size().map_err(RenderError::Backend)?;
    Ok(NonZeroU32::new(width)
        .zip(NonZeroU32::new(height))
        .map(|(columns, rows)| Dimensions::new(columns, rows)))
}

fn validate_dimensions(
    width: NonZeroU32,
    height: NonZeroU32,
) -> Result<(), RenderError> {
    if width.get().max(height.get()) > max_int_value() {
        return Err(RenderError::InvalidDimensions);
    }
    Ok(())
}

pub(in crate::software) fn open_window(
    title: &str,
    width: NonZeroU32,
    height: NonZeroU32,
    hidden: bool,
) -> Result<WindowPort, RenderError> {
    Ok(Box::new(LinuxWindow::open(title, width, height, hidden)?))
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::capability::{
        HapticCommand, HapticEffect, HapticError, Haptics as _,
    };
    use games::input::physical::KeyboardKey;
    use games::input::touch::{
        ContactId, ContactPhase, TouchLayout, TouchPoint, TouchZone,
    };
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use games::presentation::{DisplaySettings, PresentationMode, WindowMode};
    use games::replay::{InputStep, LiveInputLimits, LiveSimulation};
    use games::timing::StepClock;
    use sdl2::controller::{Axis, Button};
    use sdl2::event::Event;
    use sdl2::keyboard::{Mod, Scancode};
    use sdl2::pixels::PixelFormatEnum;
    use sdl2::surface::Surface;
    use sdl2::video::FullscreenType;
    use sdl2::{EventSubsystem, TimerSubsystem};

    use super::{
        FrameLayout, FramePresentation, HIDDEN_FLAG, LinuxWindow,
        MINIMIZED_FLAG, RenderError, WindowBackend as _, create_texture,
        draw_frame, drawable_allowed, sample,
    };
    use crate::ir::graphics::scene_renderer::SceneSession;
    use crate::ir::graphics::{FrameSession, NativeWindow};
    use crate::ir::hardware::{InputSample, InputTransition};
    use crate::ir::input::{
        HostEvent, InputStartup, NativeInput, key_from_name,
    };
    use crate::ir::process::fixtures::{NativeSmoke, smoke_requested};
    use crate::ir::scene::{EventBatch, SceneLimits, SpriteDraw, SpriteRect};

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[derive(Debug, Default)]
    struct ActionTrace {
        last: Option<(u64, bool)>,
    }

    impl InputStep for ActionTrace {
        type Input = LogicalAction<&'static str>;

        fn input_step(&mut self, tick: u64, actions: &[Self::Input]) {
            if let Some(action) =
                actions.iter().rfind(|item| item.action == "jump")
            {
                self.last = Some((tick, action.down));
            }
        }
    }

    fn extent() -> Result<FrameLayout, RenderError> {
        FrameLayout::new(nz(2), nz(2), budget(16))
    }

    fn budget(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
    }

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    fn require(condition: bool, reason: &str) -> Result<(), RenderError> {
        condition
            .then_some(())
            .ok_or_else(|| RenderError::Backend(reason.to_owned()))
    }

    fn verify_routed_timestamp(
        adapter: &NativeInput,
        native: &InputSample,
        event: InputEvent,
    ) -> Result<(), RenderError> {
        let timestamp = adapter.event_elapsed(native).ok_or_else(|| {
            RenderError::Backend(String::from("invalid queued input timestamp"))
        })?;
        require(timestamp <= adapter.elapsed()?, "future event")?;
        let interval = Duration::from_secs(1);
        let clock = StepClock::new(interval, nz(128))
            .map_err(|error| RenderError::Backend(format!("{error:?}")))?;
        let target = clock.tick_at(timestamp).ok_or_else(|| {
            RenderError::Backend(String::from("unrepresentable input time"))
        })?;
        let mut live = LiveSimulation::new(
            ActionTrace::default(),
            clock,
            LiveInputLimits {
                future_ticks: 127,
                pending_actions: budget(4),
            },
        );
        let mut mapping = InputMapping::new();
        mapping.bind_key(KeyboardKey::SPACE.code(), "jump");
        let mut router = InputRouter::new(mapping);
        live.enqueue_at(timestamp, router.handle(event))
            .map_err(|error| RenderError::Backend(format!("{error:?}")))?;
        let _report = live
            .advance(timestamp.saturating_add(interval))
            .map_err(|error| RenderError::Backend(format!("{error:?}")))?;
        require(
            live.state().last == Some((target, true)),
            "wrong action tick",
        )
    }

    #[test]
    fn unavailable_windows_do_not_admit_ordinary_submissions() {
        let hidden = HIDDEN_FLAG;
        let shown = 4; // SDL_WINDOW_SHOWN
        let minimized = MINIMIZED_FLAG;
        assert!(drawable_allowed(shown, false));
        assert!(!drawable_allowed(hidden, false));
        assert!(drawable_allowed(hidden, true));
        assert!(!drawable_allowed(shown | minimized, false));
        assert!(!drawable_allowed(hidden | minimized, true));
    }

    #[test]
    fn rejects_excessive_texture_bytes_and_dimensions() {
        assert_eq!(
            FrameLayout::new(nz(2), nz(2), budget(15)),
            Err(RenderError::TooLarge)
        );
        assert_eq!(
            FrameLayout::new(nz(u32::MAX), nz(1), NonZeroUsize::MAX),
            Err(RenderError::InvalidDimensions)
        );
        assert_eq!(extent().map(FrameLayout::bytes), Ok(16));
    }

    #[test]
    fn software_readback_preserves_pixels_and_letterbox()
    -> Result<(), RenderError> {
        let layout = extent()?;
        let presentation = FramePresentation {
            layout,
            mode: PresentationMode::AspectFit,
        };
        let surface = Surface::new(6, 4, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)?;
        let mut canvas = surface.into_canvas().map_err(RenderError::Backend)?;
        let creator = canvas.texture_creator();
        let mut texture = create_texture(&creator, layout)?;
        let pixels = [RED, GREEN, BLUE, WHITE];
        let drawn = draw_frame(
            &mut canvas,
            &mut texture,
            presentation,
            layout.admit(pixels.as_flattened())?,
        )?;
        require(drawn, "frame was not drawn")?;
        let result = canvas
            .read_pixels(None, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)?;
        let upper = [BLACK, RED, RED, GREEN, GREEN, BLACK];
        let lower = [BLACK, BLUE, BLUE, WHITE, WHITE, BLACK];
        let expected = [upper, upper, lower, lower];
        require(
            result.as_slice() == expected.as_flattened().as_flattened(),
            "nearest sampling or letterbox colors differ",
        )?;
        require(
            layout.admit(&[]).and_then(|frame| {
                draw_frame(&mut canvas, &mut texture, presentation, frame)
            }) == Err(RenderError::InvalidLength),
            "invalid frame accepted",
        )?;
        require(
            canvas
                .read_pixels(None, PixelFormatEnum::RGBA32)
                .map_err(RenderError::Backend)?
                == result,
            "rejected upload changed displayed pixels",
        )?;
        let replacement = [WHITE; 4];
        require(
            draw_frame(
                &mut canvas,
                &mut texture,
                presentation,
                layout.admit(replacement.as_flattened())?,
            )?,
            "second frame not drawn",
        )?;
        let row = [BLACK, WHITE, WHITE, WHITE, WHITE, BLACK];
        require(
            canvas
                .read_pixels(None, PixelFormatEnum::RGBA32)
                .map_err(RenderError::Backend)?
                == [row; 4].as_flattened().as_flattened(),
            "persistent texture did not update",
        )
    }

    #[test]
    fn fill_and_stretch_render_their_distinct_source_pixels()
    -> Result<(), RenderError> {
        let layout = FrameLayout::new(nz(4), nz(2), budget(32))?;
        let surface = Surface::new(4, 4, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)?;
        let mut canvas = surface.into_canvas().map_err(RenderError::Backend)?;
        let creator = canvas.texture_creator();
        let mut texture = create_texture(&creator, layout)?;
        let upper = [RED, GREEN, BLUE, WHITE];
        let lower = [WHITE, BLUE, GREEN, RED];
        let pixels = [upper, lower];
        for (mode, expected) in [
            (PresentationMode::Fill, [
                [GREEN, GREEN, BLUE, BLUE],
                [GREEN, GREEN, BLUE, BLUE],
                [BLUE, BLUE, GREEN, GREEN],
                [BLUE, BLUE, GREEN, GREEN],
            ]),
            (PresentationMode::Stretch, [upper, upper, lower, lower]),
            (PresentationMode::AspectFit, [
                [BLACK; 4], upper, lower, [BLACK; 4],
            ]),
        ] {
            require(
                draw_frame(
                    &mut canvas,
                    &mut texture,
                    FramePresentation { layout, mode },
                    layout.admit(pixels.as_flattened().as_flattened())?,
                )?,
                "frame was not drawn",
            )?;
            require(
                canvas
                    .read_pixels(None, PixelFormatEnum::RGBA32)
                    .map_err(RenderError::Backend)?
                    == expected.as_flattened().as_flattened(),
                "presentation mode selected incorrect pixels",
            )?;
        }
        Ok(())
    }

    #[test]
    fn integer_mode_does_not_fractionally_scale_pixels()
    -> Result<(), RenderError> {
        let layout = extent()?;
        let surface = Surface::new(5, 5, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)?;
        let mut canvas = surface.into_canvas().map_err(RenderError::Backend)?;
        let creator = canvas.texture_creator();
        let mut texture = create_texture(&creator, layout)?;
        let pixels = [RED, GREEN, BLUE, WHITE];
        require(
            draw_frame(
                &mut canvas,
                &mut texture,
                FramePresentation {
                    layout,
                    mode: PresentationMode::Integer,
                },
                layout.admit(pixels.as_flattened())?,
            )?,
            "integer frame not drawn",
        )?;
        let upper = [RED, RED, GREEN, GREEN, BLACK];
        let lower = [BLUE, BLUE, WHITE, WHITE, BLACK];
        let expected = [upper, upper, lower, lower, [BLACK; 5]];
        require(
            canvas
                .read_pixels(None, PixelFormatEnum::RGBA32)
                .map_err(RenderError::Backend)?
                == expected.as_flattened().as_flattened(),
            "integer mode did not preserve whole pixels",
        )
    }

    #[test]
    fn opt_in_native_accelerated_smoke() -> Result<(), RenderError> {
        if !smoke_requested(NativeSmoke::Render)
            .map_err(|error| RenderError::Backend(error.to_string()))?
        {
            return Ok(());
        }
        let _guard = super::super::NATIVE_SMOKE_LOCK
            .lock()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let mut window =
            LinuxWindow::open("games native smoke", nz(96), nz(48), true)?;
        verify_native_input(&window)?;
        verify_window_mode(&mut window)?;
        window
            .canvas
            .window_mut()
            .set_size(96, 48)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let renderer_name = window.renderer_name();
        require(!renderer_name.is_empty(), "missing renderer name")?;
        let expected_name = renderer_name.clone();
        verify_frame_session(&mut window)?;
        let events = window.sdl.event().map_err(RenderError::Backend)?;
        let timer = window.sdl.timer().map_err(RenderError::Backend)?;
        let window_id = window.canvas.window().id();
        let mut touch_input =
            InputStartup::new(window.input(budget(1))?, budget(1))?.input;
        touch_input.enable_touch();
        window.with_scene(
            extent()?,
            PresentationMode::AspectFit,
            &mut |backend| {
                let mut scene = SceneSession::new(
                    backend,
                    extent()?,
                    SceneLimits {
                        atlas_bytes: budget(16),
                        atlases: budget(1),
                        sprites: budget(2),
                    },
                    PresentationMode::AspectFit,
                )?;
                verify_native_batches(&mut scene, &events, window_id)?;
                verify_sprite_scene(
                    &mut scene,
                    &mut touch_input,
                    &timer,
                    window_id,
                )
            },
        )?;
        drop(window);
        require(renderer_name == expected_name, "renderer name changed")?;
        verify_public_window_mode()
    }

    fn verify_frame_session(
        window: &mut LinuxWindow,
    ) -> Result<(), RenderError> {
        window.with_frame(
            extent()?,
            PresentationMode::AspectFit,
            &mut |backend| {
                let mut frame = FrameSession::new(
                    backend,
                    extent()?,
                    PresentationMode::AspectFit,
                )?;
                let pixels = [RED, GREEN, BLUE, WHITE];
                require(frame.present(pixels.as_flattened())?, "no drawable")?;
                let replacement = [BLUE; 4];
                require(
                    frame.present(replacement.as_flattened())?,
                    "second frame not drawn",
                )
            },
        )
    }

    fn verify_public_window_mode() -> Result<(), RenderError> {
        let mut window =
            NativeWindow::open("games mode smoke", nz(96), nz(48), true)?;
        window.set_presentation_mode(PresentationMode::Stretch);
        let limits = SceneLimits {
            atlas_bytes: budget(16),
            atlases: budget(1),
            sprites: budget(1),
        };
        let edge = TouchPoint { x: 0, y: 32_768 };
        let result = window.with_scene(extent()?, limits, |scene| {
            require(scene.present([0; 3], &[])?, "no stretched scene")?;
            require(
                scene.logical_touch(edge)? == Some(edge),
                "initial requested mode was lost",
            )?;
            scene.apply_display_settings(DisplaySettings::default())?;
            Err::<(), _>(RenderError::InvalidLength)
        });
        require(
            result == Err(RenderError::InvalidLength),
            "callback failure was not propagated",
        )?;
        window.with_scene(extent()?, limits, |scene| {
            require(scene.present([0; 3], &[])?, "no fitted scene")?;
            require(
                scene.logical_touch(edge)?.is_none(),
                "successful mode change was lost after callback failure",
            )
        })
    }

    fn verify_sprite_scene(
        scene: &mut SceneSession<'_, '_>,
        input: &mut NativeInput,
        timer: &TimerSubsystem,
        window_id: u32,
    ) -> Result<(), RenderError> {
        let center = TouchPoint { x: 32_768, y: 32_768 };
        require(
            !scene.presentation_is_current()?
                && scene.logical_touch(center)?.is_none()
                && scene.surface_touch(center)?.is_none()
                && input
                    .synchronize_touch(|| scene.visible_layout())?
                    .is_none(),
            "unpresented scene admitted touch",
        )?;
        let atlas = scene
            .load_atlas(extent()?, [RED, GREEN, BLUE, WHITE].as_flattened())?;
        let rect = SpriteRect {
            height: nz(2),
            width: nz(2),
            x: 0,
            y: 0,
        };
        let sprite = SpriteDraw {
            atlas,
            destination: rect,
            flip_horizontal: false,
            flip_vertical: false,
            source: rect,
        };
        require(scene.present([0; 3], &[sprite])?, "no sprite scene")?;
        verify_rejected_draws(scene, sprite)?;
        require(
            input
                .synchronize_touch(|| scene.visible_layout())?
                .is_none(),
            "initial presented geometry cancelled a contact",
        )?;
        verify_presented_touch(input, timer, window_id)?;
        verify_touch_redraw(scene, input, sprite)?;
        require(
            scene.present([0; 3], &[SpriteDraw {
                flip_horizontal: true,
                ..sprite
            }])?,
            "second sprite scene not drawn",
        )
    }

    fn verify_rejected_draws(
        scene: &mut SceneSession<'_, '_>,
        sprite: SpriteDraw,
    ) -> Result<(), RenderError> {
        let pixels = scene.readback()?;
        let invalid = SpriteDraw {
            source: SpriteRect { x: 1, ..sprite.source },
            ..sprite
        };
        require(
            scene.present([255; 3], &[sprite, invalid])
                == Err(RenderError::InvalidSprite),
            "invalid source was presented",
        )?;
        require(
            scene.present([255; 3], &[sprite; 3]) == Err(RenderError::TooLarge),
            "excessive commands were presented",
        )?;
        require(
            scene.readback()? == pixels && scene.presentation_is_current()?,
            "rejected draw changed the presented scene",
        )
    }

    fn verify_touch_redraw(
        scene: &mut SceneSession<'_, '_>,
        input: &mut NativeInput,
        sprite: SpriteDraw,
    ) -> Result<(), RenderError> {
        let edge = TouchPoint { x: 0, y: 32_768 };
        let center = TouchPoint { x: 32_768, y: 32_768 };
        require(
            scene.presentation_is_current()?
                && scene.logical_touch(edge)?.is_none()
                && scene.logical_touch(center)?.is_some()
                && input
                    .synchronize_touch(|| scene.visible_layout())?
                    .is_none(),
            "aspect-fit touch did not reject letterbox space",
        )?;
        verify_touch_release(scene, input, center)?;
        require(scene.present([0; 3], &[sprite])?, "no stretched scene")?;
        require(
            scene.presentation_is_current()?
                && scene.logical_touch(edge)? == Some(edge)
                && scene.surface_touch(edge)? == Some(edge)
                && input
                    .synchronize_touch(|| scene.visible_layout())?
                    .is_none(),
            "redrawn stretch touch did not use the full surface",
        )?;
        scene.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Adaptive,
            ..DisplaySettings::default()
        })?;
        let maximum = FrameLayout::new(nz(4), nz(2), budget(32))?;
        require(
            scene.fit_adaptive_width(maximum)? == maximum,
            "adaptive test target did not widen",
        )?;
        require(
            !scene.presentation_is_current()?
                && scene.logical_touch(center)?.is_none()
                && scene.surface_touch(center)?.is_none(),
            "touch used a new target before presentation",
        )?;
        require(scene.present([0; 3], &[sprite])?, "no adaptive scene")?;
        require(
            scene.presentation_is_current()?
                && scene.logical_touch(edge)? == Some(edge)
                && scene.surface_touch(edge)? == Some(edge)
                && input
                    .synchronize_touch(|| scene.visible_layout())?
                    .is_some()
                && input
                    .synchronize_touch(|| scene.visible_layout())?
                    .is_none(),
            "redrawn adaptive touch did not cancel old geometry",
        )
    }

    fn touch_smoke_router() -> Result<InputRouter<&'static str>, RenderError> {
        let mut mapping = InputMapping::new();
        let zone = TouchZone::new(1, TouchPoint { x: 0, y: 0 }, TouchPoint {
            x: u16::MAX,
            y: u16::MAX,
        })
        .ok_or_else(|| {
            RenderError::Backend(String::from("invalid test zone"))
        })?;
        mapping.bind_touch(1, "jump");
        mapping.bind_key(KeyboardKey::SPACE.code(), "move");
        mapping.set_touch_layout(TouchLayout::new(vec![zone], budget(1)));
        Ok(InputRouter::new(mapping))
    }

    fn verify_touch_release(
        scene: &mut SceneSession<'_, '_>,
        input: &mut NativeInput,
        center: TouchPoint,
    ) -> Result<(), RenderError> {
        let mut router = touch_smoke_router()?;
        let contact = |phase| InputEvent::Contact {
            id: ContactId { device: 1, finger: 1 },
            phase,
        };
        let key = |down| InputEvent::Key {
            code: KeyboardKey::SPACE.code(),
            down,
        };
        let action = |action, down| LogicalAction { action, down };
        let pressed = scene.logical_touch(center)?.ok_or_else(|| {
            RenderError::Backend(String::from("center touch not projected"))
        })?;
        require(
            router.handle(contact(ContactPhase::Down(pressed)))
                == [action("jump", true)],
            "contact was not admitted",
        )?;
        require(
            router.handle(key(true)) == [action("move", true)],
            "missing key",
        )?;
        scene.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Stretch,
            ..DisplaySettings::default()
        })?;
        require(
            !scene.presentation_is_current()?
                && scene.logical_touch(center)?.is_none()
                && scene.surface_touch(center)?.is_none(),
            "touch used a layout not yet drawn",
        )?;
        require(
            input
                .synchronize_touch(|| scene.visible_layout())?
                .is_some(),
            "missing cancellation",
        )?;
        require(
            router.handle(InputEvent::TouchCancelled)
                == [action("jump", false)],
            "geometry change did not release the held contact",
        )?;
        require(
            input
                .synchronize_touch(|| scene.visible_layout())?
                .is_none(),
            "duplicate cancellation",
        )?;
        require(
            router
                .handle(contact(ContactPhase::Motion(Some(pressed))))
                .is_empty()
                && router.handle(contact(ContactPhase::Up)).is_empty(),
            "old contact resumed after cancellation",
        )?;
        require(
            router.handle(key(false)) == [action("move", false)],
            "geometry cancellation released a held keyboard key",
        )
    }

    fn verify_window_mode(window: &mut LinuxWindow) -> Result<(), RenderError> {
        for (mode, expected) in [
            (WindowMode::Fullscreen, FullscreenType::Desktop),
            (WindowMode::Windowed, FullscreenType::Off),
        ] {
            require(window.apply_window_mode(mode)? == mode, "wrong receipt")?;
            require(
                window.canvas.window().fullscreen_state() == expected,
                "wrong fullscreen state",
            )?;
        }
        require(
            window.apply_window_mode(WindowMode::default())?
                == WindowMode::default(),
            "wrong default receipt",
        )
    }

    fn observed_key(event: &InputSample) -> Option<KeyboardKey> {
        if let InputTransition::Event(HostEvent::Input(InputEvent::Key {
            code,
            down: true,
        })) = event.transition()
        {
            Some(KeyboardKey::from_code(code))
        } else {
            None
        }
    }

    fn verify_native_batches(
        scene: &mut SceneSession<'_, '_>,
        events: &EventSubsystem,
        window_id: u32,
    ) -> Result<(), RenderError> {
        let keys = [Scancode::F22, Scancode::F23, Scancode::F24];
        let expected = keys.map(|key| key_from_name(key.name()));
        for code in keys {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id,
                    keycode: None,
                    scancode: Some(code),
                    keymod: Mod::NOMOD,
                    repeat: false,
                })
                .map_err(RenderError::Backend)?;
        }
        let mut observed = Vec::new();
        for _batch in 0u16..256 {
            let mut count = 0usize;
            let progress =
                scene.dispatch_events(budget(2), 0, |_scene, event| {
                    count = count.saturating_add(1);
                    observed.extend(
                        observed_key(event)
                            .map(Some)
                            .filter(|code| expected.contains(code)),
                    );
                    Ok(true)
                })?;
            require(count <= 2, "native event batch exceeded its budget")?;
            require(
                (count == 2) == (progress == EventBatch::Limited),
                "native batch limit status differs",
            )?;
            if observed.len() >= expected.len() {
                break;
            }
        }
        require(observed == expected, "queued native key order differs")
    }

    fn verify_native_input(window: &LinuxWindow) -> Result<(), RenderError> {
        let mut startup =
            InputStartup::new(window.input(budget(4))?, budget(4))?;
        require(startup.attached.len() <= 4, "controller limit exceeded")?;
        require(
            startup
                .attached
                .iter()
                .all(|event| matches!(event, InputEvent::Attached { .. })),
            "initial event is not an attachment",
        )?;
        require(
            startup.input.discover_available()?.is_empty(),
            "repeated discovery attached an existing controller",
        )?;
        verify_unknown_controller(window, &mut startup.input)?;
        verify_touch_opt_in(window, &mut startup.input)?;
        let event = Event::KeyDown {
            timestamp: 0,
            window_id: window.canvas.window().id(),
            keycode: None,
            scancode: Some(Scancode::Space),
            keymod: Mod::NOMOD,
            repeat: false,
        };
        let events = window.sdl.event().map_err(RenderError::Backend)?;
        events.push_event(event).map_err(RenderError::Backend)?;
        let mut pump = window.sdl.event_pump().map_err(RenderError::Backend)?;
        let expected = InputEvent::Key {
            code: KeyboardKey::SPACE.code(),
            down: true,
        };
        for native in pump.poll_iter().take(256) {
            let sample = window.sample(&native)?;
            if let Some(HostEvent::Input(input)) =
                startup.input.translate(&sample)
                && input == expected
            {
                return verify_routed_timestamp(&startup.input, &sample, input);
            }
        }
        Err(RenderError::Backend(String::from(
            "queued native key did not reach the input router",
        )))
    }

    fn verify_touch_opt_in(
        window: &LinuxWindow,
        input: &mut NativeInput,
    ) -> Result<(), RenderError> {
        let down = Event::FingerDown {
            timestamp: 0,
            touch_id: 3,
            finger_id: 4,
            x: 0.5,
            y: 0.5,
            dx: 0.0,
            dy: 0.0,
            pressure: 1.0,
        };
        require(
            input.translate(&window.sample(&down)?).is_none(),
            "touch enabled by default",
        )?;
        input.enable_touch();
        require(
            input.translate(&window.sample(&down)?).is_none(),
            "touch admitted before a presented geometry",
        )
    }

    fn verify_presented_touch(
        input: &mut NativeInput,
        timer: &TimerSubsystem,
        window_id: u32,
    ) -> Result<(), RenderError> {
        let down = Event::FingerDown {
            timestamp: timer.ticks(),
            touch_id: 3,
            finger_id: 4,
            x: 0.5,
            y: 0.5,
            dx: 0.0,
            dy: 0.0,
            pressure: 1.0,
        };
        let id = ContactId { device: 3, finger: 4 };
        require(
            input.translate(&sample(&down, window_id, timer.ticks64()))
                == Some(HostEvent::Input(InputEvent::Contact {
                    id,
                    phase: ContactPhase::Down(TouchPoint {
                        x: 32_768,
                        y: 32_768,
                    }),
                })),
            "enabled native touch did not deliver contact",
        )?;
        let up = Event::FingerUp {
            timestamp: timer.ticks(),
            touch_id: 3,
            finger_id: 4,
            x: 0.5,
            y: 0.5,
            dx: 0.0,
            dy: 0.0,
            pressure: 0.0,
        };
        require(
            input.translate(&sample(&up, window_id, timer.ticks64()))
                == Some(HostEvent::Input(InputEvent::Contact {
                    id,
                    phase: ContactPhase::Up,
                })),
            "native touch release did not preserve contact identity",
        )
    }

    fn verify_unknown_controller(
        window: &LinuxWindow,
        input: &mut NativeInput,
    ) -> Result<(), RenderError> {
        require(
            input.support(u64::MAX).is_none()
                && input.submit(HapticCommand {
                    device: u64::MAX,
                    effect: HapticEffect::Stop,
                }) == Err(HapticError::Disconnected),
            "unknown controller accepted haptic feedback",
        )?;
        for unknown in [
            Event::ControllerButtonDown {
                timestamp: 0,
                which: u32::MAX,
                button: Button::A,
            },
            Event::ControllerAxisMotion {
                timestamp: 0,
                which: u32::MAX,
                axis: Axis::LeftX,
                value: i16::MIN,
            },
            Event::ControllerDeviceRemapped {
                timestamp: 0,
                which: u32::MAX,
            },
        ] {
            require(
                input.translate(&window.sample(&unknown)?).is_none(),
                "unknown controller produced an action",
            )?;
        }
        let stale_add = Event::ControllerDeviceAdded {
            timestamp: 0,
            which: u32::MAX,
        };
        require(
            input.translate(&window.sample(&stale_add)?).is_none(),
            "device-added notification must not directly attach",
        )?;
        let removed = Event::ControllerDeviceRemoved {
            timestamp: 0,
            which: u32::MAX,
        };
        require(
            input.translate(&window.sample(&removed)?).is_none(),
            "unknown controller removal produced a detach",
        )?;
        require(
            input.discover_after_drain()?.is_empty(),
            "stale discovery hint attached a phantom device",
        )
    }
}
