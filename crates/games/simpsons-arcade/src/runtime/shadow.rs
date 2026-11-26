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

#[derive(Debug)]
pub(super) struct ShadowArt(Art);

impl ShadowArt {
    pub(super) fn draw(&self, point: Point) -> SpriteDraw {
        let art = self.0;
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
        Art::upload(scene, (width, height), &pixels).map(Self)
    }
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
                visible(player, Maneuver::Special { tick, hit: false })
                    == (player != Player::Bart || (1..=4).contains(&tick))
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
