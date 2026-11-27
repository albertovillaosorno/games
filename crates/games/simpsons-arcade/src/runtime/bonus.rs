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
//   - Optional verified balloon bonus art and localized presentation.
// - Must-Not:
//   - Advance bonus simulation or decode original media.
// - Allows:
//   - Inputs: Canonical sprite bundle, locale and read-only bonus state.
//   - Outputs: Original-width scene draws with provisional inflation.
//   - Side effects: Bounded scene-local atlas allocation at startup.
// - Split-When:
//   - Another independently presented bonus scene is supported.
// - Merge-When:
//   - Shared presentation can own the complete balloon scene contract.
// - Summary:
//   - Present the bonus as an independent, optional playable scene.
// - Description:
//   - Verified isolated sprites; scaling and prompts are provisional.
// - Usage:
//   - Select the complete bonus bundle alongside the late wrestler art.
// - Defaults:
//   - No bonus art or additional atlases without explicit selection.

//! Optional original-width balloon bonus presentation.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::BundleSource;
use games_platform::ir::ui::{PlateLayout, TextAtlas, TextLine};

use super::{
    Art, dimension, failure, open_bundle, size, station_hud, title_messages,
};
use crate::{BONUS_MAX_INFLATION, Bonus, BonusPhase, Player};

const HEADER_WIDTH: u16 = 288;
const PLATE_HEIGHT: u16 = 12;
const RANK_WIDTH: u16 = 48;
const PHASE_KEYS: [&str; 5] = [
    "title.bonus.instructions",
    "title.bonus.ready",
    "title.bonus.pumping",
    "title.bonus.results",
    "title.bonus.complete",
];
const RANKS: [&str; 4] = ["1", "2", "3", "4"];
const HEADER_BYTES: usize = 288 * 12 * 5 * 4;
const RANK_BYTES: usize = 48 * 12 * 4 * 4;

pub(super) const ATLAS_BYTES: usize =
    333_572 + HEADER_BYTES * title_messages::LOCALES.len() + RANK_BYTES;
pub(super) const ATLAS_COUNT: usize = 10 + title_messages::LOCALES.len();

const BACKGROUND: SpriteSpec = SpriteSpec {
    height: 224,
    key: "scene/background",
    width: 320,
    x: 0,
    y: 0,
};
const BALLOONS: [SpriteSpec; 4] = [
    SpriteSpec {
        height: 48,
        key: "bonus/marge-balloon",
        width: 24,
        x: 48,
        y: 112,
    },
    SpriteSpec {
        height: 32,
        key: "bonus/homer-balloon",
        width: 21,
        x: 113,
        y: 128,
    },
    SpriteSpec {
        height: 31,
        key: "bonus/bart-balloon",
        width: 21,
        x: 177,
        y: 128,
    },
    SpriteSpec {
        height: 32,
        key: "bonus/lisa-balloon",
        width: 30,
        x: 237,
        y: 128,
    },
];
const PUMPS: [SpriteSpec; 4] = [
    SpriteSpec {
        height: 75,
        key: "bonus/marge-pump",
        width: 34,
        x: 61,
        y: 125,
    },
    SpriteSpec {
        height: 64,
        key: "bonus/homer-pump",
        width: 37,
        x: 123,
        y: 136,
    },
    SpriteSpec {
        height: 48,
        key: "bonus/bart-pump",
        width: 35,
        x: 187,
        y: 152,
    },
    SpriteSpec {
        height: 48,
        key: "bonus/lisa-pump",
        width: 35,
        x: 249,
        y: 152,
    },
];

#[derive(Clone, Copy, Debug)]
struct SpriteSpec {
    height: u32,
    key: &'static str,
    width: u32,
    x: i32,
    y: i32,
}

impl SpriteSpec {
    fn destination(self, inflation: u8) -> Result<SpriteRect, RenderError> {
        let amount = u32::from(inflation.min(BONUS_MAX_INFLATION));
        let cap = u32::from(BONUS_MAX_INFLATION);
        let extra_width = self
            .width
            .saturating_mul(amount)
            .checked_div(cap.saturating_mul(2))
            .unwrap_or_default();
        let extra_height = 96u32
            .saturating_mul(amount)
            .checked_div(cap)
            .unwrap_or_default();
        let left =
            i32::try_from(extra_width.checked_div(2).unwrap_or_default())
                .map_err(failure)?;
        let rise = i32::try_from(extra_height).map_err(failure)?;
        Ok(SpriteRect {
            x: self.x.saturating_sub(left),
            y: self.y.saturating_sub(rise),
            width: dimension(self.width.saturating_add(extra_width))?,
            height: dimension(self.height.saturating_add(extra_height))?,
        })
    }
}

#[derive(Debug)]
pub(super) struct BonusArt {
    background: Art,
    balloons: [Art; 4],
    headers: Vec<TextAtlas>,
    pumps: [Art; 4],
    ranks: TextAtlas,
}

impl BonusArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        digest: ContentHash,
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, digest)?;
        let mut load = |spec: SpriteSpec| {
            let (art, pixels) =
                Art::load_with_pixels(scene, &bundle, spec.key)?;
            if art.rect.width.get() != spec.width
                || art.rect.height.get() != spec.height
            {
                return Err(failure("unexpected bonus sprite extent"));
            }
            if spec.key == BACKGROUND.key
                && pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel.last() != Some(&255))
            {
                return Err(failure("bonus background must be opaque"));
            }
            Ok(art)
        };
        let background = load(BACKGROUND)?;
        let [marge, homer, bart, lisa] = BALLOONS;
        let balloons = [load(marge)?, load(homer)?, load(bart)?, load(lisa)?];
        let [first, second, third, fourth] = PUMPS;
        let pumps = [load(first)?, load(second)?, load(third)?, load(fourth)?];
        let catalogue = title_messages::catalogue()?;
        let mut headers = Vec::with_capacity(title_messages::LOCALES.len());
        for index in 0..title_messages::LOCALES.len() {
            let locale = title_messages::locale_tag(index)?;
            let text = title_messages::TitleText {
                catalog: &catalogue,
                locale: &locale,
            };
            let mut labels = Vec::with_capacity(PHASE_KEYS.len());
            for key in PHASE_KEYS {
                labels.push(text.label(key)?);
            }
            headers.push(plates(scene, HEADER_WIDTH, HEADER_BYTES, &labels)?);
        }
        let ranks = plates(scene, RANK_WIDTH, RANK_BYTES, &RANKS)?;
        Ok(Self {
            background,
            balloons,
            headers,
            pumps,
            ranks,
        })
    }

    pub(super) fn queue(
        &self,
        bonus: Bonus,
        (language, digits): (usize, &TextAtlas),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let phase = match bonus.phase() {
            BonusPhase::Victory => return Err(RenderError::InvalidSprite),
            BonusPhase::Instructions => 0,
            BonusPhase::Ready => 1,
            BonusPhase::Pumping => 2,
            BonusPhase::Results => 3,
            BonusPhase::Complete => 4,
        };
        draws.push(self.background.draw(self.background.rect, false));
        for player in Player::ALL {
            self.queue_player(bonus, (player, digits), draws)?;
        }
        let header = self
            .headers
            .get(language)
            .ok_or(RenderError::InvalidSprite)?;
        draws.push(header.draw(phase, 16, 12)?);
        if let Some(countdown) = bonus.countdown() {
            station_hud::queue_centiseconds(
                digits,
                countdown,
                (175i32, 26i32),
                draws,
            )?;
        }
        Ok(())
    }

    fn queue_player(
        &self,
        bonus: Bonus,
        (player, digits): (Player, &TextAtlas),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        if bonus.inflation(player).is_none() {
            return Ok(());
        }
        let index = player.index();
        let balloon = BALLOONS.get(index).ok_or(RenderError::InvalidSprite)?;
        let pump = PUMPS.get(index).ok_or(RenderError::InvalidSprite)?;
        let balloon_art =
            self.balloons.get(index).ok_or(RenderError::InvalidSprite)?;
        let pump_art =
            self.pumps.get(index).ok_or(RenderError::InvalidSprite)?;
        if let Some(amount) = bonus.visual_inflation(player) {
            draws.push(balloon_art.draw(balloon.destination(amount)?, false));
            draws.push(pump_art.draw(pump.destination(0)?, false));
        }
        if let Some((rank, score)) = bonus.displayed_result(player) {
            let tile = usize::from(rank.saturating_sub(1));
            draws.push(self.ranks.draw(
                tile,
                balloon.x.saturating_sub(8),
                36,
            )?);
            station_hud::queue_decimal(
                digits,
                u16::from(score),
                (balloon.x.saturating_add(24), 52i32),
                draws,
            )?;
        }
        Ok(())
    }
}

fn plates(
    scene: &mut SceneSession<'_, '_>,
    width: u16,
    bytes: usize,
    labels: &[&str],
) -> Result<TextAtlas, RenderError> {
    let lines: Vec<_> = labels
        .iter()
        .map(|label| {
            vec![
                TextLine {
                    color: [255, 255, 255, 255],
                    scale: 2,
                    text: label,
                    top: 1,
                }
                .fit_width(width),
            ]
        })
        .collect();
    TextAtlas::load(
        scene,
        PlateLayout {
            background: [24, 22, 30, 255],
            height: PLATE_HEIGHT,
            max_bytes: size(bytes)?,
            width,
        },
        &lines,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflation_preserves_bottom_and_grows_within_lane() {
        for sprite in BALLOONS {
            let start = sprite.destination(0);
            let maximum = sprite.destination(BONUS_MAX_INFLATION);
            assert!(start.is_ok());
            assert!(maximum.is_ok());
            let (Ok(initial), Ok(full)) = (start, maximum) else {
                return;
            };
            assert_eq!(initial.x, sprite.x);
            assert_eq!(initial.y, sprite.y);
            assert!(full.width > initial.width);
            assert!(full.height > initial.height);
            assert_eq!(
                full.y.saturating_add_unsigned(full.height.get()),
                initial.y.saturating_add_unsigned(initial.height.get()),
            );
        }
    }
}
