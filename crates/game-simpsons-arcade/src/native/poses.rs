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
//   - Resolve optional native attack and jump art.
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

use super::{Art, KEYS, Maneuver, Player, failure, open_bundle};

const MAX_POSES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Attack,
    Jump,
}

impl Kind {
    const fn maneuver(self, tick: u8) -> Maneuver {
        match self {
            Self::Attack => Maneuver::Attack { tick },
            Self::Jump => Maneuver::Jump { tick },
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
        if !complete {
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
        maneuver: Maneuver,
    ) -> Option<Art> {
        let (kind, tick) = match maneuver {
            Maneuver::Attack { tick } => (Kind::Attack, tick),
            Maneuver::Jump { tick } => (Kind::Jump, tick),
            Maneuver::Idle => return None,
        };
        select_entry(&self.entries, (player, kind, tick)).copied()
    }
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
        _ => return Err(failure("unknown pose maneuver")),
    };
    let tick = elapsed.parse::<u8>().map_err(failure)?;
    if tick >= kind.maneuver(tick).duration(player) {
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
}
