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
//   - Registered visible Marge instruction balloon states.
// - Must-Not:
//   - Decode original media or advance simulation.
// - Allows:
//   - Inputs: Optional canonical grids and read-only inverse axes.
//   - Outputs: One unscaled registered sprite draw.
//   - Side effects: Bounded scene-local atlas allocation during load.
// - Split-When:
//   - Another independent original-backed instruction track is admitted.
// - Merge-When:
//   - Shared presentation owns all registered bonus balloon states.
// - Summary:
//   - Present the observed baseline balloon without resampling a large pose.
// - Description:
//   - Unknown axes and other characters retain the existing approximation.
// - Usage:
//   - Select two pages published from the verified Marge instruction trace.
// - Defaults:
//   - No additional art or allocation without explicit selection.

//! Optional exact visible states of the baseline instruction balloon.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};

use super::{Art, Scene, dimension, failure, verify_registered_sprite};
use crate::runtime::open_bundles;
use crate::{BonusPhase, Cabinet, Player};

pub(in crate::runtime) const ATLAS_BYTES: usize = 1_424_640;
pub(in crate::runtime) const BUNDLE_COUNT: usize = 2;
pub(in crate::runtime) const STATE_COUNT: usize = 70;

// Inverse axes in the observed baseline's descending order. Repeated
// high bytes share the same registered visible object geometry.
const AXES: [(u8, u8); STATE_COUNT] = [
    (0xb0, 0xa2),
    (0xb0, 0x97),
    (0xb0, 0x8d),
    (0xb0, 0x85),
    (0xb0, 0x7d),
    (0xb0, 0x76),
    (0xb0, 0x70),
    (0xb0, 0x6b),
    (0xa2, 0x6b),
    (0xa2, 0x66),
    (0x97, 0x66),
    (0x8d, 0x66),
    (0x8d, 0x61),
    (0x8d, 0x5d),
    (0x85, 0x5d),
    (0x7d, 0x5d),
    (0x7d, 0x5a),
    (0x76, 0x5a),
    (0x76, 0x56),
    (0x76, 0x53),
    (0x70, 0x53),
    (0x70, 0x50),
    (0x70, 0x4d),
    (0x70, 0x4b),
    (0x70, 0x48),
    (0x70, 0x46),
    (0x70, 0x44),
    (0x70, 0x42),
    (0x70, 0x40),
    (0x70, 0x3e),
    (0x70, 0x3d),
    (0x70, 0x3b),
    (0x70, 0x39),
    (0x70, 0x38),
    (0x70, 0x37),
    (0x70, 0x35),
    (0x70, 0x34),
    (0x6b, 0x34),
    (0x66, 0x34),
    (0x66, 0x33),
    (0x61, 0x33),
    (0x61, 0x32),
    (0x5d, 0x32),
    (0x5d, 0x31),
    (0x5d, 0x30),
    (0x5d, 0x2f),
    (0x5d, 0x2e),
    (0x5a, 0x2e),
    (0x5a, 0x2d),
    (0x56, 0x2d),
    (0x56, 0x2c),
    (0x53, 0x2c),
    (0x50, 0x2c),
    (0x50, 0x2b),
    (0x50, 0x2a),
    (0x50, 0x29),
    (0x4d, 0x29),
    (0x4d, 0x28),
    (0x4d, 0x27),
    (0x4d, 0x26),
    (0x4d, 0x25),
    (0x4b, 0x25),
    (0x4b, 0x24),
    (0x48, 0x24),
    (0x48, 0x23),
    (0x46, 0x23),
    (0x44, 0x23),
    (0x44, 0x22),
    (0x44, 0x21),
    (0x44, 0x20),
];
const CELL_BYTES: usize = 48 * 106 * 4;
const CELL_HEIGHT: u32 = 106;
const CELL_WIDTH: u32 = 48;
const CELLS_PER_PAGE: usize = 35;
const COLUMNS: usize = 7;
const PAGE_HEIGHT: u32 = 530;
const PAGE_STRIDE: usize = 336 * 4;
const PAGE_WIDTH: u32 = 336;

type Selection = (ContentHash, SpriteDraw);
pub(in crate::runtime) type Digests = [ContentHash; BUNDLE_COUNT];

#[derive(Debug)]
struct Page {
    art: Art,
    digests: Vec<ContentHash>,
}

impl Page {
    fn load(
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
    ) -> Result<Self, RenderError> {
        let (art, pixels) =
            Art::load_with_pixels(scene, bundle, "scene/actor-grid")?;
        if art.rect.width.get() != PAGE_WIDTH
            || art.rect.height.get() != PAGE_HEIGHT
        {
            return Err(failure("unexpected instruction balloon grid extent"));
        }
        let digests = (0..CELLS_PER_PAGE)
            .map(|index| cell_digest(&pixels, cell(index)?))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { art, digests })
    }

    fn selection(&self, index: usize) -> Result<Selection, RenderError> {
        let digest =
            *self.digests.get(index).ok_or(RenderError::InvalidSprite)?;
        let art = Art {
            rect: cell(index)?,
            ..self.art
        };
        let destination = SpriteRect { x: 74, y: 0, ..art.rect };
        Ok((digest, art.draw(destination, false)))
    }
}

#[derive(Debug)]
pub(super) struct BalloonArt {
    pages: [Page; BUNDLE_COUNT],
}

impl BalloonArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        hashes: Digests,
    ) -> Result<Self, RenderError> {
        let [first, second] = open_bundles(store, hashes)?;
        Ok(Self {
            pages: [Page::load(scene, &first)?, Page::load(scene, &second)?],
        })
    }

    pub(super) fn selection(
        &self,
        axes: (u8, u8),
    ) -> Result<Option<Selection>, RenderError> {
        let Some((page, index)) = locate(axes) else {
            return Ok(None);
        };
        self.pages
            .get(page)
            .ok_or(RenderError::InvalidSprite)?
            .selection(index)
            .map(Some)
    }
}

fn cell(index: usize) -> Result<SpriteRect, RenderError> {
    if index >= CELLS_PER_PAGE {
        return Err(RenderError::InvalidSprite);
    }
    let column = index
        .checked_rem(COLUMNS)
        .ok_or(RenderError::InvalidSprite)?;
    let row = index
        .checked_div(COLUMNS)
        .ok_or(RenderError::InvalidSprite)?;
    Ok(SpriteRect {
        height: dimension(CELL_HEIGHT)?,
        width: dimension(CELL_WIDTH)?,
        x: i32::try_from(column).map_err(failure)?.saturating_mul(48),
        y: i32::try_from(row).map_err(failure)?.saturating_mul(106),
    })
}

fn cell_digest(
    bytes: &[u8],
    rect: SpriteRect,
) -> Result<ContentHash, RenderError> {
    let left = usize::try_from(rect.x).map_err(failure)?.saturating_mul(4);
    let top = usize::try_from(rect.y).map_err(failure)?;
    let width = usize::try_from(rect.width.get()).map_err(failure)?;
    let height = usize::try_from(rect.height.get()).map_err(failure)?;
    let end = left.saturating_add(width.saturating_mul(4));
    let mut cropped = Vec::with_capacity(CELL_BYTES);
    for row in bytes
        .as_chunks::<PAGE_STRIDE>()
        .0
        .iter()
        .skip(top)
        .take(height)
    {
        cropped.extend_from_slice(
            row.get(left..end).ok_or(RenderError::InvalidLength)?,
        );
    }
    let (pixels, remainder) = cropped.as_chunks::<4>();
    if cropped.len() != CELL_BYTES
        || !remainder.is_empty()
        || pixels
            .iter()
            .any(|pixel| !matches!(pixel, [0, 0, 0, 0] | [_, _, _, 255]))
        || !pixels.iter().any(|pixel| pixel.last() == Some(&255))
    {
        return Err(failure("invalid instruction balloon grid cell"));
    }
    Ok(ContentHash::calculate(&cropped))
}

fn locate(axes: (u8, u8)) -> Option<(usize, usize)> {
    let index = AXES
        .binary_search_by(|candidate| axes.cmp(candidate))
        .ok()?;
    Some((
        index.checked_div(CELLS_PER_PAGE)?,
        index.checked_rem(CELLS_PER_PAGE)?,
    ))
}

/// Verify one selected state in the routed instruction presentation.
pub(in crate::runtime) fn verify(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let balloon = art
        .poses
        .bonus
        .as_ref()
        .and_then(|bonus| bonus.instruction_balloon.as_ref())
        .ok_or(RenderError::InvalidSprite)?;
    let bonus = cabinet.bonus().ok_or(RenderError::InvalidSprite)?;
    if bonus.phase() != BonusPhase::Instructions
        || bonus.instruction_player() != Some(Player::Marge)
    {
        return Err(failure("instruction balloon smoke requires Marge"));
    }
    let axes = bonus
        .visual_axes(Player::Marge)
        .ok_or(RenderError::InvalidSprite)?;
    let selection =
        balloon.selection(axes)?.ok_or(RenderError::InvalidSprite)?;
    let actual = scene.readback()?;
    verify_registered_sprite(scene, &art.draws, &actual, selection)?;
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_states_select_distinct_registered_cells() {
        assert!(
            AXES.windows(2).all(|pair| {
                matches!(pair, [first, second] if first > second)
            })
        );
        for (index, axes) in AXES.into_iter().enumerate() {
            let expected = index
                .checked_div(CELLS_PER_PAGE)
                .zip(index.checked_rem(CELLS_PER_PAGE));
            assert_eq!(locate(axes), expected);
        }
        assert_eq!(locate((0xb0, 0xb0)), None);
        assert_eq!(locate((0x44, 0x1f)), None);
        assert_eq!(
            cell(0).map(|rect| (rect.x, rect.y)).ok(),
            Some((0i32, 0i32))
        );
        assert_eq!(
            cell(34).map(|rect| (rect.x, rect.y)).ok(),
            Some((288i32, 424i32))
        );
        assert!(cell(35).is_err());
    }
}
