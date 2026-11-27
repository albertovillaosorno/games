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
//   - Complete bounded local wrestler art admission and composition.
// - Must-Not:
//   - Advance gameplay, infer combat rules or decode original media.
// - Allows:
//   - Inputs: Canonical art bundles, cabinet state and world projection.
//   - Outputs: At most one fighter draw and one decorative rig draw.
//   - Side effects: Bounded atlas allocation during scene startup.
// - Split-When:
//   - Another independently rendered boss requires distinct ownership.
// - Merge-When:
//   - Shared boss pose selection subsumes this title-local composition.
// - Summary:
//   - Present sparse exact reference poses during the experimental fight.
// - Description:
//   - Requires the complete track and verified late arena backdrop.
// - Usage:
//   - Supply wrestler records with canonical manifest hashes.
// - Defaults:
//   - No wrestler art or combat without the complete local selection.

//! Optional wrestler art selection for the experimental late fight.

use std::collections::BTreeMap;

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::BundleSource;

use super::world::View;
use super::{Art, failure, ground_position, open_bundle};
use crate::{Facing, WRESTLER_ATTACK_STRIKE, Wrestler, WrestlerAction};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Defeat,
    Flex,
    Intro,
    Left,
    RedFlex,
    RedLeft,
    RedRight,
    Rig,
    Right,
    Seated,
    Stand,
}

impl Kind {
    const ALL: [Self; 11] = [
        Self::Defeat,
        Self::Flex,
        Self::Intro,
        Self::Left,
        Self::RedFlex,
        Self::RedLeft,
        Self::RedRight,
        Self::Rig,
        Self::Right,
        Self::Seated,
        Self::Stand,
    ];

    const fn asset_key(self) -> &'static str {
        if matches!(self, Self::Rig) {
            "actor/wrestler-rig"
        } else {
            "actor/wrestler"
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "defeat" => Some(Self::Defeat),
            "flex" => Some(Self::Flex),
            "intro" => Some(Self::Intro),
            "left" => Some(Self::Left),
            "red-flex" => Some(Self::RedFlex),
            "red-left" => Some(Self::RedLeft),
            "red-right" => Some(Self::RedRight),
            "rig" => Some(Self::Rig),
            "right" => Some(Self::Right),
            "seated" => Some(Self::Seated),
            "stand" => Some(Self::Stand),
            _ => None,
        }
    }

    const fn valid_extent(self, art: Art) -> bool {
        let width = art.rect.width.get();
        let height = art.rect.height.get();
        if matches!(self, Self::Rig) {
            width == 253 && height == 56
        } else {
            width <= 128 && height <= 128
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Catalogue {
    entries: BTreeMap<Kind, ContentHash>,
}

impl Catalogue {
    pub(super) fn complete(&self) -> bool {
        self.entries.is_empty() || self.entries.len() == Kind::ALL.len()
    }

    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn digests(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.entries.values().copied()
    }

    pub(super) fn insert(&mut self, line: &str) -> Result<(), RenderError> {
        let mut words = line.split_whitespace();
        let (Some("wrestler"), Some(name), Some(hash), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            return Err(failure("invalid wrestler record"));
        };
        let kind = Kind::parse(name)
            .ok_or_else(|| failure("unknown wrestler pose"))?;
        let digest = hash.parse().map_err(failure)?;
        if self.entries.insert(kind, digest).is_some() {
            return Err(failure("duplicate wrestler pose"));
        }
        Ok(())
    }

    pub(super) fn load(
        &self,
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<WrestlerArt, RenderError> {
        let mut entries = BTreeMap::new();
        for (&kind, &digest) in &self.entries {
            let bundle = open_bundle(store, digest)?;
            let art = Art::load(scene, &bundle, kind.asset_key())?;
            if !kind.valid_extent(art) {
                return Err(failure("unexpected wrestler pose extent"));
            }
            let _prior = entries.insert(kind, art);
        }
        Ok(WrestlerArt { entries })
    }
}

#[derive(Debug, Default)]
pub(super) struct WrestlerArt {
    entries: BTreeMap<Kind, Art>,
}

impl WrestlerArt {
    pub(super) fn enabled(&self) -> bool {
        self.entries.len() == Kind::ALL.len()
    }

    pub(super) fn fighter(
        &self,
        boss: Wrestler,
        view: View,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        if !boss.visible() {
            return Ok(None);
        }
        let Some(&art) = self.entries.get(&kind_for(boss)) else {
            return Ok(None);
        };
        let mut point = view.project(boss.position);
        if matches!(boss.action, WrestlerAction::Intro { .. }) {
            // Fixed sparse entrance pose sampled at the first visible
            // landmark. This is not the original descent trajectory.
            point.y = 130;
        }
        let destination = ground_position(art, point, 0)?;
        Ok(Some(art.draw(destination, false)))
    }

    pub(super) fn rig(&self, boss: Wrestler) -> Option<SpriteDraw> {
        if !boss.visible()
            && !matches!(boss.action, WrestlerAction::Defeat { .. })
        {
            return None;
        }
        let art = *self.entries.get(&Kind::Rig)?;
        Some(art.draw(SpriteRect { x: 37, y: 0, ..art.rect }, false))
    }
}

const fn kind_for(boss: Wrestler) -> Kind {
    let red = boss.health <= 4;
    match boss.action {
        WrestlerAction::Intro { .. } => Kind::Intro,
        WrestlerAction::Approach if red => Kind::RedFlex,
        WrestlerAction::Approach => Kind::Stand,
        WrestlerAction::Attack { tick }
            if tick < WRESTLER_ATTACK_STRIKE && red =>
        {
            Kind::RedFlex
        },
        WrestlerAction::Attack { tick } if tick < WRESTLER_ATTACK_STRIKE => {
            Kind::Flex
        },
        WrestlerAction::Attack { .. } => match (boss.facing, red) {
            (Facing::Left, false) => Kind::Left,
            (Facing::Right, false) => Kind::Right,
            (Facing::Left, true) => Kind::RedLeft,
            (Facing::Right, true) => Kind::RedRight,
        },
        WrestlerAction::Stagger { .. } => Kind::Seated,
        WrestlerAction::Defeat { .. } | WrestlerAction::Cleared => Kind::Defeat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_requires_every_named_pose_once() {
        let mut catalogue = Catalogue::default();
        let hash = "0".repeat(64);
        assert!(catalogue.complete());
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
        ] {
            assert!(
                catalogue.insert(&format!("wrestler {name} {hash}")).is_ok()
            );
            assert!(!catalogue.complete());
        }
        assert!(catalogue.insert(&format!("wrestler stand {hash}")).is_ok());
        assert!(catalogue.complete());
        assert_eq!(catalogue.count(), Kind::ALL.len());
        assert!(catalogue.insert(&format!("wrestler stand {hash}")).is_err());
        assert!(catalogue.insert(&format!("wrestler extra {hash}")).is_err());
    }
}
