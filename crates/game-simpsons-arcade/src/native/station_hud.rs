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
        participation: Participation,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let tile = match participation {
            Participation::Absent => player.index(),
            Participation::Continuing { ticks_left } => {
                let seconds =
                    ticks_left.div_ceil(SECOND_TICKS).clamp(1, LAST_SECONDS);
                NAMES
                    .len()
                    .saturating_add(usize::from(seconds.saturating_sub(1)))
            },
            Participation::Playing => return Ok(None),
        };
        let x =
            i32::try_from(player.index().saturating_mul(80).saturating_add(8))
                .map_err(failure)?;
        self.atlas.draw(tile, x, 2).map(Some)
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
        for (name, hint) in NAMES.into_iter().zip(&joins) {
            plates.push(plate(name, hint));
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
                    max_bytes: size(147_456)?,
                    width: WIDTH,
                },
                &plates,
            )?,
        })
    }
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
