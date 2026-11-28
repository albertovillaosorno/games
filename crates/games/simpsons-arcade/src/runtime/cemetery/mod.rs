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
//   - Select optional verified grounded poses through the entrance boundary.
// - Usage:
//   - Select all roles and an explicit verified rotation in the catalogue.
// - Defaults:
//   - No cemetery presentation or campaign transition.

//! Optional registered third-stage entrance art.

pub(super) mod layers;

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::BundleSource;

use super::{Art, Scene, failure, open_bundle, scenery, world};
use crate::{
    Cabinet, CemeteryEntrance, CemeteryPhase, CemeteryPose, Player, Point,
};

pub(super) const ROLE_COUNT: usize = 16;
const LANDING_POSE_COUNT: usize = CemeteryEntrance::LANDING_POSE_COUNT;
const EXTRA_LANDING_COUNT: usize = 4 * (LANDING_POSE_COUNT - 1);
const ANIMATED_ROLE_COUNT: usize = ROLE_COUNT + EXTRA_LANDING_COUNT;
pub(super) const ATLAS_BYTES: usize = 1_678_356;
const ANIMATED_ATLAS_BYTES: usize = ATLAS_BYTES + 210_300;
// One background or card, plus up to four bodies.
pub(super) const EXTRA_SPRITES: usize = 5;

const CARD: Role = Role::new("scene/background", (320, 224), (0i32, 0i32));
const BACKGROUND: Role =
    Role::new("scene/background", (320, 196), (0i32, 28i32));

// Offsets register the verified crop origin against the original station
// ground coordinate and the preceding fixed-point elevation sample.
const ROLES: [Role; ANIMATED_ROLE_COUNT] = [
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
    // Grounded continuation, seven further families per station.
    Role::new("actor/land-marge", (56, 48), (-66i32, -24i32)),
    Role::new("actor/land-marge", (54, 42), (-62i32, -27i32)),
    Role::new("actor/land-marge", (84, 24), (-64i32, 0i32)),
    Role::new("actor/land-marge", (74, 23), (-73i32, 0i32)),
    Role::new("actor/land-marge", (71, 39), (-78i32, -15i32)),
    Role::new("actor/land-marge", (43, 58), (-77i32, -34i32)),
    Role::new("actor/land-marge", (36, 74), (-78i32, -50i32)),
    Role::new("actor/land-homer", (63, 31), (-71i32, -7i32)),
    Role::new("actor/land-homer", (60, 31), (-69i32, -15i32)),
    Role::new("actor/land-homer", (64, 24), (-72i32, 0i32)),
    Role::new("actor/land-homer", (61, 31), (-68i32, -7i32)),
    Role::new("actor/land-homer", (55, 35), (-65i32, -12i32)),
    Role::new("actor/land-homer", (43, 49), (-68i32, -26i32)),
    Role::new("actor/land-homer", (25, 64), (-76i32, -40i32)),
    Role::new("actor/land-bart", (52, 32), (-83i32, -9i32)),
    Role::new("actor/land-bart", (54, 35), (-83i32, -11i32)),
    Role::new("actor/land-bart", (55, 23), (-83i32, 0i32)),
    Role::new("actor/land-bart", (50, 31), (-81i32, -8i32)),
    Role::new("actor/land-bart", (47, 47), (-81i32, -24i32)),
    Role::new("actor/land-bart", (37, 54), (-81i32, -31i32)),
    Role::new("actor/land-bart", (37, 58), (-81i32, -35i32)),
    Role::new("actor/land-lisa", (46, 36), (-60i32, -12i32)),
    Role::new("actor/land-lisa", (48, 30), (-62i32, -6i32)),
    Role::new("actor/land-lisa", (53, 23), (-67i32, 1i32)),
    Role::new("actor/land-lisa", (48, 27), (-70i32, -3i32)),
    Role::new("actor/land-lisa", (43, 32), (-65i32, -8i32)),
    Role::new("actor/land-lisa", (40, 39), (-66i32, -15i32)),
    Role::new("actor/land-lisa", (39, 47), (-91i32, -23i32)),
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

    fn valid_pixels(self, bytes: &[u8]) -> bool {
        let (pixels, remainder) = bytes.as_chunks::<4>();
        if pixels.is_empty() || !remainder.is_empty() {
            return false;
        }
        if self.key == "scene/background" {
            pixels.iter().all(|pixel| pixel.last() == Some(&255))
        } else {
            pixels.iter().any(|pixel| pixel.last() != Some(&0))
        }
    }
}

#[derive(Debug)]
pub(super) struct Selection {
    first: Player,
    hashes: Vec<ContentHash>,
}

impl Selection {
    pub(super) const fn animated(&self) -> bool {
        self.count() == ANIMATED_ROLE_COUNT
    }

    pub(super) const fn atlas_bytes(&self) -> usize {
        if self.count() == ANIMATED_ROLE_COUNT {
            ANIMATED_ATLAS_BYTES
        } else {
            ATLAS_BYTES
        }
    }

    pub(super) const fn count(&self) -> usize {
        self.hashes.len()
    }

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
            .take(ANIMATED_ROLE_COUNT.saturating_add(1))
            .map(|field| field.parse::<ContentHash>().map_err(failure))
            .collect::<Result<Vec<_>, _>>()?;
        if !matches!(hashes.len(), ROLE_COUNT | ANIMATED_ROLE_COUNT) {
            return Err(failure(
                "expected sixteen or forty-four cemetery roles",
            ));
        }
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
    layers: Option<layers::Layers>,
    roles: Vec<RoleArt>,
}

impl CemeteryArt {
    pub(super) const fn animated(&self) -> bool {
        self.roles.len() == ANIMATED_ROLE_COUNT
    }

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
        let animated = self.animated();
        let (index, point, elevation) =
            body_sample(entrance, player, animated)?;
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
        layer_hashes: Option<&[ContentHash; layers::PHASES]>,
    ) -> Result<Self, RenderError> {
        let roles = selection
            .digests()
            .zip(ROLES)
            .map(|(hash, role)| {
                let bundle = open_bundle(store, hash)?;
                let (art, pixels) =
                    Art::load_with_pixels(scene, &bundle, role.key)?;
                if (art.rect.width.get(), art.rect.height.get()) != role.extent
                    || !role.valid_pixels(&pixels)
                {
                    return Err(failure(
                        "invalid cemetery art extent or visibility",
                    ));
                }
                let digest = bundle
                    .manifest()
                    .record(role.key)
                    .ok_or(RenderError::InvalidSprite)?
                    .digest;
                Ok(RoleArt { art, digest })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            first: selection.first,
            layers: layer_hashes
                .map(|hashes| layers::Layers::load(store, scene, *hashes))
                .transpose()?,
            roles,
        })
    }

    pub(super) fn queue(
        &self,
        entrance: CemeteryEntrance,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<usize, RenderError> {
        let start = draws.len();
        if let (Some(layers), Some(phase)) =
            (&self.layers, entrance.background_phase())
            && entrance.phase() == CemeteryPhase::Landed
        {
            let offset = entrance
                .room()
                .map_or(0, crate::CemeteryRoom::presentation_depth_offset);
            layers.queue(phase, offset, draws)?;
        } else {
            draws.extend(self.background(entrance));
        }
        let background_count = draws.len().saturating_sub(start);
        draws.extend(
            Player::ALL
                .into_iter()
                .filter_map(|player| self.body(entrance, player)),
        );
        Ok(background_count)
    }

    fn role(&self, index: usize) -> Option<(Art, Role)> {
        Some((self.roles.get(index)?.art, *ROLES.get(index)?))
    }

    pub(super) fn verify_layer_phases(
        &self,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<(), RenderError> {
        let Some(layers) = &self.layers else {
            return Ok(());
        };
        for phase in 0u8..4 {
            let expected = self
                .roles
                .get(usize::from(phase).saturating_add(2))
                .ok_or(RenderError::InvalidSprite)?
                .digest;
            verify_layer_offsets(scene, layers, phase, expected)?;
        }
        Ok(())
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
    animated: bool,
) -> Option<BodySample> {
    if entrance.room_body_visibility() == Some(false) {
        return None;
    }
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
    let pose = entrance.landing_pose(player)?;
    let index = if animated && pose > 0 {
        ROLE_COUNT
            .saturating_add(
                player
                    .index()
                    .saturating_mul(LANDING_POSE_COUNT.saturating_sub(1)),
            )
            .saturating_add(pose.saturating_sub(1))
    } else {
        player.index().saturating_add(12)
    };
    Some((index, entrance.position(player)?, 0))
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
    let background_count = cemetery.queue(entrance, &mut expected)?;
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
        let backgrounds = expected
            .get(..background_count)
            .ok_or(RenderError::InvalidSprite)?;
        verify_background(scene, backgrounds, *draw, digest)?;
        if expected.len() > background_count {
            let baseline = scene.readback()?;
            let _composed = scene.present([24, 22, 30], &expected)?;
            let bounds: Vec<_> = expected
                .iter()
                .skip(background_count)
                .map(|body_draw| body_draw.destination)
                .collect();
            scenery::verify_pixels(
                &scene.readback()?,
                &baseline,
                world::BASE_WIDTH,
                &bounds,
            )?;
        }
        let _restored = art.present(scene, cabinet)?;
    }
    Ok(())
}

fn background_digest(
    scene: &mut SceneSession<'_, '_>,
    draws: &[SpriteDraw],
    draw: SpriteDraw,
) -> Result<ContentHash, RenderError> {
    let pitch = scene.layout().pitch();
    let offset = pitch
        .saturating_mul(usize::try_from(draw.destination.y).map_err(failure)?);
    let bytes = pitch.saturating_mul(
        usize::try_from(draw.destination.height.get()).map_err(failure)?,
    );
    let _isolated = scene.present([24, 22, 30], draws)?;
    let pixels = scene.readback()?;
    let region = pixels
        .get(offset..offset.saturating_add(bytes))
        .ok_or(RenderError::InvalidLength)?;
    Ok(ContentHash::calculate(region))
}

fn verify_background(
    scene: &mut SceneSession<'_, '_>,
    draws: &[SpriteDraw],
    draw: SpriteDraw,
    digest: ContentHash,
) -> Result<(), RenderError> {
    if background_digest(scene, draws, draw)? != digest {
        return Err(failure("isolated cemetery background pixels differ"));
    }
    Ok(())
}

fn verify_layer_offsets(
    scene: &mut SceneSession<'_, '_>,
    layers: &layers::Layers,
    phase: u8,
    expected: ContentHash,
) -> Result<(), RenderError> {
    let mut draws = Vec::new();
    for offset in 0i16..=31 {
        draws.clear();
        layers.queue(phase, offset, &mut draws)?;
        let draw = *draws.first().ok_or(RenderError::InvalidSprite)?;
        let digest = background_digest(scene, &draws, draw)?;
        if offset == 0 && digest != expected {
            return Err(failure("cemetery layer phase differs from entrance"));
        }
        write_status(format_args!("Cemetery layer: {phase} {offset} {digest}"))
            .map_err(failure)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    type LandingOrigins = [(i32, i32); LANDING_POSE_COUNT];

    #[test]
    fn atlas_budget_matches_the_exact_roles() {
        for (count, budget) in [
            (ROLE_COUNT, ATLAS_BYTES),
            (ANIMATED_ROLE_COUNT, ANIMATED_ATLAS_BYTES),
        ] {
            let bytes = ROLES.iter().take(count).fold(0u32, |sum, role| {
                sum.saturating_add(
                    role.extent
                        .0
                        .saturating_mul(role.extent.1)
                        .saturating_mul(4),
                )
            });
            assert_eq!(usize::try_from(bytes).ok(), Some(budget));
        }
    }

    #[test]
    fn cemetery_roles_require_visible_bodies_and_opaque_backdrops() {
        let transparent = [90, 30, 20, 0];
        let opaque = [90, 30, 20, 255];
        let mixed = [90, 30, 20, 0, 90, 30, 20, 255];
        let body = Role::new("actor/fall-marge", (2, 1), (0i32, 0i32));
        assert!(BACKGROUND.valid_pixels(&opaque));
        assert!(!BACKGROUND.valid_pixels(&transparent));
        assert!(!BACKGROUND.valid_pixels(&mixed));
        assert!(body.valid_pixels(&mixed));
        assert!(!body.valid_pixels(&transparent));
        assert!(!body.valid_pixels(&[]));
        assert!(!body.valid_pixels(&[90, 30, 20]));
    }

    fn assert_animated_landing(player: Player, expected: LandingOrigins) {
        let mut state = CemeteryEntrance::new([true; 4], player);
        for _ in 0u64..381 {
            state.advance(false);
        }
        let mut seen = [false; LANDING_POSE_COUNT];
        for _ in 0u64..49 {
            let pose = state.landing_pose(player).unwrap_or(usize::MAX);
            assert!(pose < LANDING_POSE_COUNT);
            let offset = body_sample(state, player, true).and_then(
                |(index, point, elevation)| {
                    ROLES
                        .get(index)
                        .map(|role| role.body_offset(point, elevation))
                },
            );
            assert_eq!(offset, expected.get(pose).copied());
            if let Some(value) = seen.get_mut(pose) {
                *value = true;
            }
            let legacy =
                body_sample(state, player, false).map(|(index, _, _)| index);
            assert_eq!(legacy, Some(player.index().saturating_add(12)));
            state.advance(false);
        }
        assert!(seen.into_iter().all(|value| value));
    }

    #[test]
    fn animated_roles_register_all_ground_families() {
        let origins: [LandingOrigins; 4] = [
            [
                (44, 138),
                (50, 152),
                (54, 149),
                (52, 176),
                (43, 176),
                (38, 161),
                (39, 142),
                (38, 126),
            ],
            [
                (65, 137),
                (73, 169),
                (75, 161),
                (72, 176),
                (76, 169),
                (79, 164),
                (76, 150),
                (68, 136),
            ],
            [
                (90, 150),
                (89, 167),
                (89, 165),
                (89, 176),
                (91, 168),
                (91, 152),
                (91, 145),
                (91, 141),
            ],
            [
                (114, 162),
                (140, 164),
                (138, 170),
                (133, 177),
                (130, 173),
                (135, 168),
                (134, 161),
                (109, 153),
            ],
        ];
        for (player, expected) in Player::ALL.into_iter().zip(origins) {
            assert_animated_landing(player, expected);
        }
    }

    #[test]
    fn protected_room_omits_bodies_only_on_the_observed_hidden_sample() {
        let mut entrance = CemeteryEntrance::new([true; 4], Player::Marge);
        for _ in 0u64..65 {
            entrance.advance(true);
        }
        for _ in 0u64..240 {
            entrance.advance(false);
        }
        assert!(body_sample(entrance, Player::Marge, true).is_some());
        entrance.enable_room();
        for player in Player::ALL {
            assert_eq!(body_sample(entrance, player, true), None);
        }
        entrance.advance(false);
        for player in Player::ALL {
            assert!(body_sample(entrance, player, true).is_some());
        }
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
            let actual = body_sample(entrance, player, false).and_then(
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
