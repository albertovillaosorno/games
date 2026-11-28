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
//   - Optional title pose catalogue admission and fixed-tick art selection.
// - Must-Not:
//   - Interpret original media or advance simulation.
// - Allows:
//   - Inputs: Bounded local catalogue, canonical bundles and maneuver state.
//   - Outputs: Verified sprite handles selected by station and tick.
//   - Side effects: Bounded local bundle reads and GPU atlas allocation.
// - Split-When:
//   - Multiple titles need a shared animation catalogue contract.
// - Merge-When:
//   - Another title-local module duplicates pose admission.
// - Summary:
//   - Resolve optional native attack, jump and walking art.
// - Description:
//   - Every admitted sequence starts at tick zero and is bounded.
// - Usage:
//   - Supply explicit canonical manifest hashes for named poses.
// - Defaults:
//   - No catalogue; the existing idle art remains available.

//! Optional verified maneuver pose catalogue.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::str::from_utf8;

use games::animation::{
    cycle_tick, keyframe as select_entry, optional_track_complete,
};
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::storage::bundle::BundleSource;
use games_platform::ir::storage::file::snapshot;

use super::world::{self, StageArt, WorldArt};
use super::{
    Art, KEYS, Maneuver, Player, Station, arrival, backdrop, balloon, bonus,
    cemetery, failure, linked, open_bundle, scenery, wrestler,
};
use crate::{
    BART_LISA_JUMP_TICKS, Cabinet, ENEMY_ATTACK_TICKS, ENEMY_DEFEAT_TICKS,
    ENEMY_WALK_TICKS, EnemyAction, JumpMotion, MID_STREET_SCRIPT_ORIGIN, Pair,
    PairMotion, WalkMotion,
};

const MAX_POSES: usize = 220;
pub(super) const MAX_CATALOGUE_BYTES: NonZeroU64 =
    NonZeroU64::MIN.saturating_add(35_887);
const REPELLED_KEYS: [u8; 12] = [0, 5, 10, 16, 19, 25, 29, 34, 49, 56, 77, 83];
const RESTRAINT_KEYS: [u8; 6] = [0, 7, 14, 20, 27, 45];
const MARGE_HOMER_JUMP_KEYS: [u8; 8] = [0, 2, 8, 10, 12, 14, 16, 18];
const MARGE_HOMER_JUMP_PHASES: [u8; 29] = [
    0, 0, 2, 2, 0, 0, 2, 2, 8, 8, 10, 10, 12, 12, 14, 14, 16, 16, 18, 18, 0, 0,
    2, 2, 8, 8, 10, 10, 12,
];
const MARGE_HOMER_LANDED_PHASES: [u8; 8] = [12, 14, 16, 18, 0, 2, 8, 10];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    AirAttack,
    Attack,
    AttackMoving,
    Defeat,
    Hurt,
    Idle,
    Jump,
    Lift,
    Repelled,
    Restraint,
    Special,
    Walk,
    WalkUp,
}

impl Kind {
    const fn duration(self, player: Player) -> u8 {
        match self {
            Self::AirAttack => 60,
            Self::Attack | Self::AttackMoving => {
                Maneuver::Attack { moving: false, tick: 0 }
                    .pose_duration(player)
            },
            Self::Defeat | Self::Idle | Self::Repelled | Self::Restraint => 0,
            Self::Hurt => Maneuver::Hurt { tick: 0 }.pose_duration(player),
            Self::Jump => Maneuver::jump().pose_duration(player),
            Self::Lift => 1,
            Self::Special => 30,
            Self::Walk | Self::WalkUp => walk_duration(player),
        }
    }
}

type Entry = (Key, ContentHash);
type Key = (Subject, Kind, u8);
type LoadedPoses = (PoseEntries, Vec<cemetery::shadow::Source>);
type PoseEntries = BTreeMap<Key, Art>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Subject {
    Enemy,
    Pair(Pair),
    Station(Player),
}

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    arrival: Option<ContentHash>,
    pub(super) backdrop: backdrop::Catalogue,
    balloon: Option<[ContentHash; balloon::POSE_COUNT]>,
    bonus: Option<ContentHash>,
    bonus_departure: Option<ContentHash>,
    bonus_inflated: Option<ContentHash>,
    cemetery: Option<Box<cemetery::Selection>>,
    cemetery_layers: Option<[ContentHash; cemetery::layers::PHASES]>,
    entries: BTreeMap<Key, ContentHash>,
    landing_markers: Option<ContentHash>,
    linked: Option<[ContentHash; linked::POSE_COUNT]>,
    scenery: scenery::Catalogue,
    stage_two: Option<[ContentHash; 2]>,
    wrestler: wrestler::Catalogue,
}

impl Catalogue {
    pub(super) fn admit_world(
        &self,
        world: Option<WorldArt>,
    ) -> Result<(), RenderError> {
        if self.backdrop.street_count() != 0
            && world.map_or(Ok(0), WorldArt::scroll_limit)?
                != u16::try_from(crate::LATE_BACKDROP_ORIGIN)
                    .map_err(failure)?
        {
            return Err(failure("backdrop requires the verified long street"));
        }
        if self.has_wrestler() && self.backdrop.street_count() == 0 {
            return Err(failure("wrestler requires the late backdrop"));
        }
        if self.scenery.has_street()
            && world.map_or(Ok(0), WorldArt::scroll_limit)?
                < u16::try_from(MID_STREET_SCRIPT_ORIGIN).map_err(failure)?
        {
            return Err(failure("scenic poses require the mid-street world"));
        }
        Ok(())
    }

    pub(super) fn atlas_bytes(&self) -> usize {
        self.entries
            .len()
            .saturating_add(self.wrestler.count())
            .saturating_mul(65_536)
            .saturating_add(
                usize::from(self.linked.is_some())
                    .saturating_mul(linked::POSE_COUNT)
                    .saturating_mul(65_536),
            )
            .saturating_add(
                usize::from(self.balloon.is_some())
                    .saturating_mul(balloon::ATLAS_BYTES),
            )
            .saturating_add(
                self.cemetery
                    .as_deref()
                    .map_or(0, cemetery::Selection::atlas_bytes),
            )
            .saturating_add(self.scenery.atlas_bytes())
            .saturating_add(
                usize::from(self.cemetery_layers.is_some())
                    .saturating_mul(cemetery::layers::ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.arrival.is_some())
                    .saturating_mul(arrival::ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.landing_markers.is_some())
                    .saturating_mul(arrival::MARKER_ATLAS_BYTES),
            )
            .saturating_add(
                self.backdrop_count().saturating_mul(backdrop::ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.bonus.is_some())
                    .saturating_mul(bonus::ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.bonus_departure.is_some())
                    .saturating_mul(bonus::DEPARTURE_ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.bonus_inflated.is_some())
                    .saturating_mul(bonus::INFLATED_ATLAS_BYTES),
            )
            .saturating_add(
                usize::from(self.stage_two.is_some())
                    .saturating_mul(world::stage_atlas_budget()),
            )
    }

    pub(super) fn atlas_count(&self) -> usize {
        self.count()
            .saturating_add(usize::from(
                self.cemetery
                    .as_deref()
                    .is_some_and(cemetery::Selection::room_walking),
            ))
            .saturating_add(
                usize::from(self.cemetery_layers.is_some()).saturating_mul(
                    cemetery::layers::ATLAS_COUNT
                        .saturating_sub(cemetery::layers::PHASES),
                ),
            )
            .saturating_add(
                usize::from(self.arrival.is_some())
                    .saturating_mul(arrival::ATLAS_COUNT.saturating_sub(1)),
            )
            .saturating_add(
                usize::from(self.landing_markers.is_some()).saturating_mul(
                    arrival::MARKER_ATLAS_COUNT.saturating_sub(1),
                ),
            )
            .saturating_add(
                usize::from(self.bonus.is_some())
                    .saturating_mul(bonus::ATLAS_COUNT.saturating_sub(1)),
            )
            .saturating_add(
                usize::from(self.bonus_departure.is_some()).saturating_mul(
                    bonus::DEPARTURE_ATLAS_COUNT.saturating_sub(1),
                ),
            )
            .saturating_add(
                usize::from(self.bonus_inflated.is_some()).saturating_mul(
                    bonus::INFLATED_ATLAS_COUNT.saturating_sub(1),
                ),
            )
    }

    pub(super) fn backdrop_count(&self) -> usize {
        self.backdrop.count()
    }

    pub(super) fn balloon_sprite_count(&self) -> usize {
        usize::from(self.balloon.is_some())
    }

    pub(super) fn bonus_departure_sprite_count(&self) -> usize {
        usize::from(self.bonus_departure.is_some())
            .saturating_mul(bonus::DEPARTURE_EXTRA_SPRITES)
    }

    fn cemetery_complete(&self) -> bool {
        (self.cemetery.is_none() || self.balloon.is_some())
            && (self.cemetery_layers.is_none()
                || self
                    .cemetery
                    .as_deref()
                    .is_some_and(cemetery::Selection::animated))
            && (!self
                .cemetery
                .as_deref()
                .is_some_and(cemetery::Selection::room_walking)
                || self.cemetery_layers.is_some())
            && (!self
                .cemetery
                .as_deref()
                .is_some_and(cemetery::Selection::room_specials)
                || room_special_entries_complete(&self.entries))
            && (!self
                .cemetery
                .as_deref()
                .is_some_and(cemetery::Selection::room_ground_attacks)
                || self.entries.contains_key(&station_key(
                    Player::Bart,
                    Kind::Jump,
                    3,
                )))
            && (!self
                .cemetery
                .as_deref()
                .is_some_and(cemetery::Selection::room_air_attacks)
                || room_air_entries_complete(&self.entries))
    }

    fn complete(&self) -> bool {
        let entries = &self.entries;
        let complete = Player::ALL.into_iter().all(|player| {
            [Kind::Attack, Kind::Jump]
                .into_iter()
                .all(|kind| entries.contains_key(&station_key(player, kind, 0)))
        });
        let optional = [
            Kind::AirAttack,
            Kind::AttackMoving,
            Kind::Hurt,
            Kind::Lift,
            Kind::Special,
            Kind::Walk,
            Kind::WalkUp,
        ]
        .into_iter()
        .all(|kind| optional_complete(entries, kind));
        complete
            && optional
            && enemy_complete(entries)
            && interaction_complete(entries, Kind::Restraint, &RESTRAINT_KEYS)
            && interaction_complete(entries, Kind::Repelled, &REPELLED_KEYS)
            && repelled_requires_restraint(entries)
            && pair_complete(entries)
            && self.scenery.complete()
            && (!(self.scenery.has_entrance() || self.arrival.is_some())
                || self.stage_two.is_some())
            && (self.landing_markers.is_none() || self.arrival.is_some())
            && self.backdrop.complete()
            && (!self.backdrop.has_boardwalk() || self.stage_two.is_some())
            && self.wrestler.complete()
            && (self.bonus.is_none() || self.has_wrestler())
            && (self.bonus_departure.is_none() || self.bonus.is_some())
            && (self.bonus_inflated.is_none() || self.bonus.is_some())
            && (self.stage_two.is_none() || self.bonus.is_some())
            && (self.balloon.is_none()
                || (self.linked.is_some() && self.backdrop.has_boardwalk()))
            && self.cemetery_complete()
            && (self.linked.is_none() || self.stage_two.is_some())
    }

    pub(super) fn count(&self) -> usize {
        self.entries
            .len()
            .saturating_add(self.scenery.count())
            .saturating_add(
                usize::from(self.linked.is_some())
                    .saturating_mul(linked::POSE_COUNT),
            )
            .saturating_add(usize::from(self.arrival.is_some()))
            .saturating_add(usize::from(self.landing_markers.is_some()))
            .saturating_add(self.backdrop_count())
            .saturating_add(
                usize::from(self.balloon.is_some())
                    .saturating_mul(balloon::POSE_COUNT),
            )
            .saturating_add(
                self.cemetery
                    .as_deref()
                    .map_or(0, cemetery::Selection::count),
            )
            .saturating_add(self.wrestler.count())
            .saturating_add(
                usize::from(self.cemetery_layers.is_some())
                    .saturating_mul(cemetery::layers::PHASES),
            )
            .saturating_add(usize::from(self.bonus.is_some()))
            .saturating_add(usize::from(self.bonus_departure.is_some()))
            .saturating_add(usize::from(self.bonus_inflated.is_some()))
            .saturating_add(
                usize::from(self.stage_two.is_some()).saturating_mul(2),
            )
    }

    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.entries
            .values()
            .copied()
            .chain(self.scenery.digests())
            .chain(self.backdrop.digests())
            .chain(self.balloon.into_iter().flatten())
            .chain(
                self.cemetery
                    .iter()
                    .flat_map(|selection| selection.digests()),
            )
            .chain(self.wrestler.digests())
            .chain(self.cemetery_layers.into_iter().flatten())
            .chain(self.bonus)
            .chain(self.bonus_departure)
            .chain(self.bonus_inflated)
            .chain(self.arrival)
            .chain(self.landing_markers)
            .chain(self.stage_two.into_iter().flatten())
            .chain(self.linked.into_iter().flatten())
    }

    pub(super) const fn has_arrival(&self) -> bool {
        self.arrival.is_some()
    }

    pub(super) fn has_scenery(&self) -> bool {
        self.scenery.count() != 0
    }

    pub(super) fn has_wrestler(&self) -> bool {
        self.wrestler.count() != 0
    }

    fn insert(&mut self, line: &str) -> Result<(), RenderError> {
        match line.split_whitespace().next() {
            Some("balloon-boss") => {
                self.balloon =
                    Some(balloon::parse(line, self.balloon.is_some())?);
            },
            Some("cemetery-entrance" | "cemetery-room-layers") => {
                self.insert_cemetery(line)?;
            },
            Some("linked-actors") => {
                self.linked = Some(linked::parse(line, self.linked.is_some())?);
            },
            Some("stage-two") => {
                self.stage_two = Some(parse_stage_two(line, self.stage_two)?);
            },
            Some("bonus") => {
                self.bonus =
                    Some(parse_single_bundle("bonus", line, self.bonus)?);
            },
            Some("bonus-departure") => {
                self.bonus_departure = Some(parse_single_bundle(
                    "bonus-departure",
                    line,
                    self.bonus_departure,
                )?);
            },
            Some("bonus-inflated") => {
                self.bonus_inflated = Some(parse_single_bundle(
                    "bonus-inflated",
                    line,
                    self.bonus_inflated,
                )?);
            },
            Some("arrival") => {
                self.arrival =
                    Some(parse_single_bundle("arrival", line, self.arrival)?);
            },
            Some("landing-markers") => {
                self.landing_markers = Some(parse_single_bundle(
                    "landing-markers",
                    line,
                    self.landing_markers,
                )?);
            },
            Some("wrestler") => self.wrestler.insert(line)?,
            Some("backdrop" | "stage-two-backdrop") => {
                self.backdrop.insert(line)?;
            },
            Some("scenic-entrance" | "scenic-pair" | "scenic-stationary") => {
                self.scenery.insert(line)?;
            },
            _ => {
                let (key, digest) = parse_line(line)?;
                if self.entries.len() >= MAX_POSES
                    || self.entries.insert(key, digest).is_some()
                {
                    return Err(failure("excess or duplicate poses"));
                }
            },
        }
        Ok(())
    }

    fn insert_cemetery(&mut self, line: &str) -> Result<(), RenderError> {
        if line.split_whitespace().next() == Some("cemetery-room-layers") {
            return self.insert_cemetery_layers(line);
        }
        if self.cemetery.is_some() {
            return Err(failure("duplicate cemetery entrance"));
        }
        self.cemetery = Some(Box::new(cemetery::Selection::parse(line)?));
        Ok(())
    }

    fn insert_cemetery_layers(
        &mut self,
        line: &str,
    ) -> Result<(), RenderError> {
        if self.cemetery_layers.is_some() {
            return Err(failure("duplicate cemetery layer record"));
        }
        self.cemetery_layers = Some(cemetery::layers::parse(line)?);
        Ok(())
    }

    pub(super) fn linked_sprite_count(&self) -> usize {
        usize::from(self.linked.is_some()).saturating_mul(linked::EXTRA_SPRITES)
    }

    pub(super) fn load(
        &self,
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<PoseArt, RenderError> {
        let (entries, sources) = self.load_entries(store, scene)?;
        Ok(PoseArt {
            arrival: self
                .arrival
                .map(|digest| {
                    arrival::ArrivalArt::load(
                        store,
                        scene,
                        (digest, self.landing_markers),
                    )
                })
                .transpose()?,
            backdrop: self.backdrop.load(store, scene)?,
            balloon: self
                .balloon
                .map(|hashes| {
                    balloon::BalloonArt::load(store, scene, hashes)
                        .map(Box::new)
                })
                .transpose()?,
            bonus: self
                .bonus
                .map(|digest| {
                    bonus::BonusArt::load(
                        store,
                        scene,
                        (digest, self.bonus_departure, self.bonus_inflated),
                    )
                })
                .transpose()?,
            cemetery: self
                .cemetery
                .as_deref()
                .map(|selection| {
                    cemetery::CemeteryArt::load(
                        store,
                        scene,
                        selection,
                        (
                            self.cemetery_layers.as_ref(),
                            room_sprites(&entries, sources),
                        ),
                    )
                    .map(Box::new)
                })
                .transpose()?,
            entries,
            linked: self
                .linked
                .as_ref()
                .map(|poses| linked::LinkedArt::load(store, scene, poses))
                .transpose()?,
            scenery: self.scenery.load(store, scene)?,
            stage_two: self
                .stage_two
                .map(|hashes| StageArt::load(store, scene, hashes))
                .transpose()?,
            wrestler: self.wrestler.load(store, scene)?,
        })
    }

    fn load_entries(
        &self,
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<LoadedPoses, RenderError> {
        let mut entries = BTreeMap::new();
        let mut sources = Vec::new();
        let room = self
            .cemetery
            .as_deref()
            .is_some_and(cemetery::Selection::room_walking);
        for (&key, digest) in &self.entries {
            let bundle = open_bundle(store, *digest)?;
            let name = match (key.0, key.1) {
                (Subject::Station(player), _) => KEYS
                    .get(player.index())
                    .ok_or(RenderError::InvalidSprite)?,
                (Subject::Enemy, Kind::Walk) => &"enemy/approach",
                (Subject::Enemy, Kind::Defeat) => &"enemy/defeat",
                (Subject::Enemy, Kind::Repelled) => &"enemy/repelled",
                (Subject::Enemy, Kind::Restraint) => &"actor/interaction",
                (Subject::Enemy, _) => &"enemy/strike",
                (Subject::Pair(Pair::MargeHomer), _) => &"actor/duo",
                (Subject::Pair(Pair::BartLisa), _) => &"actor/bart-lisa",
            };
            let (art, pixels) = Art::load_with_pixels(scene, &bundle, name)?;
            if art.rect.width.get() > 128 || art.rect.height.get() > 128 {
                return Err(failure("unexpected pose extent"));
            }
            if room && room_source(key) {
                sources.push(cemetery::shadow::Source::new(art, &pixels)?);
            }
            let _prior = entries.insert(key, art);
        }
        Ok((entries, sources))
    }

    pub(super) fn open(path: &OsStr) -> Result<Self, RenderError> {
        let bytes = snapshot(path, MAX_CATALOGUE_BYTES).map_err(failure)?;
        Self::parse(from_utf8(&bytes).map_err(failure)?)
    }

    pub(super) fn parse(text: &str) -> Result<Self, RenderError> {
        if u64::try_from(text.len())
            .map_or(true, |length| length > MAX_CATALOGUE_BYTES.get())
        {
            return Err(failure("pose catalogue exceeds byte limit"));
        }
        let mut catalogue = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            catalogue.insert(line)?;
        }
        if !catalogue.complete() {
            return Err(failure("missing tick-zero pose"));
        }
        Ok(catalogue)
    }

    pub(super) fn scenic_sprite_count(&self) -> usize {
        self.scenery
            .track_count()
            .saturating_add(self.linked_sprite_count())
            .saturating_add(self.balloon_sprite_count())
            .saturating_add(
                usize::from(self.cemetery_layers.is_some())
                    .saturating_mul(cemetery::layers::EXTRA_DRAWS),
            )
            .saturating_add(
                self.cemetery
                    .as_deref()
                    .map_or(0, cemetery::Selection::sprite_count),
            )
            .saturating_add(
                usize::from(self.arrival.is_some())
                    .saturating_mul(arrival::EXTRA_SPRITES),
            )
            .saturating_add(
                usize::from(self.landing_markers.is_some())
                    .saturating_mul(arrival::MARKER_EXTRA_SPRITES),
            )
    }

    pub(super) fn stage_two_strip(&self) -> Option<ContentHash> {
        self.stage_two.map(|[_, strip]| strip)
    }
}

#[derive(Debug, Default)]
pub(super) struct PoseArt {
    pub(super) arrival: Option<arrival::ArrivalArt>,
    pub(super) backdrop: backdrop::BackdropArt,
    pub(super) balloon: Option<Box<balloon::BalloonArt>>,
    pub(super) bonus: Option<bonus::BonusArt>,
    pub(super) cemetery: Option<Box<cemetery::CemeteryArt>>,
    entries: PoseEntries,
    pub(super) linked: Option<linked::LinkedArt>,
    pub(super) scenery: scenery::PlacedArt,
    pub(super) stage_two: Option<StageArt>,
    pub(super) wrestler: wrestler::WrestlerArt,
}

impl PoseArt {
    pub(super) fn configure_cabinet(
        &self,
        cabinet: &Cabinet,
    ) -> Result<Cabinet, RenderError> {
        let configured = Pair::ALL.into_iter().fold(
            cabinet
                .with_cooperative_pairs(self.paired_families())
                .with_marge_restraint(self.restraint_enabled())
                .with_late_wrestler(self.wrestler.enabled())
                .with_bonus(self.bonus.is_some()),
            |state, pair| state.with_pair_jump(pair, self.pair_jump(pair)),
        );
        self.configure_stage_two(&configured)
    }

    pub(super) fn configure_stage_two(
        &self,
        cabinet: &Cabinet,
    ) -> Result<Cabinet, RenderError> {
        if let Some(stage) = self.stage_two {
            let travel = stage.world.scroll_limit()?;
            if self.backdrop.has_boardwalk()
                && i16::try_from(travel).map_err(failure)?
                    != crate::BOARDWALK_BACKDROP_ORIGIN
            {
                return Err(failure(
                    "boardwalk backdrop requires the endpoint",
                ));
            }
            if self.linked.is_some()
                && i16::try_from(travel).map_err(failure)?
                    < crate::LINKED_MIN_TRAVEL
            {
                return Err(failure(
                    "linked actors require late second-stage travel",
                ));
            }
            (*cabinet)
                .with_stage_two(travel)
                .map(|state| state.with_linked_actors(self.linked.is_some()))
                .map(|state| state.with_balloon_boss(self.balloon.is_some()))
                .map(|state| {
                    state.with_cemetery_entrance(
                        self.cemetery
                            .as_deref()
                            .map(cemetery::CemeteryArt::first),
                    )
                })
                .map_err(failure)
        } else {
            Ok(*cabinet)
        }
    }

    pub(super) fn enemy(&self, action: EnemyAction) -> Option<Art> {
        let (kind, tick) = match action {
            EnemyAction::Approach { tick } => (Kind::Walk, tick),
            EnemyAction::Attack { tick } => (Kind::Attack, tick),
            EnemyAction::Defeat { tick } => (Kind::Defeat, tick),
            EnemyAction::Repelled { tick } if self.repelled_enabled() => {
                (Kind::Repelled, repelled_phase(tick))
            },
            EnemyAction::Repelled { tick } => (Kind::Defeat, tick.min(51)),
            EnemyAction::Restrain(motion) => {
                (Kind::Restraint, motion.pose_tick())
            },
        };
        select_entry(&self.entries, (Subject::Enemy, kind, tick)).copied()
    }

    pub(super) fn entrance(&self, player: Player) -> Option<Art> {
        select_entry(&self.entries, station_key(player, Kind::Jump, 30))
            .copied()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn pair(&self, pair: Pair, motion: PairMotion) -> Option<Art> {
        let (kind, tick) = match motion {
            PairMotion::AfterJump { tick } => (
                Kind::Jump,
                *MARGE_HOMER_LANDED_PHASES.get(usize::from(tick >> 1u32))?,
            ),
            PairMotion::Idle { tick } => (Kind::Idle, tick),
            PairMotion::Jump { tick } if pair == Pair::MargeHomer => {
                (Kind::Jump, *MARGE_HOMER_JUMP_PHASES.get(usize::from(tick))?)
            },
            PairMotion::Jump { tick } => (Kind::Jump, tick),
            // The stationary Primary crop matches the landing payload.
            PairMotion::Primary => (Kind::Jump, BART_LISA_JUMP_TICKS - 1),
            PairMotion::Walk { tick } => (Kind::Walk, tick),
        };
        select_entry(&self.entries, (Subject::Pair(pair), kind, tick)).copied()
    }

    pub(super) fn pair_jump(&self, pair: Pair) -> bool {
        self.entries
            .contains_key(&(Subject::Pair(pair), Kind::Jump, 0))
    }

    pub(super) fn paired_families(&self) -> [bool; 2] {
        Pair::ALL.map(|pair| {
            self.entries
                .contains_key(&(Subject::Pair(pair), Kind::Idle, 0))
        })
    }

    pub(super) fn repelled_enabled(&self) -> bool {
        self.entries
            .contains_key(&(Subject::Enemy, Kind::Repelled, 0))
    }

    pub(super) fn restore_enemy(&mut self, mut enemy: Self) {
        self.entries.append(&mut enemy.entries);
    }

    pub(super) fn restraint_enabled(&self) -> bool {
        self.entries
            .contains_key(&(Subject::Enemy, Kind::Restraint, 0))
    }

    pub(super) fn select(
        &self,
        player: Player,
        station: Station,
    ) -> Option<Art> {
        match station.maneuver {
            Maneuver::Attack { moving, tick } => {
                ground_attack_entry(&self.entries, player, moving, tick)
                    .copied()
            },
            Maneuver::AirAttack(motion) => {
                air_attack_entry(&self.entries, player, motion.tick()).copied()
            },
            Maneuver::Special { tick, .. } => select_entry(
                &self.entries,
                station_key(player, Kind::Special, tick),
            )
            .or_else(|| air_attack_entry(&self.entries, player, tick))
            .copied(),
            Maneuver::Hurt { tick } => select_entry(
                &self.entries,
                station_key(player, Kind::Hurt, tick),
            )
            .copied(),
            Maneuver::Jump(motion) => {
                jump_entry(&self.entries, player, motion).copied()
            },
            Maneuver::Restrained => None,
            Maneuver::Idle => {
                walking_entry(&self.entries, player, station.walking?).copied()
            },
        }
    }

    pub(super) fn take_enemy(&mut self) -> Self {
        Self {
            arrival: None,
            backdrop: backdrop::BackdropArt::default(),
            balloon: None,
            bonus: None,
            cemetery: None,
            entries: self
                .entries
                .extract_if(.., |key, _| key.0 == Subject::Enemy)
                .collect(),
            linked: None,
            scenery: scenery::PlacedArt::default(),
            stage_two: None,
            wrestler: wrestler::WrestlerArt::default(),
        }
    }
}

const fn station_key(player: Player, kind: Kind, tick: u8) -> Key {
    (Subject::Station(player), kind, tick)
}

fn ground_attack_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    player: Player,
    moving: bool,
    tick: u8,
) -> Option<&Value> {
    if moving
        && let Some(entry) =
            select_entry(entries, station_key(player, Kind::AttackMoving, tick))
    {
        return Some(entry);
    }
    select_entry(entries, station_key(player, Kind::Attack, tick))
}

fn walking_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    player: Player,
    motion: WalkMotion,
) -> Option<&Value> {
    let tick = walking_offset(player, motion.tick)?;
    if motion.away
        && let Some(entry) =
            select_entry(entries, station_key(player, Kind::WalkUp, tick))
    {
        return Some(entry);
    }
    select_entry(entries, station_key(player, Kind::Walk, tick))
}

fn air_attack_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    player: Player,
    tick: u8,
) -> Option<&Value> {
    select_entry(entries, station_key(player, Kind::AirAttack, tick)).or_else(
        || select_entry(entries, station_key(player, Kind::Attack, tick)),
    )
}

fn optional_complete<Value>(
    entries: &BTreeMap<Key, Value>,
    kind: Kind,
) -> bool {
    !entries.keys().any(|(subject, existing, _)| {
        matches!(subject, Subject::Station(_)) && *existing == kind
    }) || Player::ALL
        .into_iter()
        .all(|player| entries.contains_key(&station_key(player, kind, 0)))
}

fn enemy_complete<Value>(entries: &BTreeMap<Key, Value>) -> bool {
    [Kind::Attack, Kind::Defeat, Kind::Walk]
        .into_iter()
        .all(|kind| {
            optional_track_complete(entries, Subject::Enemy, kind, &[0])
        })
}

fn interaction_complete<Value>(
    entries: &BTreeMap<Key, Value>,
    kind: Kind,
    keys: &[u8],
) -> bool {
    optional_track_complete(entries, Subject::Enemy, kind, keys)
}

fn repelled_requires_restraint<Value>(entries: &BTreeMap<Key, Value>) -> bool {
    !entries.contains_key(&(Subject::Enemy, Kind::Repelled, 0))
        || entries.contains_key(&(Subject::Enemy, Kind::Restraint, 0))
}

const fn repelled_phase(tick: u8) -> u8 {
    match tick {
        0..=28 => tick,
        29..=33 | 42..=46 => 29,
        34..=41 | 47..=48 => 34,
        49..=55 | 63..=69 => 49,
        56..=62 | 70..=76 => 56,
        77..=82 | 89..=90 => 77,
        _ => 83,
    }
}

fn room_jump_keys(player: Player) -> [(Kind, u8); 3] {
    let descent = if player == Player::Bart {
        3
    } else {
        4
    };
    [(Kind::Jump, 0), (Kind::Lift, 0), (Kind::Jump, descent)]
}

fn room_jump_sprites(entries: &PoseEntries) -> cemetery::AirSprites {
    Player::ALL.map(|player| {
        room_jump_keys(player).map(|(kind, tick)| {
            entries.get(&station_key(player, kind, tick)).copied()
        })
    })
}

fn room_air_entries_complete(entries: &BTreeMap<Key, ContentHash>) -> bool {
    Player::ALL.into_iter().all(|player| {
        cemetery::air_attack_keys(player)
            .into_iter()
            .flatten()
            .all(|tick| {
                entries.contains_key(&station_key(
                    player,
                    Kind::AirAttack,
                    tick,
                ))
            })
    })
}

fn room_special_entries_complete(entries: &BTreeMap<Key, ContentHash>) -> bool {
    Player::ALL.into_iter().all(|player| {
        cemetery::special_keys(player).into_iter().all(|tick| {
            entries.contains_key(&station_key(player, Kind::Special, tick))
        })
    })
}

fn room_source((subject, kind, tick): Key) -> bool {
    let Subject::Station(player) = subject else {
        return false;
    };
    match kind {
        Kind::AirAttack => {
            cemetery::air_attack_keys(player).contains(&Some(tick))
        },
        Kind::Jump | Kind::Lift => {
            room_jump_keys(player).contains(&(kind, tick))
        },
        Kind::Special => cemetery::special_keys(player).contains(&tick),
        Kind::Attack
        | Kind::AttackMoving
        | Kind::Defeat
        | Kind::Hurt
        | Kind::Idle
        | Kind::Repelled
        | Kind::Restraint
        | Kind::Walk
        | Kind::WalkUp => false,
    }
}

fn room_sprites(
    entries: &PoseEntries,
    sources: Vec<cemetery::shadow::Source>,
) -> cemetery::RoomSprites {
    cemetery::RoomSprites {
        air_attacks: Player::ALL.map(|player| {
            cemetery::air_attack_keys(player).map(|tick| {
                entries
                    .get(&station_key(player, Kind::AirAttack, tick?))
                    .copied()
            })
        }),
        jumps: room_jump_sprites(entries),
        sources,
        specials: Player::ALL.map(|player| {
            cemetery::special_keys(player).map(|tick| {
                entries
                    .get(&station_key(player, Kind::Special, tick))
                    .copied()
            })
        }),
    }
}

fn jump_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    player: Player,
    motion: JumpMotion,
) -> Option<&Value> {
    if let Some(lift) = entries.get(&station_key(player, Kind::Lift, 0)) {
        if motion.opening_pose() {
            return entries.get(&station_key(player, Kind::Jump, 0));
        }
        if motion.raised_pose() {
            return Some(lift);
        }
    }
    select_entry(entries, station_key(player, Kind::Jump, motion.pose_tick()))
}

fn pair_complete<Value>(entries: &BTreeMap<Key, Value>) -> bool {
    Pair::ALL.into_iter().all(|pair| {
        let subject = Subject::Pair(pair);
        let jump_keys: &[u8] = match pair {
            Pair::BartLisa => &[0, 4, 28],
            Pair::MargeHomer => &MARGE_HOMER_JUMP_KEYS,
        };
        let jump_complete =
            optional_track_complete(entries, subject, Kind::Jump, jump_keys);
        jump_complete
            && (!entries.keys().any(|(existing, _, _)| *existing == subject)
                || [Kind::Idle, Kind::Walk]
                    .into_iter()
                    .all(|kind| entries.contains_key(&(subject, kind, 0))))
    })
}

fn parse_stage_two(
    line: &str,
    selected: Option<[ContentHash; 2]>,
) -> Result<[ContentHash; 2], RenderError> {
    let mut fields = line.split_whitespace();
    let (Some("stage-two"), Some(base), Some(strip), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(failure("invalid stage-two record"));
    };
    if selected.is_some() || base == strip {
        return Err(failure("duplicate stage-two record or role"));
    }
    Ok([
        base.parse().map_err(failure)?,
        strip.parse().map_err(failure)?,
    ])
}

fn parse_single_bundle(
    role: &str,
    line: &str,
    selected: Option<ContentHash>,
) -> Result<ContentHash, RenderError> {
    let mut fields = line.split_whitespace();
    let (Some(kind), Some(digest), None) =
        (fields.next(), fields.next(), fields.next())
    else {
        return Err(failure("invalid single-bundle record"));
    };
    if kind != role || selected.is_some() {
        return Err(failure("unexpected or duplicate single-bundle record"));
    }
    digest.parse().map_err(failure)
}

fn parse_line(line: &str) -> Result<Entry, RenderError> {
    let [name, action, elapsed, digest] =
        <[&str; 4]>::try_from(line.split_whitespace().collect::<Vec<_>>())
            .map_err(|_error| failure("expected NAME ACTION TICK SHA256"))?;
    let subject = match name {
        "marge" => Subject::Station(Player::Marge),
        "homer" => Subject::Station(Player::Homer),
        "bart" => Subject::Station(Player::Bart),
        "lisa" => Subject::Station(Player::Lisa),
        "enemy" => Subject::Enemy,
        "marge-homer" => Subject::Pair(Pair::MargeHomer),
        "bart-lisa" => Subject::Pair(Pair::BartLisa),
        _ => return Err(failure("unknown pose subject")),
    };
    let kind = match action {
        "air-attack" => Kind::AirAttack,
        "attack" => Kind::Attack,
        "attack-moving" => Kind::AttackMoving,
        "defeat" => Kind::Defeat,
        "hurt" => Kind::Hurt,
        "idle" => Kind::Idle,
        "jump" => Kind::Jump,
        "jump-lift" => Kind::Lift,
        "repelled" => Kind::Repelled,
        "restraint" => Kind::Restraint,
        "special" => Kind::Special,
        "walk" => Kind::Walk,
        "walk-up" => Kind::WalkUp,
        _ => return Err(failure("unknown pose maneuver")),
    };
    let tick = elapsed.parse::<u8>().map_err(failure)?;
    let duration = match (subject, kind) {
        (Subject::Station(player), _) => kind.duration(player),
        (Subject::Enemy, Kind::Attack) => ENEMY_ATTACK_TICKS,
        (Subject::Enemy, Kind::Defeat) => ENEMY_DEFEAT_TICKS,
        (Subject::Enemy, Kind::Walk) => ENEMY_WALK_TICKS,
        (Subject::Enemy, Kind::Repelled) if REPELLED_KEYS.contains(&tick) => 84,
        (Subject::Enemy, Kind::Restraint) if RESTRAINT_KEYS.contains(&tick) => {
            46
        },
        (Subject::Pair(pair), Kind::Idle) => pair.idle_end().saturating_add(1),
        (Subject::Pair(pair), Kind::Walk) => pair.walk_duration(),
        (Subject::Pair(Pair::BartLisa), Kind::Jump) => BART_LISA_JUMP_TICKS,
        (Subject::Pair(Pair::MargeHomer), Kind::Jump)
            if MARGE_HOMER_JUMP_KEYS.contains(&tick) =>
        {
            BART_LISA_JUMP_TICKS
        },
        (Subject::Pair(_), _) => {
            return Err(failure("unknown pair pose maneuver"));
        },
        (Subject::Enemy, _) => {
            return Err(failure("unknown enemy pose maneuver"));
        },
    };
    if tick >= duration {
        return Err(failure("pose tick outside maneuver"));
    }
    Ok(((subject, kind, tick), digest.parse().map_err(failure)?))
}

const fn walk_duration(player: Player) -> u8 {
    match player {
        Player::Marge => 42,
        Player::Homer | Player::Lisa => 40,
        Player::Bart => 25,
    }
}

pub(super) fn walking_offset(player: Player, elapsed: u64) -> Option<u8> {
    cycle_tick(elapsed, walk_duration(player))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    const HASH: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn scenic_fixture() -> String {
        let mut text = String::new();
        for tick in (8u16..=328).step_by(8) {
            assert!(writeln!(text, "scenic-pair {tick} 248 52 {HASH}").is_ok());
        }
        text
    }

    fn stationary_fixture() -> String {
        format!(
            "scenic-stationary 0 804 81 {HASH}\n\
             scenic-stationary 1 804 82 {HASH}\n\
             scenic-stationary 2 804 81 {HASH}\n"
        )
    }

    fn complete() -> String {
        ["marge", "homer", "bart", "lisa"]
            .into_iter()
            .flat_map(|name| {
                ["attack", "jump"]
                    .map(|kind| format!("{name} {kind} 0 {HASH}\n"))
            })
            .collect()
    }

    #[test]
    fn enemy_attack_requires_tick_zero_and_a_bounded_sequence() {
        let base = complete();
        assert!(
            Catalogue::parse(&format!("{base}enemy attack 3 {HASH}")).is_err()
        );
        let opening = format!("{base}enemy attack 0 {HASH}\n");
        assert!(Catalogue::parse(&opening).is_ok());
        assert!(
            Catalogue::parse(&format!("{opening}enemy attack 33 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{opening}enemy attack 34 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{opening}enemy jump 0 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{opening}enemy attack 0 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn enemy_attack_selects_latest_pose_without_using_station_art() {
        let entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Attack, 0), "marge"),
            ((Subject::Enemy, Kind::Attack, 0), "windup"),
            ((Subject::Enemy, Kind::Attack, 12), "strike"),
            ((Subject::Enemy, Kind::Attack, 32), "recover"),
        ]);
        for (tick, pose) in [
            (0, "windup"),
            (11, "windup"),
            (12, "strike"),
            (31, "strike"),
            (33, "recover"),
        ] {
            assert_eq!(
                select_entry(&entries, (Subject::Enemy, Kind::Attack, tick)),
                Some(&pose)
            );
        }
        assert_eq!(
            select_entry(
                &entries,
                station_key(Player::Marge, Kind::Attack, 33)
            ),
            Some(&"marge")
        );
    }

    #[test]
    fn optional_enemy_walk_is_independent_of_station_and_punch_sequences() {
        let base = complete();
        let walk = format!("{base}enemy walk 0 {HASH}\n");
        assert!(Catalogue::parse(&walk).is_ok());
        assert!(
            Catalogue::parse(&format!("{walk}enemy walk 47 {HASH}")).is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{walk}enemy walk 48 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}enemy walk 6 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{walk}enemy attack 3 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{walk}marge walk 0 {HASH}")).is_err()
        );
    }

    #[test]
    fn optional_enemy_defeat_has_its_own_bounded_timeline() {
        let base = complete();
        let defeat = format!("{base}enemy defeat 0 {HASH}\n");
        assert!(Catalogue::parse(&defeat).is_ok());
        assert!(
            Catalogue::parse(&format!("{defeat}enemy defeat 65 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{defeat}enemy defeat 66 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}enemy defeat 7 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}marge defeat 0 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{defeat}enemy defeat 0 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn optional_hurt_art_requires_every_station_and_respects_duration() {
        let base = complete();
        let hurt = format!(
            "marge hurt 0 {HASH}\n\
             homer hurt 0 {HASH}\n\
             bart hurt 0 {HASH}\n\
             lisa hurt 0 {HASH}\n"
        );
        assert!(Catalogue::parse(&base).is_ok());
        assert!(
            Catalogue::parse(&format!("{base}marge hurt 0 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}{hurt}bart hurt 16 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{base}{hurt}bart hurt 17 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}{hurt}lisa hurt 14 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn hurt_pose_holds_until_the_next_observed_offset() {
        let entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Hurt, 0), "first"),
            (station_key(Player::Marge, Kind::Hurt, 6), "second"),
            (station_key(Player::Bart, Kind::Hurt, 0), "opening"),
            (station_key(Player::Bart, Kind::Hurt, 4), "middle"),
            (station_key(Player::Bart, Kind::Hurt, 9), "last"),
        ]);
        for (player, tick, expected) in [
            (Player::Marge, 5, Some(&"first")),
            (Player::Marge, 6, Some(&"second")),
            (Player::Bart, 3, Some(&"opening")),
            (Player::Bart, 8, Some(&"middle")),
            (Player::Bart, 9, Some(&"last")),
            (Player::Lisa, 0, None),
        ] {
            assert_eq!(
                select_entry(&entries, station_key(player, Kind::Hurt, tick)),
                expected
            );
        }
    }

    #[test]
    fn walking_wraps_at_observed_cycle_lengths() {
        for (player, period) in [
            (Player::Marge, 42),
            (Player::Homer, 40),
            (Player::Bart, 25),
            (Player::Lisa, 40),
        ] {
            assert_eq!(
                walking_offset(player, period - 1),
                u8::try_from(period - 1).ok()
            );
            assert_eq!(walking_offset(player, period), Some(0));
            assert_eq!(walking_offset(player, period * 2 + 1), Some(1));
        }
    }

    #[test]
    fn moving_ground_pose_is_optional_and_does_not_replace_neutral() {
        let entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Attack, 0), "neutral"),
            (station_key(Player::Marge, Kind::AttackMoving, 0), "moving"),
            (
                station_key(Player::Marge, Kind::AttackMoving, 4),
                "moving-late",
            ),
            (station_key(Player::Bart, Kind::Attack, 0), "bart"),
        ]);
        assert_eq!(
            ground_attack_entry(&entries, Player::Marge, true, 5),
            Some(&"moving-late")
        );
        assert_eq!(
            ground_attack_entry(&entries, Player::Marge, false, 5),
            Some(&"neutral")
        );
        assert_eq!(
            ground_attack_entry(&entries, Player::Bart, true, 5),
            Some(&"bart")
        );
    }

    #[test]
    fn optional_moving_attack_requires_every_station_and_valid_ticks() {
        let base = complete();
        let moving = format!(
            "marge attack-moving 0 {HASH}\n\
             homer attack-moving 0 {HASH}\n\
             bart attack-moving 0 {HASH}\n\
             lisa attack-moving 0 {HASH}\n"
        );
        assert!(
            Catalogue::parse(&format!("{base}marge attack-moving 0 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!(
                "{base}{moving}bart attack-moving 17 {HASH}"
            ))
            .is_ok()
        );
        assert!(
            Catalogue::parse(&format!(
                "{base}{moving}marge attack-moving 17 {HASH}"
            ))
            .is_err()
        );
    }

    #[test]
    fn optional_upward_walk_requires_each_station_and_valid_offsets() {
        let base = complete();
        let up = format!(
            "marge walk-up 0 {HASH}\n\
             homer walk-up 0 {HASH}\n\
             bart walk-up 0 {HASH}\n\
             lisa walk-up 0 {HASH}\n"
        );
        assert!(
            Catalogue::parse(&format!("{base}marge walk-up 0 {HASH}")).is_err()
        );
        assert_eq!(
            Catalogue::parse(&format!("{base}{up}"))
                .map(|value| value.count())
                .ok(),
            Some(12)
        );
        assert!(
            Catalogue::parse(&format!("{base}{up}bart walk-up 24 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{base}{up}bart walk-up 25 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn walking_selects_direction_at_shared_phase_and_falls_back() {
        let entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Walk, 0), "forward"),
            (station_key(Player::Marge, Kind::Walk, 5), "forward-late"),
            (station_key(Player::Marge, Kind::WalkUp, 0), "away"),
            (station_key(Player::Marge, Kind::WalkUp, 5), "away-late"),
            (station_key(Player::Bart, Kind::Walk, 0), "bart"),
        ]);
        assert_eq!(
            walking_entry(&entries, Player::Marge, WalkMotion {
                away: true,
                tick: 48
            }),
            Some(&"away-late")
        );
        assert_eq!(
            walking_entry(&entries, Player::Marge, WalkMotion {
                away: false,
                tick: 48
            }),
            Some(&"forward-late")
        );
        assert_eq!(
            walking_entry(&entries, Player::Marge, WalkMotion {
                away: true,
                tick: 42
            }),
            Some(&"away")
        );
        assert_eq!(
            walking_entry(&entries, Player::Bart, WalkMotion {
                away: true,
                tick: 5
            }),
            Some(&"bart")
        );
    }

    #[test]
    fn pair_catalogue_requires_both_sequences_and_bounds_ticks() {
        let base = complete();
        assert!(
            Catalogue::parse(&format!("{base}marge-homer idle 0 {HASH}"))
                .is_err()
        );
        let pair = format!(
            "{base}marge-homer idle 0 {HASH}\n\
             marge-homer walk 0 {HASH}\n"
        );
        assert!(Catalogue::parse(&pair).is_ok());
        assert!(
            Catalogue::parse(&format!("{pair}marge-homer idle 13 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{pair}marge-homer idle 14 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{pair}marge-homer walk 32 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{pair}bart-lisa attack 0 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{pair}marge idle 0 {HASH}")).is_err()
        );
    }

    #[test]
    fn second_pair_catalogue_bounds_ticks() {
        let pair = complete();
        let both = format!(
            "{pair}bart-lisa idle 0 {HASH}\n\
             bart-lisa walk 0 {HASH}\n"
        );
        assert!(
            Catalogue::parse(&format!("{both}bart-lisa idle 9 {HASH}")).is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{both}bart-lisa idle 10 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{both}bart-lisa walk 23 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{both}bart-lisa walk 24 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn paired_jump_requires_all_observed_phases() {
        let base = complete();
        let pair = format!(
            "{base}bart-lisa idle 0 {HASH}\n\
             bart-lisa walk 0 {HASH}\n"
        );
        let opening = format!("{pair}bart-lisa jump 0 {HASH}\n");
        let raised = format!("{opening}bart-lisa jump 4 {HASH}\n");
        let complete = format!("{raised}bart-lisa jump 28 {HASH}\n");
        assert!(Catalogue::parse(&opening).is_err());
        assert!(Catalogue::parse(&raised).is_err());
        assert!(Catalogue::parse(&complete).is_ok());
        assert!(
            Catalogue::parse(&format!("{complete}bart-lisa jump 29 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{pair}marge-homer jump 0 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn marge_homer_jump_requires_every_reused_phase() {
        let base = complete();
        let pair = format!(
            "{base}marge-homer idle 0 {HASH}\nmarge-homer walk 0 {HASH}\n"
        );
        for missing in MARGE_HOMER_JUMP_KEYS {
            let partial = MARGE_HOMER_JUMP_KEYS
                .into_iter()
                .filter(|tick| *tick != missing)
                .fold(String::new(), |mut text, tick| {
                    use std::fmt::Write as _;
                    assert!(
                        writeln!(text, "marge-homer jump {tick} {HASH}")
                            .is_ok()
                    );
                    text
                });
            assert!(Catalogue::parse(&format!("{pair}{partial}")).is_err());
        }
        let entries = MARGE_HOMER_JUMP_KEYS.into_iter().fold(
            String::new(),
            |mut text, tick| {
                use std::fmt::Write as _;
                assert!(
                    writeln!(text, "marge-homer jump {tick} {HASH}").is_ok()
                );
                text
            },
        );
        let full = format!("{pair}{entries}");
        assert!(Catalogue::parse(&full).is_ok());
        assert!(
            Catalogue::parse(&format!("{full}marge-homer jump 4 {HASH}"))
                .is_err()
        );
        assert!(Catalogue::parse(&format!("{base}{entries}")).is_err());
    }

    fn backdrop_fixture(missing: Option<u16>) -> String {
        let mut text = String::new();
        for tick in backdrop::KEYS {
            if Some(tick) != missing {
                assert!(writeln!(text, "backdrop {tick} {HASH}").is_ok());
            }
        }
        text
    }

    #[test]
    fn backdrop_track_is_optional_complete_and_requires_long_street() {
        let base = complete();
        let track = backdrop_fixture(None);
        let parsed = Catalogue::parse(&format!("{base}{track}"));
        assert!(parsed.is_ok_and(|value| {
            value.count() == 23
                && value.backdrop_count() == 15
                && value.digests().count() == 23
                && value.admit_world(None).is_err()
        }));
        for missing in backdrop::KEYS {
            let partial = backdrop_fixture(Some(missing));
            assert!(Catalogue::parse(&format!("{base}{partial}")).is_err());
        }
        assert!(
            Catalogue::parse(&format!("{base}{track}backdrop 0 {HASH}\n"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}{track}backdrop 266 {HASH}\n"))
                .is_err()
        );
    }

    #[test]
    fn catalogue_enforces_total_pose_budget() {
        let extra: String = ["marge", "homer", "bart", "lisa"]
            .into_iter()
            .flat_map(|name| {
                (0u8..24)
                    .map(move |tick| format!("{name} walk {tick} {HASH}\n"))
            })
            .chain(
                [
                    ("walk", ENEMY_WALK_TICKS),
                    ("defeat", ENEMY_DEFEAT_TICKS),
                    ("attack", 2),
                ]
                .into_iter()
                .flat_map(|(kind, count)| {
                    (0..count).map(move |tick| {
                        format!("enemy {kind} {tick} {HASH}\n")
                    })
                }),
            )
            .collect();
        let text = format!("{}{extra}", complete());
        let scenic = scenic_fixture();
        assert_eq!(
            Catalogue::parse(&format!("{text}{scenic}"))
                .map(|value| value.count())
                .ok(),
            Some(MAX_POSES.saturating_add(41))
        );
        let stationary = stationary_fixture();
        let late = backdrop_fixture(None);
        assert_eq!(
            Catalogue::parse(&format!("{text}{scenic}{stationary}{late}"))
                .map(|value| value.count())
                .ok(),
            Some(MAX_POSES.saturating_add(59))
        );
        assert_eq!(
            Catalogue::parse(&format!("{text}{scenic}{stationary}"))
                .map(|value| (value.count(), value.scenic_sprite_count()))
                .ok(),
            Some((MAX_POSES.saturating_add(44), 2))
        );
        assert_eq!(
            Catalogue::parse(&text).map(|value| value.count()).ok(),
            Some(MAX_POSES)
        );
        assert!(
            Catalogue::parse(&format!("{text}marge walk 24 {HASH}\n")).is_err()
        );
    }

    #[test]
    fn scenic_track_preserves_the_regular_pose_budget() {
        let scenic = scenic_fixture();
        let text = format!("{}{scenic}", complete());
        let result = Catalogue::parse(&text);
        assert!(result.is_ok());
        let Ok(catalogue) = result else {
            return;
        };
        assert_eq!(catalogue.count(), 49);
        assert_eq!(catalogue.digests().count(), 49);
        assert!(catalogue.admit_world(None).is_err());
        assert!(
            Catalogue::parse(&format!(
                "{}scenic-pair 8 248 52 {HASH}",
                complete()
            ))
            .is_err()
        );
    }

    #[test]
    fn stationary_catalogue_is_optional_but_complete() {
        let text = format!("{}{}", complete(), stationary_fixture());
        let result = Catalogue::parse(&text);
        assert_eq!(
            result
                .as_ref()
                .map(|value| (value.count(), value.scenic_sprite_count()))
                .ok(),
            Some((11, 1))
        );
        assert!(result.is_ok_and(|value| value.admit_world(None).is_err()));
        assert!(
            Catalogue::parse(&format!(
                "{}scenic-stationary 0 804 81 {HASH}",
                complete()
            ))
            .is_err()
        );
    }

    #[test]
    fn memory_pose_text_has_the_same_byte_budget_as_local_text() {
        let valid = complete();
        let padded = format!(
            "{valid}\n{}",
            " ".repeat(
                usize::try_from(MAX_CATALOGUE_BYTES.get()).unwrap_or_default()
            )
        );
        assert!(Catalogue::parse(&valid).is_ok());
        assert!(Catalogue::parse(&padded).is_err());
    }

    #[test]
    fn selects_latest_pose_without_crossing_sequence() {
        let entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Attack, 0), "first"),
            (station_key(Player::Marge, Kind::Attack, 3), "second"),
            (station_key(Player::Marge, Kind::Jump, 0), "air"),
            (station_key(Player::Homer, Kind::Attack, 0), "homer"),
        ]);
        assert_eq!(
            select_entry(&entries, station_key(Player::Marge, Kind::Attack, 2)),
            Some(&"first")
        );
        assert_eq!(
            select_entry(&entries, station_key(Player::Marge, Kind::Attack, 3)),
            Some(&"second")
        );
        assert_eq!(
            select_entry(
                &entries,
                station_key(Player::Marge, Kind::Attack, 16)
            ),
            Some(&"second")
        );
        assert_eq!(
            select_entry(&entries, station_key(Player::Marge, Kind::Jump, 16)),
            Some(&"air")
        );
        assert_eq!(
            select_entry(&entries, station_key(Player::Homer, Kind::Jump, 16)),
            None
        );
        assert_eq!(
            select_entry(&entries, station_key(Player::Bart, Kind::Attack, 16)),
            None
        );
    }

    #[test]
    fn repelled_catalogue_requires_restraint_and_complete_recovery() {
        use std::fmt::Write as _;
        let recovery =
            REPELLED_KEYS
                .into_iter()
                .fold(String::new(), |mut text, tick| {
                    assert!(
                        writeln!(text, "enemy repelled {tick} {HASH}").is_ok()
                    );
                    text
                });
        let base = complete();
        assert!(Catalogue::parse(&format!("{base}{recovery}")).is_err());
        let restraint =
            RESTRAINT_KEYS
                .into_iter()
                .fold(String::new(), |mut text, tick| {
                    assert!(
                        writeln!(text, "enemy restraint {tick} {HASH}").is_ok()
                    );
                    text
                });
        let full = format!("{base}{restraint}{recovery}");
        assert!(Catalogue::parse(&full).is_ok());
        assert!(
            Catalogue::parse(&format!(
                "{base}{restraint}enemy repelled 0 {HASH}"
            ))
            .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{full}enemy repelled 6 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn repelled_ground_pose_repeats_without_defeat_blink() {
        for (tick, expected) in [
            (29, 29),
            (34, 34),
            (42, 29),
            (47, 34),
            (49, 49),
            (56, 56),
            (63, 49),
            (70, 56),
            (77, 77),
            (83, 83),
            (89, 77),
            (102, 83),
        ] {
            assert_eq!(repelled_phase(tick), expected);
        }
    }

    #[test]
    fn restraint_catalogue_requires_all_observed_phases() {
        use std::fmt::Write as _;

        let base = complete();
        let mut full = base.clone();
        for tick in RESTRAINT_KEYS {
            assert!(
                Catalogue::parse(&format!(
                    "{base}enemy restraint {tick} {HASH}"
                ))
                .is_err()
            );
            assert!(writeln!(full, "enemy restraint {tick} {HASH}").is_ok());
        }
        assert!(Catalogue::parse(&full).is_ok());
        assert!(
            Catalogue::parse(&format!("{full}enemy restraint 8 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{full}marge restraint 0 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn bonus_requires_complete_wrestler_and_has_independent_atlas_budget() {
        let base = complete();
        let record = format!("bonus {HASH}\n");
        assert!(Catalogue::parse(&format!("{base}{record}")).is_err());
        let mut fighter = base;
        for name in [
            "defeat",
            "flex",
            "intro",
            "left",
            "red-flex",
            "red-left",
            "red-right",
            "rig",
            "right",
            "seated",
            "stand",
        ] {
            use std::fmt::Write as _;
            assert!(writeln!(fighter, "wrestler {name} {HASH}").is_ok());
        }
        let previous = Catalogue::parse(&fighter);
        let expanded = Catalogue::parse(&format!("{fighter}{record}"));
        assert!(previous.is_ok());
        assert!(expanded.is_ok());
        let (Ok(plain), Ok(selected)) = (previous, expanded) else {
            return;
        };
        assert_bonus_budget(&plain, &selected);
        assert_departure_catalogue(&fighter, &record, &selected);
        assert_inflated_catalogue(&fighter, &record, &selected);

        assert_stage_two_catalogue(&fighter, &record, &selected);
        assert!(
            Catalogue::parse(&format!("{fighter}{record}{record}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{fighter}bonus {HASH} extra")).is_err()
        );
    }

    fn assert_departure_catalogue(
        fighter: &str,
        bonus_record: &str,
        selected: &Catalogue,
    ) {
        let other = "c".repeat(64);
        let departure = format!("bonus-departure {other}\n");
        assert!(Catalogue::parse(&format!("{fighter}{departure}")).is_err());
        let text = format!("{fighter}{bonus_record}{departure}");
        let parsed = Catalogue::parse(&text);
        assert!(parsed.is_ok());
        if let Ok(value) = parsed {
            assert_departure_budget(selected, &value, &other);
        }
        assert!(Catalogue::parse(&format!("{text}{departure}")).is_err());
        assert!(
            Catalogue::parse(&format!(
                "{fighter}{bonus_record}bonus-departure invalid"
            ))
            .is_err()
        );
    }

    fn assert_inflated_catalogue(
        fighter: &str,
        bonus_record: &str,
        selected: &Catalogue,
    ) {
        let other = "d".repeat(64);
        let inflated = format!("bonus-inflated {other}\n");
        assert!(Catalogue::parse(&format!("{fighter}{inflated}")).is_err());
        let text = format!("{fighter}{bonus_record}{inflated}");
        let parsed = Catalogue::parse(&text);
        assert!(parsed.is_ok());
        if let Ok(value) = parsed {
            assert_eq!(value.count(), selected.count().saturating_add(1));
            assert_eq!(value.digests().count(), value.count());
            assert_eq!(
                value.atlas_count(),
                selected
                    .atlas_count()
                    .saturating_add(bonus::INFLATED_ATLAS_COUNT)
            );
            assert_eq!(
                value.atlas_bytes(),
                selected
                    .atlas_bytes()
                    .saturating_add(bonus::INFLATED_ATLAS_BYTES)
            );
        }
        assert!(Catalogue::parse(&format!("{text}{inflated}")).is_err());
    }

    fn assert_departure_budget(
        selected: &Catalogue,
        value: &Catalogue,
        other: &str,
    ) {
        assert_eq!(value.count(), selected.count().saturating_add(1));
        assert_eq!(value.digests().count(), value.count());
        assert!(
            value
                .digests()
                .any(|digest| Some(digest) == other.parse().ok())
        );
        assert_eq!(
            value.atlas_count(),
            selected
                .atlas_count()
                .saturating_add(bonus::DEPARTURE_ATLAS_COUNT)
        );
        assert_eq!(
            value.atlas_bytes(),
            selected
                .atlas_bytes()
                .saturating_add(bonus::DEPARTURE_ATLAS_BYTES)
        );
        assert_eq!(
            value.bonus_departure_sprite_count(),
            bonus::DEPARTURE_EXTRA_SPRITES
        );
    }

    fn assert_stage_two_catalogue(
        fighter: &str,
        bonus_record: &str,
        selected: &Catalogue,
    ) {
        let other = "b".repeat(64);
        let stage = format!("stage-two {HASH} {other}\n");
        assert!(Catalogue::parse(&format!("{fighter}{stage}")).is_err());
        let text = format!("{fighter}{bonus_record}{stage}");
        let parsed = Catalogue::parse(&text);
        assert!(parsed.is_ok());
        if let Ok(value) = parsed {
            assert_stage_two_budget(selected, &value, &other);
            assert_arrival_catalogue(&text, fighter, bonus_record, &value);
            assert_entrance_catalogue(&text, fighter, bonus_record, &value);
            assert_boardwalk_catalogue(&text, fighter, bonus_record, &value);
            assert_linked_catalogue(&text, fighter, bonus_record, &value);
            assert_balloon_catalogue(&text);
        }
        assert!(Catalogue::parse(&format!("{text}{stage}")).is_err());
        assert!(
            parse_stage_two(&format!("stage-two {HASH} {HASH}"), None).is_err()
        );
        assert!(parse_stage_two(&format!("stage-two {HASH}"), None).is_err());
    }

    fn assert_linked_catalogue(
        stage_text: &str,
        fighter: &str,
        bonus_record: &str,
        stage: &Catalogue,
    ) {
        let hashes: Vec<_> = ['1', '2', '3', '4', '5', '6', '7']
            .into_iter()
            .map(|digit| digit.to_string().repeat(64))
            .collect();
        let record = format!("linked-actors {}\n", hashes.join(" "));
        let full = format!("{stage_text}{record}");
        assert!(Catalogue::parse(&full).is_ok_and(|value| {
            value.count() == stage.count().saturating_add(linked::POSE_COUNT)
                && value.atlas_count()
                    == stage.atlas_count().saturating_add(linked::POSE_COUNT)
                && value.atlas_bytes()
                    == stage.atlas_bytes().saturating_add(
                        linked::POSE_COUNT.saturating_mul(65_536),
                    )
                && value.linked_sprite_count() == linked::EXTRA_SPRITES
                && value.digests().count() == value.count()
        }));
        assert!(
            Catalogue::parse(&format!("{fighter}{bonus_record}{record}"))
                .is_err()
        );
        assert!(Catalogue::parse(&format!("{full}{record}")).is_err());
        for hash in hashes {
            let partial = record.replace(&hash, "");
            assert!(
                Catalogue::parse(&format!("{stage_text}{partial}")).is_err()
            );
        }
        assert!(
            Catalogue::parse(&format!("{} extra", full.trim_end())).is_err()
        );
    }

    fn assert_balloon_budget(base: &Catalogue, value: &Catalogue) {
        assert_eq!(
            value.count(),
            base.count().saturating_add(balloon::POSE_COUNT)
        );
        assert_eq!(
            value.atlas_count(),
            base.atlas_count().saturating_add(balloon::POSE_COUNT)
        );
        assert_eq!(
            value.atlas_bytes(),
            base.atlas_bytes().saturating_add(balloon::ATLAS_BYTES)
        );
        assert_eq!(value.digests().count(), value.count());
        assert_eq!(value.balloon_sprite_count(), 1);
    }

    fn assert_balloon_catalogue(stage_text: &str) {
        let linked_hashes: Vec<_> = ['1', '2', '3', '4', '5', '6', '7']
            .into_iter()
            .map(|digit| digit.to_string().repeat(64))
            .collect();
        let linked = format!("linked-actors {}\n", linked_hashes.join(" "));
        let mut boardwalk = String::new();
        for tick in backdrop::BOARDWALK_KEYS {
            assert!(
                writeln!(boardwalk, "stage-two-backdrop {tick} {HASH}").is_ok()
            );
        }
        let complete = format!("{stage_text}{linked}{boardwalk}");
        let hashes: Vec<_> = ['8', '9', 'a', 'b']
            .into_iter()
            .map(|digit| digit.to_string().repeat(64))
            .collect();
        let record = format!("balloon-boss {}\n", hashes.join(" "));
        let baseline = Catalogue::parse(&complete);
        let selected = Catalogue::parse(&format!("{complete}{record}"));
        assert!(baseline.is_ok() && selected.is_ok());
        if let (Ok(base), Ok(value)) = (baseline, selected) {
            assert_balloon_budget(&base, &value);
        }
        assert!(Catalogue::parse(&format!("{stage_text}{record}")).is_err());
        assert!(
            Catalogue::parse(&format!("{stage_text}{linked}{record}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{complete}{record}{record}")).is_err()
        );
        assert_cemetery_catalogue(&format!("{complete}{record}"), &complete);
    }

    fn assert_boardwalk_catalogue(
        stage_text: &str,
        fighter: &str,
        bonus_record: &str,
        stage: &Catalogue,
    ) {
        let mut track = String::new();
        for tick in backdrop::BOARDWALK_KEYS {
            assert!(
                writeln!(track, "stage-two-backdrop {tick} {HASH}").is_ok()
            );
        }
        let full = format!("{stage_text}{track}");
        assert!(Catalogue::parse(&full).is_ok_and(|value| {
            value.count() == stage.count().saturating_add(7)
                && value.atlas_count() == stage.atlas_count().saturating_add(7)
                && value.atlas_bytes()
                    == stage.atlas_bytes().saturating_add(
                        7usize.saturating_mul(backdrop::ATLAS_BYTES),
                    )
                && value.digests().count() == value.count()
        }));
        assert!(
            Catalogue::parse(&format!("{fighter}{bonus_record}{track}"))
                .is_err()
        );
        for key in backdrop::BOARDWALK_KEYS {
            let partial = track
                .replace(&format!("stage-two-backdrop {key} {HASH}\n"), "");
            assert!(
                Catalogue::parse(&format!("{stage_text}{partial}")).is_err()
            );
        }
        assert!(
            Catalogue::parse(&format!("{full}stage-two-backdrop 0 {HASH}\n"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{full}stage-two-backdrop 31 {HASH}\n"))
                .is_err()
        );
    }

    fn assert_cemetery_budget(base: &Catalogue, selected: &Catalogue) {
        assert_eq!(
            selected.count(),
            base.count().saturating_add(cemetery::ROLE_COUNT)
        );
        assert_eq!(
            selected.atlas_count(),
            base.atlas_count().saturating_add(cemetery::ROLE_COUNT)
        );
        assert_eq!(
            selected.atlas_bytes(),
            base.atlas_bytes().saturating_add(cemetery::ATLAS_BYTES)
        );
        assert_eq!(
            selected.scenic_sprite_count(),
            base.scenic_sprite_count()
                .saturating_add(cemetery::EXTRA_SPRITES)
        );
        assert_eq!(selected.digests().count(), selected.count());
    }

    fn assert_animated_cemetery_catalogue(with_boss: &str, record: &str) {
        let animated: Vec<_> = (0u32..28)
            .map(|index| format!("{:064x}", index.saturating_add(100)))
            .collect();
        let extended =
            format!("{} {}\n", record.trim_end(), animated.join(" "));
        assert_cemetery_layer_catalogue(with_boss, record, &extended);
        let selected = Catalogue::parse(&format!("{with_boss}{extended}"));
        assert!(selected.is_ok());
        if let (Ok(baseline), Ok(value)) =
            (Catalogue::parse(with_boss), selected)
        {
            assert_eq!(value.count(), baseline.count().saturating_add(44));
            assert_eq!(
                value.atlas_count(),
                baseline.atlas_count().saturating_add(44),
            );
            assert_eq!(
                value.atlas_bytes(),
                baseline.atlas_bytes().saturating_add(1_888_656),
            );
            assert_eq!(value.digests().count(), value.count());
            assert_eq!(
                value.scenic_sprite_count(),
                baseline
                    .scenic_sprite_count()
                    .saturating_add(cemetery::EXTRA_SPRITES),
            );
        }
        let partial = animated.iter().take(27).cloned().collect::<Vec<_>>();
        assert!(
            Catalogue::parse(&format!(
                "{with_boss}{} {}\n",
                record.trim_end(),
                partial.join(" ")
            ))
            .is_err()
        );
        let repeated = animated.first().map_or_else(String::new, |first| {
            let duplicate = format!(" {first}").repeat(28);
            format!("{}{duplicate}\n", record.trim_end())
        });
        assert!(Catalogue::parse(&format!("{with_boss}{repeated}")).is_err());
    }

    fn assert_cemetery_layer_budgets(base_text: &str, selected_text: &str) {
        let baseline = Catalogue::parse(base_text);
        let parsed = Catalogue::parse(selected_text);
        assert!(baseline.is_ok() && parsed.is_ok());
        if let (Ok(base), Ok(selected)) = (baseline, parsed) {
            assert_eq!(selected.count(), base.count().saturating_add(4));
            assert_eq!(selected.digests().count(), selected.count());
            assert_eq!(
                selected.atlas_count(),
                base.atlas_count().saturating_add(7)
            );
            assert_eq!(
                selected.atlas_bytes(),
                base.atlas_bytes().saturating_add(3_145_732)
            );
            assert_eq!(
                selected.scenic_sprite_count(),
                base.scenic_sprite_count().saturating_add(3)
            );
        }
    }

    fn assert_cemetery_walking_catalogue(
        with_boss: &str,
        animated: &str,
        layers: &str,
    ) {
        let walking = (300u32..360)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let marge_first = animated.replacen(
            "cemetery-entrance homer",
            "cemetery-entrance marge",
            1,
        );
        let full = format!("{} {walking}\n", marge_first.trim_end());
        assert!(Catalogue::parse(&format!("{with_boss}{full}")).is_err());
        let baseline =
            Catalogue::parse(&format!("{with_boss}{marge_first}{layers}"));
        let selected = Catalogue::parse(&format!("{with_boss}{full}{layers}"));
        assert!(baseline.is_ok() && selected.is_ok());
        if let (Ok(base), Ok(value)) = (baseline, selected) {
            assert_eq!(value.count(), base.count().saturating_add(60));
            assert_eq!(
                value.atlas_count(),
                base.atlas_count().saturating_add(61)
            );
            assert_eq!(
                value.atlas_bytes(),
                base.atlas_bytes()
                    .saturating_add(452_452)
                    .saturating_add(cemetery::shadow::ATLAS_BYTES)
            );
            assert_eq!(
                value.scenic_sprite_count(),
                base.scenic_sprite_count().saturating_add(1)
            );
            assert_eq!(value.digests().count(), value.count());
        }
        assert_cemetery_walking_rejections(with_boss, &full, layers);
        assert_cemetery_marker_catalogue(with_boss, &full, layers);
    }

    fn assert_cemetery_marker_catalogue(
        with_boss: &str,
        walking: &str,
        layers: &str,
    ) {
        // Four keys can refer to a single canonical marker bundle.
        let hash = format!("{:064x}", 400u32);
        let markers = format!(" {hash}").repeat(4);
        let full = format!("{}{markers}\n", walking.trim_end());
        let baseline =
            Catalogue::parse(&format!("{with_boss}{walking}{layers}"));
        let selected = Catalogue::parse(&format!("{with_boss}{full}{layers}"));
        assert!(baseline.is_ok() && selected.is_ok());
        if let (Ok(base), Ok(value)) = (baseline, selected) {
            assert_eq!(value.count(), base.count().saturating_add(4));
            assert_eq!(
                value.atlas_count(),
                base.atlas_count().saturating_add(4)
            );
            assert_eq!(
                value.atlas_bytes(),
                base.atlas_bytes().saturating_add(15_872)
            );
            assert_eq!(
                value.scenic_sprite_count(),
                base.scenic_sprite_count().saturating_add(4)
            );
            assert_eq!(value.digests().count(), value.count());
        }
        assert!(Catalogue::parse(&format!("{with_boss}{full}")).is_err());
        assert_cemetery_walking_rejections(with_boss, &full, layers);
        assert_partial_marker_rejections(with_boss, walking, layers, &hash);
        assert_cemetery_special_catalogue(with_boss, &full, layers);
    }

    fn cemetery_special_fixture() -> String {
        let mut reused = String::new();
        for (name, player) in ["marge", "homer", "bart", "lisa"]
            .into_iter()
            .zip(Player::ALL)
        {
            for tick in cemetery::special_keys(player) {
                assert!(
                    writeln!(reused, "{name} special {tick} {HASH}").is_ok()
                );
            }
        }
        reused
    }

    fn assert_cemetery_special_catalogue(
        with_boss: &str,
        markers: &str,
        layers: &str,
    ) {
        let reused = cemetery_special_fixture();
        let suffix = (500u32..512)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let full = format!("{} {suffix}\n", markers.trim_end());
        let baseline =
            Catalogue::parse(&format!("{with_boss}{markers}{layers}{reused}"));
        let selected =
            Catalogue::parse(&format!("{with_boss}{full}{layers}{reused}"));
        assert!(baseline.is_ok() && selected.is_ok());
        if let (Ok(base), Ok(value)) = (baseline, selected) {
            assert_eq!(value.count(), base.count().saturating_add(12));
            assert_eq!(
                value.atlas_count(),
                base.atlas_count().saturating_add(12)
            );
            assert_eq!(
                value.atlas_bytes(),
                base.atlas_bytes().saturating_add(122_708)
            );
            assert_eq!(value.scenic_sprite_count(), base.scenic_sprite_count());
            assert_eq!(value.digests().count(), value.count());
        }
        assert!(
            Catalogue::parse(&format!("{with_boss}{full}{layers}")).is_err()
        );
        let partial = reused.lines().take(11).collect::<Vec<_>>().join("\n");
        assert!(
            Catalogue::parse(&format!("{with_boss}{full}{layers}{partial}"))
                .is_err()
        );
        assert_cemetery_walking_rejections(with_boss, &full, layers);
        assert_cemetery_ground_catalogue(with_boss, &full, layers, &reused);
    }

    fn assert_cemetery_ground_catalogue(
        with_boss: &str,
        specials: &str,
        layers: &str,
        reused: &str,
    ) {
        let descent = format!("bart jump 3 {HASH}\n");
        let suffix = (600u32..634)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let full = format!("{} {suffix}\n", specials.trim_end());
        let baseline = Catalogue::parse(&format!(
            "{with_boss}{specials}{layers}{reused}{descent}"
        ));
        let selected = Catalogue::parse(&format!(
            "{with_boss}{full}{layers}{reused}{descent}"
        ));
        assert!(baseline.is_ok() && selected.is_ok());
        if let (Ok(base), Ok(value)) = (baseline, selected) {
            assert_eq!(value.count(), base.count().saturating_add(34));
            assert_eq!(
                value.atlas_count(),
                base.atlas_count().saturating_add(34)
            );
            assert_eq!(
                value.atlas_bytes(),
                base.atlas_bytes().saturating_add(404_236)
            );
            assert_eq!(value.scenic_sprite_count(), base.scenic_sprite_count());
            assert_eq!(value.digests().count(), value.count());
        }
        assert!(
            Catalogue::parse(&format!("{with_boss}{full}{layers}{reused}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{with_boss}{full}{layers}{descent}"))
                .is_err()
        );
        let base = format!("{with_boss}{reused}{descent}");
        assert_cemetery_walking_rejections(&base, &full, layers);
        assert_cemetery_air_catalogue(&base, &full, layers);
    }

    fn assert_cemetery_air_catalogue(base: &str, ground: &str, layers: &str) {
        let keys = Player::ALL
            .into_iter()
            .flat_map(|player| {
                cemetery::air_attack_keys(player).into_iter().flatten().map(
                    move |tick| {
                        format!(
                            "{} air-attack {tick} {HASH}\n",
                            ["marge", "homer", "bart", "lisa"]
                                .get(player.index())
                                .copied()
                                .unwrap_or_default()
                        )
                    },
                )
            })
            .collect::<String>();
        let air =
            format!("{} {:064x} {:064x}\n", ground.trim_end(), 634u32, 635u32);
        let selected = Catalogue::parse(&format!("{base}{keys}{air}{layers}"));
        let baseline =
            Catalogue::parse(&format!("{base}{keys}{ground}{layers}"));
        assert!(selected.is_ok() && baseline.is_ok());
        if let (Ok(value), Ok(previous)) = (selected, baseline) {
            assert_eq!(value.count(), previous.count().saturating_add(2));
            assert_eq!(
                value.atlas_bytes(),
                previous.atlas_bytes().saturating_add(14_700)
            );
            assert_eq!(
                value.scenic_sprite_count(),
                previous.scenic_sprite_count()
            );
        }
        assert!(Catalogue::parse(&format!("{base}{air}{layers}")).is_err());
        let partial = format!("{} {:064x}\n", ground.trim_end(), 634u32);
        assert!(
            Catalogue::parse(&format!("{base}{keys}{partial}{layers}"))
                .is_err()
        );
    }

    fn assert_partial_marker_rejections(
        with_boss: &str,
        walking: &str,
        layers: &str,
        hash: &str,
    ) {
        for count in 1..4 {
            let partial = format!(
                "{}{}\n",
                walking.trim_end(),
                format!(" {hash}").repeat(count)
            );
            assert!(
                Catalogue::parse(&format!("{with_boss}{partial}{layers}"))
                    .is_err()
            );
        }
    }

    fn assert_cemetery_walking_rejections(
        with_boss: &str,
        full: &str,
        layers: &str,
    ) {
        let wrong_rank = full.replacen(
            "cemetery-entrance marge",
            "cemetery-entrance homer",
            1,
        );
        assert!(
            Catalogue::parse(&format!("{with_boss}{wrong_rank}{layers}"))
                .is_err()
        );
        let incomplete = full
            .trim_end()
            .rsplit_once(' ')
            .map_or("", |(prefix, _)| prefix);
        assert!(
            Catalogue::parse(&format!("{with_boss}{incomplete}\n{layers}"))
                .is_err()
        );
    }

    fn assert_cemetery_layer_catalogue(
        with_boss: &str,
        sparse: &str,
        animated: &str,
    ) {
        let hashes = (200u32..204)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let layers = format!("cemetery-room-layers {hashes}\n");
        assert_cemetery_walking_catalogue(with_boss, animated, &layers);
        assert!(Catalogue::parse(&format!("{with_boss}{layers}")).is_err());
        assert!(
            Catalogue::parse(&format!("{with_boss}{sparse}{layers}")).is_err()
        );
        let full = format!("{with_boss}{animated}");
        assert!(Catalogue::parse(&format!("{full}{layers}{layers}")).is_err());
        assert_cemetery_layer_budgets(&full, &format!("{layers}{full}"));
    }

    fn assert_cemetery_catalogue(with_boss: &str, without_boss: &str) {
        let hashes: Vec<_> = "0123456789abcdef"
            .chars()
            .map(|digit| digit.to_string().repeat(64))
            .collect();
        let record = format!("cemetery-entrance homer {}\n", hashes.join(" "));
        assert_animated_cemetery_catalogue(with_boss, &record);
        let base = Catalogue::parse(with_boss);
        let selected = Catalogue::parse(&format!("{with_boss}{record}"));
        assert!(base.is_ok() && selected.is_ok());
        if let (Ok(baseline), Ok(value)) = (base, selected) {
            assert_cemetery_budget(&baseline, &value);
        }
        assert!(Catalogue::parse(&format!("{without_boss}{record}")).is_err());
        assert!(
            Catalogue::parse(&format!("{with_boss}{record}{record}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!(
                "{with_boss}{}",
                record.replace("homer", "unknown")
            ))
            .is_err()
        );
        assert!(
            Catalogue::parse(&format!(
                "{with_boss}{}",
                record.replace(&"f".repeat(64), "")
            ))
            .is_err()
        );
        assert!(
            Catalogue::parse(&format!(
                "{with_boss}{}",
                record.replace(&"1".repeat(64), &"0".repeat(64))
            ))
            .is_err()
        );
    }

    fn assert_arrival_catalogue(
        stage_text: &str,
        fighter: &str,
        bonus_record: &str,
        stage: &Catalogue,
    ) {
        let record = format!("arrival {HASH}\n");
        assert!(
            Catalogue::parse(&format!("{fighter}{bonus_record}{record}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{stage_text}{record}{record}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{stage_text}arrival {HASH} extra"))
                .is_err()
        );
        let parsed = Catalogue::parse(&format!("{stage_text}{record}"));
        assert!(parsed.is_ok());
        let Ok(selected) = parsed else {
            return;
        };
        assert_arrival_budget(stage, &selected);
        assert_marker_catalogue(stage_text, &record, &selected);
    }

    fn assert_arrival_budget(stage: &Catalogue, selected: &Catalogue) {
        assert!(selected.has_arrival());
        assert_eq!(selected.count(), stage.count().saturating_add(1));
        assert_eq!(selected.digests().count(), selected.count());
        assert_eq!(
            selected.atlas_bytes(),
            stage.atlas_bytes().saturating_add(arrival::ATLAS_BYTES)
        );
        assert_eq!(
            selected.atlas_count(),
            stage.atlas_count().saturating_add(arrival::ATLAS_COUNT)
        );
        assert_eq!(
            selected.scenic_sprite_count(),
            stage
                .scenic_sprite_count()
                .saturating_add(arrival::EXTRA_SPRITES)
        );
    }

    fn assert_marker_catalogue(
        stage_text: &str,
        arrival_record: &str,
        arrival: &Catalogue,
    ) {
        let marker = format!("landing-markers {HASH}\n");
        assert!(Catalogue::parse(&format!("{stage_text}{marker}")).is_err());
        let text = format!("{stage_text}{arrival_record}{marker}");
        assert!(Catalogue::parse(&format!("{text}{marker}")).is_err());
        let parsed = Catalogue::parse(&text);
        assert!(parsed.is_ok());
        let Ok(selected) = parsed else {
            return;
        };
        assert_eq!(selected.count(), arrival.count().saturating_add(1));
        assert_eq!(selected.digests().count(), selected.count());
        assert_eq!(
            selected.atlas_bytes(),
            arrival
                .atlas_bytes()
                .saturating_add(arrival::MARKER_ATLAS_BYTES)
        );
        assert_eq!(
            selected.atlas_count(),
            arrival
                .atlas_count()
                .saturating_add(arrival::MARKER_ATLAS_COUNT)
        );
        assert_eq!(
            selected.scenic_sprite_count(),
            arrival
                .scenic_sprite_count()
                .saturating_add(arrival::MARKER_EXTRA_SPRITES)
        );
    }

    fn assert_entrance_catalogue(
        stage_text: &str,
        fighter: &str,
        bonus_record: &str,
        stage: &Catalogue,
    ) {
        use std::fmt::Write as _;
        let mut track = format!("scenic-entrance 2 67 0 {HASH}\n");
        for tick in (8u16..=216).step_by(8) {
            assert!(
                writeln!(track, "scenic-entrance {tick} 80 0 {HASH}").is_ok()
            );
        }
        assert!(Catalogue::parse(&format!("{stage_text}{track}")).is_err());
        assert!(writeln!(track, "scenic-entrance 221 183 0 {HASH}").is_ok());
        assert!(
            Catalogue::parse(&format!("{fighter}{bonus_record}{track}"))
                .is_err()
        );
        let parsed = Catalogue::parse(&format!("{stage_text}{track}"));
        assert!(parsed.is_ok());
        let Ok(selected) = parsed else {
            return;
        };
        assert_eq!(selected.count(), stage.count().saturating_add(29));
        assert_eq!(
            selected.atlas_bytes(),
            stage.atlas_bytes().saturating_add(29 * 166 * 185 * 4)
        );
    }

    fn assert_stage_two_budget(
        selected: &Catalogue,
        value: &Catalogue,
        other: &str,
    ) {
        assert_eq!(value.count(), selected.count().saturating_add(2));
        assert_eq!(value.digests().count(), value.count());
        assert_eq!(
            value.atlas_count(),
            selected.atlas_count().saturating_add(2)
        );
        assert_eq!(
            value.atlas_bytes(),
            selected
                .atlas_bytes()
                .saturating_add(world::stage_atlas_budget())
        );
        assert_eq!(value.stage_two_strip(), other.parse().ok());
    }

    fn assert_bonus_budget(plain: &Catalogue, selected: &Catalogue) {
        assert_eq!(selected.count(), plain.count().saturating_add(1));
        assert_eq!(selected.digests().count(), selected.count());
        assert_eq!(
            selected.atlas_count(),
            plain.atlas_count().saturating_add(bonus::ATLAS_COUNT),
        );
        assert_eq!(
            selected.atlas_bytes(),
            plain.atlas_bytes().saturating_add(bonus::ATLAS_BYTES),
        );
    }

    #[test]
    fn complete_catalogue_admits_only_valid_unique_ticks() {
        let valid = complete();
        assert_eq!(
            Catalogue::parse(&valid).map(|value| value.count()).ok(),
            Some(8)
        );
        assert!(
            Catalogue::parse(&format!("{valid}marge attack 0 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{valid}marge attack 17 {HASH}"))
                .is_err()
        );
        assert!(
            Catalogue::parse(&format!("{valid}bart attack 17 {HASH}")).is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{valid}lisa jump 31 {HASH}")).is_err()
        );
        assert!(Catalogue::parse(&format!("marge attack 0 {HASH}")).is_err());
    }

    fn jump_at(tick: u8, hold: u8) -> Option<JumpMotion> {
        let mut state = Maneuver::jump();
        for elapsed in 1..=tick {
            state = state.advance(Player::Marge, elapsed < hold);
        }
        if let Maneuver::Jump(motion) = state {
            Some(motion)
        } else {
            None
        }
    }

    #[test]
    fn optional_aerial_poses_require_each_station_and_bound_ticks() {
        let base = complete();
        let air = format!(
            "marge air-attack 0 {HASH}\n\
             homer air-attack 0 {HASH}\n\
             bart air-attack 0 {HASH}\n\
             lisa air-attack 0 {HASH}\n"
        );
        assert!(
            Catalogue::parse(&format!("{base}marge air-attack 0 {HASH}"))
                .is_err()
        );
        assert_eq!(
            Catalogue::parse(&format!("{base}{air}"))
                .map(|catalogue| catalogue.count())
                .ok(),
            Some(12)
        );
        assert!(
            Catalogue::parse(&format!("{base}{air}lisa air-attack 59 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{base}{air}lisa air-attack 60 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn special_poses_require_each_station_and_bound_ticks() {
        let base = complete();
        let special = ["marge", "homer", "bart", "lisa"]
            .map(|name| format!("{name} special 0 {HASH}\n"))
            .concat();
        assert!(
            Catalogue::parse(&format!("{base}marge special 0 {HASH}")).is_err()
        );
        assert!(
            Catalogue::parse(&format!("{base}{special}lisa special 29 {HASH}"))
                .is_ok()
        );
        assert!(
            Catalogue::parse(&format!("{base}{special}lisa special 30 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn aerial_pose_selection_prefers_its_sequence_and_preserves_legacy() {
        let mut entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Attack, 0), "ground"),
            (station_key(Player::Marge, Kind::Attack, 3), "ground-late"),
            (station_key(Player::Lisa, Kind::AirAttack, 0), "other"),
        ]);
        assert_eq!(
            air_attack_entry(&entries, Player::Marge, 2),
            Some(&"ground")
        );
        assert_eq!(
            air_attack_entry(&entries, Player::Marge, 4),
            Some(&"ground-late")
        );
        let _prior = entries
            .insert(station_key(Player::Marge, Kind::AirAttack, 0), "air");
        let _later = entries
            .insert(station_key(Player::Marge, Kind::AirAttack, 4), "air-late");
        assert_eq!(air_attack_entry(&entries, Player::Marge, 3), Some(&"air"));
        assert_eq!(
            air_attack_entry(&entries, Player::Marge, 31),
            Some(&"air-late")
        );
    }

    #[test]
    fn held_pose_catalogue_requires_all_four_tick_zero_entries() {
        let base = complete();
        let lift = format!(
            "marge jump-lift 0 {HASH}\n\
             homer jump-lift 0 {HASH}\n\
             bart jump-lift 0 {HASH}\n\
             lisa jump-lift 0 {HASH}\n"
        );
        assert!(
            Catalogue::parse(&format!("{base}marge jump-lift 0 {HASH}"))
                .is_err()
        );
        assert_eq!(
            Catalogue::parse(&format!("{base}{lift}"))
                .map(|value| value.count())
                .ok(),
            Some(12)
        );
        assert!(
            Catalogue::parse(&format!("{base}{lift}lisa jump-lift 1 {HASH}"))
                .is_err()
        );
    }

    #[test]
    fn held_pose_selection_preserves_legacy_and_short_tap() {
        let mut entries = BTreeMap::from([
            (station_key(Player::Marge, Kind::Jump, 0), "opening"),
            (station_key(Player::Marge, Kind::Jump, 4), "landing"),
        ]);
        let select = |map: &BTreeMap<_, _>, tick, hold| {
            jump_at(tick, hold)
                .and_then(|motion| jump_entry(map, Player::Marge, motion))
                .copied()
        };
        assert_eq!(select(&entries, 20, 60), Some("landing"));
        let _prior =
            entries.insert(station_key(Player::Marge, Kind::Lift, 0), "raised");
        for (tick, hold, expected) in [
            (10, 60, "opening"),
            (11, 60, "raised"),
            (30, 60, "raised"),
            (31, 60, "landing"),
            (32, 60, "landing"),
            (12, 3, "landing"),
            (11, 10, "landing"),
            (14, 15, "raised"),
            (15, 15, "raised"),
            (16, 15, "landing"),
            (10, 10, "opening"),
        ] {
            assert_eq!(select(&entries, tick, hold), Some(expected));
        }
    }
}
