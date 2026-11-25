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
//   - Native selection, active remote status and free-rejoin indicators.
// - Must-Not:
//   - Modify connection state, the countdown or title simulation.
// - Allows:
//   - Inputs: Station index, verified connection, profile and remaining ticks.
//   - Outputs: Bounded atlas sprites for the applicable message.
//   - Side effects: One small persistent atlas allocation.
// - Split-When:
//   - General text, localisation or dynamic HUD layout is required.
// - Merge-When:
//   - A title-local HUD already owns these continue indicators.
// - Summary:
//   - Present selection, rejoin and active remote availability.
// - Description:
//   - Uses independently drawn compact glyphs, not original HUD pixels.
// - Usage:
//   - Load once and update from the authenticated fixed-step cabinet.
// - Defaults:
//   - Hide vacant stations and control bindings during play.

//! Station join, verified remote status and free-rejoin presentation.

use games_platform_graphics::frame::RenderError;
use games_platform_graphics::linux::sprites::SceneSession;
use games_platform_graphics::scene::SpriteDraw;
use games_platform_graphics::ui::{PlateLayout, TextAtlas, TextLine};

use super::{Profiles, failure, key_label, size};
use crate::{CONTINUE_TICKS, Participation, Player};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 24;
const BINDING_TOP: u16 = 12;
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

    fn draw(
        &self,
        player: Player,
        seat: Player,
        participation: Participation,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let offline = self.offline_join(seat);
        let tile = match participation {
            Participation::Absent => return Ok(None),
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
            .zip(profiles.buttons)
            .map(|([_, key, ..], [_, button, ..])| {
                [
                    binding_hint("JOIN", key.map_or("NONE", key_label)),
                    binding_hint("PAD", button.profile_name()),
                ]
            })
            .collect();
        let prompts = (1..=LAST_SECONDS)
            .map(|seconds| format!("REJOIN{seconds:02}"))
            .collect::<Vec<_>>();
        let labels = Player::ALL
            .into_iter()
            .flat_map(|seat| {
                NAMES.map(|name| {
                    format!("P{} {name}", seat.index().saturating_add(1))
                })
            })
            .collect::<Vec<_>>();
        let mut plates = join_plates(&labels, &joins, reserved);
        for timer in &prompts {
            plates.push(plate(timer, ""));
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

    pub(super) fn queue(
        &self,
        (player, seat): (Player, Player),
        (participation, offset): (Participation, i32),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<bool, RenderError> {
        let Some(mut prompt) = self.draw(player, seat, participation)? else {
            return Ok(false);
        };
        prompt.destination.x = prompt.destination.x.saturating_add(offset);
        draws.push(prompt);
        Ok(true)
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
            plates.push(plate(hint, "OFFLINE"));
        }
        Some(base)
    } else {
        None
    };
    (bases, rejoin)
}

fn join_plates<'text>(
    labels: &'text [String],
    joins: &'text [[String; 2]],
    reserved: [bool; 4],
) -> Vec<Vec<TextLine<'text>>> {
    let mut plates = Vec::with_capacity(JOIN_TILES);
    for ((names, hints), remote) in labels
        .as_chunks::<{ NAMES.len() }>()
        .0
        .iter()
        .zip(joins)
        .zip(reserved)
    {
        if remote {
            plates.extend(names.iter().map(|name| plate(name, "REMOTE START")));
        } else {
            plates.extend(names.iter().map(|name| binding_plate(name, hints)));
        }
    }
    plates
}

fn station_x(player: Player) -> Result<i32, RenderError> {
    i32::try_from(player.index().saturating_mul(80).saturating_add(8))
        .map_err(failure)
}

fn binding_hint(prefix: &str, name: &str) -> String {
    let compact = name.replace("Keypad ", "KP ");
    let hint = format!("{prefix} {compact}");
    if hint.len() <= 16 {
        hint
    } else if compact.len() <= 14 {
        let short = if prefix == "PAD" {
            "P"
        } else {
            "K"
        };
        format!("{short} {compact}")
    } else if compact.len() <= 16 {
        compact
    } else {
        // Some uncommon SDL names cannot fit the compact 64-pixel plate.
        // Refer to the actual profile instead of suggesting a wrong key.
        format!("{prefix} PROFILE")
    }
}

fn binding_plate<'text>(
    name: &'text str,
    hints: &'text [String; 2],
) -> Vec<TextLine<'text>> {
    let [keyboard, controller] = hints;
    vec![
        TextLine {
            color: INK,
            scale: 2,
            text: name,
            top: 1,
        }
        .fit_width(WIDTH),
        TextLine {
            color: INK,
            scale: 1,
            text: keyboard,
            top: BINDING_TOP,
        },
        TextLine {
            color: INK,
            scale: 1,
            text: controller,
            top: BINDING_TOP + 6,
        },
    ]
}

fn plate<'text>(top: &'text str, bottom: &'text str) -> Vec<TextLine<'text>> {
    let mut lines = vec![
        TextLine {
            color: INK,
            scale: 2,
            text: top,
            top: 1,
        }
        .fit_width(WIDTH),
    ];
    if !bottom.is_empty() {
        lines.push(
            TextLine {
                color: INK,
                scale: 2,
                text: bottom,
                top: 13,
            }
            .fit_width(WIDTH),
        );
    }
    lines
}

#[cfg(test)]
pub(super) mod tests {
    use games_platform_graphics::frame::FrameLayout;
    use games_platform_graphics::linux::NativeWindow;
    use games_platform_graphics::scene::SceneLimits;

    use super::super::{configured_profiles, dimension};
    use super::*;

    const STRIDE: usize = 320 * 4;

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
        let offline = remote_pixels(scene, hud, Participation::Playing)?;
        if hud
            .draw(Player::Lisa, Player::Homer, Participation::Absent)?
            .is_some()
        {
            return Err(failure("vacant remote seat rendered"));
        }
        let continuing = Participation::Continuing { ticks_left: 120 };
        let offline_countdown = remote_pixels(scene, hud, continuing)?;
        let choice = hud.draw_choice(Player::Homer, Player::Lisa)?;
        let offline_choice = pixels(scene, choice)?;
        let local_choice = hud.draw_choice(Player::Marge, Player::Marge)?;
        let local = pixels(scene, local_choice)?;
        hud.update_connections([false, true, false, false]);
        if hud
            .draw(Player::Lisa, Player::Homer, Participation::Absent)?
            .is_some()
        {
            return Err(failure("vacant remote seat rendered"));
        }
        let connected_countdown = remote_pixels(scene, hud, continuing)?;
        let connected_choice =
            pixels(scene, hud.draw_choice(Player::Homer, Player::Lisa)?)?;
        let same_local =
            pixels(scene, hud.draw_choice(Player::Marge, Player::Marge)?)?;
        if offline_countdown == connected_countdown
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

    fn local_pixels(
        scene: &mut SceneSession<'_, '_>,
        hud: &StationHud,
        participation: Participation,
    ) -> Result<Vec<u8>, RenderError> {
        let mut draws = Vec::with_capacity(2);
        if !hud.queue(
            (Player::Lisa, Player::Marge),
            (participation, 0i32),
            &mut draws,
        )? || !scene.present([0, 0, 0], &draws)?
        {
            return Err(failure("local station prompt not drawn"));
        }
        scene.readback()
    }

    fn check_local(
        scene: &mut SceneSession<'_, '_>,
        hud: &StationHud,
    ) -> Result<(), RenderError> {
        let mut draws = Vec::new();
        if hud.queue(
            (Player::Lisa, Player::Marge),
            (Participation::Absent, 0i32),
            &mut draws,
        )? || !draws.is_empty()
        {
            return Err(failure("vacant local seat rendered"));
        }
        let joined =
            pixels(scene, hud.draw_choice(Player::Marge, Player::Lisa)?)?;
        let first = local_pixels(scene, hud, Participation::Continuing {
            ticks_left: 120,
        })?;
        let second = local_pixels(scene, hud, Participation::Continuing {
            ticks_left: 60,
        })?;
        let start = 2usize
            .saturating_add(usize::from(BINDING_TOP))
            .saturating_mul(STRIDE);
        let end = 2usize
            .saturating_add(usize::from(HEIGHT))
            .saturating_mul(STRIDE);
        if first == second || first.get(start..end) != second.get(start..end) {
            return Err(failure("rejoin countdown did not update"));
        }
        for row in first.as_chunks::<STRIDE>().0.iter().skip(14).take(12) {
            check_binding_row(row, 248, 0, &[])?;
        }
        let hints = [String::from("JOIN F1"), String::from("P rightshoulder")];
        let expected = binding_plate("P1 LISA", &hints);
        for (row, pixels) in joined
            .as_chunks::<STRIDE>()
            .0
            .iter()
            .skip(172)
            .take(usize::from(HEIGHT))
            .enumerate()
        {
            check_binding_row(
                pixels,
                8,
                u16::try_from(row).map_err(failure)?,
                &expected,
            )?;
        }
        Ok(())
    }

    fn check_binding_row(
        pixels: &[u8],
        x: usize,
        row: u16,
        expected: &[TextLine<'_>],
    ) -> Result<(), RenderError> {
        for (column, pixel) in pixels
            .as_chunks::<4>()
            .0
            .iter()
            .skip(x)
            .take(usize::from(WIDTH))
            .enumerate()
        {
            let horizontal = u16::try_from(column).map_err(failure)?;
            let color = if expected
                .iter()
                .any(|line| line.pixel(horizontal, row, WIDTH))
            {
                INK
            } else {
                BACKGROUND
            };
            if *pixel != color {
                return Err(failure("incorrect remapped local join label"));
            }
        }
        Ok(())
    }

    #[test]
    fn compact_binding_hints_preserve_ordinary_names() {
        assert_eq!(binding_hint("JOIN", "Enter"), "JOIN Enter");
        assert_eq!(binding_hint("JOIN", "Keypad Enter"), "JOIN KP Enter");
        assert_eq!(binding_hint("PAD", "rightshoulder"), "P rightshoulder");
        assert_eq!(
            binding_hint("JOIN", "Keypad Memory Multiply"),
            "JOIN PROFILE",
        );
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
                sprites: size(2)?,
            },
            |scene| {
                let profiles = configured_profiles(
                    "version=1\nkey 1 start=F1\nbutton 1 start=rightshoulder",
                )
                .map_err(failure)?;
                let mut hud = StationHud::load(scene, &profiles, reserved)?;
                check_remote(scene, &mut hud)?;
                check_local(scene, &hud)
            },
        )
    }
}
