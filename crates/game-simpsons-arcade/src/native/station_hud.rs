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
//   - Native station join, remote status and free-rejoin indicators.
// - Must-Not:
//   - Modify connection state, the countdown or title simulation.
// - Allows:
//   - Inputs: Station index, verified connection, profile and remaining ticks.
//   - Outputs: One bounded atlas sprite for the applicable message.
//   - Side effects: One small persistent atlas allocation.
// - Split-When:
//   - General text, localisation or dynamic HUD layout is required.
// - Merge-When:
//   - A title-local HUD already owns these continue indicators.
// - Summary:
//   - Present free join, rejoin and remote availability.
// - Description:
//   - Uses independently drawn compact glyphs, not original HUD pixels.
// - Usage:
//   - Load once and update from the authenticated fixed-step cabinet.
// - Defaults:
//   - Always advertise vacant local stations during play.

//! Station join, verified remote status and free-rejoin presentation.

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

type OfflineTiles = ([Option<usize>; 4], Option<usize>);

#[derive(Clone, Copy, Debug)]
pub(super) struct StationHud {
    atlas: TextAtlas,
    connected: [bool; 4],
    offline_joins: [Option<usize>; 4],
    offline_rejoin: Option<usize>,
}

impl StationHud {
    fn choice_tile(&self, seat: Player, character: Player) -> usize {
        self.offline_join(seat)
            .unwrap_or_else(|| seat.index().saturating_mul(NAMES.len()))
            .saturating_add(character.index())
    }

    pub(super) fn draw(
        &self,
        player: Player,
        seat: Player,
        participation: Participation,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let offline = self.offline_join(seat);
        let tile = match participation {
            Participation::Absent => self.choice_tile(seat, player),
            Participation::Continuing { ticks_left } => {
                let seconds =
                    ticks_left.div_ceil(SECOND_TICKS).clamp(1, LAST_SECONDS);
                let base = if offline.is_some() {
                    self.offline_rejoin.ok_or(RenderError::InvalidSprite)?
                } else {
                    JOIN_TILES
                };
                base.saturating_add(usize::from(seconds.saturating_sub(1)))
            },
            Participation::Playing | Participation::Recovering { .. } => {
                let Some(base) = offline else {
                    return Ok(None);
                };
                base.saturating_add(player.index())
            },
        };
        let x = station_x(player)?;
        self.atlas.draw(tile, x, 2).map(Some)
    }

    pub(super) fn draw_choice(
        &self,
        seat: Player,
        character: Player,
    ) -> Result<SpriteDraw, RenderError> {
        self.atlas.draw(
            self.choice_tile(seat, character),
            station_x(seat)?,
            172,
        )
    }

    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
        profiles: &Profiles,
        reserved: [bool; 4],
    ) -> Result<Self, RenderError> {
        let joins: Vec<_> = profiles
            .keys
            .into_iter()
            .zip(reserved)
            .map(|([_, key, ..], remote)| {
                if remote {
                    return String::from("REMOTE START");
                }
                let name = key_label(key);
                if name.len() <= 11 {
                    format!("JOIN {name}")
                } else {
                    String::from("PRESS START")
                }
            })
            .collect();
        let prompts = (1..=LAST_SECONDS)
            .map(|seconds| format!("START {seconds:02}"))
            .collect::<Vec<_>>();
        let labels = Player::ALL
            .into_iter()
            .flat_map(|seat| {
                NAMES.map(|name| {
                    format!("P{} {name}", seat.index().saturating_add(1))
                })
            })
            .collect::<Vec<_>>();
        let mut plates = Vec::new();
        for (names, hint) in
            labels.as_chunks::<{ NAMES.len() }>().0.iter().zip(&joins)
        {
            for name in names {
                plates.push(plate(name, hint));
            }
        }
        for hint in &prompts {
            plates.push(plate("REJOIN", hint));
        }
        let (offline_joins, offline_rejoin) =
            offline_plates(&mut plates, reserved, &labels, &prompts);
        Ok(Self {
            atlas: TextAtlas::load(
                scene,
                PlateLayout {
                    background: BACKGROUND,
                    height: HEIGHT,
                    max_bytes: size(atlas_bytes(reserved))?,
                    width: WIDTH,
                },
                &plates,
            )?,
            connected: [false; 4],
            offline_joins,
            offline_rejoin,
        })
    }

    fn offline_join(&self, seat: Player) -> Option<usize> {
        if self.connected.get(seat.index()) == Some(&true) {
            return None;
        }
        self.offline_joins.get(seat.index()).copied().flatten()
    }

    pub(super) const fn update_connections(&mut self, connected: [bool; 4]) {
        self.connected = connected;
    }
}

fn atlas_bytes(reserved: [bool; 4]) -> usize {
    bytes(JOIN_TILES.saturating_add(usize::from(LAST_SECONDS)))
        .saturating_add(extra_bytes(reserved))
}

fn bytes(plates: usize) -> usize {
    plates
        .saturating_mul(usize::from(WIDTH))
        .saturating_mul(usize::from(HEIGHT))
        .saturating_mul(4)
}

pub(super) fn extra_bytes(reserved: [bool; 4]) -> usize {
    let remotes = reserved.into_iter().filter(|remote| *remote).count();
    if remotes == 0 {
        return 0;
    }
    bytes(
        remotes
            .saturating_mul(NAMES.len())
            .saturating_add(usize::from(LAST_SECONDS)),
    )
}

fn offline_plates<'text>(
    plates: &mut Vec<Vec<TextLine<'text>>>,
    reserved: [bool; 4],
    labels: &'text [String],
    prompts: &'text [String],
) -> OfflineTiles {
    let mut bases = [None; 4];
    for ((base, names), remote) in bases
        .iter_mut()
        .zip(labels.as_chunks::<{ NAMES.len() }>().0.iter())
        .zip(reserved)
    {
        if remote {
            *base = Some(plates.len());
            for name in names {
                plates.push(plate(name, "REMOTE OFFLINE"));
            }
        }
    }
    let rejoin = if reserved.contains(&true) {
        let base = plates.len();
        for hint in prompts {
            plates.push(plate("OFFLINE", hint));
        }
        Some(base)
    } else {
        None
    };
    (bases, rejoin)
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

#[cfg(test)]
pub(super) mod tests {
    use games_platform_graphics::linux::sprites::SceneLimits;
    use games_platform_graphics::linux::{FrameLayout, NativeWindow};

    use super::super::{DEFAULT_PROFILES, dimension};
    use super::*;

    fn pixels(
        scene: &mut SceneSession<'_, '_>,
        draw: SpriteDraw,
    ) -> Result<Vec<u8>, RenderError> {
        if !scene.present([0, 0, 0], &[draw])? {
            return Err(failure("station plate not drawn"));
        }
        scene.readback()
    }

    fn remote_pixels(
        scene: &mut SceneSession<'_, '_>,
        hud: &StationHud,
        participation: Participation,
    ) -> Result<Vec<u8>, RenderError> {
        let draw = hud
            .draw(Player::Lisa, Player::Homer, participation)?
            .ok_or_else(|| failure("remote station plate missing"))?;
        pixels(scene, draw)
    }

    fn check_remote(
        scene: &mut SceneSession<'_, '_>,
        hud: &mut StationHud,
    ) -> Result<(), RenderError> {
        let offline = remote_pixels(scene, hud, Participation::Absent)?;
        let continuing = Participation::Continuing { ticks_left: 120 };
        let offline_countdown = remote_pixels(scene, hud, continuing)?;
        let choice = hud.draw_choice(Player::Homer, Player::Lisa)?;
        let offline_choice = pixels(scene, choice)?;
        let local_choice = hud.draw_choice(Player::Marge, Player::Marge)?;
        let local = pixels(scene, local_choice)?;
        hud.update_connections([false, true, false, false]);
        let ready = remote_pixels(scene, hud, Participation::Absent)?;
        let connected_countdown = remote_pixels(scene, hud, continuing)?;
        let connected_choice =
            pixels(scene, hud.draw_choice(Player::Homer, Player::Lisa)?)?;
        let same_local =
            pixels(scene, hud.draw_choice(Player::Marge, Player::Marge)?)?;
        if offline == ready
            || offline_countdown == connected_countdown
            || offline_choice == connected_choice
            || local != same_local
            || hud
                .draw(Player::Lisa, Player::Homer, Participation::Playing)?
                .is_some()
        {
            return Err(failure("remote status presentation not updated"));
        }
        hud.update_connections([false; 4]);
        if remote_pixels(scene, hud, Participation::Playing)? != offline
            || remote_pixels(scene, hud, continuing)? != offline_countdown
        {
            return Err(failure("disconnect indicator not restored"));
        }
        Ok(())
    }

    #[test]
    fn reserves_extra_plates_only_for_hosted_sessions() {
        assert_eq!(atlas_bytes([false; 4]), 221_184);
        assert_eq!(extra_bytes([false; 4]), 0);
        assert_eq!(extra_bytes([false, true, false, false]), 147_456);
        assert_eq!(extra_bytes([false, true, true, true]), 196_608);
    }

    pub(in crate::native) fn remote_station_indicators_smoke()
    -> Result<(), RenderError> {
        let reserved = [false, true, false, true];
        let mut window = NativeWindow::open(
            "Remote station HUD smoke",
            dimension(320)?,
            dimension(224)?,
            true,
        )?;
        let layout =
            FrameLayout::new(dimension(320)?, dimension(224)?, size(286_720)?)?;
        window.with_scene(
            layout,
            SceneLimits {
                atlas_bytes: size(atlas_bytes(reserved))?,
                atlases: size(1)?,
                sprites: size(1)?,
            },
            |scene| {
                let mut hud =
                    StationHud::load(scene, &DEFAULT_PROFILES, reserved)?;
                check_remote(scene, &mut hud)
            },
        )
    }
}
