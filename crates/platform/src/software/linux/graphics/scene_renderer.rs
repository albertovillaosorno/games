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
//   - Native texture atlases and ordered alpha-blended sprite composition.
// - Must-Not:
//   - Decode original assets, interpret gameplay or advance simulation.
// - Allows:
//   - Inputs: Bounded RGBA atlases, rectangles and ordered draw commands.
//   - Outputs: A composed logical frame presented by the shared viewport.
//   - Side effects: Allocates bounded native textures and presents frames.
// - Split-When:
//   - Atlas lifetime management becomes independently reusable.
// - Merge-When:
//   - Another native adapter duplicates atlas or sprite composition policy.
// - Summary:
//   - Persistent textures and logical render targets for native 2D scenes.
// - Description:
//   - Validates all commands before rendering and preserves draw order.
// - Usage:
//   - Load atlases once, then present bounded lists of sprite commands.
// - Defaults:
//   - Nearest sampling, straight alpha, opaque background and clipping.

//! Ordered native sprite composition over persistent texture atlases.

use std::fmt::{self, Debug, Formatter};
use std::num::NonZeroU32;

use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::{Rect, max_int_value, min_int_value};
use sdl2::render::{
    BlendMode, Canvas, RenderTarget, ScaleMode, Texture, TextureCreator,
    WindowCanvas,
};
use sdl2::video::WindowContext;
use sdl2::{EventPump, TimerSubsystem};

use super::super::input::sample;
use super::{
    apply_window_mode, copy_to_viewport, drawable_allowed, drawable_viewport,
    validate_dimensions,
};
use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::graphics::scene_renderer::SceneBackend;
use crate::ir::hardware::InputSample;
use crate::ir::scene::{AtlasId, SpriteDraw, SpriteRect};
use crate::ir::security::scene::AdmittedDraws;

struct Atlases<'texture, Context> {
    creator: &'texture TextureCreator<Context>,
    entries: Vec<Texture<'texture>>,
}

impl<'texture, Context> Atlases<'texture, Context> {
    fn load(
        &mut self,
        id: AtlasId,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<(), RenderError> {
        if id.0 != self.entries.len() {
            return Err(RenderError::InvalidSprite);
        }
        self.entries
            .try_reserve(1)
            .map_err(|_error| RenderError::TooLarge)?;
        let mut texture = self
            .creator
            .create_texture_static(
                PixelFormatEnum::RGBA32,
                layout.width().get(),
                layout.height().get(),
            )
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        texture.set_blend_mode(BlendMode::BLEND);
        texture.set_scale_mode(ScaleMode::Nearest);
        texture
            .update(None, pixels, layout.pitch())
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        self.entries.push(texture);
        Ok(())
    }

    const fn new(creator: &'texture TextureCreator<Context>) -> Self {
        Self {
            creator,
            entries: Vec::new(),
        }
    }

    fn validate(&self, sprites: &[SpriteDraw]) -> Result<(), RenderError> {
        for sprite in sprites {
            if self.entries.get(sprite.atlas.0).is_none() {
                return Err(RenderError::InvalidSprite);
            }
            let _source = native_rectangle(sprite.source)?;
            let _destination = native_rectangle(sprite.destination)?;
        }
        Ok(())
    }
}

pub(super) struct SceneTextures<'texture, Context> {
    atlases: Atlases<'texture, Context>,
    layout: FrameLayout,
    mode: PresentationMode,
    target: Texture<'texture>,
}

impl<'texture, Context> SceneTextures<'texture, Context> {
    fn draw<Target>(
        &mut self,
        canvas: &mut Canvas<Target>,
        background: [u8; 3],
        admitted: AdmittedDraws<'_>,
    ) -> Result<Option<PresentationLayout>, RenderError>
    where
        Target: RenderTarget,
    {
        let sprites = admitted.as_slice();
        self.atlases.validate(sprites)?;
        let Some(drawable) = drawable_viewport(canvas, self.layout, self.mode)?
        else {
            return Ok(None);
        };
        let mut result = Ok(());
        canvas
            .with_texture_canvas(&mut self.target, |logical| {
                result = compose(
                    logical,
                    &self.atlases.entries,
                    background,
                    sprites,
                );
            })
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        result?;
        copy_to_viewport(canvas, &self.target, drawable)?;
        Ok(Some(drawable))
    }

    pub(super) fn new(
        creator: &'texture TextureCreator<Context>,
        layout: FrameLayout,
        mode: PresentationMode,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            atlases: Atlases::new(creator),
            layout,
            mode,
            target: create_target(creator, layout)?,
        })
    }

    fn readback<Target>(
        &mut self,
        canvas: &mut Canvas<Target>,
    ) -> Result<Vec<u8>, RenderError>
    where
        Target: RenderTarget,
    {
        let mut result = Err(RenderError::InvalidLength);
        let region = Rect::new(
            0,
            0,
            self.layout.width().get(),
            self.layout.height().get(),
        );
        canvas
            .with_texture_canvas(&mut self.target, |logical| {
                result = logical
                    .read_pixels(region, PixelFormatEnum::RGBA32)
                    .map_err(RenderError::Backend);
            })
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let pixels = result?;
        if pixels.len() != self.layout.bytes() {
            return Err(RenderError::InvalidLength);
        }
        Ok(pixels)
    }

    fn resize(&mut self, layout: FrameLayout) -> Result<(), RenderError> {
        if layout != self.layout {
            let target = create_target(self.atlases.creator, layout)?;
            self.target = target;
            self.layout = layout;
        }
        Ok(())
    }
}

/// A logical render target, persistent atlases and a native event pump.
pub(super) struct LinuxScene<'canvas, 'texture> {
    canvas: &'canvas mut WindowCanvas,
    diagnostic_hidden: bool,
    events: EventPump,
    last_presented: Option<PresentationLayout>,
    textures: SceneTextures<'texture, WindowContext>,
    timer: TimerSubsystem,
}

impl Debug for LinuxScene<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxScene")
            .field("layout", &self.textures.layout)
            .finish_non_exhaustive()
    }
}

impl SceneBackend for LinuxScene<'_, '_> {
    /// Change the current window and viewport without rebuilding atlases.
    ///
    /// # Errors
    /// Reports a rejected native fullscreen transition.
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError> {
        apply_window_mode(self.canvas, settings.window)?;
        self.textures.mode = settings.presentation;
        Ok(())
    }

    /// Exact layout of the last completed presentation attempt.
    /// None before presentation or when no drawable was available.
    fn last_presented_layout(&self) -> Option<PresentationLayout> {
        self.last_presented
    }

    /// Current logical target, including any admitted adaptive extension.
    fn layout(&self) -> FrameLayout {
        self.textures.layout
    }

    /// Upload one tightly packed RGBA atlas for reuse in subsequent frames.
    /// # Errors
    /// Rejects an inconsistent atlas identity or native upload failure.
    fn load_atlas(
        &mut self,
        id: AtlasId,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<(), RenderError> {
        self.textures.atlases.load(id, layout, pixels)
    }

    fn poll_event(&mut self, wait_ms: u32) -> Option<InputSample> {
        let event = if wait_ms == 0 {
            self.events.poll_event()
        } else {
            self.events.wait_event_timeout(wait_ms)
        }?;
        Some(sample(
            &event,
            self.canvas.window().id(),
            self.timer.ticks64(),
        ))
    }

    /// Composite a frame with no full-frame CPU pixel upload.
    /// # Errors
    /// Rejects invalid draw commands before rendering or native failures.
    fn present(
        &mut self,
        background: [u8; 3],
        sprites: AdmittedDraws<'_>,
    ) -> Result<bool, RenderError> {
        if !drawable_allowed(
            self.canvas.window().window_flags(),
            self.diagnostic_hidden,
        ) {
            self.textures.atlases.validate(sprites.as_slice())?;
            self.last_presented = None;
            return Ok(false);
        }
        let drawable = self.textures.draw(self.canvas, background, sprites)?;
        if drawable.is_some() {
            self.canvas.present();
        }
        self.last_presented = drawable;
        Ok(drawable.is_some())
    }

    fn presentation_mode(&self) -> PresentationMode {
        self.textures.mode
    }

    /// Read the current logical scene as RGBA bytes.
    ///
    /// Call only immediately after successful presentation. This diagnostic
    /// allocates one bounded frame and synchronises the renderer; do not use
    /// per frame.
    ///
    /// # Errors
    /// Rejects failed native readback or an unexpected byte extent.
    fn readback(&mut self) -> Result<Vec<u8>, RenderError> {
        self.textures.readback(self.canvas)
    }

    fn resize_target(
        &mut self,
        layout: FrameLayout,
    ) -> Result<(), RenderError> {
        self.textures.resize(layout)
    }

    fn surface_dimensions(&self) -> Result<Option<Dimensions>, RenderError> {
        let (width, height) =
            self.canvas.output_size().map_err(RenderError::Backend)?;
        Ok(NonZeroU32::new(width)
            .zip(NonZeroU32::new(height))
            .map(|(columns, rows)| Dimensions::new(columns, rows)))
    }

    /// Observe the last presented layout if it matches current geometry.
    /// # Errors
    /// Reports failure to query the drawable dimensions.
    fn visible_layout(
        &self,
    ) -> Result<Option<PresentationLayout>, RenderError> {
        if !drawable_allowed(
            self.canvas.window().window_flags(),
            self.diagnostic_hidden,
        ) {
            return Ok(None);
        }
        let current = drawable_viewport(
            self.canvas,
            self.textures.layout,
            self.textures.mode,
        )?;
        // A resize or policy change can precede the next completed draw.
        // Never interpret a touch against controls not yet presented.
        Ok(self
            .last_presented
            .filter(|layout| Some(*layout) == current))
    }
}

impl<'canvas, 'texture> LinuxScene<'canvas, 'texture> {
    pub(super) fn new(
        (canvas, diagnostic_hidden): (&'canvas mut WindowCanvas, bool),
        events: EventPump,
        textures: SceneTextures<'texture, WindowContext>,
        timer: TimerSubsystem,
    ) -> Result<Self, RenderError> {
        if !canvas.render_target_supported() {
            return Err(RenderError::Backend(String::from(
                "SDL render targets are unavailable",
            )));
        }
        Ok(Self {
            canvas,
            diagnostic_hidden,
            events,
            last_presented: None,
            textures,
            timer,
        })
    }
}

fn native_rectangle(rectangle: SpriteRect) -> Result<Rect, RenderError> {
    validate_dimensions(rectangle.width, rectangle.height)?;
    let max = i32::try_from(max_int_value())
        .map_err(|_error| RenderError::InvalidDimensions)?;
    if !(min_int_value()..=max).contains(&rectangle.x)
        || !(min_int_value()..=max).contains(&rectangle.y)
    {
        return Err(RenderError::InvalidSprite);
    }
    Ok(Rect::new(
        rectangle.x,
        rectangle.y,
        rectangle.width.get(),
        rectangle.height.get(),
    ))
}

fn compose<Target>(
    canvas: &mut Canvas<Target>,
    atlases: &[Texture<'_>],
    background: [u8; 3],
    sprites: &[SpriteDraw],
) -> Result<(), RenderError>
where
    Target: RenderTarget,
{
    let [red, green, blue] = background;
    canvas.set_draw_color(Color::RGB(red, green, blue));
    canvas.clear();
    for sprite in sprites {
        let atlas = atlases
            .get(sprite.atlas.0)
            .ok_or(RenderError::InvalidSprite)?;
        let source = native_rectangle(sprite.source)?;
        let destination = native_rectangle(sprite.destination)?;
        if sprite.flip_horizontal || sprite.flip_vertical {
            canvas
                .copy_ex(
                    atlas,
                    source,
                    destination,
                    0.0,
                    None,
                    sprite.flip_horizontal,
                    sprite.flip_vertical,
                )
                .map_err(RenderError::Backend)?;
        } else {
            canvas
                .copy(atlas, source, destination)
                .map_err(RenderError::Backend)?;
        }
    }
    Ok(())
}

fn create_target<Context>(
    creator: &TextureCreator<Context>,
    layout: FrameLayout,
) -> Result<Texture<'_>, RenderError> {
    let mut target = creator
        .create_texture_target(
            PixelFormatEnum::RGBA32,
            layout.width().get(),
            layout.height().get(),
        )
        .map_err(|error| RenderError::Backend(error.to_string()))?;
    target.set_blend_mode(BlendMode::NONE);
    target.set_scale_mode(ScaleMode::Nearest);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use games::presentation::{Dimensions, PresentationMode};
    use sdl2::pixels::PixelFormatEnum;
    use sdl2::render::Canvas;
    use sdl2::surface::{Surface, SurfaceContext};

    use super::{
        Atlases, FrameLayout, RenderError, SceneTextures, SpriteDraw,
        SpriteRect, native_rectangle,
    };
    use crate::ir::frame::AdaptiveWidthRequest;
    use crate::ir::scene::{AtlasId, SceneLimits};
    use crate::ir::security::scene::SceneAdmission;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const TRANSPARENT: [u8; 4] = [255, 255, 255, 0];

    fn atlas_layout() -> Result<FrameLayout, RenderError> {
        FrameLayout::new(nz(2), nz(1), budget(8))
    }

    fn budget(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
    }

    fn canvas() -> Result<Canvas<Surface<'static>>, RenderError> {
        Surface::new(4, 2, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)?
            .into_canvas()
            .map_err(RenderError::Backend)
    }

    fn upload(
        admission: &mut SceneAdmission,
        atlases: &mut Atlases<'_, SurfaceContext<'_>>,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError> {
        let layout = atlas_layout()?;
        admission
            .load(layout, pixels.len(), |id| atlases.load(id, layout, pixels))
    }

    fn commands(
        admission: &mut SceneAdmission,
        atlases: &mut Atlases<'_, SurfaceContext<'_>>,
    ) -> Result<[SpriteDraw; 2], RenderError> {
        let first = upload(admission, atlases, [RED, GREEN].as_flattened())?;
        let second =
            upload(admission, atlases, [TRANSPARENT, BLUE].as_flattened())?;
        Ok([
            SpriteDraw {
                atlas: first,
                destination: rect(0, 0, 4, 2),
                flip_horizontal: true,
                flip_vertical: false,
                source: rect(0, 0, 2, 1),
            },
            SpriteDraw {
                atlas: second,
                destination: rect(1, 0, 2, 1),
                flip_horizontal: false,
                flip_vertical: false,
                source: rect(0, 0, 2, 1),
            },
        ])
    }

    fn layout() -> Result<FrameLayout, RenderError> {
        FrameLayout::new(nz(4), nz(2), budget(32))
    }

    fn limits() -> SceneLimits {
        SceneLimits {
            atlas_bytes: budget(16),
            atlases: budget(2),
            sprites: budget(2),
        }
    }

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    fn read(canvas: &Canvas<Surface<'_>>) -> Result<Vec<u8>, RenderError> {
        canvas
            .read_pixels(None, PixelFormatEnum::RGBA32)
            .map_err(RenderError::Backend)
    }

    fn rect(left: i32, top: i32, width: u32, height: u32) -> SpriteRect {
        SpriteRect {
            height: nz(height),
            width: nz(width),
            x: left,
            y: top,
        }
    }

    fn require(condition: bool, reason: &str) -> Result<(), RenderError> {
        condition
            .then_some(())
            .ok_or_else(|| RenderError::Backend(reason.to_owned()))
    }

    #[test]
    fn adaptive_target_changes_extent_and_preserves_atlases()
    -> Result<(), RenderError> {
        let mut canvas = canvas()?;
        let creator = canvas.texture_creator();
        let base = layout()?;
        let maximum = FrameLayout::new(nz(6), nz(2), budget(48))?;
        let mut scene =
            SceneTextures::new(&creator, base, PresentationMode::Adaptive)?;
        let mut admission = SceneAdmission::new(limits());
        let [sprite, _overlay] = commands(&mut admission, &mut scene.atlases)?;
        let wide = Some(Dimensions::new(nz(12), nz(2)));
        let request = AdaptiveWidthRequest::new(base, maximum)?;
        scene.resize(request.select(scene.mode, wide)?)?;
        require(scene.layout == maximum, "width cap differs")?;
        let edge = SpriteDraw {
            destination: rect(4, 1, 2, 1),
            flip_horizontal: false,
            ..sprite
        };
        require(
            scene.draw(&mut canvas, [0; 3], admission.draws(&[edge])?)?
                == Some(PresentationMode::Adaptive.layout(
                    Dimensions::new(nz(4), nz(2)),
                    Dimensions::new(nz(6), nz(2)),
                )),
            "submitted layout differs",
        )?;
        let mut expected = [BLACK; 12];
        expected
            .get_mut(10..12)
            .ok_or(RenderError::InvalidLength)?
            .copy_from_slice(&[RED, GREEN]);
        require(
            scene.readback(&mut canvas)? == expected.as_flattened(),
            "resized target or persistent atlas differs",
        )?;
        let wrong_height = FrameLayout::new(nz(6), nz(3), budget(72))?;
        require(
            AdaptiveWidthRequest::new(base, wrong_height)
                == Err(RenderError::InvalidDimensions)
                && scene.layout == maximum,
            "invalid extent changed target",
        )?;
        scene.mode = PresentationMode::AspectFit;
        scene.resize(request.select(scene.mode, wide)?)?;
        require(scene.layout == base, "base not restored")?;
        require(
            scene
                .draw(&mut canvas, [0; 3], admission.draws(&[sprite])?)?
                .is_some(),
            "atlas lost",
        )?;
        let expected_base = [GREEN, GREEN, RED, RED, GREEN, GREEN, RED, RED];
        require(
            scene.readback(&mut canvas)? == expected_base.as_flattened(),
            "restored target differs",
        )
    }

    #[test]
    fn atlas_limits_reject_without_consuming_budget() -> Result<(), RenderError>
    {
        let canvas = canvas()?;
        let creator = canvas.texture_creator();
        let mut atlases = Atlases::new(&creator);
        let mut admission = SceneAdmission::new(SceneLimits {
            atlases: budget(3),
            ..limits()
        });
        require(
            upload(&mut admission, &mut atlases, &[])
                == Err(RenderError::InvalidLength),
            "invalid upload accepted",
        )?;
        let _commands = commands(&mut admission, &mut atlases)?;
        require(
            upload(&mut admission, &mut atlases, [RED, GREEN].as_flattened())
                == Err(RenderError::TooLarge),
            "atlas byte limit exceeded",
        )?;
        let mut limited = Atlases::new(&creator);
        let mut count = SceneAdmission::new(SceneLimits {
            atlas_bytes: budget(32),
            ..limits()
        });
        let _loaded = commands(&mut count, &mut limited)?;
        require(
            upload(&mut count, &mut limited, [RED, GREEN].as_flattened())
                == Err(RenderError::TooLarge),
            "atlas count exceeded",
        )
    }

    #[test]
    fn ordered_sprites_blend_flip_and_clip() -> Result<(), RenderError> {
        let mut canvas = canvas()?;
        let creator = canvas.texture_creator();
        let mut scene = SceneTextures::new(
            &creator,
            layout()?,
            PresentationMode::AspectFit,
        )?;
        let mut admission = SceneAdmission::new(limits());
        let [base, overlay] = commands(&mut admission, &mut scene.atlases)?;
        require(
            scene
                .draw(&mut canvas, [0; 3], admission.draws(&[base, overlay])?)?
                .is_some(),
            "scene not drawn",
        )?;
        let expected = [GREEN, GREEN, BLUE, RED, GREEN, GREEN, RED, RED];
        require(
            scene.readback(&mut canvas)? == expected.as_flattened(),
            "logical target pixels differ",
        )?;
        require(
            read(&canvas)? == expected.as_flattened(),
            "sprite pixels differ",
        )?;
        let clipped = SpriteDraw {
            destination: rect(-1, 1, 2, 1),
            flip_horizontal: false,
            ..base
        };
        require(
            scene
                .draw(&mut canvas, [0; 3], admission.draws(&[clipped])?)?
                .is_some(),
            "clipped scene not drawn",
        )?;
        let clipped_pixels =
            [BLACK, BLACK, BLACK, BLACK, GREEN, BLACK, BLACK, BLACK];
        require(
            read(&canvas)? == clipped_pixels.as_flattened(),
            "clipping or frame clear differs",
        )
    }

    #[test]
    fn invalid_commands_preserve_the_display() -> Result<(), RenderError> {
        let mut canvas = canvas()?;
        let creator = canvas.texture_creator();
        let mut scene = SceneTextures::new(
            &creator,
            layout()?,
            PresentationMode::AspectFit,
        )?;
        let mut admission = SceneAdmission::new(limits());
        let [base, overlay] = commands(&mut admission, &mut scene.atlases)?;
        require(
            scene
                .draw(&mut canvas, [0; 3], admission.draws(&[base, overlay])?)?
                .is_some(),
            "scene not drawn",
        )?;
        let result = read(&canvas)?;
        let invalid = SpriteDraw {
            source: rect(1, 0, 2, 1),
            ..base
        };
        require(
            admission
                .draws(&[base, invalid])
                .and_then(|admitted| scene.draw(&mut canvas, [0; 3], admitted))
                == Err(RenderError::InvalidSprite),
            "invalid source accepted",
        )?;
        let oversized = [base, base, overlay];
        require(
            admission
                .draws(&oversized)
                .and_then(|admitted| scene.draw(&mut canvas, [0; 3], admitted))
                == Err(RenderError::TooLarge),
            "command limit exceeded",
        )?;
        let invalid_native = SpriteDraw {
            destination: rect(i32::MAX, 0, 2, 1),
            ..base
        };
        require(
            admission
                .draws(&[base, invalid_native])
                .and_then(|admitted| scene.draw(&mut canvas, [0; 3], admitted))
                == Err(RenderError::InvalidSprite),
            "portable coordinate overflow accepted",
        )?;
        require(
            native_rectangle(invalid_native.destination)
                == Err(RenderError::InvalidSprite),
            "native coordinate overflow accepted",
        )?;
        require(
            read(&canvas)? == result,
            "rejected scene changed the output",
        )
    }
}
