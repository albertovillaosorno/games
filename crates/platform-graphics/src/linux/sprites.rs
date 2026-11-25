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
use std::num::{NonZeroU32, NonZeroUsize};

use games::input::touch::TouchPoint;
use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
};
use sdl2::EventPump;
use sdl2::event::Event;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::{Rect, max_int_value, min_int_value};
use sdl2::render::{
    BlendMode, Canvas, RenderTarget, ScaleMode, Texture, TextureCreator,
    WindowCanvas,
};
use sdl2::video::WindowContext;

use super::{
    FrameLayout, RenderError, apply_window_mode, copy_to_viewport,
    drawable_viewport, validate_dimensions,
};

/// An atlas identifier scoped to a scene session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasId(usize);

/// Limits on atlas allocation and per-frame command count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneLimits {
    /// Total logical RGBA bytes permitted across loaded atlases.
    pub atlas_bytes: NonZeroUsize,
    /// Maximum number of simultaneously loaded atlases.
    pub atlases: NonZeroUsize,
    /// Maximum number of draw commands per frame.
    pub sprites: NonZeroUsize,
}

/// Result of processing one bounded batch of native events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventBatch {
    /// At least one event was delivered before a poll returned no event.
    Drained,
    /// The initial poll returned no event.
    Empty,
    /// The event budget was reached; more events may be waiting.
    Limited,
    /// The caller requested termination from its event handler.
    Stopped,
}

/// A nonempty integer rectangle in atlas or logical scene pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpriteRect {
    /// Rectangle height.
    pub height: NonZeroU32,
    /// Rectangle width.
    pub width: NonZeroU32,
    /// Horizontal origin.
    pub x: i32,
    /// Vertical origin.
    pub y: i32,
}

impl SpriteRect {
    fn native(self) -> Result<Rect, RenderError> {
        validate_dimensions(self.width, self.height)?;
        let max = i32::try_from(max_int_value())
            .map_err(|_error| RenderError::InvalidDimensions)?;
        if !(min_int_value()..=max).contains(&self.x)
            || !(min_int_value()..=max).contains(&self.y)
        {
            return Err(RenderError::InvalidSprite);
        }
        Ok(Rect::new(
            self.x,
            self.y,
            self.width.get(),
            self.height.get(),
        ))
    }

    fn source(self, layout: FrameLayout) -> Result<Rect, RenderError> {
        let left = u32::try_from(self.x)
            .map_err(|_error| RenderError::InvalidSprite)?;
        let top = u32::try_from(self.y)
            .map_err(|_error| RenderError::InvalidSprite)?;
        if left
            .checked_add(self.width.get())
            .is_none_or(|right| right > layout.width().get())
            || top
                .checked_add(self.height.get())
                .is_none_or(|bottom| bottom > layout.height().get())
        {
            return Err(RenderError::InvalidSprite);
        }
        self.native()
    }
}

/// One ordered sprite command; later commands are drawn over earlier ones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpriteDraw {
    /// Atlas to sample.
    pub atlas: AtlasId,
    /// Destination in logical scene pixels (may be clipped).
    pub destination: SpriteRect,
    /// Reverse horizontal pixel order.
    pub flip_horizontal: bool,
    /// Reverse vertical pixel order.
    pub flip_vertical: bool,
    /// Source rectangle entirely within the atlas.
    pub source: SpriteRect,
}

struct Atlas<'texture> {
    layout: FrameLayout,
    texture: Texture<'texture>,
}

struct Atlases<'texture, Context> {
    creator: &'texture TextureCreator<Context>,
    entries: Vec<Atlas<'texture>>,
    limits: SceneLimits,
    remaining_bytes: usize,
}

impl<'texture, Context> Atlases<'texture, Context> {
    fn load(
        &mut self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError> {
        if pixels.len() != layout.bytes() {
            return Err(RenderError::InvalidLength);
        }
        if self.entries.len() >= self.limits.atlases.get() {
            return Err(RenderError::TooLarge);
        }
        let remaining = self
            .remaining_bytes
            .checked_sub(layout.bytes())
            .ok_or(RenderError::TooLarge)?;
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
            .update(None, pixels, layout.pitch)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let id = AtlasId(self.entries.len());
        self.entries.push(Atlas { layout, texture });
        self.remaining_bytes = remaining;
        Ok(id)
    }

    const fn new(
        creator: &'texture TextureCreator<Context>,
        limits: SceneLimits,
    ) -> Self {
        Self {
            creator,
            entries: Vec::new(),
            limits,
            remaining_bytes: limits.atlas_bytes.get(),
        }
    }

    fn validate(&self, sprites: &[SpriteDraw]) -> Result<(), RenderError> {
        if sprites.len() > self.limits.sprites.get() {
            return Err(RenderError::TooLarge);
        }
        for sprite in sprites {
            let atlas = self
                .entries
                .get(sprite.atlas.0)
                .ok_or(RenderError::InvalidSprite)?;
            let _source = sprite.source.source(atlas.layout)?;
            let _destination = sprite.destination.native()?;
        }
        Ok(())
    }
}

pub(super) struct SceneTextures<'texture, Context> {
    atlases: Atlases<'texture, Context>,
    base: FrameLayout,
    layout: FrameLayout,
    mode: PresentationMode,
    target: Texture<'texture>,
}

impl<'texture, Context> SceneTextures<'texture, Context> {
    fn draw<Target>(
        &mut self,
        canvas: &mut Canvas<Target>,
        background: [u8; 3],
        sprites: &[SpriteDraw],
    ) -> Result<Option<PresentationLayout>, RenderError>
    where
        Target: RenderTarget,
    {
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

    fn fit(
        &mut self,
        surface: Option<Dimensions>,
        maximum: FrameLayout,
    ) -> Result<FrameLayout, RenderError> {
        if maximum.height() != self.base.height()
            || maximum.width() < self.base.width()
        {
            return Err(RenderError::InvalidDimensions);
        }
        let base = Dimensions::new(self.base.width(), self.base.height());
        let design = surface.map_or(base, |extent| {
            self.mode.design_for_surface(extent, base, maximum.width())
        });
        if design.width != self.layout.width() {
            let budget = NonZeroUsize::new(maximum.bytes())
                .ok_or(RenderError::TooLarge)?;
            let layout = FrameLayout::new(design.width, design.height, budget)?;
            let target = create_target(self.atlases.creator, layout)?;
            self.target = target;
            self.layout = layout;
        }
        Ok(self.layout)
    }

    pub(super) fn new(
        creator: &'texture TextureCreator<Context>,
        layout: FrameLayout,
        limits: SceneLimits,
        mode: PresentationMode,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            atlases: Atlases::new(creator, limits),
            base: layout,
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
}

/// A logical render target, persistent atlases and a native event pump.
pub struct SceneSession<'canvas, 'texture> {
    canvas: &'canvas mut WindowCanvas,
    events: EventPump,
    last_presented: Option<PresentationLayout>,
    textures: SceneTextures<'texture, WindowContext>,
}

impl Debug for SceneSession<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneSession")
            .field("layout", &self.textures.layout)
            .finish_non_exhaustive()
    }
}

impl<'canvas, 'texture> SceneSession<'canvas, 'texture> {
    /// Change the current window and viewport without rebuilding atlases.
    ///
    /// # Errors
    /// Reports a rejected native fullscreen transition.
    pub fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError> {
        apply_window_mode(self.canvas, settings.window)?;
        self.textures.mode = settings.presentation;
        Ok(())
    }

    /// Dispatch at most `budget` events in native queue order.
    ///
    /// Only the first poll may wait, if `wait_ms` is nonzero. Reaching the
    /// limit does not consume or inspect the next event. Callers can perform
    /// simulation and network work between batches even during input floods.
    /// # Errors
    /// Propagates an event handler failure without draining further events.
    pub fn dispatch_events<Dispatch>(
        &mut self,
        budget: NonZeroUsize,
        wait_ms: u32,
        dispatch: Dispatch,
    ) -> Result<EventBatch, RenderError>
    where
        Dispatch: FnMut(&Self, &Event) -> Result<bool, RenderError>,
    {
        dispatch_batch(
            self,
            budget,
            |scene, first| {
                if first && wait_ms != 0 {
                    scene.events.wait_event_timeout(wait_ms)
                } else {
                    scene.events.poll_event()
                }
            },
            dispatch,
        )
    }

    /// Choose the bounded logical extent for the next frame.
    ///
    /// The maximum must retain the initial height and contain its width.
    /// Only adaptive mode may extend; other modes restore the base target.
    /// Atlases stay resident, and the old target survives allocation failure.
    /// # Errors
    /// Rejects an invalid maximum or native query/allocation failure.
    pub fn fit_adaptive_width(
        &mut self,
        maximum: FrameLayout,
    ) -> Result<FrameLayout, RenderError> {
        if self.textures.mode != PresentationMode::Adaptive
            || maximum.width() <= self.textures.base.width()
        {
            return self.textures.fit(None, maximum);
        }
        let (width, height) =
            self.canvas.output_size().map_err(RenderError::Backend)?;
        let surface = NonZeroU32::new(width)
            .zip(NonZeroU32::new(height))
            .map(|(columns, rows)| Dimensions::new(columns, rows));
        self.textures.fit(surface, maximum)
    }

    /// Exact layout of the last completed presentation attempt.
    /// None before presentation or when no drawable was available.
    #[must_use]
    pub const fn last_presented_layout(&self) -> Option<PresentationLayout> {
        self.last_presented
    }

    /// Current logical target, including any admitted adaptive extension.
    #[must_use]
    pub const fn layout(&self) -> FrameLayout {
        self.textures.layout
    }

    /// Upload one tightly packed RGBA atlas for reuse in subsequent frames.
    /// # Errors
    /// Rejects byte/count limit violations or failed native allocation.
    pub fn load_atlas(
        &mut self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError> {
        self.textures.atlases.load(layout, pixels)
    }

    /// Project a surface-normalized touch point into the displayed scene.
    /// Letterbox space, unavailable or not yet redrawn surfaces return None.
    /// # Errors
    /// Reports failure to query the drawable dimensions.
    pub fn logical_touch(
        &self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        Ok(self
            .visible_layout()?
            .and_then(|layout| point.in_presentation(layout)))
    }

    pub(super) fn new(
        canvas: &'canvas mut WindowCanvas,
        events: EventPump,
        textures: SceneTextures<'texture, WindowContext>,
    ) -> Result<Self, RenderError> {
        if !canvas.render_target_supported() {
            return Err(RenderError::Backend(String::from(
                "SDL render targets are unavailable",
            )));
        }
        Ok(Self {
            canvas,
            events,
            last_presented: None,
            textures,
        })
    }

    /// Composite a frame with no full-frame CPU pixel upload.
    /// # Errors
    /// Rejects invalid draw commands before rendering or native failures.
    pub fn present(
        &mut self,
        background: [u8; 3],
        sprites: &[SpriteDraw],
    ) -> Result<bool, RenderError> {
        let drawable = self.textures.draw(self.canvas, background, sprites)?;
        if drawable.is_some() {
            self.canvas.present();
        }
        self.last_presented = drawable;
        Ok(drawable.is_some())
    }

    pub(super) const fn presentation_mode(&self) -> PresentationMode {
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
    pub fn readback(&mut self) -> Result<Vec<u8>, RenderError> {
        self.textures.readback(self.canvas)
    }

    /// Project a displayed logical point onto the physical surface.
    /// Returns None until the current geometry has been presented.
    /// # Errors
    /// Reports failure to query the drawable dimensions.
    pub fn surface_touch(
        &self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        Ok(self
            .visible_layout()?
            .and_then(|layout| point.on_presentation(layout)))
    }

    pub(super) fn visible_layout(
        &self,
    ) -> Result<Option<PresentationLayout>, RenderError> {
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

fn dispatch_batch<Source, Next, Dispatch>(
    source: &mut Source,
    budget: NonZeroUsize,
    mut next: Next,
    mut dispatch: Dispatch,
) -> Result<EventBatch, RenderError>
where
    Next: FnMut(&mut Source, bool) -> Option<Event>,
    Dispatch: FnMut(&Source, &Event) -> Result<bool, RenderError>,
{
    for index in 0..budget.get() {
        let first = index == 0;
        let Some(event) = next(source, first) else {
            return Ok(if first {
                EventBatch::Empty
            } else {
                EventBatch::Drained
            });
        };
        if !dispatch(source, &event)? {
            return Ok(EventBatch::Stopped);
        }
    }
    Ok(EventBatch::Limited)
}

fn compose<Target>(
    canvas: &mut Canvas<Target>,
    atlases: &[Atlas<'_>],
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
        let source = sprite.source.source(atlas.layout)?;
        let destination = sprite.destination.native()?;
        if sprite.flip_horizontal || sprite.flip_vertical {
            canvas
                .copy_ex(
                    &atlas.texture,
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
                .copy(&atlas.texture, source, destination)
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
    use std::collections::VecDeque;
    use std::num::{NonZeroU32, NonZeroUsize};

    use games::presentation::{Dimensions, PresentationMode};
    use sdl2::event::Event;
    use sdl2::pixels::PixelFormatEnum;
    use sdl2::render::Canvas;
    use sdl2::surface::{Surface, SurfaceContext};

    use super::{
        Atlases, EventBatch, FrameLayout, RenderError, SceneLimits,
        SceneTextures, SpriteDraw, SpriteRect, dispatch_batch,
    };

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

    fn commands(
        atlases: &mut Atlases<'_, SurfaceContext<'_>>,
    ) -> Result<[SpriteDraw; 2], RenderError> {
        let first =
            atlases.load(atlas_layout()?, [RED, GREEN].as_flattened())?;
        let second = atlases
            .load(atlas_layout()?, [TRANSPARENT, BLUE].as_flattened())?;
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
    fn bounded_native_batches_preserve_order_and_queue_tail()
    -> Result<(), RenderError> {
        let mut queue = (1..=5)
            .map(|timestamp| Event::Quit { timestamp })
            .collect::<VecDeque<_>>();
        let mut delivered = Vec::new();
        let mut first_polls = Vec::new();
        for (expected, count) in [
            (EventBatch::Limited, 2),
            (EventBatch::Limited, 4),
            (EventBatch::Drained, 5),
            (EventBatch::Empty, 5),
        ] {
            let result = dispatch_batch(
                &mut queue,
                budget(2),
                |source, first| {
                    first_polls.push(first);
                    source.pop_front()
                },
                |_source, event| {
                    delivered.push(event.get_timestamp());
                    Ok(true)
                },
            )?;
            require(result == expected, "incorrect event batch result")?;
            require(delivered.len() == count, "unexpected queue consumption")?;
        }
        require(delivered == [1, 2, 3, 4, 5], "event order differs")?;
        require(
            first_polls == [true, false, true, false, true, false, true],
            "only the initial poll may wait",
        )
    }

    #[test]
    fn event_stop_or_failure_does_not_consume_next_native_event()
    -> Result<(), RenderError> {
        let mut queue = (1..=3)
            .map(|timestamp| Event::Quit { timestamp })
            .collect::<VecDeque<_>>();
        require(
            dispatch_batch(
                &mut queue,
                budget(3),
                |source, _first| source.pop_front(),
                |_source, _event| Ok(false),
            )? == EventBatch::Stopped,
            "stop not reported",
        )?;
        require(
            queue.front() == Some(&Event::Quit { timestamp: 2 }),
            "stop consumed the next event",
        )?;
        require(
            dispatch_batch(
                &mut queue,
                budget(3),
                |source, _first| source.pop_front(),
                |_source, _event| Err(RenderError::InvalidLength),
            ) == Err(RenderError::InvalidLength),
            "handler error not propagated",
        )?;
        require(
            queue.front() == Some(&Event::Quit { timestamp: 3 }),
            "failure consumed the next event",
        )
    }

    #[test]
    fn adaptive_target_changes_extent_and_preserves_atlases()
    -> Result<(), RenderError> {
        let mut canvas = canvas()?;
        let creator = canvas.texture_creator();
        let base = layout()?;
        let maximum = FrameLayout::new(nz(6), nz(2), budget(48))?;
        let mut scene = SceneTextures::new(
            &creator,
            base,
            limits(),
            PresentationMode::Adaptive,
        )?;
        let [sprite, _overlay] = commands(&mut scene.atlases)?;
        let wide = Some(Dimensions::new(nz(12), nz(2)));
        require(scene.fit(wide, maximum)? == maximum, "width cap differs")?;
        let edge = SpriteDraw {
            destination: rect(4, 1, 2, 1),
            flip_horizontal: false,
            ..sprite
        };
        require(
            scene.draw(&mut canvas, [0; 3], &[edge])?
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
            scene.fit(wide, wrong_height)
                == Err(RenderError::InvalidDimensions)
                && scene.layout == maximum,
            "invalid extent changed target",
        )?;
        scene.mode = PresentationMode::AspectFit;
        require(scene.fit(wide, maximum)? == base, "base not restored")?;
        require(
            scene.draw(&mut canvas, [0; 3], &[sprite])?.is_some(),
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
        let mut atlases = Atlases::new(&creator, SceneLimits {
            atlases: budget(3),
            ..limits()
        });
        require(
            atlases.load(atlas_layout()?, &[])
                == Err(RenderError::InvalidLength),
            "invalid upload accepted",
        )?;
        let _commands = commands(&mut atlases)?;
        require(
            atlases.load(atlas_layout()?, [RED, GREEN].as_flattened())
                == Err(RenderError::TooLarge),
            "atlas byte limit exceeded",
        )?;
        let mut limited = Atlases::new(&creator, SceneLimits {
            atlas_bytes: budget(32),
            ..limits()
        });
        let _loaded = commands(&mut limited)?;
        require(
            limited.load(atlas_layout()?, [RED, GREEN].as_flattened())
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
            limits(),
            PresentationMode::AspectFit,
        )?;
        let [base, overlay] = commands(&mut scene.atlases)?;
        require(
            scene.draw(&mut canvas, [0; 3], &[base, overlay])?.is_some(),
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
            scene.draw(&mut canvas, [0; 3], &[clipped])?.is_some(),
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
            limits(),
            PresentationMode::AspectFit,
        )?;
        let [base, overlay] = commands(&mut scene.atlases)?;
        require(
            scene.draw(&mut canvas, [0; 3], &[base, overlay])?.is_some(),
            "scene not drawn",
        )?;
        let result = read(&canvas)?;
        let invalid = SpriteDraw {
            source: rect(1, 0, 2, 1),
            ..base
        };
        require(
            scene.draw(&mut canvas, [0; 3], &[base, invalid])
                == Err(RenderError::InvalidSprite),
            "invalid source accepted",
        )?;
        require(
            scene.draw(&mut canvas, [0; 3], &[base, base, overlay])
                == Err(RenderError::TooLarge),
            "command limit exceeded",
        )?;
        require(
            read(&canvas)? == result,
            "rejected scene changed the output",
        )
    }
}
