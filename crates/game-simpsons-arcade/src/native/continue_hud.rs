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
//   - Experimental native continue and paid-start indicators.
// - Must-Not:
//   - Modify the countdown, credits or title simulation.
// - Allows:
//   - Inputs: Station index, remaining ticks and paid status.
//   - Outputs: One bounded atlas sprite for the applicable message.
//   - Side effects: One small persistent atlas allocation.
// - Split-When:
//   - General text, localisation or dynamic HUD layout is required.
// - Merge-When:
//   - A title-local HUD already owns these continue indicators.
// - Summary:
//   - Present the observed two-step continuation opportunity.
// - Description:
//   - Uses independently drawn compact glyphs, not original HUD pixels.
// - Usage:
//   - Load once with combat art; draw once per continuing station.
// - Defaults:
//   - No continue atlas without the optional combat encounter.

//! Prototype countdown and paid-start presentation.

use games_platform_graphics::linux::sprites::{
    AtlasId, SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};

use super::{Art, dimension, failure, glyph, size};
use crate::{CONTINUE_TICKS, Player};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 24;
const SECOND_TICKS: u16 = 60;
const LAST_SECONDS: u16 = CONTINUE_TICKS.div_ceil(SECOND_TICKS);
const PAID_TILE: u16 = LAST_SECONDS;
const BACKGROUND: [u8; 4] = [24, 22, 30, 255];
const INK: [u8; 4] = [248, 228, 168, 255];

#[derive(Debug)]
pub(super) struct ContinueArt {
    atlas: AtlasId,
}

impl ContinueArt {
    pub(super) fn draw(
        &self,
        player: Player,
        paid: bool,
        ticks_left: u16,
    ) -> Result<SpriteDraw, RenderError> {
        let tile = if paid {
            PAID_TILE
        } else {
            ticks_left
                .div_ceil(SECOND_TICKS)
                .clamp(1, LAST_SECONDS)
                .saturating_sub(1)
        };
        let rect = SpriteRect {
            height: dimension(u32::from(HEIGHT))?,
            width: dimension(u32::from(WIDTH))?,
            x: 0,
            y: i32::from(tile.saturating_mul(HEIGHT)),
        };
        let art = Art { atlas: self.atlas, rect };
        Ok(art.draw(
            SpriteRect {
                x: i32::try_from(
                    player.index().saturating_mul(80).saturating_add(8),
                )
                .map_err(failure)?,
                y: 2,
                ..rect
            },
            false,
        ))
    }

    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
    ) -> Result<Self, RenderError> {
        let atlas_height = HEIGHT.saturating_mul(PAID_TILE.saturating_add(1));
        let layout = FrameLayout::new(
            dimension(u32::from(WIDTH))?,
            dimension(u32::from(atlas_height))?,
            size(131_072)?,
        )?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(layout.bytes()).map_err(failure)?;
        for tile in 0..=PAID_TILE {
            let seconds = format!("{:02}", tile.saturating_add(1));
            let (top, bottom) = if tile == PAID_TILE {
                ("PRESS", "START")
            } else {
                ("CONTINUE", seconds.as_str())
            };
            plate(&mut pixels, top, bottom);
        }
        Ok(Self {
            atlas: scene.load_atlas(layout, &pixels)?,
        })
    }
}

fn plate(pixels: &mut Vec<u8>, top: &str, bottom: &str) {
    for row in 0..HEIGHT {
        for column in 0..WIDTH {
            let ink = text_pixel(top, column, row, 1)
                || text_pixel(bottom, column, row, 13);
            pixels.extend_from_slice(if ink {
                &INK
            } else {
                &BACKGROUND
            });
        }
    }
}

fn text_pixel(text: &str, column: u16, row: u16, top: u16) -> bool {
    let Ok(length) = u16::try_from(text.len()) else {
        return false;
    };
    let text_width = length.saturating_mul(8).saturating_sub(2);
    let left = WIDTH.saturating_sub(text_width).checked_div(2).unwrap_or(0);
    let Some(local_x) = column.checked_sub(left) else {
        return false;
    };
    let Some(local_y) = row.checked_sub(top) else {
        return false;
    };
    let character = local_x.checked_div(8).unwrap_or(0);
    let glyph_x = local_x
        .checked_rem(8)
        .unwrap_or(0)
        .checked_div(2)
        .unwrap_or(0);
    let glyph_y = local_y.checked_div(2).unwrap_or(0);
    text.as_bytes()
        .get(usize::from(character))
        .is_some_and(|letter| glyph::ink(&rows(*letter), 3, glyph_x, glyph_y))
}

const fn rows(letter: u8) -> [u8; 5] {
    match letter {
        b'0' | b'O' => [7, 5, 5, 5, 7],
        b'1' => [2, 6, 2, 2, 7],
        b'2' => [7, 1, 7, 4, 7],
        b'3' => [7, 1, 7, 1, 7],
        b'4' => [5, 5, 7, 1, 1],
        b'5' | b'S' => [7, 4, 7, 1, 7],
        b'6' => [7, 4, 7, 5, 7],
        b'7' => [7, 1, 2, 2, 2],
        b'8' => [7, 5, 7, 5, 7],
        b'9' => [7, 5, 7, 1, 7],
        b'A' => [2, 5, 7, 5, 5],
        b'C' => [7, 4, 4, 4, 7],
        b'E' => [7, 4, 6, 4, 7],
        b'I' => [7, 2, 2, 2, 7],
        b'N' => [5, 7, 7, 5, 5],
        b'P' => [6, 5, 6, 4, 4],
        b'R' => [6, 5, 6, 5, 5],
        b'T' => [7, 2, 2, 2, 2],
        b'U' => [5, 5, 5, 5, 7],
        _ => [0; 5],
    }
}
