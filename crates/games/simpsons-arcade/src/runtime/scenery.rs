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
//   - Optional bounded mid-street scripted actor art admission and placement.
// - Must-Not:
//   - Resolve combat, change camera gates or interpret original object data.
// - Allows:
//   - Inputs: Pinned isolated art, fixed-step scene tick and world viewport.
//   - Outputs: At most one positioned, clipped sprite draw.
//   - Side effects: Load admitted canonical atlases during scene startup.
// - Split-When:
//   - Additional independent scripted scenes require distinct tracks.
// - Merge-When:
//   - Shared placement-aware pose tracks subsume this local composition.
// - Summary:
//   - Present the independently observed noncombat suited-actor sequence.
// - Description:
//   - Hold sparse observed frames at their measured original raster origins.
// - Usage:
//   - Add optional scenic-pair records to the existing pose catalogue.
// - Defaults:
//   - No scenery without the complete bounded track.

//! Optional, sparse world-anchored mid-street scene presentation.

use std::collections::BTreeMap;
use std::mem::take;

use games::animation::{Keyframes, keyframe};
use games::input::LogicalAction;
use games::replay::InputStep as _;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::BundleSource;

use super::world::{View, camera_at_limit};
use super::{Art, Scene, dimension, failure, open_bundle};
use crate::{
    Action, BASE_WIDTH, Control, MID_STREET_SCRIPT_ORIGIN,
    MID_STREET_SCRIPT_TICKS, Player, Point, SIDE,
};

const FIRST_TICK: u16 = 8;
const STEP: u16 = 8;
const LAST_TICK: u16 = MID_STREET_SCRIPT_TICKS - STEP;
const MAX_GAP: u16 = STEP * 2;
const KEY: &str = "actor/scenic-pair";

#[derive(Clone, Copy, Debug)]
struct Sample {
    digest: ContentHash,
    origin: Point,
}

type Track<T> = Keyframes<(), (), u16, T>;

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    entries: Track<Sample>,
}

impl Catalogue {
    pub(super) fn complete(&self) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let mut previous = FIRST_TICK;
        if !self.entries.contains_key(&((), (), FIRST_TICK)) {
            return false;
        }
        for &((), (), tick) in self.entries.keys() {
            if tick.saturating_sub(previous) > MAX_GAP {
                return false;
            }
            previous = tick;
        }
        previous == LAST_TICK
    }

    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.entries.values().map(|sample| sample.digest)
    }

    pub(super) fn insert(&mut self, line: &str) -> Result<(), RenderError> {
        let ["scenic-pair", raw_tick, raw_left, raw_top, digest] =
            <[&str; 5]>::try_from(line.split_whitespace().collect::<Vec<_>>())
                .map_err(|_error| failure("invalid scenic pose record"))?
        else {
            return Err(failure("unknown scenic pose"));
        };
        let tick = raw_tick.parse::<u16>().map_err(failure)?;
        let left = raw_left.parse::<i16>().map_err(failure)?;
        let top = raw_top.parse::<i16>().map_err(failure)?;
        if !(FIRST_TICK..=LAST_TICK).contains(&tick)
            || !tick.is_multiple_of(STEP)
            || !(0..320).contains(&left)
            || !(28..224).contains(&top)
        {
            return Err(failure("scenic pose outside observed interval"));
        }
        let sample = Sample {
            digest: digest.parse().map_err(failure)?,
            origin: Point { x: left, y: top },
        };
        if self.entries.insert(((), (), tick), sample).is_some() {
            return Err(failure("duplicate scenic pose"));
        }
        Ok(())
    }

    pub(super) fn load(
        &self,
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<PlacedArt, RenderError> {
        let mut entries = BTreeMap::new();
        for (&key, sample) in &self.entries {
            let bundle = open_bundle(store, sample.digest)?;
            let art = Art::load(scene, &bundle, KEY)?;
            if art.rect.width.get() > 128
                || art.rect.height.get() > 128
                || u32::try_from(sample.origin.x)
                    .map_err(failure)?
                    .saturating_add(art.rect.width.get())
                    > BASE_WIDTH
                || u32::try_from(sample.origin.y)
                    .map_err(failure)?
                    .saturating_add(art.rect.height.get())
                    > 224
            {
                return Err(failure("scenic pose exceeds original raster"));
            }
            let _prior = entries.insert(key, (art, sample.origin));
        }
        Ok(PlacedArt { entries })
    }
}

#[derive(Debug, Default)]
pub(super) struct PlacedArt {
    entries: Track<(Art, Point)>,
}

impl PlacedArt {
    pub(super) fn draw(
        &self,
        tick: u16,
        view: View,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        if tick >= MID_STREET_SCRIPT_TICKS {
            return Ok(None);
        }
        let Some(&(art, origin)) = keyframe(&self.entries, ((), (), tick))
        else {
            return Ok(None);
        };
        let world = Point {
            x: MID_STREET_SCRIPT_ORIGIN.saturating_add(origin.x),
            y: origin.y,
        };
        clipped_draw(art, view.project(world), view.width)
    }
}

fn clipped_draw(
    art: Art,
    point: Point,
    view_width: u32,
) -> Result<Option<SpriteDraw>, RenderError> {
    let side = if view_width == BASE_WIDTH {
        SIDE
    } else {
        0
    };
    let x = i32::from(point.x);
    let left = x.max(i32::try_from(side).map_err(failure)?);
    let right = x
        .saturating_add(i32::try_from(art.rect.width.get()).map_err(failure)?)
        .min(i32::try_from(view_width.saturating_sub(side)).map_err(failure)?);
    if right <= left {
        return Ok(None);
    }
    let width =
        dimension(u32::try_from(right.saturating_sub(left)).map_err(failure)?)?;
    let source = SpriteRect {
        x: art.rect.x.saturating_add(left.saturating_sub(x)),
        width,
        ..art.rect
    };
    let destination = SpriteRect {
        x: left,
        y: i32::from(point.y),
        ..source
    };
    Ok(Some(Art { rect: source, ..art }.draw(destination, false)))
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
) -> Result<(), RenderError> {
    if art.poses.scenery.entries.is_empty() {
        return Ok(());
    }
    let origin = u16::try_from(MID_STREET_SCRIPT_ORIGIN).map_err(failure)?;
    let mut cabinet = camera_at_limit(origin)?;
    cabinet.input_step(1000, &[LogicalAction {
        action: Action {
            control: Control::Right,
            player: Player::Marge,
        },
        down: false,
    }]);
    for tick in 1001..=1095 {
        cabinet.input_step(tick, &[]);
    }
    let tick = cabinet
        .scripted_scene_tick()
        .ok_or_else(|| failure("scripted scene did not start"))?;
    let view = art.presentation_view(scene, &cabinet)?;
    let draw =
        art.poses.scenery.draw(tick, view)?.ok_or_else(|| {
            failure("scripted scene did not provide visible art")
        })?;
    let _drawn = art.present(scene, &cabinet)?;
    let actual = scene.readback()?;
    let saved = take(&mut art.poses.scenery);
    let baseline = art
        .present(scene, &cabinet)
        .and_then(|_baseline_drawn| scene.readback());
    art.poses.scenery = saved;
    verify_pixels(&actual, &baseline?, view.width, draw.destination)?;
    write_status(format_args!("Scripted scenic pair: visible at tick {tick}"))
        .map_err(failure)?;
    Ok(())
}

fn verify_pixels(
    actual: &[u8],
    baseline: &[u8],
    width: u32,
    bounds: SpriteRect,
) -> Result<(), RenderError> {
    if actual.len() != baseline.len() {
        return Err(failure("scripted scene changed readback extent"));
    }
    let mut changed = false;
    for (index, (pixel, prior)) in actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(baseline.as_chunks::<4>().0)
        .enumerate()
    {
        if pixel == prior {
            continue;
        }
        let pixel_index = u32::try_from(index).map_err(failure)?;
        let horizontal = pixel_index
            .checked_rem(width)
            .ok_or_else(|| failure("zero width"))?;
        let vertical = pixel_index
            .checked_div(width)
            .ok_or_else(|| failure("zero width"))?;
        if !bounds.contains_pixel(horizontal, vertical) {
            return Err(failure("scripted scene changed unrelated pixels"));
        }
        changed = true;
    }
    if !changed {
        return Err(failure("scripted scene did not change pixels"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn optional_track_requires_bounded_complete_samples() {
        let mut track = Catalogue::default();
        assert!(track.complete());
        for tick in (FIRST_TICK..=LAST_TICK).step_by(usize::from(STEP)) {
            if tick != 16 {
                assert!(
                    track
                        .insert(&format!("scenic-pair {tick} 248 52 {HASH}"))
                        .is_ok()
                );
            }
        }
        assert!(track.complete());
        assert!(
            track
                .insert(&format!("scenic-pair 8 248 52 {HASH}"))
                .is_err()
        );
        assert!(track.entries.remove(&((), (), 24)).is_some());
        assert!(!track.complete());
    }

    #[test]
    fn rejects_samples_outside_original_scene() {
        for invalid in [
            "0 248 52",
            "9 248 52",
            "336 248 52",
            "8 320 52",
            "8 248 27",
            "8 -1 52",
            "8 248 224",
        ] {
            assert!(
                Catalogue::default()
                    .insert(&format!("scenic-pair {invalid} {HASH}"))
                    .is_err()
            );
        }
    }
}
