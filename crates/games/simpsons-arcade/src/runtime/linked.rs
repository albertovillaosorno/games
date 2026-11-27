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
//   - Canonical sparse art admission and linked encounter presentation.
// - Must-Not:
//   - Interpret source media or change gameplay state.
// - Allows:
//   - Inputs: Pinned canonical bundles, observable encounter, viewport.
//   - Outputs: Depth-ordered sprite draws.
//   - Side effects: Bounded atlas uploads during initialization.
// - Split-When:
//   - Additional linked actors require distinct art ownership.
// - Merge-When:
//   - Another title-local module duplicates these pose roles.
// - Summary:
//   - Optional boardwalk parent and child poses.
// - Description:
//   - Selects seven independently matched sparse poses.
// - Usage:
//   - Provide the complete linked-actors catalogue record.
// - Defaults:
//   - Disabled without verified local assets.

//! Optional linked encounter presentation.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::storage::bundle::BundleSource;

use super::{
    Art, Scene, SpriteDraw, SpriteRect, failure, ground_position, open_bundle,
    world,
};
use crate::{Cabinet, ChildPose, Facing, ParentPose, Point};

pub(super) const POSE_COUNT: usize = 7;
pub(super) const EXTRA_SPRITES: usize =
    crate::LINKED_PARENT_CAPACITY.saturating_add(crate::LINKED_CHILD_CAPACITY);

#[derive(Debug)]
pub(super) struct LinkedArt {
    active: Art,
    closed: Art,
    emerging: Art,
    holding: Art,
    hurt: Art,
    intermediate: Art,
    open: Art,
}

impl LinkedArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        poses: &[ContentHash; POSE_COUNT],
    ) -> Result<Self, RenderError> {
        let [closed, intermediate, open, emerging, holding, active, hurt] =
            *poses;
        let mut load = |digest, role| {
            let bundle = open_bundle(store, digest)?;
            let art = Art::load(scene, &bundle, role)?;
            if art.rect.width.get() > 128 || art.rect.height.get() > 128 {
                return Err(failure("unexpected linked actor extent"));
            }
            Ok(art)
        };
        Ok(Self {
            active: load(active, "actor/linked")?,
            closed: load(closed, "actor/parent")?,
            emerging: load(emerging, "actor/linked")?,
            holding: load(holding, "actor/linked")?,
            hurt: load(hurt, "actor/linked")?,
            intermediate: load(intermediate, "actor/parent")?,
            open: load(open, "actor/parent")?,
        })
    }

    pub(super) fn queue(
        &self,
        cabinet: &Cabinet,
        view: world::View,
        draws: &mut Vec<(i16, SpriteDraw)>,
    ) -> Result<(), RenderError> {
        for parent in cabinet.linked_parents() {
            let Some(pose) = parent.pose() else {
                continue;
            };
            let art = match pose {
                ParentPose::Closed => self.closed,
                ParentPose::Intermediate => self.intermediate,
                ParentPose::Open => self.open,
            };
            let position = view.project(Point { x: parent.x(), y: 60 });
            let rect = SpriteRect {
                x: i32::from(position.x).saturating_sub(
                    i32::try_from(art.rect.width.get().saturating_div(2))
                        .map_err(failure)?,
                ),
                y: i32::from(position.y),
                ..art.rect
            };
            draws.push((60, art.draw(rect, false)));
        }
        for child in cabinet.linked_children() {
            let Some(pose) = child.pose() else {
                continue;
            };
            let (art, elevation) = match pose {
                ChildPose::Emerging => (self.emerging, 10),
                ChildPose::Holding => (self.holding, 2),
                ChildPose::Active => (self.active, 0),
                ChildPose::Hurt => (self.hurt, 0),
            };
            let enemy = child.enemy();
            let rect =
                ground_position(art, view.project(enemy.position), elevation)?;
            draws.push((
                enemy.position.y,
                art.draw(rect, enemy.facing == Facing::Right),
            ));
        }
        Ok(())
    }
}

// Observe distinct, visible poses during the routed campaign. Omit only
// linked art from the identical state to establish an actual pixel effect.
pub(super) fn observe(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    samples: &mut u8,
) -> Result<(), RenderError> {
    if art.poses.linked.is_none() {
        return Ok(());
    }
    let view = art.presentation_view(scene, cabinet)?;
    let sample = sample_poses(cabinet, view);
    if sample & *samples != sample {
        verify_visibility(scene, art, cabinet, true)?;
        *samples |= sample;
    }
    Ok(())
}

fn sample_poses(cabinet: &Cabinet, view: world::View) -> u8 {
    let visible = |anchor: i16| {
        let projected = anchor.saturating_sub(view.origin);
        projected >= 32
            && u32::try_from(projected)
                .is_ok_and(|column| column <= view.width.saturating_sub(32))
    };
    let parents = cabinet
        .linked_parents()
        .filter(|parent| visible(parent.x()))
        .filter_map(crate::LinkedParent::pose)
        .fold(0u8, |mask, pose| {
            mask | match pose {
                ParentPose::Closed => 1,
                ParentPose::Intermediate => 2,
                ParentPose::Open => 4,
            }
        });
    cabinet
        .linked_children()
        .filter(|child| visible(child.enemy().position.x))
        .filter_map(crate::LinkedChild::pose)
        .fold(parents, |mask, pose| {
            mask | match pose {
                ChildPose::Emerging => 8,
                ChildPose::Holding => 16,
                ChildPose::Active => 32,
                ChildPose::Hurt => 64,
            }
        })
}

pub(super) fn verify_route(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    samples: u8,
) -> Result<(), RenderError> {
    if art.poses.linked.is_none() {
        return Ok(());
    }
    if samples != 127
        || cabinet.linked_children().count() != crate::LINKED_CHILD_CAPACITY
        || cabinet
            .linked_children()
            .any(|child| child.enemy().health != 0 || child.pose().is_some())
        || cabinet
            .linked_parents()
            .any(|parent| parent.pose().is_some())
    {
        return Err(failure("linked route milestones were incomplete"));
    }
    verify_visibility(scene, art, cabinet, false)?;
    write_status(format_args!(
        "Linked actors: seven pose families and seven routed defeats verified"
    ))
    .map_err(failure)
}

fn verify_visibility(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    let _drawn = art.present(scene, cabinet)?;
    let pixels = scene.readback()?;
    let saved = art.poses.linked.take();
    let omitted = art.present(scene, cabinet);
    art.poses.linked = saved;
    let _omitted = omitted?;
    if (pixels != scene.readback()?) != visible {
        return Err(failure("linked actor art visibility differs from state"));
    }
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

pub(super) fn parse(
    line: &str,
    selected: bool,
) -> Result<[ContentHash; POSE_COUNT], RenderError> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("linked-actors") {
        return Err(failure("invalid linked-actors record"));
    }
    if selected {
        return Err(failure("duplicate linked-actors record"));
    }
    let mut read = || {
        fields
            .next()
            .ok_or_else(|| failure("missing linked actor pose"))?
            .parse::<ContentHash>()
            .map_err(failure)
    };
    let poses = [
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
    ];
    if fields.next().is_some() {
        return Err(failure("extra linked actor pose"));
    }
    Ok(poses)
}
