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
    Cabinet, CemeteryEntrance, CemeteryPhase, CemeteryPose, CemeteryRoomBody,
    Facing, Player, Point, WalkMotion,
};

pub(super) const ROLE_COUNT: usize = 16;
const LANDING_POSE_COUNT: usize = CemeteryEntrance::LANDING_POSE_COUNT;
const EXTRA_LANDING_COUNT: usize = 4 * (LANDING_POSE_COUNT - 1);
const ANIMATED_ROLE_COUNT: usize = ROLE_COUNT + EXTRA_LANDING_COUNT;
const WALKING_ROLE_COUNT: usize = ANIMATED_ROLE_COUNT + 60;
pub(super) const ATLAS_BYTES: usize = 1_678_356;
const ANIMATED_ATLAS_BYTES: usize = ATLAS_BYTES + 210_300;
const WALKING_ATLAS_BYTES: usize = ANIMATED_ATLAS_BYTES + 452_452;
// One background or card, plus up to four bodies.
pub(super) const EXTRA_SPRITES: usize = 5;

const CARD: Role = Role::new("scene/background", (320, 224), (0i32, 0i32));
const BACKGROUND: Role =
    Role::new("scene/background", (320, 196), (0i32, 28i32));

// Offsets register the verified crop origin against the original station
// ground coordinate and the preceding fixed-point elevation sample.
const ROLES: [Role; WALKING_ROLE_COUNT] = [
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
    // Registered room walking: front then away, in cabinet order.
    Role::new("actor/room-marge", (36, 75), (-81i32, -51i32)),
    Role::new("actor/room-marge", (38, 74), (-82i32, -50i32)),
    Role::new("actor/room-marge", (39, 72), (-82i32, -48i32)),
    Role::new("actor/room-marge", (35, 76), (-80i32, -52i32)),
    Role::new("actor/room-marge", (36, 75), (-81i32, -51i32)),
    Role::new("actor/room-marge", (34, 74), (-82i32, -50i32)),
    Role::new("actor/room-marge", (32, 72), (-82i32, -48i32)),
    Role::new("actor/room-marge", (32, 76), (-80i32, -52i32)),
    Role::new("actor/room-marge", (22, 76), (-72i32, -52i32)),
    Role::new("actor/room-marge", (22, 75), (-72i32, -51i32)),
    Role::new("actor/room-marge", (27, 74), (-75i32, -50i32)),
    Role::new("actor/room-marge", (26, 76), (-74i32, -52i32)),
    Role::new("actor/room-marge", (23, 74), (-72i32, -50i32)),
    Role::new("actor/room-marge", (23, 74), (-72i32, -50i32)),
    Role::new("actor/room-marge", (24, 72), (-72i32, -48i32)),
    Role::new("actor/room-marge", (22, 75), (-72i32, -51i32)),
    Role::new("actor/room-homer", (31, 63), (-79i32, -39i32)),
    Role::new("actor/room-homer", (36, 63), (-84i32, -39i32)),
    Role::new("actor/room-homer", (30, 63), (-78i32, -40i32)),
    Role::new("actor/room-homer", (25, 64), (-76i32, -41i32)),
    Role::new("actor/room-homer", (26, 63), (-77i32, -40i32)),
    Role::new("actor/room-homer", (35, 63), (-84i32, -39i32)),
    Role::new("actor/room-homer", (24, 64), (-76i32, -40i32)),
    Role::new("actor/room-homer", (26, 64), (-76i32, -40i32)),
    Role::new("actor/room-homer", (30, 63), (-77i32, -39i32)),
    Role::new("actor/room-homer", (29, 62), (-77i32, -39i32)),
    Role::new("actor/room-homer", (25, 63), (-76i32, -38i32)),
    Role::new("actor/room-homer", (26, 64), (-77i32, -38i32)),
    Role::new("actor/room-homer", (33, 63), (-79i32, -39i32)),
    Role::new("actor/room-homer", (27, 62), (-77i32, -39i32)),
    Role::new("actor/room-homer", (27, 63), (-76i32, -38i32)),
    Role::new("actor/room-homer", (29, 64), (-77i32, -38i32)),
    Role::new("actor/room-bart", (37, 52), (-84i32, -29i32)),
    Role::new("actor/room-bart", (38, 51), (-84i32, -28i32)),
    Role::new("actor/room-bart", (39, 52), (-85i32, -28i32)),
    Role::new("actor/room-bart", (46, 50), (-87i32, -26i32)),
    Role::new("actor/room-bart", (47, 51), (-85i32, -28i32)),
    Role::new("actor/room-bart", (52, 51), (-85i32, -28i32)),
    Role::new("actor/room-bart", (30, 54), (-79i32, -31i32)),
    Role::new("actor/room-bart", (28, 53), (-79i32, -29i32)),
    Role::new("actor/room-bart", (30, 53), (-79i32, -29i32)),
    Role::new("actor/room-bart", (32, 52), (-79i32, -28i32)),
    Role::new("actor/room-bart", (33, 51), (-79i32, -28i32)),
    Role::new("actor/room-bart", (34, 54), (-79i32, -30i32)),
    Role::new("actor/room-lisa", (37, 47), (-89i32, -23i32)),
    Role::new("actor/room-lisa", (37, 48), (-89i32, -24i32)),
    Role::new("actor/room-lisa", (35, 48), (-87i32, -24i32)),
    Role::new("actor/room-lisa", (36, 47), (-88i32, -23i32)),
    Role::new("actor/room-lisa", (36, 47), (-88i32, -23i32)),
    Role::new("actor/room-lisa", (37, 48), (-89i32, -24i32)),
    Role::new("actor/room-lisa", (36, 48), (-88i32, -24i32)),
    Role::new("actor/room-lisa", (38, 47), (-90i32, -23i32)),
    Role::new("actor/room-lisa", (32, 49), (-77i32, -22i32)),
    Role::new("actor/room-lisa", (31, 50), (-77i32, -23i32)),
    Role::new("actor/room-lisa", (31, 50), (-77i32, -23i32)),
    Role::new("actor/room-lisa", (31, 49), (-77i32, -22i32)),
    Role::new("actor/room-lisa", (30, 49), (-77i32, -22i32)),
    Role::new("actor/room-lisa", (30, 50), (-77i32, -23i32)),
    Role::new("actor/room-lisa", (30, 50), (-77i32, -23i32)),
    Role::new("actor/room-lisa", (30, 49), (-77i32, -22i32)),
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

    fn room_offset(self, point: Point, flip: bool) -> (i32, i32) {
        // Room records use inverted depth. The horizontal reflection
        // pivot is 64 pixels to the left of the station record.
        let dx = if flip {
            (-128i32).saturating_sub(self.offset.0).saturating_sub(
                i32::try_from(self.extent.0).unwrap_or_default(),
            )
        } else {
            self.offset.0
        };
        (
            i32::from(point.x).saturating_add(dx),
            352i32
                .saturating_sub(i32::from(point.y))
                .saturating_add(self.offset.1),
        )
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
        self.count() >= ANIMATED_ROLE_COUNT
    }

    pub(super) const fn atlas_bytes(&self) -> usize {
        if self.count() == WALKING_ROLE_COUNT {
            WALKING_ATLAS_BYTES
        } else if self.count() >= ANIMATED_ROLE_COUNT {
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
            .take(WALKING_ROLE_COUNT.saturating_add(1))
            .map(|field| field.parse::<ContentHash>().map_err(failure))
            .collect::<Result<Vec<_>, _>>()?;
        if !matches!(
            hashes.len(),
            ROLE_COUNT | ANIMATED_ROLE_COUNT | WALKING_ROLE_COUNT
        ) {
            return Err(failure(
                "expected sixteen, forty-four or 104 cemetery roles",
            ));
        }
        if hashes.len() == WALKING_ROLE_COUNT && first != Player::Marge {
            return Err(failure("room walking requires the Marge-first rank"));
        }
        let selection = Self { first, hashes };
        if !selection.distinct_roles() {
            return Err(failure("duplicate cemetery art role"));
        }
        Ok(selection)
    }

    pub(super) const fn room_walking(&self) -> bool {
        self.count() == WALKING_ROLE_COUNT
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
        self.roles.len() >= ANIMATED_ROLE_COUNT
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
        if self.room_walking()
            && let Some(visible) = entrance.room_body_visibility()
        {
            if !visible {
                return None;
            }
            let body = entrance.room()?.presentation_bodies();
            return self.room_draw(player, *body.get(player.index())?);
        }
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

    fn room_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
    ) -> Option<SpriteDraw> {
        let (index, flip) = room_role(player, body)?;
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.room_offset(body.position, flip);
        Some(art.draw(destination, flip))
    }

    const fn room_walking(&self) -> bool {
        self.roles.len() == WALKING_ROLE_COUNT
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
        self.verify_room_walk(scene)
    }

    fn verify_room_body(
        &self,
        scene: &mut SceneSession<'_, '_>,
        sample: (Player, Option<WalkMotion>),
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        let (player, walking) = sample;
        let mut body = CemeteryRoomBody {
            action: false,
            facing: Facing::Right,
            position: Point { x: 224, y: 176 },
            walking,
        };
        let right = self
            .room_draw(player, body)
            .ok_or(RenderError::InvalidSprite)?;
        let _presented = scene.present([24, 22, 30], &[right])?;
        let right_pixels = scene.readback()?;
        scenery::verify_pixels(&right_pixels, baseline, world::BASE_WIDTH, &[
            right.destination,
        ])?;
        body.facing = Facing::Left;
        let left = self
            .room_draw(player, body)
            .ok_or(RenderError::InvalidSprite)?;
        let _reflected = scene.present([24, 22, 30], &[left])?;
        let left_pixels = scene.readback()?;
        if !reflected_room_pixels(&right_pixels, &left_pixels) {
            return Err(failure("reflected cemetery body pixels differ"));
        }
        Ok(())
    }

    fn verify_room_walk(
        &self,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<(), RenderError> {
        if !self.room_walking() {
            return Ok(());
        }
        let _presented = scene.present([24, 22, 30], &[])?;
        let baseline = scene.readback()?;
        for player in Player::ALL {
            self.verify_room_body(scene, (player, None), &baseline)?;
            let (_, keys) = room_walk_track(player);
            for (away, &key) in [false, true]
                .into_iter()
                .flat_map(|away| keys.iter().map(move |key| (away, key)))
            {
                let walking = WalkMotion {
                    away,
                    tick: u64::from(key),
                };
                self.verify_room_body(
                    scene,
                    (player, Some(walking)),
                    &baseline,
                )?;
            }
        }
        write_status(format_args!("Cemetery walking: 64 reflected body pairs"))
            .map_err(failure)
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

// A body centered on record x=224 reflects around the 320-pixel viewport.
fn reflected_room_pixels(right: &[u8], left: &[u8]) -> bool {
    const PITCH: usize = 320 * 4;
    right.len() == left.len()
        && right.len().checked_rem(PITCH) == Some(0)
        && right
            .as_chunks::<PITCH>()
            .0
            .iter()
            .zip(left.as_chunks::<PITCH>().0)
            .all(|(right_row, left_row)| {
                right_row.as_chunks::<4>().0.iter().eq(left_row
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .rev())
            })
}

// The isolated room controls independently reproduce the street walking
// periods and key boundaries for all four stations.
const fn room_walk_track(player: Player) -> (usize, &'static [u8]) {
    match player {
        Player::Marge => (44, &[0, 5, 10, 16, 21, 26, 31, 37]),
        Player::Homer => (60, &[0, 5, 10, 15, 20, 25, 30, 35]),
        Player::Bart => (76, &[0, 4, 8, 12, 17, 21]),
        Player::Lisa => (88, &[0, 5, 10, 15, 20, 25, 30, 35]),
    }
}

fn room_role(player: Player, body: CemeteryRoomBody) -> Option<(usize, bool)> {
    if body.action {
        return None;
    }
    if let Some(walking) = body.walking {
        let (start, keys) = room_walk_track(player);
        let tick = super::poses::walking_offset(player, walking.tick)?;
        let ordinal =
            keys.partition_point(|key| *key <= tick).checked_sub(1)?;
        let track = start.saturating_add(
            usize::from(walking.away).saturating_mul(keys.len()),
        );
        let canonical_right =
            matches!(player, Player::Marge | Player::Lisa) != walking.away;
        return Some((
            track.saturating_add(ordinal),
            canonical_right != (body.facing == Facing::Right),
        ));
    }
    let index = ANIMATED_ROLE_COUNT.saturating_sub(1).saturating_sub(
        (3usize.saturating_sub(player.index()))
            .saturating_mul(LANDING_POSE_COUNT.saturating_sub(1)),
    );
    Some((index, body.facing == Facing::Left))
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
            (WALKING_ROLE_COUNT, WALKING_ATLAS_BYTES),
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

    #[test]
    fn room_idle_origins_follow_delayed_depth_and_reflection() {
        for (player, (point, expected)) in Player::ALL.into_iter().zip([
            (Point { x: 116, y: 176 }, (38i32, 126i32)),
            (Point { x: 144, y: 176 }, (68i32, 136i32)),
            (Point { x: 172, y: 176 }, (91i32, 141i32)),
            (Point { x: 200, y: 176 }, (109i32, 153i32)),
        ]) {
            let mut body = CemeteryRoomBody {
                action: false,
                facing: Facing::Right,
                position: point,
                walking: None,
            };
            let actual = room_role(player, body).and_then(|(index, flip)| {
                ROLES.get(index).map(|role| role.room_offset(point, flip))
            });
            assert_eq!(actual, Some(expected));
            body.position.y = point.y.saturating_add(31);
            let shifted = room_role(player, body).and_then(|(index, flip)| {
                ROLES
                    .get(index)
                    .map(|role| role.room_offset(body.position, flip))
            });
            assert_eq!(
                shifted,
                Some((expected.0, expected.1.saturating_sub(31)))
            );
            body.action = true;
            assert_eq!(room_role(player, body), None);
        }
    }

    fn assert_room_reflection(
        player: Player,
        walking: Option<WalkMotion>,
        index: usize,
    ) {
        let mut body = CemeteryRoomBody {
            action: false,
            facing: Facing::Right,
            position: Point { x: 224, y: 176 },
            walking,
        };
        let right = room_role(player, body);
        body.facing = Facing::Left;
        let left = room_role(player, body);
        assert_eq!(right.map(|(role, _)| role), Some(index));
        assert_eq!(left.map(|(role, _)| role), Some(index));
        if let (Some((_, right_flip)), Some((_, left_flip)), Some(role)) =
            (right, left, ROLES.get(index))
        {
            assert_ne!(left_flip, right_flip);
            let (rx, ry) = role.room_offset(body.position, right_flip);
            let (lx, ly) = role.room_offset(body.position, left_flip);
            let width = i32::try_from(role.extent.0).unwrap_or_default();
            assert_eq!(rx.saturating_add(lx).saturating_add(width), 320i32);
            assert_eq!(ry, ly);
        }
    }

    fn assert_room_track(
        player: Player,
        first: usize,
        expected: &[usize],
        frames: usize,
    ) {
        for away in [false, true] {
            for (tick, ordinal) in expected
                .iter()
                .copied()
                .cycle()
                .take(expected.len().saturating_mul(2))
                .enumerate()
            {
                let index = first
                    .saturating_add(ordinal)
                    .saturating_add(usize::from(away).saturating_mul(frames));
                assert_room_reflection(
                    player,
                    Some(WalkMotion {
                        away,
                        tick: u64::try_from(tick).unwrap_or_default(),
                    }),
                    index,
                );
            }
        }
    }

    #[test]
    fn room_walk_tracks_match_the_verified_periods_and_mirror_anchors() {
        let marge = [
            0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4,
            4, 4, 4, 4, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7,
        ];
        let bart = [
            0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5,
            5, 5, 5,
        ];
        let even = [
            0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4,
            4, 4, 4, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7,
        ];
        assert_room_track(Player::Marge, 44, &marge, 8);
        assert_room_track(Player::Homer, 60, &even, 8);
        assert_room_track(Player::Bart, 76, &bart, 6);
        assert_room_track(Player::Lisa, 88, &even, 8);
        for (player, index) in Player::ALL.into_iter().zip([22, 29, 36, 43]) {
            assert_room_reflection(player, None, index);
        }
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
                (38i32, 126i32),
            ],
            [
                (65, 137),
                (73, 169),
                (75, 161),
                (72, 176),
                (76, 169),
                (79, 164),
                (76, 150),
                (68i32, 136i32),
            ],
            [
                (90, 150),
                (89, 167),
                (89, 165),
                (89, 176),
                (91, 168),
                (91, 152),
                (91, 145),
                (91i32, 141i32),
            ],
            [
                (114, 162),
                (140, 164),
                (138, 170),
                (133, 177),
                (130, 173),
                (135, 168),
                (134, 161),
                (109i32, 153i32),
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
