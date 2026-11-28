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

mod air;
mod ground;
pub(super) mod layers;
mod special;

use std::cmp::Reverse;

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::BundleSource;

use super::{Art, Scene, failure, open_bundle, scenery, world};
use crate::{
    Cabinet, CemeteryEntrance, CemeteryGroundAttack, CemeteryPhase,
    CemeteryPose, CemeteryRoomBody, Facing, JumpMotion, Maneuver, Player,
    Point, WalkMotion,
};

pub(super) const ROLE_COUNT: usize = 16;
const LANDING_POSE_COUNT: usize = CemeteryEntrance::LANDING_POSE_COUNT;
const EXTRA_LANDING_COUNT: usize = 4 * (LANDING_POSE_COUNT - 1);
const ANIMATED_ROLE_COUNT: usize = ROLE_COUNT + EXTRA_LANDING_COUNT;
const WALKING_ROLE_COUNT: usize = ANIMATED_ROLE_COUNT + 60;
const MARKER_ROLE_COUNT: usize = WALKING_ROLE_COUNT + 4;
const SPECIAL_ROLE_COUNT: usize = MARKER_ROLE_COUNT + 12;
const GROUND_ROLE_COUNT: usize = SPECIAL_ROLE_COUNT + 34;
const AIR_ROLE_COUNT: usize = GROUND_ROLE_COUNT + 2;
pub(super) const ATLAS_BYTES: usize = 1_678_356;
const ANIMATED_ATLAS_BYTES: usize = ATLAS_BYTES + 210_300;
const WALKING_ATLAS_BYTES: usize = ANIMATED_ATLAS_BYTES + 452_452;
const MARKER_ATLAS_BYTES: usize = WALKING_ATLAS_BYTES + 15_872;
const SPECIAL_ATLAS_BYTES: usize = MARKER_ATLAS_BYTES + 122_708;
const GROUND_ATLAS_BYTES: usize = SPECIAL_ATLAS_BYTES + 404_236;
const AIR_ATLAS_BYTES: usize = GROUND_ATLAS_BYTES + 14_700;
// One background or card, plus up to four bodies.
pub(super) const EXTRA_SPRITES: usize = 5;

const CARD: Role = Role::new("scene/background", (320, 224), (0i32, 0i32));
const BACKGROUND: Role =
    Role::new("scene/background", (320, 196), (0i32, 28i32));

// Offsets register the verified crop origin against the original station
// ground coordinate and the preceding fixed-point elevation sample.
const ROLES: [Role; AIR_ROLE_COUNT] = [
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
    // Persistent opaque room objects, in cabinet order.
    Role::new("actor/shadow-marge", (32, 31), (-80i32, -87i32)),
    Role::new("actor/shadow-homer", (32, 31), (-80i32, -71i32)),
    Role::new("actor/shadow-bart", (32, 31), (-80i32, -55i32)),
    Role::new("actor/shadow-lisa", (32, 31), (-80i32, -55i32)),
    // New special families; the other twelve reuse street handles.
    Role::new("actor/room-homer", (53, 61), (-85i32, -38i32)),
    Role::new("actor/room-homer", (42, 62), (-79i32, -39i32)),
    Role::new("actor/room-homer", (46, 62), (-74i32, -38i32)),
    Role::new("actor/room-homer", (58, 60), (-82i32, -38i32)),
    Role::new("actor/room-bart", (36, 52), (-72i32, -37i32)),
    Role::new("actor/room-bart", (45, 53), (-71i32, -35i32)),
    Role::new("actor/room-bart", (33, 51), (-73i32, -37i32)),
    Role::new("actor/room-bart", (42, 54), (-90i32, -38i32)),
    Role::new("actor/room-lisa", (41, 55), (-90i32, -31i32)),
    Role::new("actor/room-lisa", (48, 55), (-82i32, -31i32)),
    Role::new("actor/room-lisa", (55, 51), (-73i32, -27i32)),
    Role::new("actor/room-lisa", (52, 50), (-73i32, -26i32)),
    // Registered neutral then moving ground-action families.
    Role::new("actor/room-marge", (59, 72), (-110i32, -48i32)),
    Role::new("actor/room-marge", (42, 64), (-90i32, -40i32)),
    Role::new("actor/room-marge", (37, 62), (-87i32, -38i32)),
    Role::new("actor/room-marge", (65, 60), (-93i32, -36i32)),
    Role::new("actor/room-marge", (81, 63), (-103i32, -39i32)),
    Role::new("actor/room-homer", (40, 63), (-69i32, -40i32)),
    Role::new("actor/room-homer", (40, 62), (-68i32, -39i32)),
    Role::new("actor/room-homer", (44, 62), (-68i32, -39i32)),
    Role::new("actor/room-homer", (52, 63), (-68i32, -40i32)),
    Role::new("actor/room-homer", (51, 62), (-69i32, -39i32)),
    Role::new("actor/room-homer", (38, 63), (-68i32, -40i32)),
    Role::new("actor/room-homer", (37, 63), (-67i32, -40i32)),
    Role::new("actor/room-homer", (47, 62), (-64i32, -39i32)),
    Role::new("actor/room-bart", (46, 43), (-74i32, -19i32)),
    Role::new("actor/room-bart", (45, 41), (-66i32, -17i32)),
    Role::new("actor/room-bart", (56, 38), (-71i32, -14i32)),
    Role::new("actor/room-lisa", (39, 47), (-94i32, -23i32)),
    Role::new("actor/room-lisa", (44, 47), (-88i32, -23i32)),
    Role::new("actor/room-lisa", (75, 47), (-77i32, -23i32)),
    Role::new("actor/room-lisa", (62, 47), (-77i32, -23i32)),
    Role::new("actor/room-marge", (58, 71), (-85i32, -47i32)),
    Role::new("actor/room-marge", (73, 72), (-86i32, -48i32)),
    Role::new("actor/room-marge", (78, 72), (-86i32, -48i32)),
    Role::new("actor/room-homer", (47, 63), (-76i32, -40i32)),
    Role::new("actor/room-homer", (40, 63), (-69i32, -40i32)),
    Role::new("actor/room-homer", (44, 63), (-65i32, -40i32)),
    Role::new("actor/room-homer", (41, 62), (-64i32, -39i32)),
    Role::new("actor/room-bart", (32, 60), (-76i32, -37i32)),
    Role::new("actor/room-bart", (68, 57), (-72i32, -34i32)),
    Role::new("actor/room-bart", (67, 48), (-72i32, -25i32)),
    Role::new("actor/room-bart", (67, 49), (-72i32, -25i32)),
    Role::new("actor/room-lisa", (39, 48), (-94i32, -24i32)),
    Role::new("actor/room-lisa", (37, 58), (-92i32, -34i32)),
    Role::new("actor/room-lisa", (56, 47), (-70i32, -23i32)),
    Role::new("actor/room-lisa", (37, 49), (-87i32, -32i32)),
    Role::new("actor/room-lisa", (38, 49), (-88i32, -32i32)),
];

// Opening, raised and descending families reuse the street atlas handles.
const JUMP_ROLES: [[Role; 3]; 4] = [
    [
        Role::new("actor/marge", (40, 69), (-82i32, -45i32)),
        Role::new("actor/marge", (38, 64), (-88i32, -40i32)),
        Role::new("actor/marge", (38, 85), (-86i32, -61i32)),
    ],
    [
        Role::new("actor/homer", (41, 64), (-86i32, -40i32)),
        Role::new("actor/homer", (41, 65), (-86i32, -41i32)),
        Role::new("actor/homer", (44, 66), (-92i32, -42i32)),
    ],
    [
        Role::new("actor/bart", (36, 57), (-85i32, -35i32)),
        Role::new("actor/bart", (41, 56), (-89i32, -36i32)),
        Role::new("actor/bart", (41, 63), (-85i32, -39i32)),
    ],
    [
        Role::new("actor/lisa", (32, 54), (-82i32, -23i32)),
        Role::new("actor/lisa", (44, 44), (-97i32, -22i32)),
        Role::new("actor/lisa", (37, 50), (-90i32, -28i32)),
    ],
];

pub(super) type AirSprites = [[Option<Art>; 3]; 4];

#[derive(Debug)]
pub(super) struct RoomSprites {
    pub(super) air_attacks: [[Option<Art>; 5]; 4],
    pub(super) jumps: AirSprites,
    pub(super) specials: AirSprites,
}
type RoomSupplement<'supplement> = (
    Option<&'supplement [ContentHash; layers::PHASES]>,
    &'supplement RoomSprites,
);
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
        } else if self.key.starts_with("actor/shadow-") {
            pixels
                .iter()
                .all(|pixel| matches!(pixel, [0, 0, 0, 0] | [_, _, _, 255]))
                && pixels.iter().any(|pixel| pixel.last() == Some(&255))
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
        if self.room_air_attacks() {
            AIR_ATLAS_BYTES
        } else if self.room_ground_attacks() {
            GROUND_ATLAS_BYTES
        } else if self.room_specials() {
            SPECIAL_ATLAS_BYTES
        } else if self.count() == MARKER_ROLE_COUNT {
            MARKER_ATLAS_BYTES
        } else if self.room_walking() {
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
            .take(AIR_ROLE_COUNT.saturating_add(1))
            .map(|field| field.parse::<ContentHash>().map_err(failure))
            .collect::<Result<Vec<_>, _>>()?;
        if !matches!(
            hashes.len(),
            ROLE_COUNT
                | ANIMATED_ROLE_COUNT
                | WALKING_ROLE_COUNT
                | MARKER_ROLE_COUNT
                | SPECIAL_ROLE_COUNT
                | GROUND_ROLE_COUNT
                | AIR_ROLE_COUNT
        ) {
            return Err(failure(
                "expected 16, 44, 104, 108, 120, 154 or 156 cemetery roles",
            ));
        }
        if hashes.len() >= WALKING_ROLE_COUNT && first != Player::Marge {
            return Err(failure("room walking requires the Marge-first rank"));
        }
        let selection = Self { first, hashes };
        if !selection.distinct_roles() {
            return Err(failure("duplicate cemetery art role"));
        }
        Ok(selection)
    }

    pub(super) const fn room_air_attacks(&self) -> bool {
        self.count() == AIR_ROLE_COUNT
    }

    pub(super) const fn room_ground_attacks(&self) -> bool {
        self.count() >= GROUND_ROLE_COUNT
    }

    pub(super) const fn room_specials(&self) -> bool {
        self.count() >= SPECIAL_ROLE_COUNT
    }

    pub(super) const fn room_walking(&self) -> bool {
        self.count() >= WALKING_ROLE_COUNT
    }

    pub(super) const fn sprite_count(&self) -> usize {
        if self.count() >= MARKER_ROLE_COUNT {
            EXTRA_SPRITES + 4
        } else {
            EXTRA_SPRITES
        }
    }
}

#[derive(Debug)]
pub(super) struct CemeteryArt {
    air_attacks: [[Option<Art>; 5]; 4],
    first: Player,
    jumps: AirSprites,
    layers: Option<layers::Layers>,
    roles: Vec<RoleArt>,
    specials: AirSprites,
}

impl CemeteryArt {
    fn air_attack_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
        tick: u8,
    ) -> Option<SpriteDraw> {
        if self.roles.len() != AIR_ROLE_COUNT {
            return None;
        }
        let (art, role) = match air::source(player, tick)? {
            air::Source::Room(index) => {
                self.role(GROUND_ROLE_COUNT.saturating_add(index))?
            },
            air::Source::Street(index) => (
                *self.air_attacks.get(player.index())?.get(index)?.as_ref()?,
                air::street_role(player, index)?,
            ),
        };
        airborne_draw(art, role, body)
    }

    pub(super) const fn animated(&self) -> bool {
        self.roles.len() >= ANIMATED_ROLE_COUNT
    }

    fn background(&self, entrance: &CemeteryEntrance) -> Option<SpriteDraw> {
        let index = background_index(entrance)?;
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.offset;
        Some(art.draw(destination, false))
    }

    fn body(
        &self,
        entrance: &CemeteryEntrance,
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

    fn ground_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
        sample: CemeteryGroundAttack,
    ) -> Option<SpriteDraw> {
        let (art, role) = match ground::source(player, sample)? {
            ground::Source::Room(index) => {
                self.role(SPECIAL_ROLE_COUNT.saturating_add(index))?
            },
            ground::Source::Idle => {
                let idle = CemeteryRoomBody {
                    maneuver: Maneuver::Idle,
                    walking: None,
                    ..body
                };
                let (index, _) = room_role(player, idle)?;
                self.role(index)?
            },
            ground::Source::BartDescent => (
                *self.jumps.get(Player::Bart.index())?.get(2)?.as_ref()?,
                *JUMP_ROLES.get(Player::Bart.index())?.get(2)?,
            ),
        };
        airborne_draw(art, role, body)
    }

    fn jump_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
        motion: JumpMotion,
    ) -> Option<SpriteDraw> {
        let phase = jump_phase(player, motion);
        let art = self.jumps.get(player.index())?.get(phase)?.as_ref()?;
        let role = JUMP_ROLES.get(player.index())?.get(phase)?;
        airborne_draw(*art, *role, body)
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        selection: &Selection,
        (layer_hashes, sprites): RoomSupplement<'_>,
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
            air_attacks: sprites.air_attacks,
            first: selection.first,
            jumps: if selection.count() >= MARKER_ROLE_COUNT {
                sprites.jumps
            } else {
                [[None; 3]; 4]
            },
            layers: layer_hashes
                .map(|hashes| layers::Layers::load(store, scene, *hashes))
                .transpose()?,
            roles,
            specials: if selection.room_specials() {
                sprites.specials
            } else {
                [[None; 3]; 4]
            },
        })
    }

    fn marker_draw(
        &self,
        player: Player,
        position: Point,
    ) -> Option<SpriteDraw> {
        let index = WALKING_ROLE_COUNT.saturating_add(player.index());
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.room_offset(position, false);
        Some(art.draw(destination, false))
    }

    pub(super) fn queue(
        &self,
        entrance: &CemeteryEntrance,
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
        if self.room_walking()
            && let Some(visible) = entrance.room_body_visibility()
            && let Some(room) = entrance.room()
        {
            self.queue_room(&room.presentation_bodies(), visible, draws);
        } else {
            draws.extend(
                Player::ALL
                    .into_iter()
                    .filter_map(|player| self.body(entrance, player)),
            );
        }
        Ok(background_count)
    }

    fn queue_room(
        &self,
        bodies: &[CemeteryRoomBody; 4],
        visible: bool,
        draws: &mut Vec<SpriteDraw>,
    ) {
        let order = room_order(bodies);
        if visible {
            draws.extend(
                order
                    .into_iter()
                    .filter_map(|(player, body)| self.room_draw(player, body)),
            );
        }
        // The lower original ordering bytes place persistent markers
        // in front of every body, even when the latter are hidden.
        draws.extend(order.into_iter().filter_map(|(player, body)| {
            self.marker_draw(player, body.position)
        }));
    }

    fn role(&self, index: usize) -> Option<(Art, Role)> {
        Some((self.roles.get(index)?.art, *ROLES.get(index)?))
    }

    fn room_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
    ) -> Option<SpriteDraw> {
        if self.roles.len() >= GROUND_ROLE_COUNT
            && let Some(sample) = body.ground_attack
        {
            return self.ground_draw(player, body, sample);
        }
        if let Maneuver::AirAttack(motion) = body.maneuver {
            return self.air_attack_draw(player, body, motion.tick());
        }
        if let Maneuver::Jump(motion) = body.maneuver {
            return self.jump_draw(player, body, motion);
        }
        if let Maneuver::Special { tick, .. } = body.maneuver {
            return self.special_draw(player, body, tick);
        }
        let (index, flip) = room_role(player, body)?;
        let (art, role) = self.role(index)?;
        let mut destination = art.rect;
        (destination.x, destination.y) = role.room_offset(body.position, flip);
        Some(art.draw(destination, flip))
    }

    const fn room_walking(&self) -> bool {
        self.roles.len() >= WALKING_ROLE_COUNT
    }

    fn special_draw(
        &self,
        player: Player,
        body: CemeteryRoomBody,
        tick: u8,
    ) -> Option<SpriteDraw> {
        if self.roles.len() < SPECIAL_ROLE_COUNT {
            return None;
        }
        let (art, role) = match special::source(player, tick)? {
            special::Source::Room(index) => {
                self.role(MARKER_ROLE_COUNT.saturating_add(index))?
            },
            special::Source::Street(index) => (
                *self.specials.get(player.index())?.get(index)?.as_ref()?,
                special::street_role(player, index)?,
            ),
        };
        airborne_draw(art, role, body)
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

    fn verify_room_air_attacks(
        &self,
        scene: &mut SceneSession<'_, '_>,
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        if self.roles.len() != AIR_ROLE_COUNT {
            return Ok(());
        }
        for player in Player::ALL {
            let jump = (0u8..9).fold(Maneuver::jump(), |action, _| {
                action.advance(player, true)
            });
            let Maneuver::Jump(motion) = jump else {
                return Err(failure("expected held takeoff"));
            };
            let mut maneuver = Maneuver::air_attack(motion);
            for _ in 0u8..39 {
                self.verify_room_body(
                    scene,
                    (player, None, maneuver),
                    baseline,
                )?;
                maneuver = maneuver.advance(player, false);
            }
        }
        write_status(format_args!(
            "Cemetery air attacks: 156 reflected tick pairs"
        ))
        .map_err(failure)
    }

    fn verify_room_body(
        &self,
        scene: &mut SceneSession<'_, '_>,
        sample: (Player, Option<WalkMotion>, Maneuver),
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        let (player, walking, maneuver) = sample;
        let body = CemeteryRoomBody {
            facing: Facing::Right,
            ground_attack: None,
            maneuver,
            position: Point { x: 224, y: 176 },
            walking,
        };
        self.verify_room_sample(scene, (player, body), baseline)
    }

    fn verify_room_ground_attacks(
        &self,
        scene: &mut SceneSession<'_, '_>,
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        if self.roles.len() < GROUND_ROLE_COUNT {
            return Ok(());
        }
        for player in Player::ALL {
            self.verify_room_ground_track(scene, (player, false), baseline)?;
            self.verify_room_ground_track(scene, (player, true), baseline)?;
        }
        write_status(format_args!(
            "Cemetery ground attacks: 152 reflected sample pairs"
        ))
        .map_err(failure)
    }

    fn verify_room_ground_track(
        &self,
        scene: &mut SceneSession<'_, '_>,
        (player, moving): (Player, bool),
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        let mut maneuver = Maneuver::Attack { moving, tick: 0 };
        for tick in 0u8..=18 {
            let body = CemeteryRoomBody {
                facing: Facing::Right,
                ground_attack: Some(CemeteryGroundAttack {
                    moving: moving && tick > 0,
                    tick,
                }),
                maneuver,
                position: Point { x: 224, y: 176 },
                walking: None,
            };
            self.verify_room_sample(scene, (player, body), baseline)?;
            maneuver = maneuver.advance(player, false);
        }
        Ok(())
    }

    fn verify_room_jump(
        &self,
        scene: &mut SceneSession<'_, '_>,
        player: Player,
        (maneuver, baseline): (Maneuver, &[u8]),
    ) -> Result<bool, RenderError> {
        let Maneuver::Jump(motion) = maneuver else {
            return Err(failure("expected airborne cemetery sample"));
        };
        let phase = jump_phase(player, motion);
        if self
            .jumps
            .get(player.index())
            .and_then(|poses| poses.get(phase))
            .is_none_or(Option::is_none)
        {
            return Ok(false);
        }
        self.verify_room_body(scene, (player, None, maneuver), baseline)?;
        Ok(true)
    }

    fn verify_room_jumps(
        &self,
        scene: &mut SceneSession<'_, '_>,
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        let mut count = 0usize;
        for player in Player::ALL {
            let opening = Maneuver::jump();
            let descent = (0u8..4)
                .fold(opening, |action, _| action.advance(player, false));
            let raised = (0u8..12)
                .fold(opening, |action, _| action.advance(player, true));
            for maneuver in [opening, raised, descent] {
                count = count.saturating_add(usize::from(
                    self.verify_room_jump(scene, player, (maneuver, baseline))?,
                ));
            }
        }
        write_status(format_args!(
            "Cemetery jumps: {count} reflected reused pose pairs"
        ))
        .map_err(failure)
    }

    fn verify_room_markers(
        &self,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<(), RenderError> {
        if self.roles.len() < MARKER_ROLE_COUNT {
            return Ok(());
        }
        let point = Point { x: 224, y: 176 };
        for player in Player::ALL {
            let draw = self
                .marker_draw(player, point)
                .ok_or(RenderError::InvalidSprite)?;
            let expected = self
                .roles
                .get(WALKING_ROLE_COUNT.saturating_add(player.index()))
                .ok_or(RenderError::InvalidSprite)?
                .digest;
            let _dark = scene.present([0, 0, 0], &[draw])?;
            let dark = scene.readback()?;
            let _light = scene.present([255, 255, 255], &[draw])?;
            if marker_digest(&dark, &scene.readback()?, draw.destination)?
                != expected
            {
                return Err(failure("cemetery marker pixels differ"));
            }
        }
        let bodies = [CemeteryRoomBody {
            facing: Facing::Right,
            ground_attack: None,
            maneuver: Maneuver::Idle,
            position: point,
            walking: None,
        }; 4];
        let mut hidden = Vec::new();
        let mut visible = Vec::new();
        self.queue_room(&bodies, false, &mut hidden);
        self.queue_room(&bodies, true, &mut visible);
        if hidden.len() != 4
            || visible.len() != 8
            || !visible.ends_with(&hidden)
        {
            return Err(failure("cemetery marker order differs"));
        }
        write_status(format_args!("Cemetery markers: four exact crops"))
            .map_err(failure)
    }

    fn verify_room_sample(
        &self,
        scene: &mut SceneSession<'_, '_>,
        (player, mut body): (Player, CemeteryRoomBody),
        baseline: &[u8],
    ) -> Result<(), RenderError> {
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

    fn verify_room_specials(
        &self,
        scene: &mut SceneSession<'_, '_>,
        baseline: &[u8],
    ) -> Result<(), RenderError> {
        if self.roles.len() < SPECIAL_ROLE_COUNT {
            return Ok(());
        }
        for player in Player::ALL {
            let mut maneuver = Maneuver::special(0);
            for _ in 0u8..30 {
                self.verify_room_body(
                    scene,
                    (player, None, maneuver),
                    baseline,
                )?;
                maneuver = maneuver.advance(player, false);
            }
        }
        write_status(format_args!(
            "Cemetery specials: 120 reflected tick pairs"
        ))
        .map_err(failure)
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
            self.verify_room_body(
                scene,
                (player, None, Maneuver::Idle),
                &baseline,
            )?;
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
                    (player, Some(walking), Maneuver::Idle),
                    &baseline,
                )?;
            }
        }
        write_status(format_args!("Cemetery walking: 64 reflected body pairs"))
            .map_err(failure)?;
        self.verify_room_jumps(scene, &baseline)?;
        self.verify_room_markers(scene)?;
        self.verify_room_specials(scene, &baseline)?;
        self.verify_room_ground_attacks(scene, &baseline)?;
        self.verify_room_air_attacks(scene, &baseline)
    }
}

fn airborne_draw(
    art: Art,
    role: Role,
    body: CemeteryRoomBody,
) -> Option<SpriteDraw> {
    if (art.rect.width.get(), art.rect.height.get()) != role.extent {
        return None;
    }
    let flip = body.facing == Facing::Left;
    let mut destination = art.rect;
    (destination.x, destination.y) = role.room_offset(body.position, flip);
    destination.y = destination
        .y
        .saturating_sub(i32::from(body.maneuver.elevation()));
    Some(art.draw(destination, flip))
}

fn jump_phase(player: Player, motion: JumpMotion) -> usize {
    let descent = if player == Player::Bart {
        3
    } else {
        4
    };
    if motion.opening_pose() || motion.pose_tick() < descent {
        0
    } else if motion.raised_pose() {
        1
    } else {
        2
    }
}

// Opaque marker crops are recovered exactly using two distinct clears.
// Unlike body shadow pixels, no fractional alpha is admitted here.
fn marker_digest(
    dark: &[u8],
    light: &[u8],
    rect: SpriteRect,
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
            .checked_rem(world::BASE_WIDTH)
            .ok_or(RenderError::InvalidLength)?;
        let vertical = pixel_index
            .checked_div(world::BASE_WIDTH)
            .ok_or(RenderError::InvalidLength)?;
        if !rect.contains_pixel(horizontal, vertical) {
            continue;
        }
        if dark_pixel == light_pixel && dark_pixel.last() == Some(&255) {
            pixels.push(dark_pixel);
        } else if dark_pixel == [0, 0, 0, 255] && light_pixel == [255; 4] {
            pixels.push([0; 4]);
        } else {
            return Err(failure("cemetery marker has blended pixels"));
        }
    }
    let expected = rect.width.get().saturating_mul(rect.height.get());
    if pixels.len() != usize::try_from(expected).map_err(failure)? {
        return Err(RenderError::InvalidLength);
    }
    Ok(ContentHash::calculate(pixels.as_flattened()))
}

fn room_order(
    bodies: &[CemeteryRoomBody; 4],
) -> [(Player, CemeteryRoomBody); 4] {
    let [marge, homer, bart, lisa] = *bodies;
    let mut order = [
        (Player::Marge, marge),
        (Player::Homer, homer),
        (Player::Bart, bart),
        (Player::Lisa, lisa),
    ];
    // Original object order is front to back; native alpha composition
    // queues the opposite order. Cabinet order resolves depth ties.
    order.sort_unstable_by_key(|(player, body)| {
        Reverse((body.position.y, player.index()))
    });
    order
}

fn background_index(entrance: &CemeteryEntrance) -> Option<usize> {
    if entrance.phase() == CemeteryPhase::Card {
        Some(usize::from(entrance.tick() >= 5))
    } else {
        Some(usize::from(entrance.background_phase()?).saturating_add(2))
    }
}

fn body_sample(
    entrance: &CemeteryEntrance,
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

pub(super) const fn air_attack_keys(player: Player) -> [Option<u8>; 5] {
    air::keys(player)
}

pub(super) const fn special_keys(player: Player) -> [u8; 3] {
    special::street_keys(player)
}

fn room_role(player: Player, body: CemeteryRoomBody) -> Option<(usize, bool)> {
    if body.maneuver != Maneuver::Idle {
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
    let background_count = cemetery.queue(&entrance, &mut expected)?;
    let background = background_index(&entrance)
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
    fn room_markers_follow_projected_depth_and_remain_opaque() {
        let point = Point { x: 224, y: 176 };
        let moved = Point { x: 212, y: 157 };
        for (player, y) in
            Player::ALL.into_iter().zip([89i32, 105i32, 121i32, 121i32])
        {
            let candidate =
                ROLES.get(WALKING_ROLE_COUNT.saturating_add(player.index()));
            assert_eq!(
                candidate.map(|role| role.room_offset(point, false)),
                Some((144i32, y))
            );
            assert_eq!(
                candidate.map(|role| role.room_offset(moved, false)),
                Some((132i32, y.saturating_add(19)))
            );
            if let Some(role) = candidate {
                assert!(role.valid_pixels(&[0, 0, 0, 0, 1, 2, 3, 255]));
                assert!(!role.valid_pixels(&[0, 0, 0, 0]));
                assert!(!role.valid_pixels(&[1, 2, 3, 0, 1, 2, 3, 255]));
                assert!(!role.valid_pixels(&[0, 0, 0, 102, 1, 2, 3, 255]));
            }
        }
    }

    #[test]
    fn room_draw_order_reverses_depth_and_cabinet_ties() {
        let body = |y| CemeteryRoomBody {
            facing: Facing::Right,
            ground_attack: None,
            maneuver: Maneuver::Idle,
            position: Point { x: 224, y },
            walking: None,
        };
        let order = |bodies| room_order(&bodies).map(|(player, _)| player);
        assert_eq!(order([body(176); 4]), [
            Player::Lisa,
            Player::Bart,
            Player::Homer,
            Player::Marge,
        ]);
        assert_eq!(order([body(150), body(170), body(160), body(170)]), [
            Player::Lisa,
            Player::Homer,
            Player::Bart,
            Player::Marge,
        ]);
        assert_eq!(order([body(170), body(150), body(150), body(160)]), [
            Player::Marge,
            Player::Lisa,
            Player::Bart,
            Player::Homer,
        ]);
    }

    #[test]
    fn opaque_marker_readback_recovers_the_exact_transparent_crop() {
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
        assert_eq!(marker_digest(&dark, &light, rect), Ok(expected));
        let blended = [12, 34, 56, 255, 153, 153, 153, 255];
        assert!(marker_digest(&dark, &blended, rect).is_err());
        assert!(marker_digest(&dark, &light[..4], rect).is_err());
    }

    #[test]
    fn atlas_budget_matches_the_exact_roles() {
        for (count, budget) in [
            (ROLE_COUNT, ATLAS_BYTES),
            (ANIMATED_ROLE_COUNT, ANIMATED_ATLAS_BYTES),
            (WALKING_ROLE_COUNT, WALKING_ATLAS_BYTES),
            (MARKER_ROLE_COUNT, MARKER_ATLAS_BYTES),
            (SPECIAL_ROLE_COUNT, SPECIAL_ATLAS_BYTES),
            (GROUND_ROLE_COUNT, GROUND_ATLAS_BYTES),
            (AIR_ROLE_COUNT, AIR_ATLAS_BYTES),
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
    fn room_jump_tracks_match_short_and_held_visible_samples() {
        for player in Player::ALL {
            assert_jump_track(player, false, 30);
            assert_jump_track(player, true, 50);
        }
    }

    fn assert_jump_track(player: Player, held: bool, last: u8) {
        let mut maneuver = Maneuver::jump();
        for tick in 0u8..=last {
            if tick.is_multiple_of(2) {
                let expected = expected_jump_phase(held, tick);
                let actual = match maneuver {
                    Maneuver::Jump(motion) => Some(jump_phase(player, motion)),
                    Maneuver::AirAttack(_)
                    | Maneuver::Attack { .. }
                    | Maneuver::Hurt { .. }
                    | Maneuver::Idle
                    | Maneuver::Restrained
                    | Maneuver::Special { .. } => None,
                };
                assert_eq!(
                    actual,
                    Some(expected),
                    "{player:?} held={held} tick={tick}"
                );
            }
            maneuver =
                maneuver.advance(player, held && tick.saturating_add(1) < 20);
        }
    }

    fn expected_jump_phase(held: bool, tick: u8) -> usize {
        match (held, tick) {
            (true, ..=10) | (false, ..=2) => 0,
            (true, 12..=20) => 1,
            _ => 2,
        }
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
                facing: Facing::Right,
                ground_attack: None,
                maneuver: Maneuver::Idle,
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
            body.maneuver = Maneuver::Attack { moving: false, tick: 0 };
            assert_eq!(room_role(player, body), None);
        }
    }

    fn assert_room_reflection(
        player: Player,
        walking: Option<WalkMotion>,
        index: usize,
    ) {
        let mut body = CemeteryRoomBody {
            facing: Facing::Right,
            ground_attack: None,
            maneuver: Maneuver::Idle,
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
            let offset = body_sample(&state, player, true).and_then(
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
                body_sample(&state, player, false).map(|(index, _, _)| index);
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
        assert!(body_sample(&entrance, Player::Marge, true).is_some());
        entrance.enable_room();
        for player in Player::ALL {
            assert_eq!(body_sample(&entrance, player, true), None);
        }
        entrance.advance(false);
        for player in Player::ALL {
            assert!(body_sample(&entrance, player, true).is_some());
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
            let actual = body_sample(&entrance, player, false).and_then(
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
