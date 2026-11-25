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
//   - Check the base interior exactly and retain the black margins.
// - Usage:
//   - Select the optional pinned strip in the local launch catalogue.
// - Defaults:
//   - Fixed base extent unless the strip and adaptive mode are selected.

//! Bounded opening-street art and responsive native placement.

use std::io::{Write as _, stdout};

use games::input::LogicalAction;
use games::presentation::{DisplaySettings, PresentationMode};
use games::replay::InputStep as _;
use games_assets::AssetFormat;
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};
use games_platform_storage::bundle::CanonicalBundle;

use super::{
    Action, Art, Cabinet, Control, Controls, Player, Point, PreviewOptions,
    Scene, StreetLayout, dimension, failure, live, size, smoke_press,
};

pub(super) const BASE_WIDTH: u32 = 320;
pub(super) const BASE_HEIGHT: u32 = TOP + HEIGHT;
const MAX_VIEW_WIDTH: u32 = 512;
const MAX_STRIP_WIDTH: u32 = 736;
const HEIGHT: u32 = 196;
const SIDE: u32 = 16;
const TOP: u32 = 28;

#[derive(Clone, Copy, Debug)]
pub(super) struct WorldArt {
    art: Art,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct View {
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
        let pixels = extension.read_payload(key).map_err(failure)?;
        validate_prefix(original, &pixels, width.get())?;
        Ok(Self {
            art: Art::upload(scene, (width, height), &pixels)?,
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

fn camera_at_limit(limit: u16) -> Result<Cabinet, RenderError> {
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
    for tick in 1..400 {
        cabinet.input_step(tick, &[]);
    }
    let origin = i16::try_from(limit).map_err(failure)?;
    if cabinet.camera_origin() != origin {
        return Err(failure("camera did not reach verified street limit"));
    }
    Ok(cabinet)
}

pub(super) fn centered_offset(width: u32) -> u32 {
    width.saturating_sub(BASE_WIDTH).checked_div(2).unwrap_or(0)
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    bundle: &CanonicalBundle,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_profiles(
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
        .read_payload("scene/world-background")
        .map_err(failure)?;
    let actual = scene.readback()?;
    verify_extension(layout, &actual, width.get(), &expected)?;
    writeln!(
        stdout().lock(),
        "Adaptive logical scene: {}x{}; {} added columns verified",
        layout.width(),
        layout.height(),
        layout.width().get().saturating_sub(BASE_WIDTH),
    )
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
    writeln!(stdout().lock(), "Street camera: {origin} columns verified")
        .map_err(failure)
}

pub(super) fn station_offset(
    width: u32,
    player: Player,
) -> Result<i32, RenderError> {
    let index = u32::try_from(player.index()).map_err(failure)?;
    let extra = width.saturating_sub(BASE_WIDTH);
    i32::try_from(extra.saturating_mul(index).checked_div(3).unwrap_or(0))
        .map_err(failure)
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
    let extra = width
        .checked_sub(BASE_WIDTH)
        .ok_or(RenderError::InvalidDimensions)?;
    if width > strip_width.saturating_add(SIDE * 2)
        || actual.len() != layout.bytes()
    {
        return Err(RenderError::InvalidLength);
    }
    let stride = usize::try_from(width.saturating_mul(4)).map_err(failure)?;
    let strip_stride =
        usize::try_from(strip_width.saturating_mul(4)).map_err(failure)?;
    let start = usize::try_from((BASE_WIDTH - SIDE) * 4).map_err(failure)?;
    let end = usize::try_from(width.saturating_sub(SIDE).saturating_mul(4))
        .map_err(failure)?;
    let extra_start =
        usize::try_from((BASE_WIDTH - SIDE * 2) * 4).map_err(failure)?;
    let extra_end = extra_start.saturating_add(
        usize::try_from(extra.saturating_mul(4)).map_err(failure)?,
    );
    let top = usize::try_from(TOP).map_err(failure)?;
    let height = usize::try_from(HEIGHT).map_err(failure)?;
    if strip.len() != strip_stride.saturating_mul(height) {
        return Err(RenderError::InvalidLength);
    }
    for (frame, world) in actual
        .chunks_exact(stride)
        .skip(top)
        .take(height)
        .zip(strip.chunks_exact(strip_stride))
    {
        let margin = frame.get(end..).ok_or(RenderError::InvalidLength)?;
        if frame.get(start..end) != world.get(extra_start..extra_end)
            || margin
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
    let margin = usize::try_from(SIDE * 4).map_err(failure)?;
    let start = usize::try_from(origin).map_err(failure)?.saturating_mul(4);
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

fn viewport_width(strip_width: u32) -> u32 {
    strip_width.saturating_add(SIDE * 2).min(MAX_VIEW_WIDTH)
}

fn world_view(
    strip_width: u32,
    width: u32,
    camera: i16,
) -> Result<View, RenderError> {
    if !(BASE_WIDTH..=viewport_width(strip_width)).contains(&width)
        || camera < 0
    {
        return Err(RenderError::InvalidDimensions);
    }
    let remaining = strip_width.saturating_add(SIDE * 2).saturating_sub(width);
    Ok(View {
        origin: camera.min(i16::try_from(remaining).map_err(failure)?),
        width,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_strip_keeps_the_viewport_bounded() {
        assert_eq!(viewport_width(732), 512);
        assert_eq!(viewport_width(442), 474);
        assert_eq!(
            world_view(732, 320, 444).map(|view| view.origin).ok(),
            Some(444)
        );
        assert_eq!(
            world_view(732, 512, 444).map(|view| view.origin).ok(),
            Some(252)
        );
        assert_eq!(
            world_view(732, 512, 0).map(|view| view.origin).ok(),
            Some(0)
        );
        assert!(world_view(732, 513, 444).is_err());
        assert!(world_view(732, 320, -1).is_err());
    }

    #[test]
    fn long_strip_admission_has_a_separate_bound() {
        let base = [0, 0, 0, 255].repeat(320 * 224);
        let strip = [0, 0, 0, 255].repeat(732 * 196);
        assert!(validate_prefix(&base, &strip, 732).is_ok());
        let maximum = [0, 0, 0, 255].repeat(736 * 196);
        assert!(validate_prefix(&base, &maximum, 736).is_ok());
        let oversized = [0, 0, 0, 255].repeat(737 * 196);
        assert!(validate_prefix(&base, &oversized, 737).is_err());
        assert_eq!(atlas_budget(), 577_024);
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
