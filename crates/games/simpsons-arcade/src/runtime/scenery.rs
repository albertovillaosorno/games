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
//   - Optional bounded stage scenic art admission and placement.
// - Must-Not:
//   - Resolve combat, change camera gates or interpret original object data.
// - Allows:
//   - Inputs: Pinned isolated art, fixed-step scene tick and world viewport.
//   - Outputs: Bounded positioned, clipped sprite draws.
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
//   - Add optional scenic records to the catalogue.
// - Defaults:
//   - No scenery without a complete bounded track.

//! Optional, sparse world-anchored stage scene presentation.

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
use super::{Art, Scene, failure, open_bundle};
use crate::{
    Action, BASE_WIDTH, Cabinet, Control, MID_STREET_SCRIPT_ORIGIN,
    MID_STREET_SCRIPT_TICKS, Player, Point, SIDE, STATIONARY_SCENE_CYCLE,
    StageEntrance,
};

const FIRST_TICK: u16 = 8;
const STEP: u16 = 8;
const LAST_TICK: u16 = MID_STREET_SCRIPT_TICKS - STEP;
const MAX_GAP: u16 = STEP * 2;
const ENTRANCE_FIRST: u16 = 2;
const ENTRANCE_LAST: u16 = 221;
const STATIONARY_X: i16 = 804;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Entrance,
    Pair,
    Stationary,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Stationary, Self::Pair, Self::Entrance];

    const fn asset_key(self) -> &'static str {
        match self {
            Self::Entrance => "actor/scenic-entrance",
            Self::Pair => "actor/scenic-pair",
            Self::Stationary => "actor/scenic-stationary",
        }
    }

    const fn atlas_bytes(self) -> usize {
        match self {
            Self::Entrance => 166 * 185 * 4,
            Self::Pair | Self::Stationary => 65_536,
        }
    }

    const fn frame(self, tick: u16) -> Option<u16> {
        match self {
            Self::Entrance
                if tick >= ENTRANCE_FIRST && tick <= ENTRANCE_LAST =>
            {
                Some(tick)
            },
            Self::Pair if tick < MID_STREET_SCRIPT_TICKS => Some(tick),
            Self::Stationary if tick < STATIONARY_SCENE_CYCLE => {
                Some(stationary_frame(tick))
            },
            Self::Entrance | Self::Pair | Self::Stationary => None,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "scenic-entrance" => Some(Self::Entrance),
            "scenic-pair" => Some(Self::Pair),
            "scenic-stationary" => Some(Self::Stationary),
            _ => None,
        }
    }

    fn tick(self, cabinet: &Cabinet) -> Option<u16> {
        match self {
            Self::Entrance => cabinet.stage_entrance().map(StageEntrance::tick),
            Self::Pair => cabinet.scripted_scene_tick(),
            Self::Stationary => cabinet.stationary_scene_tick(),
        }
    }

    fn valid_sample(self, tick: u16, origin: Point) -> bool {
        match self {
            Self::Entrance => {
                (tick == ENTRANCE_FIRST
                    || tick == ENTRANCE_LAST
                    || ((8..=216).contains(&tick) && tick.is_multiple_of(STEP)))
                    && (56..=183).contains(&origin.x)
                    && origin.y == 0
            },
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
            Self::Entrance => {
                width <= 166
                    && height <= 185
                    && u32::try_from(origin.x)
                        .map_err(failure)?
                        .saturating_add(width)
                        <= BASE_WIDTH
            },
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
            Self::Entrance | Self::Stationary => origin,
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
    pub(super) fn atlas_bytes(&self) -> usize {
        self.entries
            .keys()
            .map(|(kind, (), _)| kind.atlas_bytes())
            .fold(0usize, usize::saturating_add)
    }

    fn bounded_track(&self, owner: Kind, first: u16, last: u16) -> bool {
        if !has_track(&self.entries, owner) {
            return true;
        }
        let mut previous = first;
        if !self.entries.contains_key(&(owner, (), first)) {
            return false;
        }
        for &(kind, (), tick) in self.entries.keys() {
            if kind != owner {
                continue;
            }
            if tick.saturating_sub(previous) > MAX_GAP {
                return false;
            }
            previous = tick;
        }
        previous == last
    }

    pub(super) fn complete(&self) -> bool {
        self.bounded_track(Kind::Pair, FIRST_TICK, LAST_TICK)
            && self.bounded_track(Kind::Entrance, ENTRANCE_FIRST, ENTRANCE_LAST)
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

    pub(super) fn has_entrance(&self) -> bool {
        has_track(&self.entries, Kind::Entrance)
    }

    pub(super) fn has_street(&self) -> bool {
        has_track(&self.entries, Kind::Pair)
            || has_track(&self.entries, Kind::Stationary)
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
    let destination = SpriteRect {
        x: i32::from(point.x),
        y: i32::from(point.y),
        ..art.rect
    };
    let left = i32::try_from(side).map_err(failure)?;
    let right =
        i32::try_from(view_width.saturating_sub(side)).map_err(failure)?;
    Ok(art.draw(destination, false).clipped_horizontal(left, right))
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
    for kind in [Kind::Stationary, Kind::Pair] {
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
        Kind::Entrance => {
            return Err(failure("entrance requires the second stage"));
        },
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
    let _drawn = art.present(scene, &cabinet)?;
    verify_visible(scene, art, &cabinet, (kind, tick))?;
    let name = match kind {
        Kind::Pair => "Scripted scenic pair",
        Kind::Stationary => "Stationary scenic actor",
        Kind::Entrance => "Second stage vehicle",
    };
    write_status(format_args!("{name}: visible at tick {tick}"))
        .map_err(failure)?;
    Ok(())
}

pub(super) fn verify_entrance(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    if !has_track(&art.poses.scenery.entries, Kind::Entrance) {
        return Ok(());
    }
    let tick = Kind::Entrance
        .tick(cabinet)
        .ok_or_else(|| failure("missing entrance"))?;
    if visible {
        verify_visible(scene, art, cabinet, (Kind::Entrance, tick))?;
    } else if scene.readback()?
        != readback_without_scenery(scene, art, cabinet)?
    {
        return Err(failure("second-stage vehicle outside its interval"));
    }
    let state = if visible {
        "visible"
    } else {
        "absent"
    };
    write_status(format_args!("Second stage vehicle: {state} at tick {tick}"))
        .map_err(failure)
}

fn readback_without_scenery(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<Vec<u8>, RenderError> {
    let saved = take(&mut art.poses.scenery);
    let baseline = art.present(scene, cabinet).and_then(|_| scene.readback());
    art.poses.scenery = saved;
    baseline
}

fn verify_visible(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    (kind, tick): (Kind, u16),
) -> Result<(), RenderError> {
    let view = art.presentation_view(scene, cabinet)?;
    let draw =
        art.poses.scenery.draw(kind, tick, view)?.ok_or_else(|| {
            failure("scripted scene did not provide visible art")
        })?;
    let actual = scene.readback()?;
    let baseline = readback_without_scenery(scene, art, cabinet)?;
    verify_pixels(&actual, &baseline, view.width, &[draw.destination])
}

pub(super) fn verify_pixels(
    actual: &[u8],
    baseline: &[u8],
    width: u32,
    bounds: &[SpriteRect],
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
        if !bounds
            .iter()
            .any(|rect| rect.contains_pixel(horizontal, vertical))
        {
            return Err(failure("scripted scene changed unrelated pixels"));
        }
        changed = true;
    }
    if !changed {
        return Err(failure("scripted scene did not change pixels"));
    }
    Ok(())
}

// Opaque sprite crops are recovered exactly using two distinct clears.
// Fractional alpha is deliberately rejected.
pub(super) fn opaque_sprite_digest(
    dark: &[u8],
    light: &[u8],
    rect: SpriteRect,
    width: u32,
) -> Result<ContentHash, RenderError> {
    let (dark_pixels, dark_tail) = dark.as_chunks::<4>();
    let (light_pixels, light_tail) = light.as_chunks::<4>();
    if !dark_tail.is_empty()
        || !light_tail.is_empty()
        || dark_pixels.len() != light_pixels.len()
    {
        return Err(RenderError::InvalidLength);
    }
    let mut pixels = Vec::new();
    for (index, (&dark_pixel, &light_pixel)) in
        dark_pixels.iter().zip(light_pixels).enumerate()
    {
        let pixel_index = u32::try_from(index).map_err(failure)?;
        let horizontal = pixel_index
            .checked_rem(width)
            .ok_or(RenderError::InvalidLength)?;
        let vertical = pixel_index
            .checked_div(width)
            .ok_or(RenderError::InvalidLength)?;
        if !rect.contains_pixel(horizontal, vertical) {
            continue;
        }
        if dark_pixel == light_pixel && dark_pixel.last() == Some(&255) {
            pixels.push(dark_pixel);
        } else if dark_pixel == [0, 0, 0, 255] && light_pixel == [255; 4] {
            pixels.push([0; 4]);
        } else {
            return Err(failure("opaque sprite has blended pixels"));
        }
    }
    let expected = rect.width.get().saturating_mul(rect.height.get());
    if pixels.len() != usize::try_from(expected).map_err(failure)? {
        return Err(RenderError::InvalidLength);
    }
    Ok(ContentHash::calculate(pixels.as_flattened()))
}

#[cfg(test)]
mod tests {
    use std::iter::once;

    use super::*;

    const HASH: &str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn opaque_sprite_readback_recovers_the_exact_transparent_crop() {
        use std::num::NonZeroU32;

        let rect = SpriteRect {
            x: 0,
            y: 0,
            width: NonZeroU32::MIN.saturating_add(1),
            height: NonZeroU32::MIN,
        };
        let dark = [12, 34, 56, 255, 0, 0, 0, 255];
        let light = [12, 34, 56, 255, 255, 255, 255, 255];
        let expected = ContentHash::calculate(&[12, 34, 56, 255, 0, 0, 0, 0]);
        assert_eq!(opaque_sprite_digest(&dark, &light, rect, 2), Ok(expected));
        let blended = [12, 34, 56, 255, 153, 153, 153, 255];
        assert!(opaque_sprite_digest(&dark, &blended, rect, 2).is_err());
        assert!(opaque_sprite_digest(&dark, &light[..4], rect, 2).is_err());
    }

    fn complete_entrance_track() -> Catalogue {
        let mut track = Catalogue::default();
        for tick in once(ENTRANCE_FIRST)
            .chain((8..=216).step_by(8))
            .chain(once(ENTRANCE_LAST))
            .filter(|&tick| tick != 112)
        {
            assert!(
                track
                    .insert(&format!("scenic-entrance {tick} 80 0 {HASH}"))
                    .is_ok()
            );
        }
        track
    }

    #[test]
    fn entrance_track_requires_endpoints_and_bounded_gaps() {
        assert!(Catalogue::default().complete());
        let mut track = complete_entrance_track();
        assert!(track.complete());
        assert!(track.has_entrance());
        assert!(!track.has_street());
        assert_eq!(track.atlas_bytes(), 28 * 166 * 185 * 4);
        assert!(track.entries.remove(&(Kind::Entrance, (), 120)).is_some());
        assert!(!track.complete());
    }

    #[test]
    fn entrance_track_rejects_invalid_samples_and_stops_at_exit() {
        for invalid in [
            "1 80 0", "222 80 0", "9 80 0", "8 55 0", "8 184 0", "8 80 1",
        ] {
            assert!(
                Catalogue::default()
                    .insert(&format!("scenic-entrance {invalid} {HASH}"))
                    .is_err()
            );
        }
        assert_eq!(Kind::Entrance.frame(1), None);
        assert_eq!(Kind::Entrance.frame(2), Some(2));
        assert_eq!(Kind::Entrance.frame(221), Some(221));
        assert_eq!(Kind::Entrance.frame(222), None);
    }

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
