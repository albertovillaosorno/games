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
//   - Invent world pixels, modify simulation or change camera collision.
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

use games_assets::AssetFormat;
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};
use games_platform_storage::bundle::CanonicalBundle;

use super::{
    Art, Control, Controls, Player, Profiles, Scene, dimension, failure, live,
    size, smoke_press,
};

pub(super) const BASE_WIDTH: u32 = 320;
pub(super) const BASE_HEIGHT: u32 = TOP + HEIGHT;
pub(super) const MAX_WIDTH: u32 = 512;
const HEIGHT: u32 = 196;
const SIDE: u32 = 16;
const TOP: u32 = 28;

#[derive(Clone, Copy, Debug)]
pub(super) struct WorldArt {
    art: Art,
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
        let pixels = extension.read_payload(key).map_err(failure)?;
        if height.get() != HEIGHT {
            return Err(failure("unexpected world strip height"));
        }
        validate_prefix(original, &pixels, width.get())?;
        Ok(Self {
            art: Art::upload(scene, (width, height), &pixels)?,
        })
    }

    pub(super) fn maximum(
        self,
        base: FrameLayout,
    ) -> Result<FrameLayout, RenderError> {
        let width = self.art.rect.width.get().saturating_add(SIDE * 2);
        FrameLayout::new(dimension(width)?, base.height(), size(622_592)?)
    }

    pub(super) fn queue(
        self,
        original: Art,
        width: u32,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        if width == BASE_WIDTH {
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
}

pub(super) fn centered_offset(width: u32) -> u32 {
    width.saturating_sub(BASE_WIDTH).checked_div(2).unwrap_or(0)
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    bundle: &CanonicalBundle,
    profiles: &Profiles,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_profiles(art.panel.is_some(), profiles)?;
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
    if width <= inner || width > MAX_WIDTH.saturating_sub(SIDE * 2) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
