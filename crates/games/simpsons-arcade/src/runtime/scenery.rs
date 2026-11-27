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
//   - Optional bounded mid-street scenic art admission and placement.
// - Must-Not:
//   - Resolve combat, change camera gates or interpret original object data.
// - Allows:
//   - Inputs: Pinned isolated art, fixed-step scene tick and world viewport.
//   - Outputs: At most two positioned, clipped sprite draw.
//   - Side effects: Load admitted canonical atlases during scene startup.
// - Split-When:
//   - Additional independent scripted scenes require distinct tracks.
// - Merge-When:
//   - Shared placement-aware pose tracks subsume this local composition.
// - Summary:
//   - Present independently observed noncombat scenic sequences.
// - Description:
//   - Hold sparse observed frames at their measured original raster origins.
// - Usage:
//   - Add optional scenic-pair or scenic-stationary records to the catalogue.
// - Defaults:
//   - No scenery without a complete bounded track.

//! Optional, sparse world-anchored mid-street scene presentation.

use std::collections::BTreeMap;
use std::mem::take;

use games::animation::{Keyframes, keyframe, optional_track_complete};
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
    Action, BASE_WIDTH, Cabinet, Control, MID_STREET_SCRIPT_ORIGIN,
    MID_STREET_SCRIPT_TICKS, Player, Point, SIDE, STATIONARY_SCENE_CYCLE,
};

const FIRST_TICK: u16 = 8;
const STEP: u16 = 8;
const LAST_TICK: u16 = MID_STREET_SCRIPT_TICKS - STEP;
const MAX_GAP: u16 = STEP * 2;
const STATIONARY_X: i16 = 804;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Pair,
    Stationary,
}

impl Kind {
    const ALL: [Self; 2] = [Self::Stationary, Self::Pair];

    const fn asset_key(self) -> &'static str {
        match self {
            Self::Pair => "actor/scenic-pair",
            Self::Stationary => "actor/scenic-stationary",
        }
    }

    const fn frame(self, tick: u16) -> Option<u16> {
        match self {
            Self::Pair if tick < MID_STREET_SCRIPT_TICKS => Some(tick),
            Self::Stationary if tick < STATIONARY_SCENE_CYCLE => {
                Some(stationary_frame(tick))
            },
            Self::Pair | Self::Stationary => None,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "scenic-pair" => Some(Self::Pair),
            "scenic-stationary" => Some(Self::Stationary),
            _ => None,
        }
    }

    fn tick(self, cabinet: &Cabinet) -> Option<u16> {
        match self {
            Self::Pair => cabinet.scripted_scene_tick(),
            Self::Stationary => cabinet.stationary_scene_tick(),
        }
    }

    fn valid_sample(self, tick: u16, origin: Point) -> bool {
        match self {
            Self::Pair => {
                (FIRST_TICK..=LAST_TICK).contains(&tick)
                    && tick.is_multiple_of(STEP)
                    && (0..320).contains(&origin.x)
                    && (28..224).contains(&origin.y)
            },
            Self::Stationary => {
                tick <= 2
                    && origin.x == STATIONARY_X
                    && origin.y == stationary_top(tick)
            },
        }
    }

    fn validate_art(
        self,
        frame: u16,
        art: Art,
        origin: Point,
    ) -> Result<(), RenderError> {
        let width = art.rect.width.get();
        let height = art.rect.height.get();
        let valid = match self {
            Self::Pair => {
                width <= 128
                    && height <= 128
                    && u32::try_from(origin.x)
                        .map_err(failure)?
                        .saturating_add(width)
                        <= BASE_WIDTH
                    && u32::try_from(origin.y)
                        .map_err(failure)?
                        .saturating_add(height)
                        <= 224
            },
            Self::Stationary => {
                width == 26
                    && height
                        == if frame == 1 {
                            63
                        } else {
                            64
                        }
            },
        };
        if !valid {
            return Err(failure("scenic pose exceeds observed extent"));
        }
        Ok(())
    }

    const fn world_origin(self, origin: Point) -> Point {
        match self {
            Self::Pair => Point {
                x: MID_STREET_SCRIPT_ORIGIN.saturating_add(origin.x),
                y: origin.y,
            },
            Self::Stationary => origin,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    digest: ContentHash,
    origin: Point,
}

type Track<T> = Keyframes<Kind, (), u16, T>;

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    entries: Track<Sample>,
}

impl Catalogue {
    pub(super) fn complete(&self) -> bool {
        self.pair_complete()
            && optional_track_complete(&self.entries, Kind::Stationary, (), &[
                0, 1, 2,
            ])
    }

    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.entries.values().map(|sample| sample.digest)
    }

    pub(super) fn insert(&mut self, line: &str) -> Result<(), RenderError> {
        let [name, raw_tick, raw_left, raw_top, digest] =
            <[&str; 5]>::try_from(line.split_whitespace().collect::<Vec<_>>())
                .map_err(|_error| failure("invalid scenic pose record"))?;
        let kind = Kind::from_name(name)
            .ok_or_else(|| failure("unknown scenic pose"))?;
        let tick = raw_tick.parse::<u16>().map_err(failure)?;
        let origin = Point {
            x: raw_left.parse::<i16>().map_err(failure)?,
            y: raw_top.parse::<i16>().map_err(failure)?,
        };
        if !kind.valid_sample(tick, origin) {
            return Err(failure("scenic pose outside observed interval"));
        }
        let sample = Sample {
            digest: digest.parse().map_err(failure)?,
            origin,
        };
        if self.entries.insert((kind, (), tick), sample).is_some() {
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
            let art = Art::load(scene, &bundle, key.0.asset_key())?;
            key.0.validate_art(key.2, art, sample.origin)?;
            let _prior = entries.insert(key, (art, sample.origin));
        }
        Ok(PlacedArt { entries })
    }

    fn pair_complete(&self) -> bool {
        if !has_track(&self.entries, Kind::Pair) {
            return true;
        }
        let mut previous = FIRST_TICK;
        if !self.entries.contains_key(&(Kind::Pair, (), FIRST_TICK)) {
            return false;
        }
        for &(kind, (), tick) in self.entries.keys() {
            if kind != Kind::Pair {
                continue;
            }
            if tick.saturating_sub(previous) > MAX_GAP {
                return false;
            }
            previous = tick;
        }
        previous == LAST_TICK
    }

    pub(super) fn track_count(&self) -> usize {
        Kind::ALL
            .into_iter()
            .filter(|&kind| has_track(&self.entries, kind))
            .count()
    }
}

#[derive(Debug, Default)]
pub(super) struct PlacedArt {
    entries: Track<(Art, Point)>,
}

impl PlacedArt {
    fn draw(
        &self,
        kind: Kind,
        tick: u16,
        view: View,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let Some(frame) = kind.frame(tick) else {
            return Ok(None);
        };
        let Some(&(art, origin)) = keyframe(&self.entries, (kind, (), frame))
        else {
            return Ok(None);
        };
        clipped_draw(art, view.project(kind.world_origin(origin)), view.width)
    }

    pub(super) fn queue(
        &self,
        cabinet: &Cabinet,
        view: View,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        for kind in Kind::ALL {
            if let Some(tick) = kind.tick(cabinet)
                && let Some(draw) = self.draw(kind, tick, view)?
            {
                draws.push(draw);
            }
        }
        Ok(())
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

fn has_track<T>(entries: &Track<T>, kind: Kind) -> bool {
    entries.keys().any(|&(owner, (), _)| owner == kind)
}

const fn stationary_frame(tick: u16) -> u16 {
    match tick {
        0..=12 | 26..=38 | 65..=77 => 0,
        13..=25 | 39..=51 => 1,
        _ => 2,
    }
}

const fn stationary_top(frame: u16) -> i16 {
    if frame == 1 {
        82
    } else {
        81
    }
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
) -> Result<(), RenderError> {
    for kind in Kind::ALL {
        if has_track(&art.poses.scenery.entries, kind) {
            smoke_track(scene, art, kind)?;
        }
    }
    Ok(())
}

fn smoke_track(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    kind: Kind,
) -> Result<(), RenderError> {
    let limit = match kind {
        Kind::Pair => {
            u16::try_from(MID_STREET_SCRIPT_ORIGIN).map_err(failure)?
        },
        Kind::Stationary => 566,
    };
    let mut cabinet = camera_at_limit(limit)?;
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
    let tick = kind
        .tick(&cabinet)
        .ok_or_else(|| failure("scripted scene did not start"))?;
    let view = art.presentation_view(scene, &cabinet)?;
    let draw =
        art.poses.scenery.draw(kind, tick, view)?.ok_or_else(|| {
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
    let name = match kind {
        Kind::Pair => "Scripted scenic pair",
        Kind::Stationary => "Stationary scenic actor",
    };
    write_status(format_args!("{name}: visible at tick {tick}"))
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
        assert!(track.entries.remove(&(Kind::Pair, (), 24)).is_some());
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

    #[test]
    fn stationary_track_requires_all_three_measured_poses() {
        let mut track = Catalogue::default();
        assert_eq!(track.track_count(), 0);
        for (frame, top) in [(0u16, 81i16), (1, 82)] {
            assert!(
                track
                    .insert(&format!(
                        "scenic-stationary {frame} 804 {top} {HASH}"
                    ))
                    .is_ok()
            );
        }
        assert!(!track.complete());
        assert!(
            track
                .insert(&format!("scenic-stationary 2 804 81 {HASH}"))
                .is_ok()
        );
        assert!(track.complete());
        assert_eq!(track.track_count(), 1);
        assert_eq!(track.count(), 3);
    }

    #[test]
    fn stationary_track_rejects_unobserved_poses_and_origins() {
        for invalid in ["3 804 81", "0 805 81", "1 804 81", "2 804 82"] {
            assert!(
                Catalogue::default()
                    .insert(&format!("scenic-stationary {invalid} {HASH}"))
                    .is_err()
            );
        }
    }

    #[test]
    fn stationary_phase_selects_each_observed_interval() {
        let intervals = [
            (0..13, 0),
            (13..26, 1),
            (26..39, 0),
            (39..52, 1),
            (52..65, 2),
            (65..78, 0),
            (78..100, 2),
        ];
        for (ticks, frame) in intervals {
            for tick in ticks {
                assert_eq!(Kind::Stationary.frame(tick), Some(frame));
            }
        }
        assert_eq!(Kind::Stationary.frame(100), None);
    }
}
