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
//   - Optional instruction frame and authored localized text placement.
// - Must-Not:
//   - Advance the bonus, import original media or select participants.
// - Allows:
//   - Inputs: Canonical opaque frame, locale and scene session.
//   - Outputs: Registered frame, heading and readable instruction draws.
//   - Side effects: Bounded atlas allocation at scene load.
// - Split-When:
//   - Independently authored instruction scenes need separate composition.
// - Merge-When:
//   - A shared title presentation contract owns this entire scene.
// - Summary:
//   - Use a locally selected language-neutral instruction frame.
// - Description:
//   - Original lettering is excluded; the title authors localized labels.
// - Usage:
//   - Load alongside the existing bonus art and queue around the demo.
// - Defaults:
//   - No allocation or presentation change without explicit selection.

//! Optional localized instruction frame.

use games::presentation::TranslationCatalog;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::BundleSource;
use games_platform::ir::ui::{PlateLayout, TextAtlas, TextLine};

use super::{Art, BACKGROUND, open_bundle, size, title_messages};

const WIDTH: u16 = 128;
const HEIGHT: u16 = 40;
const PLATE_BYTES: usize = 128 * 40 * 2 * 4;
const LOCALES: usize = title_messages::LOCALES.len();

pub(super) const ATLAS_BYTES: usize = 320 * 224 * 4 + PLATE_BYTES * LOCALES;
pub(super) const ATLAS_COUNT: usize = 1 + LOCALES;

#[derive(Debug)]
pub(super) struct InstructionArt {
    frame: Art,
    text: Vec<TextAtlas>,
}

impl InstructionArt {
    pub(super) fn draws(
        &self,
        language: usize,
    ) -> Result<[SpriteDraw; 3], RenderError> {
        let text = self.text.get(language).ok_or(RenderError::InvalidSprite)?;
        Ok([
            self.frame.draw(self.frame.rect, false),
            text.draw(0, 96, 22)?,
            text.draw(1, 134, 86)?,
        ])
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        digest: ContentHash,
        catalogue: &TranslationCatalog,
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, digest)?;
        let frame = BACKGROUND.load(scene, &bundle)?;
        let mut text = Vec::with_capacity(LOCALES);
        for index in 0..LOCALES {
            let locale = title_messages::locale_tag(index)?;
            let labels = title_messages::TitleText {
                catalog: catalogue,
                locale: &locale,
            };
            text.push(TextAtlas::load(
                scene,
                PlateLayout {
                    background: [0, 0, 0, 0],
                    height: HEIGHT,
                    max_bytes: size(PLATE_BYTES)?,
                    width: WIDTH,
                },
                &lines(labels)?,
            )?);
        }
        Ok(Self { frame, text })
    }
}

type InstructionLines<'text> = [Vec<TextLine<'text>>; 2];

fn lines(
    labels: title_messages::TitleText<'_>,
) -> Result<InstructionLines<'_>, RenderError> {
    let heading = TextLine {
        color: [255, 230, 102, 255],
        scale: 3,
        text: labels.label("title.bonus.heading")?,
        top: 10,
    }
    .fit_width(WIDTH);
    let prompt = TextLine {
        color: [255, 255, 255, 255],
        scale: 2,
        text: labels.label("title.bonus.instructions")?,
        top: 2,
    }
    .wrap(WIDTH, HEIGHT, 2)?;
    Ok([vec![heading], prompt])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_locales_fit_the_instruction_screen() -> Result<(), RenderError> {
        let catalogue = title_messages::catalogue()?;
        for index in 0..LOCALES {
            let locale = title_messages::locale_tag(index)?;
            let labels = title_messages::TitleText {
                catalog: &catalogue,
                locale: &locale,
            };
            let [heading, prompt] = lines(labels)?;
            if heading.len() != 1
                || heading.first().is_none_or(|line| line.scale < 2)
                || !(2..=3).contains(&prompt.len())
                || !prompt.iter().all(|line| line.scale == 2)
            {
                return Err(RenderError::InvalidDimensions);
            }
        }
        Ok(())
    }
}
