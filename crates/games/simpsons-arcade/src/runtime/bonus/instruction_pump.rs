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
//   - Marge's instruction body and hose animation from fixed phase ticks.
// - Must-Not:
//   - Advance gameplay, admit competitors or animate unverified stations.
// - Allows:
//   - Inputs: Locally selected verified sprite bundles and fixed phase ticks.
//   - Outputs: Registered body and hose draws.
//   - Side effects: Bounded atlas allocation during load.
// - Split-When:
//   - Additional character animations require separate verified tracks.
// - Merge-When:
//   - Shared bonus pose registration covers all instruction animations.
// - Summary:
//   - Two independent observed animation cycles, without duplicate sprites.
// - Description:
//   - The body cycles every five ticks; the hose has a four-tick intro and then
//     cycles every nine ticks.
// - Usage:
//   - Select only for Marge during the instruction phase.
// - Defaults:
//   - The existing stationary preview remains when no bundles are selected.

//! Optional original-backed Marge instruction pump.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};

use super::{
    Art, Scene, SpriteSpec, failure, open_bundle, verify_registered_sprite,
};
use crate::{BonusPhase, Cabinet, Player};

pub(in crate::runtime) const ATLAS_BYTES: usize = 28_328;
pub(in crate::runtime) const ATLAS_COUNT: usize = 7;
pub(in crate::runtime) const BUNDLE_COUNT: usize = 4;
const BODIES: [SpriteSpec; 3] = [
    SpriteSpec {
        height: 75,
        key: "bonus/marge-instruction-body-0",
        width: 29,
        x: 94,
        y: 69,
    },
    SpriteSpec {
        height: 73,
        key: "bonus/marge-instruction-body-1",
        width: 27,
        x: 94,
        y: 71,
    },
    SpriteSpec {
        height: 62,
        key: "bonus/marge-instruction-body-2",
        width: 28,
        x: 94,
        y: 82,
    },
];
const HOSES: [SpriteSpec; 4] = [
    SpriteSpec {
        height: 48,
        key: "bonus/marge-instruction-hose-0",
        width: 5,
        x: 89,
        y: 96,
    },
    SpriteSpec {
        height: 48,
        key: "bonus/marge-instruction-hose-1",
        width: 6,
        x: 88,
        y: 96,
    },
    SpriteSpec {
        height: 48,
        key: "bonus/marge-instruction-hose-2",
        width: 7,
        x: 87,
        y: 96,
    },
    SpriteSpec {
        height: 48,
        key: "bonus/marge-instruction-hose-3",
        width: 7,
        x: 87,
        y: 96,
    },
];

pub(in crate::runtime) type Digests = [ContentHash; BUNDLE_COUNT];
type PoseSelection = (ContentHash, SpriteDraw);

#[derive(Debug)]
struct PumpPose {
    art: Art,
    digest: ContentHash,
}

impl PumpPose {
    fn load(
        spec: SpriteSpec,
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
    ) -> Result<Self, RenderError> {
        let art = spec.load(scene, bundle)?;
        let digest = bundle
            .manifest()
            .record(spec.key)
            .ok_or(RenderError::InvalidSprite)?
            .digest;
        Ok(Self { art, digest })
    }
}

#[derive(Debug)]
pub(super) struct PumpArt {
    bodies: [PumpPose; 3],
    hoses: [PumpPose; 4],
}

impl PumpArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        hashes: &Digests,
    ) -> Result<Self, RenderError> {
        let [first, second, third, fourth] = *hashes;
        let [body_zero, body_one, body_two] = BODIES;
        let [hose_zero, hose_one, hose_two, hose_three] = HOSES;
        let initial = open_bundle(store, first)?;
        let middle = open_bundle(store, second)?;
        let lowered = open_bundle(store, third)?;
        let end = open_bundle(store, fourth)?;
        Ok(Self {
            bodies: [
                PumpPose::load(body_zero, scene, &initial)?,
                PumpPose::load(body_one, scene, &middle)?,
                PumpPose::load(body_two, scene, &lowered)?,
            ],
            hoses: [
                PumpPose::load(hose_zero, scene, &initial)?,
                PumpPose::load(hose_one, scene, &middle)?,
                PumpPose::load(hose_two, scene, &lowered)?,
                PumpPose::load(hose_three, scene, &end)?,
            ],
        })
    }

    pub(super) fn queue(
        &self,
        tick: u16,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        draws.extend(self.selection(tick)?.map(|(_, draw)| draw));
        Ok(())
    }

    fn selection(&self, tick: u16) -> Result<[PoseSelection; 2], RenderError> {
        let (body, hose) = phases(tick);
        Ok([
            selected_pose(hose, &HOSES, &self.hoses)?,
            selected_pose(body, &BODIES, &self.bodies)?,
        ])
    }

    fn verify(
        &self,
        scene: &mut SceneSession<'_, '_>,
        draws: &[SpriteDraw],
        tick: u16,
    ) -> Result<(), RenderError> {
        let actual = scene.readback()?;
        for selection in self.selection(tick)? {
            verify_registered_sprite(scene, draws, &actual, selection)?;
        }
        Ok(())
    }
}

fn selected_pose(
    index: usize,
    specs: &[SpriteSpec],
    poses: &[PumpPose],
) -> Result<PoseSelection, RenderError> {
    let spec = specs.get(index).ok_or(RenderError::InvalidSprite)?;
    let pose = poses.get(index).ok_or(RenderError::InvalidSprite)?;
    Ok((pose.digest, pose.art.draw(spec.destination(0)?, false)))
}

/// Verify the selected body and hose independently on the routed scene.
pub(in crate::runtime) fn verify(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let Some(pump) = art
        .poses
        .bonus
        .as_ref()
        .and_then(|bonus| bonus.instruction_pump.as_ref())
    else {
        return Ok(());
    };
    let bonus = cabinet.bonus().ok_or(RenderError::InvalidSprite)?;
    if bonus.phase() != BonusPhase::Instructions
        || bonus.instruction_player() != Some(Player::Marge)
    {
        return Err(failure("instruction pump smoke requires Marge"));
    }
    pump.verify(scene, &art.draws, bonus.tick())?;
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

pub(in crate::runtime) fn parse(
    line: &str,
    selected: bool,
) -> Result<Digests, RenderError> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("bonus-instruction-pump") || selected {
        return Err(failure("unexpected or duplicate instruction pump record"));
    }
    let mut read = || {
        fields
            .next()
            .ok_or_else(|| failure("missing pump pose bundle"))?
            .parse::<ContentHash>()
            .map_err(failure)
    };
    let digests = [read()?, read()?, read()?, read()?];
    if fields.next().is_some() {
        return Err(failure("extra pump pose bundle"));
    }
    Ok(digests)
}

fn phases(tick: u16) -> (usize, usize) {
    let elapsed = usize::from(tick);
    let body = elapsed.checked_div(5).unwrap_or_default().rem_euclid(3);
    let hose = if elapsed < 4 {
        0
    } else {
        elapsed
            .saturating_sub(4)
            .checked_div(9)
            .unwrap_or_default()
            .rem_euclid(3)
            .saturating_add(1)
    };
    (body, hose)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_cycles_keep_independent_boundaries() {
        for (tick, expected) in [
            (0, (0, 0)),
            (3, (0, 0)),
            (4, (0, 1)),
            (5, (1, 1)),
            (9, (1, 1)),
            (10, (2, 1)),
            (12, (2, 1)),
            (13, (2, 2)),
            (14, (2, 2)),
            (15, (0, 2)),
            (22, (1, 3)),
            (30, (0, 3)),
            (31, (0, 1)),
            (310, (2, 2)),
        ] {
            assert_eq!(phases(tick), expected);
        }
        for tick in 4u16..176 {
            assert_eq!(phases(tick).0, phases(tick.saturating_add(15)).0);
            assert_eq!(phases(tick).1, phases(tick.saturating_add(27)).1);
        }
    }
}
