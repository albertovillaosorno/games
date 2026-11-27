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
//   - Optional airborne ground-mask admission, selection and projection.
// - Must-Not:
//   - Advance simulation, interpret original media or decide collision.
// - Allows:
//   - Inputs: Verified local mask, character, maneuver and ground point.
//   - Outputs: Ground-aligned draw and native readback evidence.
//   - Side effects: Bounded atlas allocation and smoke presentation.
// - Split-When:
//   - Several independent ground effects need distinct policies.
// - Merge-When:
//   - Shared sprite effects provide this same admission and projection.
// - Summary:
//   - Present independently isolated reference ground-shadow coverage.
// - Description:
//   - A black alpha mask is projected independently of actor elevation.
// - Usage:
//   - Explicitly select a canonical bundle containing actor/shadow.
// - Defaults:
//   - No mask or extra allocation without opt-in.

//! Optional reference-derived airborne ground shadow.

use std::num::NonZeroU32;

use games::presentation::solid_mask;
use games_assets::AssetFormat;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::CanonicalBundle;

use super::{Art, Scene, failure};
use crate::{Cabinet, Maneuver, Player, Point};

const KEY: &str = "actor/shadow";
const MAX_EDGE: u32 = 64;
pub(super) const MAX_BYTES: usize = 16_384;
// The observed release mask is 24 by 8. A row has at most 12 disjoint
// one-pixel runs, so four moving masks require at most 384 extra draws.
const RELEASE_WIDTH: u32 = 24;
const RELEASE_HEIGHT: usize = 8;
pub(super) const RELEASE_EXTRA_SPRITES: usize = 384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleaseMask([u32; RELEASE_HEIGHT]);

impl ReleaseMask {
    // Translate an earlier (possibly clipped) draw into the new row's
    // coordinates. Every source texel retains its original logical size.
    fn coverage(
        self,
        source: SpriteRect,
        destination: SpriteRect,
        (origin_x, origin_y): (i32, i32),
    ) -> u32 {
        let local = origin_y.saturating_sub(destination.y);
        let Ok(offset) = u32::try_from(local) else {
            return 0;
        };
        if offset >= destination.height.get() {
            return 0;
        }
        let row = usize::try_from(source.y.saturating_add(local))
            .ok()
            .and_then(|index| self.0.get(index))
            .copied()
            .unwrap_or_default();
        let column = u32::try_from(source.x).unwrap_or(u32::MAX);
        let bits = row.checked_shr(column).unwrap_or_default()
            & low_bits(source.width.get());
        let delta = destination.x.saturating_sub(origin_x);
        if delta < 0 {
            bits.checked_shr(delta.unsigned_abs()).unwrap_or_default()
        } else {
            bits.checked_shl(delta.unsigned_abs()).unwrap_or_default()
        }
    }

    fn draw_contains(self, draw: SpriteDraw, others: &[SpriteDraw]) -> bool {
        (0..draw.destination.height.get()).all(|offset| {
            let origin = (
                draw.destination.x,
                draw.destination
                    .y
                    .saturating_add(i32::try_from(offset).unwrap_or_default()),
            );
            let bits = self.coverage(draw.source, draw.destination, origin);
            let union = others.iter().fold(0u32, |covered, other| {
                covered | self.coverage(other.source, other.destination, origin)
            });
            bits & !union == 0
        })
    }

    fn draw_disjoint(self, draw: SpriteDraw, others: &[SpriteDraw]) -> bool {
        (0..draw.destination.height.get()).all(|offset| {
            let origin = (
                draw.destination.x,
                draw.destination
                    .y
                    .saturating_add(i32::try_from(offset).unwrap_or_default()),
            );
            let bits = self.coverage(draw.source, draw.destination, origin);
            others.iter().all(|other| {
                bits & self.coverage(other.source, other.destination, origin)
                    == 0
            })
        })
    }

    fn read(width: u32, height: u32, pixels: &[u8]) -> Option<Self> {
        if width != RELEASE_WIDTH || height != 8 {
            return None;
        }
        let mut rows = [0; RELEASE_HEIGHT];
        let (rgba, remainder) = pixels.as_chunks::<4>();
        if !remainder.is_empty() {
            return None;
        }
        let (source_rows, row_remainder) = rgba.as_chunks::<24>();
        if !row_remainder.is_empty() || source_rows.len() != RELEASE_HEIGHT {
            return None;
        }
        for (row, source) in rows.iter_mut().zip(source_rows) {
            for (column, pixel) in source.iter().enumerate() {
                let alpha = pixel.get(3).copied()?;
                *row |= u32::from(alpha != 0)
                    .checked_shl(u32::try_from(column).ok()?)?;
            }
        }
        Some(Self(rows))
    }
}

#[derive(Debug)]
pub(super) struct ShadowArt {
    art: Art,
    release: Option<ReleaseMask>,
}

impl ShadowArt {
    pub(super) fn draw(&self, point: Point) -> SpriteDraw {
        let art = self.art;
        let destination =
            art.rect.centered_at(i32::from(point.x), i32::from(point.y));
        art.draw(destination, false)
    }

    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
    ) -> Result<Self, RenderError> {
        let record = bundle
            .manifest()
            .record(KEY)
            .ok_or_else(|| failure("missing shadow record"))?;
        let AssetFormat::Rgba8 { height, width } = record.format else {
            return Err(failure("shadow must be RGBA8"));
        };
        if width.get() > MAX_EDGE || height.get() > MAX_EDGE {
            return Err(failure("unexpected shadow extent"));
        }
        let pixels = bundle.view_payload(KEY).map_err(failure)?;
        if !valid_mask(&pixels) {
            return Err(failure("unexpected shadow coverage"));
        }
        let release = ReleaseMask::read(width.get(), height.get(), &pixels);
        Ok(Self {
            art: Art::upload(scene, (width, height), &pixels)?,
            release,
        })
    }

    pub(super) fn matches_union(
        &self,
        draws: &[SpriteDraw],
        expected: &[SpriteDraw],
    ) -> bool {
        let Some(mask) = self.release else {
            return false;
        };
        let actual: Vec<_> = draws
            .iter()
            .copied()
            .filter(|draw| draw.atlas == self.art.atlas)
            .collect();
        actual.iter().enumerate().all(|(index, draw)| {
            mask.draw_contains(*draw, expected)
                && actual
                    .get(index.saturating_add(1)..)
                    .is_some_and(|rest| mask.draw_disjoint(*draw, rest))
        }) && expected
            .iter()
            .all(|draw| mask.draw_contains(*draw, &actual))
    }

    pub(super) fn queue_unique(
        &self,
        point: Point,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let draw = self.draw(point);
        let Some(mask) = self.release else {
            draws.push(draw);
            return Ok(());
        };
        let mut rows = mask.0;
        for earlier in
            draws.iter().filter(|earlier| earlier.atlas == draw.atlas)
        {
            for (row, bits) in rows.iter_mut().enumerate() {
                let y = draw
                    .destination
                    .y
                    .saturating_add(i32::try_from(row).map_err(failure)?);
                *bits &= !mask.coverage(
                    earlier.source,
                    earlier.destination,
                    (draw.destination.x, y),
                );
            }
            if rows.iter().all(|bits| *bits == 0) {
                return Ok(());
            }
        }
        if rows == mask.0 {
            draws.push(draw);
            return Ok(());
        }
        for (row, bits) in rows.into_iter().enumerate() {
            clipped_row(
                draw,
                i32::try_from(row).map_err(failure)?,
                bits,
                draws,
            )?;
        }
        Ok(())
    }

    pub(super) const fn supports_release(&self) -> bool {
        self.release.is_some()
    }
}

fn low_bits(width: u32) -> u32 {
    u32::MAX
        .checked_shr(32u32.saturating_sub(width))
        .unwrap_or_default()
}

fn take_run(bits: &mut u32) -> Option<(u32, NonZeroU32)> {
    let start = bits.trailing_zeros();
    let width = NonZeroU32::new(bits.checked_shr(start)?.trailing_ones())?;
    *bits &= !u32::MAX
        .checked_shr(32u32.saturating_sub(width.get()))
        .unwrap_or_default()
        .checked_shl(start)?;
    Some((start, width))
}

fn clipped_row(
    draw: SpriteDraw,
    row: i32,
    mut bits: u32,
    draws: &mut Vec<SpriteDraw>,
) -> Result<(), RenderError> {
    while let Some((start, width)) = take_run(&mut bits) {
        let x = i32::try_from(start).map_err(failure)?;
        let source = SpriteRect {
            width,
            height: NonZeroU32::MIN,
            x,
            y: row,
        };
        draws.push(SpriteDraw {
            source,
            destination: SpriteRect {
                x: draw.destination.x.saturating_add(x),
                y: draw.destination.y.saturating_add(row),
                ..source
            },
            ..draw
        });
    }
    Ok(())
}

fn valid_mask(pixels: &[u8]) -> bool {
    solid_mask(pixels, [0; 4], [0, 0, 0, 102])
}

pub(super) const fn visible(player: Player, maneuver: Maneuver) -> bool {
    match (player, maneuver) {
        (Player::Bart, Maneuver::Special { tick, .. }) => {
            tick >= 1 && tick <= 4
        },
        (Player::Bart, _) => false,
        (_, _) => maneuver.airborne(),
    }
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    expected: bool,
) -> Result<(), RenderError> {
    let Some(shadow) = art.shadow.take() else {
        return Ok(());
    };
    let unshadowed = art.present(scene, cabinet).and_then(|_| scene.readback());
    art.shadow = Some(shadow);
    let without = unshadowed?;
    let _drawn = art.present(scene, cabinet)?;
    let with = scene.readback()?;
    if !expected && with == without {
        return Ok(());
    }
    if expected {
        let [marge, ..] = cabinet.stations();
        let view = art.presentation_view(scene, cabinet)?;
        let mask = art.shadow.as_ref().ok_or(RenderError::InvalidSprite)?;
        let bounds = mask.draw(view.project(marge.position)).destination;
        if darkens(&without, &with, scene.layout().width(), bounds) {
            return Ok(());
        }
    }
    Err(failure("unexpected ground shadow in native readback"))
}

fn within(bounds: SpriteRect, index: usize, width: usize) -> bool {
    let column = index
        .checked_rem(width)
        .and_then(|value| u32::try_from(value).ok());
    let row = index
        .checked_div(width)
        .and_then(|value| u32::try_from(value).ok());
    column
        .zip(row)
        .is_some_and(|(x, y)| bounds.contains_pixel(x, y))
}

fn darkens(
    without: &[u8],
    with: &[u8],
    columns: NonZeroU32,
    bounds: SpriteRect,
) -> bool {
    let Ok(stride) = usize::try_from(columns.get()) else {
        return false;
    };
    if without.len() != with.len() || !without.len().is_multiple_of(4) {
        return false;
    }
    let mut changed = false;
    for (index, (prior, current)) in without
        .as_chunks::<4>()
        .0
        .iter()
        .zip(with.as_chunks::<4>().0)
        .enumerate()
    {
        for (before, after) in prior.iter().zip(current).take(3) {
            if after > before
                || (after < before && !within(bounds, index, stride))
            {
                return false;
            }
            changed |= after < before;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_runs_cover_every_texel_once_with_a_bounded_draw_count() {
        for original in
            [0, 1, 0x00ff_ffff, 0x0055_5555, 0x00aa_aaaa, 0x005a_0a5a]
        {
            let mut remaining = original;
            let mut rebuilt = 0u32;
            let mut count = 0usize;
            while let Some((start, width)) = take_run(&mut remaining) {
                assert!(start.saturating_add(width.get()) <= RELEASE_WIDTH);
                let run = u32::MAX
                    .checked_shr(32u32.saturating_sub(width.get()))
                    .unwrap_or_default()
                    .checked_shl(start)
                    .unwrap_or_default();
                assert_eq!(rebuilt & run, 0);
                rebuilt |= run;
                count = count.saturating_add(1);
            }
            assert_eq!(remaining, 0);
            assert_eq!(rebuilt, original);
            assert!(count <= 12);
        }
    }

    #[test]
    fn release_mask_translates_and_crops_coverage_without_repainting() {
        let mask = ReleaseMask([0b0011_1100; 8]);
        let rect = SpriteRect {
            width: NonZeroU32::new(24).unwrap_or(NonZeroU32::MIN),
            height: NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            x: 0,
            y: 0,
        };
        let destination = SpriteRect { x: 10, y: 15, ..rect };
        assert_eq!(
            mask.coverage(rect, destination, (8i32, 15i32)),
            0b1111_0000
        );
        assert_eq!(
            mask.coverage(rect, destination, (12i32, 22i32)),
            0b0000_1111
        );
        assert_eq!(mask.coverage(rect, destination, (10i32, 23i32)), 0);
        assert_eq!(mask.coverage(rect, destination, (10i32, 14i32)), 0);
        assert_eq!(mask.coverage(rect, destination, (50i32, 15i32)), 0);
    }

    #[test]
    fn release_mask_crops_and_subtracts_overlap() {
        let mask = ReleaseMask([0b0011_1100; 8]);
        let rect = SpriteRect {
            width: NonZeroU32::new(24).unwrap_or(NonZeroU32::MIN),
            height: NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            x: 0,
            y: 0,
        };
        let destination = SpriteRect { x: 10, y: 15, ..rect };
        let cropped = SpriteRect {
            width: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            height: NonZeroU32::MIN,
            x: 3,
            y: 4,
        };
        let placed = SpriteRect { x: 13, y: 19, ..cropped };
        assert_eq!(mask.coverage(cropped, placed, (10i32, 19i32)), 0b0001_1000);
        assert_eq!(mask.coverage(cropped, placed, (10i32, 20i32)), 0);
        let overlap = mask.coverage(rect, destination, (12i32, 15i32));
        let original = 0b0011_1100;
        let remaining = original & !overlap;
        assert_eq!(remaining, 0b0011_0000);
        assert_eq!(remaining & overlap, 0);
        assert_eq!(remaining | overlap, 0b0011_1111);
    }

    #[test]
    fn release_coverage_uses_loaded_pixels_and_rejects_other_extents() {
        let mut bytes = [0u8; 768];
        if let Some(alpha) = bytes.get_mut(3) {
            *alpha = 102;
        }
        if let Some(alpha) = bytes.get_mut(767) {
            *alpha = 102;
        }
        let mask = ReleaseMask::read(24, 8, &bytes)
            .unwrap_or(ReleaseMask([0; RELEASE_HEIGHT]));
        assert_eq!(mask.0.first(), Some(&1));
        assert_eq!(mask.0.last(), Some(&(1u32 << 23u32)));
        assert!(ReleaseMask::read(8, 24, &bytes).is_none());
        assert!(
            ReleaseMask::read(24, 8, bytes.get(..767).unwrap_or_default())
                .is_none()
        );
    }

    #[test]
    fn coverage_is_bounded_black_alpha_and_nonempty() {
        assert!(valid_mask(&[0, 0, 0, 102, 0, 0, 0, 0]));
        assert!(!valid_mask(&[0; 4]));
        assert!(!valid_mask(&[1, 0, 0, 102]));
        assert!(!valid_mask(&[0, 0, 0, 255]));
        assert!(!valid_mask(&[0, 0, 0, 102, 0]));
    }

    #[test]
    fn observed_character_and_maneuver_coverage() -> Result<(), String> {
        for player in Player::ALL {
            if visible(player, Maneuver::Idle) {
                return Err(String::from("idle shadow"));
            }
            if visible(player, Maneuver::Attack { moving: false, tick: 0 }) {
                return Err(String::from("ground attack shadow"));
            }
            let Maneuver::Jump(jump) = Maneuver::jump() else {
                return Err(String::from("missing jump"));
            };
            if visible(player, Maneuver::Jump(jump)) != (player != Player::Bart)
            {
                return Err(String::from("jump shadow"));
            }
            if visible(player, Maneuver::air_attack(jump))
                != (player != Player::Bart)
            {
                return Err(String::from("air attack shadow"));
            }
            if !(0..30).all(|tick| {
                visible(player, Maneuver::Special {
                    drift_quarters: 0,
                    tick,
                    hit: false,
                }) == (player != Player::Bart || (1..=4).contains(&tick))
            }) {
                return Err(String::from("special shadow"));
            }
        }
        Ok(())
    }

    #[test]
    fn readback_must_darken_only_within_ground_mask() {
        let width = NonZeroU32::MIN.saturating_add(1);
        let bounds = SpriteRect {
            height: NonZeroU32::MIN,
            width: NonZeroU32::MIN,
            x: 1,
            y: 0,
        };
        let before = [100, 100, 100, 255, 100, 100, 100, 255];
        assert!(!darkens(&before, &before, width, bounds));
        assert!(darkens(
            &before,
            &[100, 100, 100, 255, 60, 60, 60, 255],
            width,
            bounds
        ));
        assert!(!darkens(
            &before,
            &[99, 100, 100, 255, 60, 60, 60, 255],
            width,
            bounds
        ));
        assert!(!darkens(
            &before,
            &[100, 100, 100, 255, 60, 101, 60, 255],
            width,
            bounds
        ));
    }
}
