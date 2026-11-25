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
//   - Native local join and free-rejoin indicators.
// - Must-Not:
//   - Modify the countdown or title simulation.
// - Allows:
//   - Inputs: Station index, input profile and remaining ticks.
//   - Outputs: One bounded atlas sprite for the applicable message.
//   - Side effects: One small persistent atlas allocation.
// - Split-When:
//   - General text, localisation or dynamic HUD layout is required.
// - Merge-When:
//   - A title-local HUD already owns these continue indicators.
// - Summary:
//   - Present free join and rejoin opportunities.
// - Description:
//   - Uses independently drawn compact glyphs, not original HUD pixels.
// - Usage:
//   - Load once; draw for each inactive or continuing station.
// - Defaults:
//   - Always advertise vacant local stations during play.

//! Local station join and free-rejoin countdown presentation.

use games_platform_graphics::linux::RenderError;
use games_platform_graphics::linux::sprites::{SceneSession, SpriteDraw};
use games_platform_graphics::linux::ui::{PlateLayout, TextAtlas, TextLine};

use super::{Profiles, failure, key_label, size};
use crate::{CONTINUE_TICKS, Participation, Player};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 24;
const SECOND_TICKS: u16 = 60;
const LAST_SECONDS: u16 = CONTINUE_TICKS.div_ceil(SECOND_TICKS);
const JOIN_TILES: usize = 16;
const BACKGROUND: [u8; 4] = [24, 22, 30, 255];
const INK: [u8; 4] = [248, 228, 168, 255];

pub(super) const NAMES: [&str; 4] = ["MARGE", "HOMER", "BART", "LISA"];

#[derive(Clone, Copy, Debug)]
pub(super) struct StationHud {
    atlas: TextAtlas,
}

impl StationHud {
    pub(super) fn draw(
        &self,
        player: Player,
        seat: Player,
        participation: Participation,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let tile = match participation {
            Participation::Absent => join_tile(seat, player),
            Participation::Continuing { ticks_left } => {
                let seconds =
                    ticks_left.div_ceil(SECOND_TICKS).clamp(1, LAST_SECONDS);
                JOIN_TILES
                    .saturating_add(usize::from(seconds.saturating_sub(1)))
            },
            Participation::Playing => return Ok(None),
        };
        let x = station_x(player)?;
        self.atlas.draw(tile, x, 2).map(Some)
    }

    pub(super) fn draw_choice(
        &self,
        seat: Player,
        character: Player,
    ) -> Result<SpriteDraw, RenderError> {
        self.atlas
            .draw(join_tile(seat, character), station_x(seat)?, 172)
    }

    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
        profiles: &Profiles,
    ) -> Result<Self, RenderError> {
        let joins = profiles.keys.map(|[_, key, ..]| {
            let name = key_label(key);
            if name.len() <= 11 {
                format!("JOIN {name}")
            } else {
                String::from("PRESS START")
            }
        });
        let prompts = (1..=LAST_SECONDS)
            .map(|seconds| format!("START {seconds:02}"))
            .collect::<Vec<_>>();
        let mut plates = Vec::new();
        let labels = Player::ALL
            .into_iter()
            .flat_map(|seat| {
                NAMES.map(|name| {
                    format!("P{} {name}", seat.index().saturating_add(1))
                })
            })
            .collect::<Vec<_>>();
        for (index, hint) in joins.iter().enumerate() {
            let start = index.saturating_mul(NAMES.len());
            let end = start.saturating_add(NAMES.len());
            let names =
                labels.get(start..end).ok_or(RenderError::InvalidSprite)?;
            for name in names {
                plates.push(plate(name, hint));
            }
        }
        for hint in &prompts {
            plates.push(plate("REJOIN", hint));
        }
        Ok(Self {
            atlas: TextAtlas::load(
                scene,
                PlateLayout {
                    background: BACKGROUND,
                    height: HEIGHT,
                    max_bytes: size(221_184)?,
                    width: WIDTH,
                },
                &plates,
            )?,
        })
    }
}

const fn join_tile(seat: Player, character: Player) -> usize {
    seat.index()
        .saturating_mul(NAMES.len())
        .saturating_add(character.index())
}

fn station_x(player: Player) -> Result<i32, RenderError> {
    i32::try_from(player.index().saturating_mul(80).saturating_add(8))
        .map_err(failure)
}

fn plate<'text>(top: &'text str, bottom: &'text str) -> Vec<TextLine<'text>> {
    vec![
        TextLine {
            color: INK,
            scale: 2,
            text: top,
            top: 1,
        }
        .fit_width(WIDTH),
        TextLine {
            color: INK,
            scale: 2,
            text: bottom,
            top: 13,
        }
        .fit_width(WIDTH),
    ]
}
