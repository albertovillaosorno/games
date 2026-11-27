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
//   - Verified optional late encounter backdrop selection and rendering.
// - Must-Not:
//   - Resolve boss combat, move the camera or invent widened art.
// - Allows:
//   - Inputs: Complete canonical backdrop catalogue and cabinet tick.
//   - Outputs: One opaque original-width backdrop sprite.
//   - Side effects: Load bounded canonical atlases during scene startup.
// - Split-When:
//   - More stages need independent backdrop sequencing.
// - Merge-When:
//   - A shared background animation contract subsumes this composition.
// - Summary:
//   - Present a measured late encounter background sequence.
// - Description:
//   - Use observed phase keys; hold the last known phase after tick 320.
// - Usage:
//   - Select all backdrop records alongside the verified long street.
// - Defaults:
//   - No replacement backdrop unless the complete sequence is selected.

//! Optional measured late encounter background sequence.

use std::collections::BTreeMap;

use games::animation::cycle_tick;
use games::replay::InputStep as _;
use games_assets::{AssetFormat, ContentHash};
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::BundleSource;

use super::{Art, Cabinet, Scene, Sources, failure, open_bundle, world};

const TOP: i32 = 28;
const HEIGHT: u32 = 196;
pub(super) const ATLAS_BYTES: usize = 320 * 196 * 4;
pub(super) const KEYS: [u16; 15] = [
    0, 4, 9, 14, 19, 24, 257, 258, 259, 261, 262, 263, 264, 265, 267,
];
const PERIOD: [u16; 8] = [264, 265, 264, 267, 264, 267, 264, 265];

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    entries: BTreeMap<u16, ContentHash>,
}

impl Catalogue {
    pub(super) fn complete(&self) -> bool {
        self.entries.is_empty() || self.entries.len() == KEYS.len()
    }

    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.entries.values().copied()
    }

    pub(super) fn insert(&mut self, line: &str) -> Result<(), RenderError> {
        let mut fields = line.split_whitespace();
        let (Some("backdrop"), Some(key_text), Some(digest_text), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(failure("invalid backdrop record"));
        };
        let key = key_text.parse::<u16>().map_err(failure)?;
        let digest = digest_text.parse::<ContentHash>().map_err(failure)?;
        if !KEYS.contains(&key) || self.entries.insert(key, digest).is_some() {
            return Err(failure("unexpected or duplicate backdrop phase"));
        }
        Ok(())
    }

    pub(super) fn load(
        &self,
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<BackdropArt, RenderError> {
        let mut entries = BTreeMap::new();
        for (&key, &digest) in &self.entries {
            let bundle = open_bundle(store, digest)?;
            let record = bundle
                .manifest()
                .record("scene/background")
                .ok_or(RenderError::InvalidSprite)?;
            let AssetFormat::Rgba8 { height, width } = record.format else {
                return Err(failure("backdrop must be RGBA8"));
            };
            let pixels =
                bundle.view_payload("scene/background").map_err(failure)?;
            if (width.get(), height.get()) != (world::BASE_WIDTH, HEIGHT)
                || !pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel.last() == Some(&255))
            {
                return Err(failure("invalid backdrop extent or opacity"));
            }
            let _prior = entries
                .insert(key, Art::upload(scene, (width, height), &pixels)?);
        }
        Ok(BackdropArt { entries })
    }
}

#[derive(Debug, Default)]
pub(super) struct BackdropArt {
    entries: BTreeMap<u16, Art>,
}

impl BackdropArt {
    pub(super) fn draw(&self, cabinet: &Cabinet) -> Option<SpriteDraw> {
        if !cabinet.session().world_visible() {
            return None;
        }
        let tick = cabinet.late_backdrop_tick()?;
        let art = self.entries.get(&phase_key(tick))?;
        let mut destination = art.rect;
        destination.y = TOP;
        Some(art.draw(destination, false))
    }
}

fn phase_key(tick: u16) -> u16 {
    match tick {
        0..=3 => 0,
        4..=8 => 4,
        9..=13 => 9,
        14..=18 => 14,
        19..=23 => 19,
        24..=256 => 24,
        257..=259 | 261..=263 => tick,
        260 => 258,
        264..=319 => cycle_tick(u64::from(tick.saturating_sub(264)), 8u8)
            .and_then(|phase| PERIOD.get(usize::from(phase)))
            .copied()
            .unwrap_or(264),
        _ => 264,
    }
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    source: &Sources<'_>,
) -> Result<(), RenderError> {
    let catalogue = &source.poses.backdrop;
    if catalogue.count() == 0 {
        return Ok(());
    }
    let limit = u16::try_from(crate::LATE_BACKDROP_ORIGIN).map_err(failure)?;
    let mut cabinet = world::camera_at_limit(limit)?;
    for tick in 0..=crate::LATE_BACKDROP_HOLD_TICK {
        if KEYS.contains(&tick) || tick == crate::LATE_BACKDROP_HOLD_TICK {
            let digest = catalogue
                .entries
                .get(&phase_key(tick))
                .ok_or(RenderError::InvalidSprite)?;
            let bundle = open_bundle(source.store, *digest)?;
            let pixels =
                bundle.view_payload("scene/background").map_err(failure)?;
            verify_frame(scene, art, &cabinet, &pixels)?;
        }
        cabinet.input_step(u64::from(tick).saturating_add(2000), &[]);
    }
    write_status(format_args!(
        "Late backdrop: {} exact original-width phases",
        KEYS.len()
    ))
    .map_err(failure)
}

fn verify_frame(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    expected: &[u8],
) -> Result<(), RenderError> {
    let draw = art
        .poses
        .backdrop
        .draw(cabinet)
        .ok_or(RenderError::InvalidSprite)?;
    let _presented = art.present(scene, cabinet)?;
    let layout = scene.layout();
    if layout.width().get() != world::BASE_WIDTH
        || art.draws.first() != Some(&draw)
    {
        return Err(failure("late backdrop viewport or layering differs"));
    }
    let pitch = layout.pitch();
    let offset = pitch.saturating_mul(usize::try_from(TOP).map_err(failure)?);
    // The original HUD is excluded and the player stands below these rows.
    let upper = pitch.saturating_mul(28);
    let composed = scene.readback()?;
    if composed.get(offset..offset.saturating_add(upper))
        != expected.get(..upper)
    {
        return Err(failure("composed late backdrop pixels differ"));
    }
    let _isolated = scene.present([24, 22, 30], &[draw])?;
    let isolated = scene.readback()?;
    if isolated.get(offset..offset.saturating_add(expected.len()))
        != Some(expected)
    {
        return Err(failure("isolated late backdrop pixels differ"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_selection_retains_nonmonotonic_observed_keys() {
        assert_eq!((0..24).map(phase_key).collect::<Vec<_>>(), [
            0, 0, 0, 0, 4, 4, 4, 4, 4, 9, 9, 9, 9, 9, 14, 14, 14, 14, 14, 19,
            19, 19, 19, 19,
        ]);
        assert!((24..257).all(|tick| phase_key(tick) == 24));
        assert_eq!((257..264).map(phase_key).collect::<Vec<_>>(), [
            257, 258, 259, 258, 261, 262, 263,
        ]);
        for start in (264..320).step_by(8) {
            assert_eq!(
                (start..start + 8).map(phase_key).collect::<Vec<_>>(),
                PERIOD
            );
        }
        assert_eq!(phase_key(320), 264);
        assert_eq!(phase_key(u16::MAX), 264);
    }
}
