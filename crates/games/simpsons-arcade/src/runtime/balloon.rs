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
//   - Sparse second-stage boss art admission and presentation.
// - Must-Not:
//   - Decode original media, modify combat or advance the campaign.
// - Allows:
//   - Inputs: Canonical local bundles, boss state and viewport.
//   - Outputs: One depth-ordered sprite command.
//   - Side effects: Bounded atlas uploads during initialization.
// - Split-When:
//   - Linked projectile art needs separate presentation ownership.
// - Merge-When:
//   - Shared actor composition subsumes the title-specific roles.
// - Summary:
//   - Present four isolated, verified reference crops.
// - Description:
//   - Approximate placement and inverse scale; no full animation claim.
// - Usage:
//   - Provide the complete optional balloon-boss catalogue record.
// - Defaults:
//   - Disabled without the four local canonical art bundles.

//! Optional second-stage boss presentation.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::storage::bundle::BundleSource;

use super::world::View;
use super::{
    Art, SpriteDraw, dimension, failure, ground_position, open_bundle,
};
use crate::{BalloonAction, BalloonBoss, Facing};

pub(super) const POSE_COUNT: usize = 4;
const EXTENTS: [(u32, u32); POSE_COUNT] =
    [(126, 96), (194, 144), (191, 144), (156, 167)];
pub(super) const ATLAS_BYTES: usize = 374_352;

#[derive(Debug)]
pub(super) struct BalloonArt {
    approach: Art,
    contact: Art,
    intro: Art,
    response: Art,
}

impl BalloonArt {
    pub(super) fn draw(
        &self,
        boss: BalloonBoss,
        view: View,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        if !boss.visible() {
            return Ok(None);
        }
        let (art, basis) = match boss.action {
            BalloonAction::Intro { .. } => (self.intro, (96, 96)),
            BalloonAction::Approach | BalloonAction::Terminal { .. } => {
                (self.approach, (64, 64))
            },
            BalloonAction::Contact { .. } => (self.contact, (64, 64)),
            BalloonAction::Response { .. } => (self.response, (80, 64)),
            BalloonAction::Cleared => return Ok(None),
        };
        let mut scaled = art;
        scaled.rect.width = dimension(scale_dimension(
            art.rect.width.get(),
            basis.0,
            boss.scale.0,
        ))?;
        scaled.rect.height = dimension(scale_dimension(
            art.rect.height.get(),
            basis.1,
            boss.scale.1,
        ))?;
        let destination =
            ground_position(scaled, view.project(boss.position), 40)?;
        Ok(Some(art.draw(destination, boss.facing == Facing::Left)))
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        hashes: [ContentHash; POSE_COUNT],
    ) -> Result<Self, RenderError> {
        let mut load = |digest, extent: (u32, u32)| {
            let bundle = open_bundle(store, digest)?;
            let art = Art::load(scene, &bundle, "actor/balloon-boss")?;
            if (art.rect.width.get(), art.rect.height.get()) != extent {
                return Err(failure("unexpected balloon boss pose extent"));
            }
            Ok(art)
        };
        let [intro, approach, contact, response] = hashes;
        let [intro_size, approach_size, contact_size, response_size] = EXTENTS;
        Ok(Self {
            approach: load(approach, approach_size)?,
            contact: load(contact, contact_size)?,
            intro: load(intro, intro_size)?,
            response: load(response, response_size)?,
        })
    }
}

fn scale_dimension(extent: u32, basis: u32, scale: u8) -> u32 {
    extent
        .saturating_mul(basis)
        .checked_div(u32::from(scale))
        .unwrap_or(extent)
}

pub(super) fn parse(
    line: &str,
    selected: bool,
) -> Result<[ContentHash; POSE_COUNT], RenderError> {
    let [role, intro, approach, contact, response] =
        <[&str; 5]>::try_from(line.split_whitespace().collect::<Vec<_>>())
            .map_err(|_error| failure("expected four balloon boss poses"))?;
    if role != "balloon-boss" || selected {
        return Err(failure("unexpected or duplicate balloon boss record"));
    }
    let hashes = [
        intro.parse().map_err(failure)?,
        approach.parse().map_err(failure)?,
        contact.parse().map_err(failure)?,
        response.parse().map_err(failure)?,
    ];
    for (index, hash) in hashes.iter().enumerate() {
        if hashes
            .iter()
            .skip(index.saturating_add(1))
            .any(|other| other == hash)
        {
            return Err(failure("duplicate balloon boss pose"));
        }
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_scale_retains_basis_and_expands_at_smaller_values() {
        assert_eq!(scale_dimension(126, 96, 96), 126);
        assert_eq!(scale_dimension(126, 96, 48), 252);
        assert_eq!(scale_dimension(194, 64, 32), 388);
        assert_eq!(scale_dimension(156, 80, 0), 156);
    }

    #[test]
    fn atlas_budget_is_the_sum_of_exact_roles() {
        let bytes = EXTENTS.into_iter().fold(0u32, |total, (width, height)| {
            total.saturating_add(width.saturating_mul(height).saturating_mul(4))
        });
        assert_eq!(usize::try_from(bytes).ok(), Some(ATLAS_BYTES));
    }
}
