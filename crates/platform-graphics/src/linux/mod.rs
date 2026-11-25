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
/// Bounded native keyboard, game-controller and touch event admission.
pub mod input;
/// Bounded GPU-resident sprite atlases and ordered scene composition.
pub mod sprites;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
    Viewport, WindowMode,
};
use sdl2::event::Event;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::{Rect, max_int_value};
use sdl2::render::{
    BlendMode, Canvas, RenderTarget, ScaleMode, Texture, TextureCreator,
    WindowCanvas,
};
use sdl2::video::FullscreenType;
use sdl2::{EventPump, Sdl};

use self::input::{InputStartup, NativeInput};
use self::sprites::{SceneLimits, SceneSession, SceneTextures};
// SDL_RENDERER_ACCELERATED in the public SDL2 renderer flags.
const ACCELERATED_FLAG: u32 = 2;

/// An explicit native rendering failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// The native display or renderer rejected an operation.
    Backend(String),
    /// The pixel or surface dimensions are not representable.
    InvalidDimensions,
    /// The supplied frame is not exactly the requested RGBA extent.
    InvalidLength,
    /// An atlas or sprite rectangle is absent or outside its source.
    InvalidSprite,
    /// The frame requires more bytes than permitted.
    TooLarge,
}

/// A bounded, tightly packed logical RGBA8 frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLayout {
    bytes: usize,
    height: NonZeroU32,
    pitch: usize,
    width: NonZeroU32,
}

impl FrameLayout {
    /// Number of bytes in one complete frame.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Logical height of the frame.
    #[must_use]
    pub const fn height(self) -> NonZeroU32 {
        self.height
    }

    /// Check dimensions and the caller's maximum texture upload size.
    ///
    /// # Errors
    /// Rejects unrepresentable dimensions, stride or total byte counts.
    pub fn new(
        width: NonZeroU32,
        height: NonZeroU32,
        max_bytes: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        validate_dimensions(width, height)?;
        let pitch = usize::try_from(width.get())
            .ok()
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RenderError::InvalidDimensions)?;
        if i32::try_from(pitch).is_err() {
            return Err(RenderError::InvalidDimensions);
        }
        let bytes = usize::try_from(height.get())
            .ok()
            .and_then(|rows| pitch.checked_mul(rows))
            .ok_or(RenderError::TooLarge)?;
        if bytes > max_bytes.get() {
            return Err(RenderError::TooLarge);
        }
        Ok(Self {
            bytes,
            height,
            pitch,
            width,
        })
    }

    /// Logical width of the frame.
    #[must_use]
    pub const fn width(self) -> NonZeroU32 {
        self.width
    }
}

/// A native window with an explicitly accelerated renderer.
pub struct NativeWindow {
    canvas: WindowCanvas,
    mode: PresentationMode,
    sdl: Sdl,
}

impl Debug for NativeWindow {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWindow").finish_non_exhaustive()
    }
}

impl NativeWindow {
    /// Apply portable display settings before starting a render session.
    /// Fullscreen uses the desktop mode, not a resolution change.
    /// # Errors
    /// Reports a native window mode transition failure.
    pub fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError> {
        let fullscreen = match settings.window {
            WindowMode::Fullscreen => FullscreenType::Desktop,
            WindowMode::Windowed => FullscreenType::Off,
        };
        self.canvas
            .window_mut()
            .set_fullscreen(fullscreen)
            .map_err(RenderError::Backend)?;
        self.set_presentation_mode(settings.presentation);
        Ok(())
    }

    /// Open native input and return initial controller attachment events.
    ///
    /// Feed the initial events to the shared router before polling new
    /// events. The adapter owns at most the requested controller count.
    ///
    /// # Errors
    /// Reports unavailable controller enumeration or failed device opens.
    pub fn input(
        &self,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        NativeInput::open(&self.sdl, self.canvas.window().id(), limit)
    }

    /// Open a resizable window without requesting vertical synchronization.
    ///
    /// A hidden window is useful for explicit native smoke tests. No
    /// software fallback is selected implicitly.
    ///
    /// # Errors
    /// Rejects invalid dimensions or unavailable native acceleration.
    pub fn open(
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
            mode: PresentationMode::default(),
            sdl,
        })
    }

    /// Owned name reported by the selected native rendering driver.
    #[must_use]
    pub fn renderer_name(&self) -> String {
        self.canvas.info().name.to_owned()
    }

    /// Select the presentation mode for subsequent frame and scene sessions.
    /// Does not change the logical design dimensions.
    pub const fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.mode = mode;
    }

    /// Reuse one texture and event pump for the caller's presentation loop.
    ///
    /// The texture never escapes its owning canvas, so a frame session
    /// cannot accidentally present another renderer's texture.
    ///
    /// # Errors
    /// Rejects unavailable textures, event pumps or callback errors.
    pub fn with_frame<Output, Run>(
        &mut self,
        layout: FrameLayout,
        run: Run,
    ) -> Result<Output, RenderError>
    where
        Run: FnOnce(&mut FrameSession<'_, '_>) -> Result<Output, RenderError>,
    {
        let creator = self.canvas.texture_creator();
        let texture = create_texture(&creator, layout)?;
        let events = self.sdl.event_pump().map_err(RenderError::Backend)?;
        let mut session = FrameSession {
            canvas: &mut self.canvas,
            events,
            presentation: FramePresentation { layout, mode: self.mode },
            texture,
        };
        run(&mut session)
    }

    /// Run an ordered sprite scene without uploading a complete CPU frame.
    ///
    /// The logical render target and bounded atlases belong to this window.
    ///
    /// # Errors
    /// Rejects unavailable render targets, event pumps or callback errors.
    pub fn with_scene<Output, Run>(
        &mut self,
        layout: FrameLayout,
        limits: SceneLimits,
        run: Run,
    ) -> Result<Output, RenderError>
    where
        Run: FnOnce(&mut SceneSession<'_, '_>) -> Result<Output, RenderError>,
    {
        let creator = self.canvas.texture_creator();
        let events = self.sdl.event_pump().map_err(RenderError::Backend)?;
        let textures = SceneTextures::new(&creator, layout, limits, self.mode)?;
        let mut session =
            SceneSession::new(&mut self.canvas, events, textures)?;
        run(&mut session)
    }
}

/// A persistent frame texture and native event pump owned by one window.
pub struct FrameSession<'canvas, 'texture> {
    canvas: &'canvas mut WindowCanvas,
    events: EventPump,
    presentation: FramePresentation,
    texture: Texture<'texture>,
}

impl Debug for FrameSession<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSession")
            .field("layout", &self.presentation.layout)
            .finish_non_exhaustive()
    }
}

impl FrameSession<'_, '_> {
    /// Poll one native event for translation by the host input adapter.
    #[must_use]
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.poll_event()
    }

    /// Upload and present a complete logical frame using nearest sampling.
    ///
    /// Returns false for a zero-sized drawable rather than stretching,
    /// allocating a replacement texture or advancing simulation.
    ///
    /// # Errors
    /// Rejects incorrect frame lengths or failed native operations.
    pub fn present(&mut self, pixels: &[u8]) -> Result<bool, RenderError> {
        let drawn = draw_frame(
            self.canvas,
            &mut self.texture,
            self.presentation,
            pixels,
        )?;
        if drawn {
            self.canvas.present();
        }
        Ok(drawn)
    }
}

#[derive(Clone, Copy, Debug)]
struct FramePresentation {
    layout: FrameLayout,
    mode: PresentationMode,
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
    canvas
        .copy(
            texture,
            native_rect(layout.source)?,
            native_rect(layout.destination)?,
        )
        .map_err(RenderError::Backend)
}

fn create_texture<Context>(
    creator: &TextureCreator<Context>,
    layout: FrameLayout,
) -> Result<Texture<'_>, RenderError> {
    let mut texture = creator
        .create_texture_streaming(
            PixelFormatEnum::RGBA32,
            layout.width.get(),
            layout.height.get(),
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
    pixels: &[u8],
) -> Result<bool, RenderError>
where
    Target: RenderTarget,
{
    let FramePresentation { layout, mode } = presentation;
    if pixels.len() != layout.bytes {
        return Err(RenderError::InvalidLength);
    }
    let Some(drawable) = drawable_viewport(canvas, layout, mode)? else {
        return Ok(false);
    };
    texture
        .update(None, pixels, layout.pitch)
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    copy_to_viewport(canvas, texture, drawable)?;
    Ok(true)
}

fn drawable_viewport<Target>(
    canvas: &Canvas<Target>,
    layout: FrameLayout,
    mode: PresentationMode,
) -> Result<Option<PresentationLayout>, RenderError>
where
    Target: RenderTarget,
{
    let (output_width, output_height) =
        canvas.output_size().map_err(RenderError::Backend)?;
    let (Some(width), Some(height)) = (
        NonZeroU32::new(output_width),
        NonZeroU32::new(output_height),
    ) else {
        return Ok(None);
    };
    validate_dimensions(width, height)?;
    Ok(Some(mode.layout(
        Dimensions::new(width, height),
        Dimensions::new(layout.width, layout.height),
    )))
}

fn native_rect(viewport: Viewport) -> Result<Rect, RenderError> {
    let left = i32::try_from(viewport.x)
        .map_err(|_error| RenderError::InvalidDimensions)?;
    let top = i32::try_from(viewport.y)
        .map_err(|_error| RenderError::InvalidDimensions)?;
    Ok(Rect::new(left, top, viewport.width, viewport.height))
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use games::presentation::{DisplaySettings, PresentationMode, WindowMode};
    use games::replay::{InputStep, LiveInputLimits, LiveSimulation};
    use games::timing::StepClock;
    use sdl2::event::Event;
    use sdl2::keyboard::Mod;
    use sdl2::pixels::PixelFormatEnum;
    use sdl2::surface::Surface;
    use sdl2::video::FullscreenType;

    use super::input::{
        Axis, Button, HostEvent, NativeInput, Scancode, key_code,
    };
    use super::sprites::{SceneLimits, SpriteDraw, SpriteRect};
    use super::{
        FrameLayout, FramePresentation, NativeWindow, RenderError,
        create_texture, draw_frame,
    };

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
        native: &Event,
        event: InputEvent,
    ) -> Result<(), RenderError> {
        let timestamp = adapter.event_elapsed(native).ok_or_else(|| {
            RenderError::Backend(String::from("invalid queued input timestamp"))
        })?;
        require(timestamp <= adapter.elapsed(), "future event")?;
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
        mapping.bind_key(key_code(Scancode::Space), "jump");
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
            pixels.as_flattened(),
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
            draw_frame(&mut canvas, &mut texture, presentation, &[])
                == Err(RenderError::InvalidLength),
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
                replacement.as_flattened(),
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
                    pixels.as_flattened().as_flattened(),
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
                pixels.as_flattened(),
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
        if env::var_os("GAMES_NATIVE_RENDER_SMOKE").is_none() {
            return Ok(());
        }
        let mut window =
            NativeWindow::open("games native smoke", nz(64), nz(64), true)?;
        verify_native_input(&window)?;
        verify_window_mode(&mut window)?;
        let renderer_name = window.renderer_name();
        require(!renderer_name.is_empty(), "missing renderer name")?;
        let expected_name = renderer_name.clone();
        window.with_frame(extent()?, |frame| {
            let pixels = [RED, GREEN, BLUE, WHITE];
            require(frame.present(pixels.as_flattened())?, "no drawable")?;
            let replacement = [BLUE; 4];
            require(
                frame.present(replacement.as_flattened())?,
                "second frame not drawn",
            )
        })?;
        window.with_scene(
            extent()?,
            SceneLimits {
                atlas_bytes: budget(16),
                atlases: budget(1),
                sprites: budget(2),
            },
            |scene| {
                let atlas = scene.load_atlas(
                    extent()?,
                    [RED, GREEN, BLUE, WHITE].as_flattened(),
                )?;
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
                require(
                    scene.present([0; 3], &[SpriteDraw {
                        flip_horizontal: true,
                        ..sprite
                    }])?,
                    "second sprite scene not drawn",
                )
            },
        )?;
        drop(window);
        require(renderer_name == expected_name, "renderer name changed")
    }

    fn verify_window_mode(
        window: &mut NativeWindow,
    ) -> Result<(), RenderError> {
        for (mode, expected) in [
            (WindowMode::Fullscreen, FullscreenType::Desktop),
            (WindowMode::Windowed, FullscreenType::Off),
        ] {
            window.apply_display_settings(DisplaySettings {
                presentation: PresentationMode::Integer,
                window: mode,
            })?;
            require(
                window.canvas.window().fullscreen_state() == expected,
                "wrong fullscreen state",
            )?;
        }
        window.apply_display_settings(DisplaySettings::default())
    }

    fn verify_native_input(window: &NativeWindow) -> Result<(), RenderError> {
        let mut startup = window.input(budget(4))?;
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
        verify_unknown_controller(&mut startup.input)?;
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
            code: key_code(Scancode::Space),
            down: true,
        };
        for native in pump.poll_iter().take(256) {
            if let Some(HostEvent::Input(input)) =
                startup.input.translate(&native)?
                && input == expected
            {
                return verify_routed_timestamp(&startup.input, &native, input);
            }
        }
        Err(RenderError::Backend(String::from(
            "queued native key did not reach the input router",
        )))
    }

    fn verify_unknown_controller(
        input: &mut NativeInput,
    ) -> Result<(), RenderError> {
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
        ] {
            require(
                input.translate(&unknown)?.is_none(),
                "unknown controller produced an action",
            )?;
        }
        let removed = Event::ControllerDeviceRemoved {
            timestamp: 0,
            which: u32::MAX,
        };
        require(
            input.translate(&removed)?.is_none(),
            "unknown controller removal produced a detach",
        )?;
        Ok(())
    }
}
