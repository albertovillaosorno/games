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
//   - Local cemetery card, background and registered body pose admission.
// - Must-Not:
//   - Decode original media, advance simulation or infer original RNG.
// - Allows:
//   - Inputs: Canonical pose roles and the admitted entrance state.
//   - Outputs: Original-width background and registered body commands.
//   - Side effects: Bounded atlas uploads at scene initialization.
// - Split-When:
//   - Cemetery traversal requires separate world presentation.
// - Merge-When:
//   - Shared registered animation supports all these roles.
// - Summary:
//   - Present the optional cemetery arrival with sparse local artwork.
// - Description:
//   - Hold the first isolated landing pose; further animation is open.
// - Usage:
//   - Select all roles and an explicit verified rotation in the catalogue.
// - Defaults:
//   - No cemetery presentation or campaign transition.

//! Optional registered third-stage entrance art.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::BundleSource;

use super::{Art, Scene, failure, open_bundle, world};
use crate::{
    Cabinet, CemeteryEntrance, CemeteryPhase, CemeteryPose, Player, Point,
};

pub(super) const ROLE_COUNT: usize = 16;
pub(super) const ATLAS_BYTES: usize = 1_678_356;
// One background or card, plus up to four bodies.
pub(super) const EXTRA_SPRITES: usize = 5;

const CARD: Role = Role::new("scene/background", (320, 224), (0i32, 0i32));
const BACKGROUND: Role =
    Role::new("scene/background", (320, 196), (0i32, 28i32));

// Offsets register the verified crop origin against the original station
// ground coordinate and the preceding fixed-point elevation sample.
const ROLES: [Role; ROLE_COUNT] = [
    CARD,
    CARD,
    BACKGROUND,
    BACKGROUND,
    BACKGROUND,
    BACKGROUND,
    Role::new("actor/fall-marge", (32, 90), (-80i32, -66i32)),
    Role::new("actor/fall-marge", (38, 85), (-86i32, -61i32)),
    Role::new("actor/fall-homer", (44, 66), (-92i32, -42i32)),
    Role::new("actor/fall-bart", (41, 63), (-85i32, -39i32)),
    Role::new("actor/fall-lisa", (39, 51), (-93i32, -29i32)),
    Role::new("actor/fall-lisa", (37, 50), (-90i32, -28i32)),
    Role::new("actor/land-marge", (48, 61), (-72i32, -38i32)),
    Role::new("actor/land-homer", (39, 62), (-79i32, -39i32)),
    Role::new("actor/land-bart", (49, 49), (-82i32, -26i32)),
    Role::new("actor/land-lisa", (57, 38), (-86i32, -14i32)),
];

type BodySample = (usize, Point, i16);

#[derive(Clone, Copy, Debug)]
struct RoleArt {
    art: Art,
    digest: ContentHash,
}

#[derive(Clone, Copy)]
struct Role {
    extent: (u32, u32),
    key: &'static str,
    offset: (i32, i32),
}

impl Role {
    fn body_offset(self, point: Point, elevation: i16) -> (i32, i32) {
        (
            i32::from(point.x).saturating_add(self.offset.0),
            i32::from(point.y)
                .saturating_add(self.offset.1)
                .saturating_sub(i32::from(elevation)),
        )
    }

    const fn new(
        key: &'static str,
        extent: (u32, u32),
        offset: (i32, i32),
    ) -> Self {
        Self { extent, key, offset }
    }
}

#[derive(Debug)]
pub(super) struct Selection {
    first: Player,
    hashes: [ContentHash; ROLE_COUNT],
}

impl Selection {
    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.hashes.iter().copied()
    }

    fn distinct_roles(&self) -> bool {
        self.hashes.iter().zip(ROLES).enumerate().all(
            |(index, (hash, role))| {
                self.hashes
                    .iter()
                    .zip(ROLES)
                    .skip(index.saturating_add(1))
                    .all(|(other_hash, other_role)| {
                        hash != other_hash || role.key != other_role.key
                    })
            },
        )
    }

    pub(super) fn parse(line: &str) -> Result<Self, RenderError> {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("cemetery-entrance") {
            return Err(failure("unexpected cemetery entrance record"));
        }
        let first = match fields.next() {
            Some("marge") => Player::Marge,
            Some("homer") => Player::Homer,
            Some("bart") => Player::Bart,
            Some("lisa") => Player::Lisa,
            _ => return Err(failure("missing cemetery rank rotation")),
        };
        let hashes = fields
            .map(|field| field.parse::<ContentHash>().map_err(failure))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_error| failure("expected sixteen cemetery roles"))?;
        let selection = Self { first, hashes };
        if !selection.distinct_roles() {
            return Err(failure("duplicate cemetery art role"));
        }
        Ok(selection)
    }
}

#[derive(Debug)]
pub(super) struct CemeteryArt {
    first: Player,
    roles: [RoleArt; ROLE_COUNT],
}

impl CemeteryArt {
    fn background(&self, entrance: CemeteryEntrance) -> Option<SpriteDraw> {
        let index = background_index(entrance)?;
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.offset;
        Some(art.draw(destination, false))
    }

    fn body(
        &self,
        entrance: CemeteryEntrance,
        player: Player,
    ) -> Option<SpriteDraw> {
        let (index, point, elevation) = body_sample(entrance, player)?;
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.body_offset(point, elevation);
        Some(art.draw(destination, false))
    }

    pub(super) const fn first(&self) -> Player {
        self.first
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        selection: &Selection,
    ) -> Result<Self, RenderError> {
        let roles = selection
            .digests()
            .zip(ROLES)
            .map(|(hash, role)| {
                let bundle = open_bundle(store, hash)?;
                let (art, pixels) =
                    Art::load_with_pixels(scene, &bundle, role.key)?;
                if (art.rect.width.get(), art.rect.height.get()) != role.extent
                    || (role.key == "scene/background"
                        && !pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|pixel| pixel.last() == Some(&255)))
                {
                    return Err(failure(
                        "invalid cemetery art extent or opacity",
                    ));
                }
                let digest = bundle
                    .manifest()
                    .record(role.key)
                    .ok_or(RenderError::InvalidSprite)?
                    .digest;
                Ok(RoleArt { art, digest })
            })
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_error| RenderError::InvalidSprite)?;
        Ok(Self {
            first: selection.first,
            roles,
        })
    }

    pub(super) fn queue(
        &self,
        entrance: CemeteryEntrance,
        draws: &mut Vec<SpriteDraw>,
    ) {
        draws.extend(self.background(entrance));
        draws.extend(
            Player::ALL
                .into_iter()
                .filter_map(|player| self.body(entrance, player)),
        );
    }

    fn role(&self, index: usize) -> Option<(Art, Role)> {
        Some((self.roles.get(index)?.art, *ROLES.get(index)?))
    }
}

fn background_index(entrance: CemeteryEntrance) -> Option<usize> {
    if entrance.phase() == CemeteryPhase::Card {
        Some(usize::from(entrance.tick() >= 5))
    } else {
        Some(usize::from(entrance.background_phase()?).saturating_add(2))
    }
}

fn body_sample(
    entrance: CemeteryEntrance,
    player: Player,
) -> Option<BodySample> {
    if let Some(body) = entrance.body(player) {
        let index = match body.pose {
            CemeteryPose::MargeA => 6,
            CemeteryPose::MargeB => 7,
            CemeteryPose::Homer => 8,
            CemeteryPose::Bart => 9,
            CemeteryPose::LisaA => 10,
            CemeteryPose::LisaB => 11,
        };
        return Some((index, body.position, body.elevation));
    }
    if entrance.height(player) == Some(0) {
        return Some((
            player.index().saturating_add(12),
            entrance.position(player)?,
            0,
        ));
    }
    None
}

pub(super) fn verify(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let entrance = cabinet
        .cemetery_entrance()
        .ok_or(RenderError::InvalidSprite)?;
    let cemetery = art
        .poses
        .cemetery
        .as_deref()
        .ok_or(RenderError::InvalidSprite)?;
    let mut expected = Vec::new();
    cemetery.queue(entrance, &mut expected);
    let background = background_index(entrance)
        .and_then(|index| cemetery.roles.get(index))
        .map(|role| role.digest);
    let _presented = art.present(scene, cabinet)?;
    if scene.layout().width().get() != world::BASE_WIDTH
        || !art.draws.starts_with(&expected)
        || (expected.is_empty() != cabinet.stage_scene_hidden())
    {
        return Err(failure("cemetery viewport or layering differs"));
    }
    if let (Some(draw), Some(digest)) = (expected.first(), background) {
        verify_background(scene, (*draw, digest))?;
        if expected.len() > 1 {
            let baseline = scene.readback()?;
            let _composed = scene.present([24, 22, 30], &expected)?;
            if baseline == scene.readback()? {
                return Err(failure("cemetery bodies did not change pixels"));
            }
        }
        let _restored = art.present(scene, cabinet)?;
    }
    Ok(())
}

fn verify_background(
    scene: &mut SceneSession<'_, '_>,
    (draw, digest): (SpriteDraw, ContentHash),
) -> Result<(), RenderError> {
    let pitch = scene.layout().pitch();
    let offset = pitch
        .saturating_mul(usize::try_from(draw.destination.y).map_err(failure)?);
    let bytes = pitch.saturating_mul(
        usize::try_from(draw.destination.height.get()).map_err(failure)?,
    );
    let _isolated = scene.present([24, 22, 30], &[draw])?;
    let pixels = scene.readback()?;
    let region = pixels
        .get(offset..offset.saturating_add(bytes))
        .ok_or(RenderError::InvalidLength)?;
    if ContentHash::calculate(region) != digest {
        return Err(failure("isolated cemetery background pixels differ"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_budget_matches_the_exact_roles() {
        let bytes = ROLES.iter().fold(0u32, |sum, role| {
            sum.saturating_add(
                role.extent
                    .0
                    .saturating_mul(role.extent.1)
                    .saturating_mul(4),
            )
        });
        assert_eq!(usize::try_from(bytes).ok(), Some(ATLAS_BYTES));
    }

    #[test]
    fn body_registration_matches_four_ground_samples() {
        let mut entrance = CemeteryEntrance::new([true; 4], Player::Homer);
        for _ in 0u64..65 {
            entrance.advance(true);
        }
        for _ in 0u64..237 {
            entrance.advance(false);
        }
        for (player, expected) in Player::ALL.into_iter().zip([
            (44i32, 138i32),
            (65i32, 137i32),
            (90i32, 150i32),
            (114i32, 162i32),
        ]) {
            let actual = body_sample(entrance, player).and_then(
                |(index, point, elevation)| {
                    ROLES
                        .get(index)
                        .map(|role| role.body_offset(point, elevation))
                },
            );
            assert_eq!(actual, Some(expected));
        }
    }
}
