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
use std::ffi::OsString;
use std::num::NonZeroU64;
use std::path::Path;
use std::str::from_utf8;

use games_assets::ContentHash;
use games_platform_graphics::linux::RenderError;
use games_platform_graphics::linux::sprites::SceneSession;
use games_platform_storage::file::snapshot;

use super::{Art, KEYS, Maneuver, Player, Station, failure, open_bundle};
use crate::JumpMotion;

const MAX_POSES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Attack,
    Jump,
    Lift,
    Walk,
}

impl Kind {
    const fn duration(self, player: Player) -> u8 {
        match self {
            Self::Attack => Maneuver::Attack { tick: 0 }.pose_duration(player),
            Self::Jump => Maneuver::jump().pose_duration(player),
            Self::Lift => 1,
            Self::Walk => walk_duration(player),
        }
    }
}

type Entry = (Key, ContentHash);
type Key = (Player, Kind, u8);

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    entries: BTreeMap<Key, ContentHash>,
}

impl Catalogue {
    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn load(
        &self,
        parent: &Path,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<PoseArt, RenderError> {
        let mut entries = BTreeMap::new();
        for (&key, digest) in &self.entries {
            let bundle =
                open_bundle(parent, &OsString::from(digest.to_string()))?;
            let name =
                KEYS.get(key.0.index()).ok_or(RenderError::InvalidSprite)?;
            let art = Art::load(scene, &bundle, name)?;
            if art.rect.width.get() > 128 || art.rect.height.get() > 128 {
                return Err(failure("unexpected pose extent"));
            }
            let _prior = entries.insert(key, art);
        }
        Ok(PoseArt { entries })
    }

    pub(super) fn open(path: &Path) -> Result<Self, RenderError> {
        let bytes = snapshot(path, NonZeroU64::MIN.saturating_add(16_383))
            .map_err(failure)?;
        Self::parse(from_utf8(&bytes).map_err(failure)?)
    }

    fn parse(text: &str) -> Result<Self, RenderError> {
        let mut entries = BTreeMap::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let (key, digest) = parse_line(line)?;
            if entries.len() >= MAX_POSES
                || entries.insert(key, digest).is_some()
            {
                return Err(failure("excess or duplicate poses"));
            }
        }
        let complete = Player::ALL.into_iter().all(|player| {
            [Kind::Attack, Kind::Jump]
                .into_iter()
                .all(|kind| entries.contains_key(&(player, kind, 0)))
        });
        let complete_optional = [Kind::Lift, Kind::Walk]
            .into_iter()
            .all(|kind| optional_complete(&entries, kind));
        if !complete || !complete_optional {
            return Err(failure("missing tick-zero pose"));
        }
        Ok(Self { entries })
    }
}

#[derive(Debug, Default)]
pub(super) struct PoseArt {
    entries: BTreeMap<Key, Art>,
}

impl PoseArt {
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn select(
        &self,
        player: Player,
        station: Station,
    ) -> Option<Art> {
        let (kind, tick) = match station.maneuver {
            Maneuver::Attack { tick } => (Kind::Attack, tick),
            Maneuver::Jump(motion) => {
                return jump_entry(&self.entries, player, motion).copied();
            },
            Maneuver::Idle => {
                (Kind::Walk, walking_offset(player, station.walking_ticks?)?)
            },
        };
        select_entry(&self.entries, (player, kind, tick)).copied()
    }
}

fn optional_complete<Value>(
    entries: &BTreeMap<Key, Value>,
    kind: Kind,
) -> bool {
    !entries.keys().any(|(_, existing, _)| *existing == kind)
        || Player::ALL
            .into_iter()
            .all(|player| entries.contains_key(&(player, kind, 0)))
}

fn jump_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    player: Player,
    motion: JumpMotion,
) -> Option<&Value> {
    if let Some(lift) = entries.get(&(player, Kind::Lift, 0)) {
        if motion.opening_pose() {
            return entries.get(&(player, Kind::Jump, 0));
        }
        if motion.raised_pose() {
            return Some(lift);
        }
    }
    select_entry(entries, (player, Kind::Jump, motion.pose_tick()))
}

fn parse_line(line: &str) -> Result<Entry, RenderError> {
    let [name, action, elapsed, digest] =
        <[&str; 4]>::try_from(line.split_whitespace().collect::<Vec<_>>())
            .map_err(|_error| failure("expected NAME ACTION TICK SHA256"))?;
    let player = match name {
        "marge" => Player::Marge,
        "homer" => Player::Homer,
        "bart" => Player::Bart,
        "lisa" => Player::Lisa,
        _ => return Err(failure("unknown pose station")),
    };
    let kind = match action {
        "attack" => Kind::Attack,
        "jump" => Kind::Jump,
        "jump-lift" => Kind::Lift,
        "walk" => Kind::Walk,
        _ => return Err(failure("unknown pose maneuver")),
    };
    let tick = elapsed.parse::<u8>().map_err(failure)?;
    if tick >= kind.duration(player) {
        return Err(failure("pose tick outside maneuver"));
    }
    Ok(((player, kind, tick), digest.parse().map_err(failure)?))
}

fn select_entry<Value>(
    entries: &BTreeMap<Key, Value>,
    key: Key,
) -> Option<&Value> {
    let (player, kind, tick) = key;
    entries
        .range((player, kind, 0)..=(player, kind, tick))
        .next_back()
        .map(|(_, value)| value)
}

const fn walk_duration(player: Player) -> u8 {
    match player {
        Player::Marge => 42,
        Player::Homer | Player::Lisa => 40,
        Player::Bart => 25,
    }
}

fn walking_offset(player: Player, elapsed: u64) -> Option<u8> {
    u8::try_from(elapsed.checked_rem(u64::from(walk_duration(player)))?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
    fn selects_latest_pose_without_crossing_sequence() {
        let entries = BTreeMap::from([
            ((Player::Marge, Kind::Attack, 0), "first"),
            ((Player::Marge, Kind::Attack, 3), "second"),
            ((Player::Marge, Kind::Jump, 0), "air"),
            ((Player::Homer, Kind::Attack, 0), "homer"),
        ]);
        assert_eq!(
            select_entry(&entries, (Player::Marge, Kind::Attack, 2)),
            Some(&"first")
        );
        assert_eq!(
            select_entry(&entries, (Player::Marge, Kind::Attack, 3)),
            Some(&"second")
        );
        assert_eq!(
            select_entry(&entries, (Player::Marge, Kind::Attack, 16)),
            Some(&"second")
        );
        assert_eq!(
            select_entry(&entries, (Player::Marge, Kind::Jump, 16)),
            Some(&"air")
        );
        assert_eq!(
            select_entry(&entries, (Player::Homer, Kind::Jump, 16)),
            None
        );
        assert_eq!(
            select_entry(&entries, (Player::Bart, Kind::Attack, 16)),
            None
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
            ((Player::Marge, Kind::Jump, 0), "opening"),
            ((Player::Marge, Kind::Jump, 4), "landing"),
        ]);
        let select = |map: &BTreeMap<_, _>, tick, hold| {
            jump_at(tick, hold)
                .and_then(|motion| jump_entry(map, Player::Marge, motion))
                .copied()
        };
        assert_eq!(select(&entries, 20, 60), Some("landing"));
        let _prior = entries.insert((Player::Marge, Kind::Lift, 0), "raised");
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
