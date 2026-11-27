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
//   - Verified local street art extension and responsive viewport geometry.
// - Must-Not:
//   - Invent world pixels or own simulation and camera collision.
// - Allows:
//   - Inputs: Base street art, canonical strip, logical frame extent.
//   - Outputs: Pixel-preserving background and placement offsets.
//   - Side effects: Loads one bounded atlas at startup.
// - Split-When:
//   - Multiple stages require independent scrolling scene strategies.
// - Merge-When:
//   - Shared world art admission can replace the title-specific bounds.
// - Summary:
//   - Extend the opening street only by verified additional columns.
// - Description:
//   - Check the base interior exactly; retain margins only at base width.
// - Usage:
//   - Select the optional pinned strip in the local launch catalogue.
// - Defaults:
//   - Fixed base extent unless the strip and adaptive mode are selected.

//! Bounded stage art and responsive native placement.

use std::num::NonZeroU64;

use games::input::LogicalAction;
use games::presentation::{
    DisplaySettings, PresentationMode, centered_expansion, expanded_anchor,
};
use games::replay::InputStep as _;
use games_assets::{AssetFormat, ContentHash, ManifestLimits};
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};

use super::{
    Action, Art, Cabinet, Control, Controls, LoadedArt, Player, Point,
    PreviewOptions, Scene, StreetLayout, dimension, failure, live, open_bundle,
    open_selected_bundle, size, smoke_press,
};
pub(super) const BASE_WIDTH: u32 = crate::BASE_WIDTH;
use crate::{MAX_VIEW_WIDTH, SIDE, view_origin, viewport_width};
pub(super) const BASE_HEIGHT: u32 = TOP + HEIGHT;
const MAX_STRIP_WIDTH: u32 = 1664;
const HEIGHT: u32 = 196;
const TOP: u32 = 28;

#[derive(Clone, Copy, Debug)]
pub(super) struct WorldArt {
    art: Art,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct StageArt {
    pub(super) background: Art,
    pub(super) world: WorldArt,
}

impl StageArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        [base, strip]: [ContentHash; 2],
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, base)?;
        let (background, original) = load_background(scene, &bundle)?;
        let extension = open_selected_bundle(store, strip, bundle_limits()?)?;
        let world = WorldArt::load(scene, &original, &extension)?;
        Ok(Self { background, world })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct View {
    // World coordinate mapped to the left edge of the rendered frame.
    pub(super) origin: i16,
    pub(super) width: u32,
}

impl View {
    pub(super) const fn project(self, point: Point) -> Point {
        Point {
            x: point.x.saturating_sub(self.origin),
            ..point
        }
    }
}

impl WorldArt {
    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
        original: &[u8],
        extension: &CanonicalBundle,
    ) -> Result<Self, RenderError> {
        let key = "scene/world-background";
        let record = extension
            .manifest()
            .record(key)
            .ok_or_else(|| failure("missing world strip"))?;
        let AssetFormat::Rgba8 { height, width } = record.format else {
            return Err(failure("world strip must be RGBA8"));
        };
        if height.get() != HEIGHT {
            return Err(failure("unexpected world strip height"));
        }
        if record.size > u64::from(atlas_budget()) {
            return Err(RenderError::TooLarge);
        }
        let layout = FrameLayout::new(
            width,
            height,
            size(usize::try_from(atlas_budget()).map_err(failure)?)?,
        )?;
        let pixels = extension.view_payload(key).map_err(failure)?;
        validate_prefix(original, &pixels, width.get())?;
        Ok(Self {
            art: Art::upload_layout(scene, layout, &pixels)?,
        })
    }

    pub(super) fn maximum(
        self,
        base: FrameLayout,
    ) -> Result<FrameLayout, RenderError> {
        let width = viewport_width(self.art.rect.width.into());
        FrameLayout::new(
            dimension(width)?,
            base.height(),
            size(
                usize::try_from(MAX_VIEW_WIDTH)
                    .map_err(failure)?
                    .saturating_mul(usize::from(super::touch::MAX_SCENE_HEIGHT))
                    .saturating_mul(4),
            )?,
        )
    }

    pub(super) fn queue(
        self,
        original: Art,
        view: View,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let width = view.width;
        if width == BASE_WIDTH && view.origin == 0 {
            draws.push(original.draw(original.rect, false));
            return Ok(());
        }
        if width > BASE_WIDTH {
            return self.queue_wide(view, draws);
        }
        if width < BASE_WIDTH {
            return Err(RenderError::InvalidDimensions);
        }
        let inner = dimension(width.saturating_sub(SIDE * 2))?;
        if inner > self.art.rect.width {
            return Err(RenderError::InvalidDimensions);
        }
        let side = dimension(SIDE)?;
        let edge = Art {
            rect: SpriteRect {
                width: side,
                ..original.rect
            },
            ..original
        };
        draws.push(edge.draw(edge.rect, false));
        draws.push(
            edge.draw(
                SpriteRect {
                    x: i32::try_from(width.saturating_sub(SIDE))
                        .map_err(failure)?,
                    ..edge.rect
                },
                false,
            ),
        );
        let world = Art {
            rect: SpriteRect {
                width: inner,
                x: i32::from(view.origin),
                ..self.art.rect
            },
            ..self.art
        };
        draws.push(world.draw(
            SpriteRect {
                x: i32::try_from(SIDE).map_err(failure)?,
                y: i32::try_from(TOP).map_err(failure)?,
                ..world.rect
            },
            false,
        ));
        Ok(())
    }

    fn queue_wide(
        self,
        view: View,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let columns = dimension(view.width)?;
        let side = i32::try_from(SIDE).map_err(failure)?;
        let strip = Art {
            rect: SpriteRect {
                width: columns,
                x: i32::from(view.origin)
                    .checked_sub(side)
                    .ok_or(RenderError::InvalidDimensions)?,
                ..self.art.rect
            },
            ..self.art
        };
        draws.push(strip.draw(
            SpriteRect {
                x: 0,
                y: i32::try_from(TOP).map_err(failure)?,
                ..strip.rect
            },
            false,
        ));
        Ok(())
    }

    pub(super) fn scroll_limit(self) -> Result<u16, RenderError> {
        u16::try_from(
            self.art
                .rect
                .width
                .get()
                .saturating_add(SIDE * 2)
                .saturating_sub(BASE_WIDTH),
        )
        .map_err(failure)
    }

    pub(super) fn view(
        self,
        width: u32,
        camera: i16,
    ) -> Result<View, RenderError> {
        world_view(self.art.rect.width.get(), width, camera)
    }
}

pub(super) const fn atlas_budget() -> u32 {
    MAX_STRIP_WIDTH * HEIGHT * 4
}

pub(super) fn load_background<'bundle>(
    scene: &mut SceneSession<'_, '_>,
    bundle: &'bundle CanonicalBundle,
) -> Result<LoadedArt<'bundle>, RenderError> {
    let (mut background, original) =
        Art::load_with_pixels(scene, bundle, "scene/background")?;
    if (background.rect.width.get(), background.rect.height.get())
        != (BASE_WIDTH, BASE_HEIGHT)
    {
        return Err(failure("unexpected background dimensions"));
    }
    // The captured HUD is replaced by live station state.
    background.rect.y = i32::try_from(TOP).map_err(failure)?;
    background.rect.height = dimension(HEIGHT)?;
    Ok((background, original))
}

pub(super) fn stage_atlas_budget() -> usize {
    usize::try_from(
        BASE_WIDTH
            .saturating_mul(BASE_HEIGHT)
            .saturating_mul(4)
            .saturating_add(atlas_budget()),
    )
    .unwrap_or_default()
}

pub(super) fn bundle_limits() -> Result<ManifestLimits, RenderError> {
    let bytes = NonZeroU64::new(u64::from(atlas_budget()))
        .ok_or(RenderError::TooLarge)?;
    Ok(ManifestLimits {
        manifest_bytes: size(32_768)?,
        payload_bytes: bytes,
        records: size(1)?,
        total_bytes: bytes,
    })
}

pub(super) fn camera_at_limit(limit: u16) -> Result<Cabinet, RenderError> {
    let mut cabinet = Cabinet::new(StreetLayout::DOWNTOWN)
        .map_err(failure)?
        .with_scroll_limit(limit)
        .map_err(failure)?;
    let start = Action {
        control: Control::Start,
        player: Player::Marge,
    };
    let right = Action {
        control: Control::Right,
        player: Player::Marge,
    };
    cabinet.input_step(0, &[
        LogicalAction {
            action: start,
            down: true,
        },
        LogicalAction {
            action: start,
            down: false,
        },
        LogicalAction {
            action: right,
            down: true,
        },
    ]);
    for tick in 1..=u64::from(limit).saturating_add(u64::from(BASE_WIDTH)) {
        cabinet.input_step(tick, &[]);
        if u16::try_from(cabinet.camera_origin()).ok() == Some(limit) {
            break;
        }
    }
    let origin = i16::try_from(limit).map_err(failure)?;
    if cabinet.camera_origin() != origin {
        return Err(failure("camera did not reach verified street limit"));
    }
    Ok(cabinet)
}

pub(super) const fn centered_offset(width: u32) -> u32 {
    centered_expansion(BASE_WIDTH, width)
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    bundle: &CanonicalBundle,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_smoke_profiles(
        art.panel.is_some(),
        &options.controls.profiles,
    )?;
    smoke_press(&mut controls, &mut simulation, Control::Start)?;
    let _drawn = art.present(scene, simulation.state())?;
    let layout = scene.layout();
    let record = bundle
        .manifest()
        .record("scene/world-background")
        .ok_or(RenderError::InvalidSprite)?;
    let AssetFormat::Rgba8 { width, .. } = record.format else {
        return Err(RenderError::InvalidSprite);
    };
    let expected = bundle
        .view_payload("scene/world-background")
        .map_err(failure)?;
    let actual = scene.readback()?;
    verify_extension(layout, &actual, width.get(), &expected)?;
    write_status(format_args!(
        "Adaptive logical scene: {}x{}; {} added columns verified",
        layout.width(),
        layout.height(),
        layout.width().get().saturating_sub(BASE_WIDTH),
    ))
    .map_err(failure)?;
    smoke_camera(scene, art, options.display, (width.get(), &expected))
}

fn smoke_camera(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    display: DisplaySettings,
    strip: (u32, &[u8]),
) -> Result<(), RenderError> {
    let limit = art
        .world_art
        .ok_or(RenderError::InvalidSprite)?
        .scroll_limit()?;
    let cabinet = camera_at_limit(limit)?;
    let origin = cabinet.camera_origin();
    scene.apply_display_settings(DisplaySettings {
        presentation: PresentationMode::AspectFit,
        ..display
    })?;
    let _drawn = art.present(scene, &cabinet)?;
    verify_camera_rows(scene.layout(), &scene.readback()?, strip, origin)?;
    scene.apply_display_settings(display)?;
    let _redrawn = art.present(scene, &cabinet)?;
    let view = art.presentation_view(scene, &cabinet)?;
    verify_camera_rows(scene.layout(), &scene.readback()?, strip, view.origin)?;
    write_status(format_args!("Street camera: {origin} columns verified"))
        .map_err(failure)
}

pub(super) fn station_offset(
    width: u32,
    player: Player,
) -> Result<i32, RenderError> {
    let index = u32::try_from(player.index()).map_err(failure)?;
    let count = dimension(u32::try_from(Player::ALL.len()).map_err(failure)?)?;
    let offset = expanded_anchor(BASE_WIDTH, width, index, count)
        .ok_or(RenderError::InvalidDimensions)?;
    i32::try_from(offset).map_err(failure)
}

fn validate_prefix(
    original: &[u8],
    strip: &[u8],
    width: u32,
) -> Result<(), RenderError> {
    let inner = BASE_WIDTH.saturating_sub(SIDE * 2);
    if width <= inner || width > MAX_STRIP_WIDTH {
        return Err(failure("unexpected world strip width"));
    }
    let original_stride = usize::try_from(BASE_WIDTH * 4).map_err(failure)?;
    let strip_stride =
        usize::try_from(width.saturating_mul(4)).map_err(failure)?;
    let margin = usize::try_from(SIDE * 4).map_err(failure)?;
    let base_height = usize::try_from(BASE_HEIGHT).map_err(failure)?;
    let height = usize::try_from(HEIGHT).map_err(failure)?;
    if original.len() != original_stride.saturating_mul(base_height)
        || strip.len() != strip_stride.saturating_mul(height)
    {
        return Err(RenderError::InvalidLength);
    }
    if strip
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel.last() != Some(&255))
    {
        return Err(failure("world strip contains transparent pixels"));
    }
    let start = usize::try_from(TOP).map_err(failure)?;
    let end = original_stride.saturating_sub(margin);
    for (base, world) in original
        .chunks_exact(original_stride)
        .skip(start)
        .zip(strip.chunks_exact(strip_stride))
    {
        let black = |bytes: &[u8]| {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 0, 255])
        };
        let left = base.get(..margin).ok_or(RenderError::InvalidLength)?;
        let right = base.get(end..).ok_or(RenderError::InvalidLength)?;
        if !black(left)
            || !black(right)
            || base.get(margin..end) != world.get(..end.saturating_sub(margin))
        {
            return Err(failure("world strip does not match base street"));
        }
    }
    Ok(())
}

fn verify_extension(
    layout: FrameLayout,
    actual: &[u8],
    strip_width: u32,
    strip: &[u8],
) -> Result<(), RenderError> {
    let width = layout.width().get();
    if !(BASE_WIDTH..=viewport_width(strip_width)).contains(&width)
        || actual.len() != layout.bytes()
    {
        return Err(RenderError::InvalidLength);
    }
    let stride = usize::try_from(width.saturating_mul(4)).map_err(failure)?;
    let strip_stride =
        usize::try_from(strip_width.saturating_mul(4)).map_err(failure)?;
    let top = usize::try_from(TOP).map_err(failure)?;
    let height = usize::try_from(HEIGHT).map_err(failure)?;
    if strip.len() != strip_stride.saturating_mul(height) {
        return Err(RenderError::InvalidLength);
    }
    if width > BASE_WIDTH {
        return verify_wide_extension(actual, stride, strip, strip_stride);
    }
    let end = usize::try_from((BASE_WIDTH - SIDE) * 4).map_err(failure)?;
    for frame in actual.chunks_exact(stride).skip(top).take(height) {
        let margin = frame.get(end..).ok_or(RenderError::InvalidLength)?;
        if margin
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| *pixel != [0, 0, 0, 255])
        {
            return Err(failure("extended native world pixels differ"));
        }
    }
    Ok(())
}

fn verify_wide_extension(
    actual: &[u8],
    stride: usize,
    strip: &[u8],
    strip_stride: usize,
) -> Result<(), RenderError> {
    let start = usize::try_from(BASE_WIDTH * 4).map_err(failure)?;
    let top = usize::try_from(TOP).map_err(failure)?;
    for (frame, world) in actual
        .chunks_exact(stride)
        .skip(top)
        .zip(strip.chunks_exact(strip_stride))
    {
        if frame.get(start..stride) != world.get(start..stride) {
            return Err(failure(
                "wide scene did not display the actual strip edge",
            ));
        }
    }
    Ok(())
}

fn verify_camera_rows(
    layout: FrameLayout,
    actual: &[u8],
    (strip_width, strip): (u32, &[u8]),
    origin: i16,
) -> Result<(), RenderError> {
    let stride = usize::try_from(layout.width().get().saturating_mul(4))
        .map_err(failure)?;
    let strip_stride =
        usize::try_from(strip_width.saturating_mul(4)).map_err(failure)?;
    let wide = layout.width().get() > BASE_WIDTH;
    let (margin, source) = if wide {
        (
            0,
            origin
                .checked_sub(i16::try_from(SIDE).map_err(failure)?)
                .ok_or(RenderError::InvalidDimensions)?,
        )
    } else {
        (usize::try_from(SIDE * 4).map_err(failure)?, origin)
    };
    let start = usize::try_from(source).map_err(failure)?.saturating_mul(4);
    let end =
        start.saturating_add(stride.saturating_sub(margin.saturating_mul(2)));
    let top = usize::try_from(TOP).map_err(failure)?;
    for (frame, world) in actual
        .chunks_exact(stride)
        .skip(top)
        .take(28)
        .zip(strip.chunks_exact(strip_stride))
    {
        if frame.get(margin..stride.saturating_sub(margin))
            != world.get(start..end)
        {
            return Err(failure("scrolled street pixels differ"));
        }
    }
    Ok(())
}

fn world_view(
    strip_width: u32,
    width: u32,
    camera: i16,
) -> Result<View, RenderError> {
    Ok(View {
        origin: view_origin(strip_width, width, camera)
            .ok_or(RenderError::InvalidDimensions)?,
        width,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_strip_keeps_the_viewport_bounded() {
        assert_eq!(viewport_width(732), 512);
        assert_eq!(viewport_width(442), 442);
        assert_eq!(viewport_width(300), BASE_WIDTH);
        assert_eq!(
            world_view(732, 320, 444).map(|view| view.origin).ok(),
            Some(444)
        );
        assert_eq!(
            world_view(732, 512, 444).map(|view| view.origin).ok(),
            Some(236)
        );
        assert_eq!(
            world_view(732, 512, 0).map(|view| view.origin).ok(),
            Some(16)
        );
        assert!(world_view(732, 513, 444).is_err());
        assert!(world_view(732, 320, -1).is_err());
    }

    #[test]
    fn wide_projection_uses_real_columns_at_each_edge()
    -> Result<(), RenderError> {
        let view = world_view(442, 400, 0)?;
        if view.project(Point { x: 16, y: 190 }) != (Point { x: 0, y: 190 })
            || view.project(Point { x: 415, y: 190 })
                != (Point { x: 399, y: 190 })
        {
            return Err(failure("wide initial projection"));
        }
        let end = world_view(442, 400, 154)?;
        if end.project(Point { x: 457, y: 190 }) != (Point { x: 399, y: 190 }) {
            return Err(failure("wide end projection"));
        }
        let base = world_view(442, 320, 0)?;
        if base.project(Point { x: 16, y: 190 }) != (Point { x: 16, y: 190 }) {
            return Err(failure("base projection"));
        }
        Ok(())
    }

    #[test]
    fn long_strip_admission_has_a_separate_bound() {
        let base = [0, 0, 0, 255].repeat(320 * 224);
        let strip = [0, 0, 0, 255].repeat(732 * 196);
        assert!(validate_prefix(&base, &strip, 732).is_ok());
        let extended = [0, 0, 0, 255].repeat(1647 * 196);
        assert!(validate_prefix(&base, &extended, 1647).is_ok());
        let maximum = [0, 0, 0, 255].repeat(1664 * 196);
        assert!(validate_prefix(&base, &maximum, 1664).is_ok());
        let oversized = [0, 0, 0, 255].repeat(1665 * 196);
        assert!(validate_prefix(&base, &oversized, 1665).is_err());
        assert_eq!(atlas_budget(), 1_304_576);
    }

    #[test]
    fn extended_camera_reaches_both_old_and_new_limits()
    -> Result<(), RenderError> {
        for limit in [80u16, 444, 1002, 1359] {
            let camera = camera_at_limit(limit)?;
            if u16::try_from(camera.camera_origin()).ok() != Some(limit) {
                return Err(failure("incorrect extended camera limit"));
            }
        }
        Ok(())
    }

    #[test]
    fn strip_must_match_exact_interior_and_black_margins()
    -> Result<(), RenderError> {
        let width = 300u32;
        let base = [0, 0, 0, 255].repeat(320 * 224);
        let strip = [0, 0, 0, 255].repeat(300 * 196);
        validate_prefix(&base, &strip, width)?;
        let mut mismatch = strip.clone();
        *mismatch.first_mut().ok_or(RenderError::InvalidLength)? = 1;
        if validate_prefix(&base, &mismatch, width).is_ok()
            || validate_prefix(&base, &strip, 288).is_ok()
            || validate_prefix(&base, &strip, 481).is_ok()
            || validate_prefix(&base, &[], width).is_ok()
        {
            return Err(failure("invalid world strip accepted"));
        }
        let mut transparent = strip.clone();
        *transparent.last_mut().ok_or(RenderError::InvalidLength)? = 0;
        if validate_prefix(&base, &transparent, width).is_ok() {
            return Err(failure("transparent world strip accepted"));
        }
        let mut margin = base;
        *margin
            .get_mut(28 * 320 * 4)
            .ok_or(RenderError::InvalidLength)? = 1;
        if validate_prefix(&margin, &strip, width).is_ok() {
            return Err(failure("nonblack side margin accepted"));
        }
        Ok(())
    }

    #[test]
    fn station_slots_expand_between_outer_anchors() -> Result<(), RenderError> {
        for (player, expected) in
            Player::ALL.into_iter().zip([0i32, 14i32, 28i32, 42i32])
        {
            if station_offset(362, player)? != expected
                || station_offset(320, player)? != 0i32
                || centered_offset(362) != 21
            {
                return Err(failure("responsive station placement differs"));
            }
        }
        Ok(())
    }
}
