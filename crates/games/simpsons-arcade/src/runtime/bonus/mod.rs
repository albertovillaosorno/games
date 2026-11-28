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

mod instruction;
pub(super) mod instruction_balloon;
pub(super) mod instruction_pump;
pub(super) mod opposition;

use games::presentation::TranslationCatalog;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};
use games_platform::ir::ui::{PlateLayout, TextAtlas, TextLine};

use super::{
    Art, Scene, dimension, failure, open_bundle, scenery, size, station_hud,
    title_messages,
};
use crate::{
    BONUS_MAX_INFLATION, Bonus, BonusDeparture, BonusPhase, Cabinet, Player,
};

const HEADER_WIDTH: u16 = 288;
const PLATE_HEIGHT: u16 = 12;
const RANK_WIDTH: u16 = 48;
const INSTRUCTION_BALLOON_ANCHOR: (i32, i32) = (92, 104);
const INSTRUCTION_PUMP_ANCHOR: (i32, i32) = (106, 144);
const PHASE_KEYS: [&str; 6] = [
    "title.bonus.instructions",
    "title.bonus.ready",
    "title.bonus.pumping",
    "title.bonus.results",
    "title.bonus.complete",
    "title.bonus.stage_two",
];
const RANKS: [&str; 4] = ["1", "2", "3", "4"];
const HEADER_BYTES: usize = 288 * 12 * PHASE_KEYS.len() * 4;
const RANK_BYTES: usize = 48 * 12 * 4 * 4;

pub(super) const ATLAS_BYTES: usize =
    333_572 + HEADER_BYTES * title_messages::LOCALES.len() + RANK_BYTES;
pub(super) const ATLAS_COUNT: usize = 10 + title_messages::LOCALES.len();
pub(super) const DEPARTURE_ATLAS_BYTES: usize = 39_096;
pub(super) const DEPARTURE_ATLAS_COUNT: usize = 5;
pub(super) const DEPARTURE_EXTRA_SPRITES: usize = 4;
pub(super) const INFLATED_ATLAS_BYTES: usize = 83_536;
pub(super) const INFLATED_ATLAS_COUNT: usize = 4;
pub(super) const INSTRUCTION_ATLAS_BYTES: usize = instruction::ATLAS_BYTES;
pub(super) const INSTRUCTION_ATLAS_COUNT: usize = instruction::ATLAS_COUNT;

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
// All four isolated assemblies were captured at inverse zoom 107, 51.
const INFLATED: [SpriteSpec; 4] = [
    SpriteSpec {
        height: 158,
        key: "bonus/marge-inflated",
        width: 38,
        x: 41,
        y: 2,
    },
    SpriteSpec {
        height: 120,
        key: "bonus/homer-inflated",
        width: 38,
        x: 105,
        y: 40,
    },
    SpriteSpec {
        height: 120,
        key: "bonus/bart-inflated",
        width: 38,
        x: 169,
        y: 40,
    },
    SpriteSpec {
        height: 120,
        key: "bonus/lisa-inflated",
        width: 48,
        x: 229,
        y: 40,
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

const HANGING: [SpriteSpec; 4] = [
    SpriteSpec {
        height: 76,
        key: "bonus/marge-hanging",
        width: 31,
        x: 59,
        y: 122,
    },
    SpriteSpec {
        height: 64,
        key: "bonus/homer-hanging",
        width: 30,
        x: 123,
        y: 134,
    },
    SpriteSpec {
        height: 80,
        key: "bonus/bart-hanging",
        width: 28,
        x: 187,
        y: 118,
    },
    SpriteSpec {
        height: 80,
        key: "bonus/lisa-hanging",
        width: 26,
        x: 252,
        y: 118,
    },
];
const GROUND_PUMP: SpriteSpec = SpriteSpec {
    height: 31,
    key: "bonus/marge-ground-pump",
    width: 38,
    x: 53,
    y: 169,
};
const GROUND_PUMPS: [SpriteSpec; 4] = [
    GROUND_PUMP,
    SpriteSpec {
        key: "bonus/homer-ground-pump",
        x: 115,
        ..GROUND_PUMP
    },
    SpriteSpec {
        key: "bonus/bart-ground-pump",
        x: 179,
        ..GROUND_PUMP
    },
    SpriteSpec {
        key: "bonus/lisa-ground-pump",
        x: 241,
        ..GROUND_PUMP
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

    fn inverse_destination(
        self,
        (inverse_width, inverse_height): (u8, u8),
        rise: u32,
    ) -> Result<SpriteRect, RenderError> {
        self.inverse_destination_at(
            (107, 51),
            (inverse_width, inverse_height),
            rise,
        )
    }

    fn inverse_destination_at(
        self,
        (captured_width, captured_height): (u32, u32),
        (inverse_width, inverse_height): (u8, u8),
        rise: u32,
    ) -> Result<SpriteRect, RenderError> {
        let width = inverse_extent(self.width, captured_width, inverse_width)?;
        let height =
            inverse_extent(self.height, captured_height, inverse_height)?;
        let center = self.x.saturating_add_unsigned(self.width >> 1);
        let bottom = self.y.saturating_add_unsigned(self.height);
        let left = center.saturating_sub_unsigned(width >> 1);
        let top = bottom
            .saturating_sub_unsigned(height)
            .saturating_sub_unsigned(rise >> 8);
        Ok(SpriteRect {
            x: left,
            y: top,
            width: dimension(width)?,
            height: dimension(height)?,
        })
    }

    fn load(
        self,
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
    ) -> Result<Art, RenderError> {
        let (art, pixels) = Art::load_with_pixels(scene, bundle, self.key)?;
        if art.rect.width.get() != self.width
            || art.rect.height.get() != self.height
        {
            return Err(failure("unexpected bonus sprite extent"));
        }
        if self.key == BACKGROUND.key
            && pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel.last() != Some(&255))
        {
            return Err(failure("bonus background must be opaque"));
        }
        Ok(art)
    }

    fn rising_destination(
        self,
        inflation: u8,
        rise: u32,
    ) -> Result<SpriteRect, RenderError> {
        let mut destination = self.destination(inflation)?;
        let pixels = i32::try_from(rise >> 8u32).map_err(failure)?;
        destination.y = destination.y.saturating_sub(pixels);
        Ok(destination)
    }
}

#[derive(Debug)]
struct DepartureArt {
    hanging: [Art; 4],
    pump: Art,
}

impl DepartureArt {
    fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        digest: ContentHash,
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, digest)?;
        let [marge, homer, bart, lisa] = HANGING;
        let hanging = [
            marge.load(scene, &bundle)?,
            homer.load(scene, &bundle)?,
            bart.load(scene, &bundle)?,
            lisa.load(scene, &bundle)?,
        ];
        let [first, second, third, fourth] = GROUND_PUMPS;
        let record = bundle
            .manifest()
            .record(first.key)
            .ok_or_else(|| failure("missing ground pump"))?;
        // All four pumps use the same observed immutable payload.
        for spec in [second, third, fourth] {
            let other = bundle
                .manifest()
                .record(spec.key)
                .ok_or_else(|| failure("missing ground pump"))?;
            if other.digest != record.digest || other.format != record.format {
                return Err(failure("ground pump art differs by station"));
            }
        }
        Ok(Self {
            hanging,
            pump: first.load(scene, &bundle)?,
        })
    }

    fn queue(
        &self,
        player: Player,
        departure: BonusDeparture,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let index = player.index();
        let pump = GROUND_PUMPS.get(index).ok_or(RenderError::InvalidSprite)?;
        draws.push(self.pump.draw(pump.destination(0)?, false));
        if let BonusDeparture::Rising { character, .. } = departure {
            let spec = HANGING.get(index).ok_or(RenderError::InvalidSprite)?;
            let art =
                self.hanging.get(index).ok_or(RenderError::InvalidSprite)?;
            draws.push(art.draw(spec.rising_destination(0, character)?, false));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct BonusDigests<'digests> {
    pub(super) base: ContentHash,
    pub(super) departure: Option<ContentHash>,
    pub(super) inflated: Option<ContentHash>,
    pub(super) instruction: Option<ContentHash>,
    pub(super) instruction_balloon: Option<instruction_balloon::Digests>,
    pub(super) instruction_pump: Option<&'digests instruction_pump::Digests>,
    pub(super) opponents: Option<&'digests opposition::Digests>,
}

#[derive(Debug)]
pub(super) struct BonusArt {
    background: Art,
    balloons: [Art; 4],
    departure: Option<DepartureArt>,
    headers: Vec<TextAtlas>,
    inflated: Option<[Art; 4]>,
    instruction: Option<instruction::InstructionArt>,
    instruction_balloon: Option<Box<instruction_balloon::BalloonArt>>,
    instruction_pump: Option<Box<instruction_pump::PumpArt>>,
    opponents: Option<Box<opposition::OpponentArt>>,
    pumps: [Art; 4],
    ranks: TextAtlas,
}

impl BonusArt {
    fn balloon_layer(
        &self,
        bonus: Bonus,
        player: Player,
        rise: u32,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let Some(axes) = bonus.visual_axes(player) else {
            return Ok(None);
        };
        let index = player.index();
        if let Some(inflated) = &self.inflated
            && axes != (0xb0, 0xb0)
        {
            let spec = INFLATED.get(index).ok_or(RenderError::InvalidSprite)?;
            let art = inflated.get(index).ok_or(RenderError::InvalidSprite)?;
            return Ok(Some(
                art.draw(spec.inverse_destination(axes, rise)?, false),
            ));
        }
        let spec = BALLOONS.get(index).ok_or(RenderError::InvalidSprite)?;
        let art = self.balloons.get(index).ok_or(RenderError::InvalidSprite)?;
        let amount = bonus
            .visual_inflation(player)
            .ok_or(RenderError::InvalidSprite)?;
        Ok(Some(
            art.draw(spec.rising_destination(amount, rise)?, false),
        ))
    }

    pub(super) const fn has_departure(&self) -> bool {
        self.departure.is_some()
    }

    pub(super) const fn has_instruction_balloon(&self) -> bool {
        self.instruction_balloon.is_some()
    }

    pub(super) const fn has_instruction_pump(&self) -> bool {
        self.instruction_pump.is_some()
    }

    pub(super) const fn has_opponents(&self) -> bool {
        self.opponents.is_some()
    }

    fn instruction_balloon_layer(
        &self,
        bonus: Bonus,
        player: Player,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        if player == Player::Marge
            && let Some(art) = &self.instruction_balloon
            && let Some(axes) = bonus.visual_axes(player)
            && let Some((_, draw)) = art.selection(axes)?
        {
            return Ok(Some(draw));
        }
        Ok(self.balloon_layer(bonus, player, 0)?.map(|mut draw| {
            draw.destination = instruction_destination(
                draw.destination,
                INSTRUCTION_BALLOON_ANCHOR,
            );
            draw
        }))
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        digests: &BonusDigests<'_>,
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, digests.base)?;
        let mut load = |spec: SpriteSpec| spec.load(scene, &bundle);
        let background = load(BACKGROUND)?;
        let [marge, homer, bart, lisa] = BALLOONS;
        let balloons = [load(marge)?, load(homer)?, load(bart)?, load(lisa)?];
        let [first, second, third, fourth] = PUMPS;
        let pumps = [load(first)?, load(second)?, load(third)?, load(fourth)?];
        let departure = digests
            .departure
            .map(|address| DepartureArt::load(store, scene, address))
            .transpose()?;
        let inflated = digests
            .inflated
            .map(|address| load_inflated(store, scene, address))
            .transpose()?;
        let catalogue = title_messages::catalogue()?;
        let instruction = digests
            .instruction
            .map(|address| {
                instruction::InstructionArt::load(
                    store, scene, address, &catalogue,
                )
            })
            .transpose()?;
        let instruction_balloon = digests
            .instruction_balloon
            .map(|hashes| {
                instruction_balloon::BalloonArt::load(store, scene, hashes)
                    .map(Box::new)
            })
            .transpose()?;
        let instruction_pump = digests
            .instruction_pump
            .map(|hashes| {
                instruction_pump::PumpArt::load(store, scene, hashes)
                    .map(Box::new)
            })
            .transpose()?;
        let opponents = digests
            .opponents
            .map(|hashes| {
                opposition::OpponentArt::load(store, scene, hashes)
                    .map(Box::new)
            })
            .transpose()?;
        let headers = phase_headers(scene, &catalogue)?;
        let ranks = plates(scene, RANK_WIDTH, RANK_BYTES, &RANKS)?;
        Ok(Self {
            background,
            balloons,
            departure,
            headers,
            inflated,
            instruction,
            instruction_balloon,
            instruction_pump,
            opponents,
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
            BonusPhase::InstructionSetup
            | BonusPhase::Instructions
            | BonusPhase::InstructionCleanup => 0,
            BonusPhase::Ready => 1,
            BonusPhase::Pumping => 2,
            BonusPhase::Results => 3,
            BonusPhase::Complete => 4,
            BonusPhase::StageCard | BonusPhase::StageSetup => 5,
        };
        let header = self
            .headers
            .get(language)
            .ok_or(RenderError::InvalidSprite)?;
        if matches!(
            bonus.phase(),
            BonusPhase::StageCard | BonusPhase::StageSetup
        ) {
            draws.push(header.draw(phase, 16, 100)?);
            return Ok(());
        }
        if bonus.phase().is_instruction()
            && let Some(instruction) = &self.instruction
        {
            let [frame, heading, prompt] = instruction.draws(language)?;
            draws.push(frame);
            if bonus.instruction_labels_visible() {
                draws.push(heading);
                self.queue_instruction(bonus, draws)?;
                draws.push(prompt);
            }
            return Ok(());
        }
        draws.push(self.background.draw(self.background.rect, false));
        if bonus.phase().is_instruction() {
            if !bonus.instruction_labels_visible() {
                return Ok(());
            }
            self.queue_instruction(bonus, draws)?;
        } else {
            for player in Player::ALL {
                self.queue_player(bonus, (player, digits), draws)?;
            }
        }
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

    fn queue_human(
        &self,
        bonus: Bonus,
        player: Player,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let index = player.index();
        let pump = PUMPS.get(index).ok_or(RenderError::InvalidSprite)?;
        let pump_art =
            self.pumps.get(index).ok_or(RenderError::InvalidSprite)?;
        if let Some((art, departure)) =
            self.departure.as_ref().zip(bonus.departure(player))
        {
            if let BonusDeparture::Rising { balloon: rise, .. } = departure
                && let Some(layer) = self.balloon_layer(bonus, player, rise)?
            {
                draws.push(layer);
            }
            art.queue(player, departure, draws)?;
        } else if let Some(layer) = self.balloon_layer(bonus, player, 0)? {
            draws.push(layer);
            draws.push(pump_art.draw(pump.destination(0)?, false));
        }

        Ok(())
    }

    fn queue_instruction(
        &self,
        bonus: Bonus,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let Some(player) = bonus.instruction_player() else {
            return Ok(());
        };
        let index = player.index();
        let Some(balloon) = self.instruction_balloon_layer(bonus, player)?
        else {
            return Ok(());
        };
        draws.push(balloon);
        if player == Player::Marge
            && let Some(pump) = &self.instruction_pump
        {
            return pump.queue(bonus.tick(), draws);
        }
        let pump = PUMPS.get(index).ok_or(RenderError::InvalidSprite)?;
        let art = self.pumps.get(index).ok_or(RenderError::InvalidSprite)?;
        let destination = instruction_destination(
            pump.destination(0)?,
            INSTRUCTION_PUMP_ANCHOR,
        );
        draws.push(art.draw(destination, false));
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
        if bonus.is_automatic(player) {
            if bonus.visual_axes(player).is_some() {
                self.opponents
                    .as_ref()
                    .ok_or(RenderError::InvalidSprite)?
                    .queue(bonus, player, draws)?;
            }
        } else {
            self.queue_human(bonus, player, draws)?;
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

fn departure_checkpoint(
    bonus: &BonusArt,
    departure: &DepartureArt,
    draws: &[SpriteDraw],
    expected: Option<(i32, i32)>,
) -> Result<Vec<SpriteDraw>, RenderError> {
    let (balloon, mut balloon_rect) = if let Some(inflated) = &bonus.inflated {
        let layer = inflated.first().ok_or(RenderError::InvalidSprite)?;
        let spec = INFLATED.first().ok_or(RenderError::InvalidSprite)?;
        (layer, spec.inverse_destination((64, 32), 0)?)
    } else {
        let layer = bonus.balloons.first().ok_or(RenderError::InvalidSprite)?;
        let spec = BALLOONS.first().ok_or(RenderError::InvalidSprite)?;
        (layer, spec.destination(BONUS_MAX_INFLATION)?)
    };
    let hanging = departure
        .hanging
        .first()
        .ok_or(RenderError::InvalidSprite)?;
    let pump = departure.pump.draw(GROUND_PUMP.destination(0)?, false);
    let mut layers = vec![pump];
    if let Some((balloon_rise, character_rise)) = expected {
        let hanging_spec = HANGING.first().ok_or(RenderError::InvalidSprite)?;
        balloon_rect.y = balloon_rect.y.saturating_sub(balloon_rise);
        let mut hanging_rect = hanging_spec.destination(0)?;
        hanging_rect.y = hanging_rect.y.saturating_sub(character_rise);
        layers.push(balloon.draw(balloon_rect, false));
        layers.push(hanging.draw(hanging_rect, false));
    } else if draws
        .iter()
        .any(|draw| draw.atlas == balloon.atlas || draw.atlas == hanging.atlas)
    {
        return Err(failure("departed bonus layers still drawn"));
    }
    if layers.iter().any(|layer| !draws.contains(layer)) {
        return Err(failure("bonus departure layer registration differs"));
    }
    Ok(layers)
}

const fn instruction_destination(
    mut destination: SpriteRect,
    (center, bottom): (i32, i32),
) -> SpriteRect {
    destination.x =
        center.saturating_sub_unsigned(destination.width.get() >> 1);
    destination.y = bottom.saturating_sub_unsigned(destination.height.get());
    destination
}

fn inverse_extent(
    extent: u32,
    captured: u32,
    current: u8,
) -> Result<u32, RenderError> {
    let inverse = u32::from(current);
    extent
        .saturating_mul(captured)
        .saturating_add(inverse >> 1)
        .checked_div(inverse)
        .ok_or(RenderError::InvalidSprite)
}

fn load_inflated(
    store: BundleSource<'_>,
    scene: &mut SceneSession<'_, '_>,
    digest: ContentHash,
) -> Result<[Art; 4], RenderError> {
    let bundle = open_bundle(store, digest)?;
    let [marge, homer, bart, lisa] = INFLATED;
    Ok([
        marge.load(scene, &bundle)?,
        homer.load(scene, &bundle)?,
        bart.load(scene, &bundle)?,
        lisa.load(scene, &bundle)?,
    ])
}

fn phase_headers(
    scene: &mut SceneSession<'_, '_>,
    catalogue: &TranslationCatalog,
) -> Result<Vec<TextAtlas>, RenderError> {
    let mut headers = Vec::with_capacity(title_messages::LOCALES.len());
    for index in 0..title_messages::LOCALES.len() {
        let locale = title_messages::locale_tag(index)?;
        let text = title_messages::TitleText {
            catalog: catalogue,
            locale: &locale,
        };
        let mut labels = Vec::with_capacity(PHASE_KEYS.len());
        for key in PHASE_KEYS {
            labels.push(text.label(key)?);
        }
        headers.push(plates(scene, HEADER_WIDTH, HEADER_BYTES, &labels)?);
    }
    Ok(headers)
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

fn verify_registered_sprite(
    scene: &mut SceneSession<'_, '_>,
    draws: &[SpriteDraw],
    actual: &[u8],
    (expected, draw): (ContentHash, SpriteDraw),
) -> Result<(), RenderError> {
    if !draws.contains(&draw) {
        return Err(failure("registered instruction sprite was not queued"));
    }
    let width = scene.layout().width().get();
    let _dark = scene.present([0; 3], &[draw])?;
    let dark = scene.readback()?;
    let _light = scene.present([255; 3], &[draw])?;
    let light = scene.readback()?;
    let digest =
        scenery::opaque_sprite_digest(&dark, &light, draw.destination, width)?;
    if digest != expected {
        return Err(failure("registered instruction sprite differs"));
    }
    let baseline: Vec<_> = draws
        .iter()
        .copied()
        .filter(|candidate| *candidate != draw)
        .collect();
    let _baseline = scene.present([24, 22, 30], &baseline)?;
    scenery::verify_pixels(actual, &scene.readback()?, width, &[
        draw.destination
    ])
}

/// Compare the actor-free instruction backdrop to its canonical payload.
pub(super) fn verify_instruction_backdrop(
    scene: &SceneSession<'_, '_>,
    art: &Scene,
    cabinet: &Cabinet,
    pixels: &[u8],
) -> Result<(), RenderError> {
    let Some(instruction) = art
        .poses
        .bonus
        .as_ref()
        .and_then(|bonus_art| bonus_art.instruction.as_ref())
    else {
        return Ok(());
    };
    let [frame, heading, prompt] =
        instruction.draws(cabinet.session().language_index())?;
    if !art.draws.contains(&frame)
        || art.draws.contains(&heading)
        || art.draws.contains(&prompt)
    {
        return Err(failure("instruction backdrop layers differ"));
    }
    let digest = scenery::opaque_sprite_digest(
        pixels,
        pixels,
        frame.destination,
        scene.layout().width().get(),
    )?;
    if digest != instruction.frame_digest() {
        return Err(failure("instruction backdrop differs from payload"));
    }
    Ok(())
}

/// Verify localized instruction plates change only their registered bounds.
pub(super) fn verify_instruction(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let Some(instruction) = art
        .poses
        .bonus
        .as_ref()
        .and_then(|bonus_art| bonus_art.instruction.as_ref())
    else {
        return Ok(());
    };
    let language = cabinet.session().language_index();
    let alternate = usize::from(language == 0);
    let [frame, heading, prompt] = instruction.draws(language)?;
    let [other_frame, other_heading, other_prompt] =
        instruction.draws(alternate)?;
    if frame != other_frame
        || ![frame, heading, prompt]
            .iter()
            .all(|draw| art.draws.contains(draw))
    {
        return Err(failure("localized instruction layers were not queued"));
    }
    let alternate_draws: Vec<_> = art
        .draws
        .iter()
        .map(|draw| {
            if *draw == heading {
                other_heading
            } else if *draw == prompt {
                other_prompt
            } else {
                *draw
            }
        })
        .collect();
    let view = art.presentation_view(scene, cabinet)?;
    let actual = scene.readback()?;
    let _alternate = scene.present([24, 22, 30], &alternate_draws)?;
    let localized = scene.readback()?;
    let _restored = art.present(scene, cabinet)?;
    scenery::verify_pixels(&actual, &localized, view.width, &[
        heading.destination,
        prompt.destination,
    ])
}

/// Verify a single admitted automatic competitor contributes visible pixels.
pub(super) fn verify_opponent(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    player: Player,
) -> Result<(), RenderError> {
    let bonus = cabinet.bonus().ok_or(RenderError::InvalidSprite)?;
    let opponents = art
        .poses
        .bonus
        .as_ref()
        .and_then(|bonus_art| bonus_art.opponents.as_ref())
        .ok_or(RenderError::InvalidSprite)?;
    if !bonus.is_automatic(player) {
        return Err(failure("opponent readback requires automatic player"));
    }
    let mut layers = Vec::new();
    opponents.queue(bonus, player, &mut layers)?;
    if layers.is_empty()
        || layers.iter().any(|layer| !art.draws.contains(layer))
    {
        return Err(failure("automatic competitor layers were not queued"));
    }
    let view = art.presentation_view(scene, cabinet)?;
    let actual = scene.readback()?;
    let baseline_draws: Vec<_> = art
        .draws
        .iter()
        .copied()
        .filter(|draw| !layers.contains(draw))
        .collect();
    let bounds: Vec<_> = layers.iter().map(|draw| draw.destination).collect();
    let _baseline = scene.present([24, 22, 30], &baseline_draws)?;
    let baseline = scene.readback()?;
    let _restored = art.present(scene, cabinet)?;
    scenery::verify_pixels(&actual, &baseline, view.width, &bounds)
}

/// Verify the winning Marge departure at independent motion checkpoints.
pub(super) fn verify_departure(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    expected: Option<(i32, i32)>,
) -> Result<(), RenderError> {
    let Some(bonus) = &art.poses.bonus else {
        return Ok(());
    };
    let Some(departure) = &bonus.departure else {
        return Ok(());
    };
    let layers = departure_checkpoint(bonus, departure, &art.draws, expected)?;
    let view = art.presentation_view(scene, cabinet)?;
    let actual = scene.readback()?;
    let baseline_draws: Vec<_> = art
        .draws
        .iter()
        .copied()
        .filter(|draw| !layers.contains(draw))
        .collect();
    let bounds: Vec<_> = layers.iter().map(|draw| draw.destination).collect();
    let _baseline = scene.present([24, 22, 30], &baseline_draws)?;
    let baseline = scene.readback()?;
    let _restored = art.present(scene, cabinet)?;
    scenery::verify_pixels(&actual, &baseline, view.width, &bounds)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn departure_translation_preserves_sprite_extent() {
        for (sprite, inflation) in HANGING
            .into_iter()
            .map(|spec| (spec, 0))
            .chain(BALLOONS.into_iter().map(|spec| (spec, BONUS_MAX_INFLATION)))
        {
            let start = sprite.destination(inflation);
            let end = sprite.rising_destination(inflation, 5_288);
            assert!(start.is_ok());
            assert!(end.is_ok());
            let (Ok(initial), Ok(rising)) = (start, end) else {
                return;
            };
            assert_eq!(rising.x, initial.x);
            assert_eq!(rising.y, initial.y.saturating_sub(20));
            assert_eq!(rising.width, initial.width);
            assert_eq!(rising.height, initial.height);
        }
    }

    #[test]
    fn instruction_layout_anchors_selected_balloon_and_pump() {
        for (balloon, pump) in BALLOONS.into_iter().zip(PUMPS) {
            assert_instruction_anchor(balloon, INSTRUCTION_BALLOON_ANCHOR);
            assert_instruction_anchor(pump, INSTRUCTION_PUMP_ANCHOR);
        }
    }

    fn assert_instruction_anchor(spec: SpriteSpec, anchor: (i32, i32)) {
        for inflation in [0, 16, BONUS_MAX_INFLATION] {
            let result = spec.destination(inflation);
            assert!(result.is_ok());
            let Ok(original) = result else {
                return;
            };
            let actual = instruction_destination(original, anchor);
            assert_eq!(actual.width, original.width);
            assert_eq!(actual.height, original.height);
            assert_eq!(
                actual.x.saturating_add_unsigned(actual.width.get() >> 1),
                anchor.0
            );
            assert_eq!(
                actual.y.saturating_add_unsigned(actual.height.get()),
                anchor.1
            );
        }
    }

    fn assert_inverse_extents(
        spec: SpriteSpec,
        base: SpriteRect,
        wider: SpriteRect,
        taller: SpriteRect,
    ) {
        assert_eq!((base.x, base.y), (spec.x, spec.y));
        assert_eq!(
            (base.width.get(), base.height.get()),
            (spec.width, spec.height)
        );
        assert!(wider.width > base.width);
        assert_eq!(wider.height, base.height);
        assert_eq!(taller.width, base.width);
        assert!(taller.height > base.height);
        assert_eq!(
            taller.y.saturating_add_unsigned(taller.height.get()),
            160i32
        );
    }

    #[test]
    fn inverse_axes_scale_independently_from_captured_assembly() {
        for spec in INFLATED {
            let captured = spec.inverse_destination((107, 51), 0);
            let wide = spec.inverse_destination((64, 51), 0);
            let tall = spec.inverse_destination((107, 32), 0);
            assert!(captured.is_ok());
            assert!(wide.is_ok());
            assert!(tall.is_ok());
            let (Ok(base), Ok(wider), Ok(taller)) = (captured, wide, tall)
            else {
                return;
            };
            assert_inverse_extents(spec, base, wider, taller);
        }
        assert_eq!(inverse_extent(38, 107, 64), Ok(64));
        assert_eq!(inverse_extent(38, 107, 75), Ok(54));
        assert!(inverse_extent(38, 107, 0).is_err());
    }

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
