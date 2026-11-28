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
//   - Native selection, remote status, free-rejoin indicators and score.
// - Must-Not:
//   - Modify connection state, the countdown or title simulation.
// - Allows:
//   - Inputs: Station index, connection, profile, remaining ticks and score.
//   - Outputs: Bounded atlas sprites for the applicable message.
//   - Side effects: Bounded persistent atlas allocations.
// - Split-When:
//   - General text, localisation or dynamic HUD layout is required.
// - Merge-When:
//   - A title-local HUD already owns these continue indicators.
// - Summary:
//   - Present selection, rejoin, remote availability and score.
// - Description:
//   - Uses independently drawn compact glyphs, not original HUD pixels.
// - Usage:
//   - Load once and update from the authenticated fixed-step cabinet.
// - Defaults:
//   - Hide vacant stations and control bindings during play.

//! Station join, remote status, free-rejoin and score presentation.

mod messages;

use games::input::physical::{ControllerButton, KeyboardKey, PointerButton};
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::ui::{PlateLayout, TextAtlas, TextLine};
pub(super) use messages::{
    STATION_MESSAGE_KEYS, StationText, register_station_messages,
};
use messages::{number, text};

use super::{Profiles, failure, key_label, size, title_messages};
use crate::{CONTINUE_TICKS, Participation, Player};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 24;
const DIGIT_WIDTH: u16 = 6;
const DIGIT_HEIGHT: u16 = 10;
// Eleven 6-by-10 RGBA glyph plates shared across locales.
pub(super) const SCORE_BYTES: usize = 11 * 6 * 10 * 4;

/// At most four decimal glyphs per active station.
pub(super) const SCORE_SPRITES: usize = 4 * 4;
const BINDING_TOP: u16 = 12;
const SECOND_TICKS: u16 = 60;
const LAST_SECONDS: u16 = CONTINUE_TICKS.div_ceil(SECOND_TICKS);
const JOIN_TILES: usize = 16;
const BACKGROUND: [u8; 4] = [24, 22, 30, 255];
const INK: [u8; 4] = [248, 228, 168, 255];

pub(super) const NAMES: [&str; 4] = ["MARGE", "HOMER", "BART", "LISA"];

type OfflineTiles = ([Option<usize>; 4], Option<usize>);
type LabelsAndPrompts<'text> = (&'text [String], &'text [String]);
type Plates<'text> = Vec<Vec<TextLine<'text>>>;

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

    pub(super) fn load_localised(
        scene: &mut SceneSession<'_, '_>,
        profiles: &Profiles,
        reserved: [bool; 4],
        messages: StationText<'_>,
    ) -> Result<Self, RenderError> {
        let joins: Vec<_> = profiles
            .keys
            .into_iter()
            .zip(profiles.buttons)
            .zip(profiles.mouse)
            .map(|(([_, key, ..], [_, button, ..]), [_, mouse, ..])| {
                join_hints(key, mouse, button, messages)
            })
            .collect::<Result<_, _>>()?;
        let prompts = rejoin_prompts(messages)?;
        let labels = station_labels(messages)?;
        let mut plates = join_plates(&labels, &joins, reserved, messages)?;
        for timer in &prompts {
            plates.push(plate(timer, ""));
        }
        let (offline_joins, offline_rejoin) = offline_plates(
            &mut plates,
            reserved,
            (&labels, &prompts),
            messages,
        )?;
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

pub(super) fn queue_centiseconds(
    digits: &TextAtlas,
    value: u16,
    (right, top): (i32, i32),
    draws: &mut Vec<SpriteDraw>,
) -> Result<(), RenderError> {
    let hundredths = value.checked_rem(100).unwrap_or_default();
    let seconds = value.checked_div(100).unwrap_or_default();
    let left = draw_number(digits, (hundredths, 2), (right, top), draws)?
        .saturating_sub(i32::from(DIGIT_WIDTH));
    draws.push(digits.draw(10, left, top)?);
    queue_decimal(digits, seconds, (left, top), draws)
}

pub(super) fn queue_decimal(
    digits: &TextAtlas,
    value: u16,
    anchor: (i32, i32),
    draws: &mut Vec<SpriteDraw>,
) -> Result<(), RenderError> {
    draw_number(digits, (value, 1), anchor, draws).map(|_| ())
}

pub(super) fn queue_score(
    digits: &TextAtlas,
    player: Player,
    (score, offset, prompted): (u16, i32, bool),
    draws: &mut Vec<SpriteDraw>,
) -> Result<(), RenderError> {
    let right = station_x(player)?.saturating_add(offset).saturating_add(64);
    let top = if prompted {
        28i32
    } else {
        2i32
    };
    queue_decimal(digits, score, (right, top), draws)
}

fn draw_number(
    digits: &TextAtlas,
    (mut remaining, minimum): (u16, u8),
    (mut left, top): (i32, i32),
    draws: &mut Vec<SpriteDraw>,
) -> Result<i32, RenderError> {
    for index in 0..4u8 {
        left = left.saturating_sub(i32::from(DIGIT_WIDTH));
        let digit = usize::from(remaining.checked_rem(10).unwrap_or_default());
        draws.push(digits.draw(digit, left, top)?);
        remaining = remaining.checked_div(10).unwrap_or_default();
        if remaining == 0 && index.saturating_add(1) >= minimum {
            break;
        }
    }
    Ok(left)
}

pub(super) fn load_digits(
    scene: &mut SceneSession<'_, '_>,
) -> Result<TextAtlas, RenderError> {
    let plates =
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "."].map(|digit| {
            vec![TextLine {
                color: INK,
                scale: 2,
                text: digit,
                top: 0,
            }]
        });
    TextAtlas::load(
        scene,
        PlateLayout {
            background: BACKGROUND,
            height: DIGIT_HEIGHT,
            max_bytes: size(SCORE_BYTES)?,
            width: DIGIT_WIDTH,
        },
        &plates,
    )
}

fn rejoin_prompts(
    messages: StationText<'_>,
) -> Result<Vec<String>, RenderError> {
    (1..=LAST_SECONDS)
        .map(|seconds| {
            messages.format("station.rejoin", &[text(
                "seconds",
                &format!("{seconds:02}"),
            )])
        })
        .collect()
}

fn station_labels(
    messages: StationText<'_>,
) -> Result<Vec<String>, RenderError> {
    Player::ALL
        .into_iter()
        .flat_map(|seat| {
            NAMES.map(|name| {
                let player = u64::try_from(seat.index().saturating_add(1))
                    .map_err(failure)?;
                messages.format("station.character", &[
                    number("seat", player),
                    text("name", name),
                ])
            })
        })
        .collect()
}

pub(super) fn atlas_bytes(reserved: [bool; 4]) -> usize {
    bytes(JOIN_TILES.saturating_add(usize::from(LAST_SECONDS)))
        .saturating_add(extra_bytes(reserved))
}

fn bytes(plates: usize) -> usize {
    plates
        .saturating_mul(usize::from(WIDTH))
        .saturating_mul(usize::from(HEIGHT))
        .saturating_mul(4)
}

fn extra_bytes(reserved: [bool; 4]) -> usize {
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
    (labels, prompts): LabelsAndPrompts<'text>,
    messages: StationText<'text>,
) -> Result<OfflineTiles, RenderError> {
    let remote_offline = messages.literal("station.remote.offline")?;
    let offline = messages.literal("station.offline")?;
    let mut bases = [None; 4];
    for ((base, names), remote) in bases
        .iter_mut()
        .zip(labels.as_chunks::<{ NAMES.len() }>().0.iter())
        .zip(reserved)
    {
        if remote {
            *base = Some(plates.len());
            for name in names {
                plates.push(plate(name, remote_offline));
            }
        }
    }
    let rejoin = if reserved.contains(&true) {
        let base = plates.len();
        for hint in prompts {
            plates.push(plate(hint, offline));
        }
        Some(base)
    } else {
        None
    };
    Ok((bases, rejoin))
}

fn join_plates<'text>(
    labels: &'text [String],
    joins: &'text [Vec<String>],
    reserved: [bool; 4],
    messages: StationText<'text>,
) -> Result<Plates<'text>, RenderError> {
    let remote_start = messages.literal("station.remote.start")?;
    let mut plates = Vec::with_capacity(JOIN_TILES);
    for ((names, hints), remote) in labels
        .as_chunks::<{ NAMES.len() }>()
        .0
        .iter()
        .zip(joins)
        .zip(reserved)
    {
        if remote {
            plates.extend(names.iter().map(|name| plate(name, remote_start)));
        } else {
            plates.extend(names.iter().map(|name| binding_plate(name, hints)));
        }
    }
    Ok(plates)
}

fn station_x(player: Player) -> Result<i32, RenderError> {
    i32::try_from(player.index().saturating_mul(80).saturating_add(8))
        .map_err(failure)
}

fn join_hints(
    key: Option<KeyboardKey>,
    mouse: Option<PointerButton>,
    controller: ControllerButton,
    messages: StationText<'_>,
) -> Result<Vec<String>, RenderError> {
    let mut hints = Vec::with_capacity(3);
    if let Some(binding) = key {
        hints.push(binding_hint("station.join", key_label(binding), messages)?);
    } else if mouse.is_none() {
        hints.push(String::from(messages.literal("station.join.none")?));
    }
    if let Some(button) = mouse {
        hints.push(binding_hint(
            "station.mouse",
            messages.literal(title_messages::mouse_key(button))?,
            messages,
        )?);
    }
    hints.push(binding_hint(
        "station.pad",
        controller.profile_name(),
        messages,
    )?);
    Ok(hints)
}

fn fits_hint(text: &str) -> bool {
    TextLine {
        color: INK,
        scale: 1,
        text,
        top: 0,
    }
    .fit_width(WIDTH)
    .scale
        == 1
}

fn binding_hint(
    key: &str,
    name: &str,
    messages: StationText<'_>,
) -> Result<String, RenderError> {
    let compact = if let Some(suffix) = name.strip_prefix("Keypad ") {
        format!("{} {suffix}", messages.literal("station.keypad")?)
    } else {
        String::from(name)
    };
    let prefix = messages.literal(key)?;
    let format = |kind, binding| {
        messages.format("station.binding", &[
            text("kind", kind),
            text("binding", binding),
        ])
    };
    let hint = format(prefix, &compact)?;
    if fits_hint(&hint) {
        return Ok(hint);
    }
    let short = if key == "station.pad" {
        "station.short.pad"
    } else {
        "station.short.key"
    };
    let abbreviated = format(messages.literal(short)?, &compact)?;
    if fits_hint(&abbreviated) {
        Ok(abbreviated)
    } else if fits_hint(&compact) {
        Ok(compact)
    } else {
        // Some uncommon physical names cannot fit the compact plate.
        // Refer to the actual profile instead of suggesting a wrong key.
        format(prefix, messages.literal("station.profile")?)
    }
}

fn binding_plate<'text>(
    name: &'text str,
    hints: &'text [String],
) -> Vec<TextLine<'text>> {
    let compact = hints.len() > 2;
    let mut lines = vec![
        TextLine {
            color: INK,
            scale: if compact {
                1
            } else {
                2
            },
            text: name,
            top: 1,
        }
        .fit_width(WIDTH),
    ];
    let mut top = if compact {
        7
    } else {
        BINDING_TOP
    };
    for hint in hints {
        lines.push(TextLine {
            color: INK,
            scale: 1,
            text: hint,
            top,
        });
        top = top.saturating_add(6);
    }
    lines
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
    use games_platform::ir::frame::FrameLayout;
    use games_platform::ir::graphics::NativeWindow;
    use games_platform::ir::scene::SceneLimits;

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
        expected: &[TextLine<'_>],
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
                expected,
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

    fn require(condition: bool, message: &str) -> Result<(), RenderError> {
        condition.then_some(()).ok_or_else(|| failure(message))
    }

    #[test]
    fn compact_hints_preserve_ordinary_names() -> Result<(), RenderError> {
        let catalog = title_messages::catalogue()?;
        let locale = title_messages::locale_tag(0)?;
        let messages = StationText {
            catalog: &catalog,
            locale: &locale,
        };
        for (key, binding, expected) in [
            ("station.join", "Enter", "JOIN Enter"),
            ("station.join", "Keypad Enter", "JOIN KP Enter"),
            ("station.pad", "rightshoulder", "P rightshoulder"),
            ("station.join", "Keypad Memory Multiply", "JOIN PROFILE"),
        ] {
            require(
                binding_hint(key, binding, messages)? == expected,
                "incorrect compact English binding",
            )?;
        }
        Ok(())
    }

    #[test]
    fn join_hints_show_admitted_mouse_and_keyboard_bindings()
    -> Result<(), RenderError> {
        let catalog = title_messages::catalogue()?;
        let locale = title_messages::locale_tag(0)?;
        let messages = StationText {
            catalog: &catalog,
            locale: &locale,
        };
        let controller = ControllerButton::Start;
        let key = Some(KeyboardKey::ENTER);
        let mouse = Some(PointerButton::Right);
        require(
            join_hints(key, None, controller, messages)?
                == ["JOIN ENTER", "PAD start"],
            "wrong keyboard-only hints",
        )?;
        require(
            join_hints(None, mouse, controller, messages)?
                == ["MOUSE RIGHT", "PAD start"],
            "wrong mouse-only hints",
        )?;
        let both = join_hints(key, mouse, controller, messages)?;
        require(
            both == ["JOIN ENTER", "MOUSE RIGHT", "PAD start"],
            "wrong mixed join hints",
        )?;
        let plate = binding_plate("P1 MARGE", &both);
        require(
            plate.iter().map(|line| line.text).collect::<Vec<_>>()
                == ["P1 MARGE", "JOIN ENTER", "MOUSE RIGHT", "PAD start"],
            "wrong mixed plate lines",
        )?;
        require(
            plate.iter().all(|line| {
                line.scale > 0
                    && line.top.saturating_add(line.scale.saturating_mul(5))
                        <= HEIGHT
            }),
            "mixed plate exceeded its height",
        )
    }

    #[test]
    fn mexican_spanish_preserves_station_identity_and_countdown()
    -> Result<(), RenderError> {
        let catalog = title_messages::catalogue()?;
        let english = title_messages::locale_tag(0)?;
        let spanish = title_messages::locale_tag(1)?;
        for locale in [&english, &spanish] {
            require(
                catalog
                    .first_missing(locale, STATION_MESSAGE_KEYS)
                    .is_none(),
                "missing station message",
            )?;
        }
        let messages = StationText {
            catalog: &catalog,
            locale: &spanish,
        };
        let labels = station_labels(messages)?;
        require(
            labels.first().map(String::as_str) == Some("J1 MARGE")
                && labels.last().map(String::as_str) == Some("J4 LISA"),
            "translation changed station identity",
        )?;
        let prompts = rejoin_prompts(messages)?;
        require(
            prompts.first().map(String::as_str) == Some("VOLVER 01")
                && prompts.len() == usize::from(LAST_SECONDS),
            "translation changed countdown",
        )?;
        require(
            messages.literal("station.offline")? == "SIN CONEXI\u{d3}N",
            "missing offline label",
        )
    }

    #[test]
    fn mexican_spanish_bindings_fit_compact_station_plates()
    -> Result<(), RenderError> {
        let catalog = title_messages::catalogue()?;
        let locale = title_messages::locale_tag(1)?;
        let messages = StationText {
            catalog: &catalog,
            locale: &locale,
        };
        let hints = join_hints(
            Some(KeyboardKey::ENTER),
            Some(PointerButton::Right),
            ControllerButton::RightShoulder,
            messages,
        )?;
        require(
            hints == ["UNIRSE ENTER", "RAT\u{d3}N DERECHO", "C rightshoulder"]
                && hints.iter().all(|hint| fits_hint(hint)),
            "wrong or oversized Spanish hints",
        )?;
        for (binding, expected) in [
            ("Keypad Memory Multiply", "UNIRSE PERFIL"),
            ("Keypad Enter", "UNIRSE NUM Enter"),
        ] {
            require(
                binding_hint("station.join", binding, messages)? == expected,
                "wrong compact Spanish binding",
            )?;
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

    pub(in crate::runtime) fn remote_station_indicators_smoke()
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
                atlas_bytes: size(
                    atlas_bytes(reserved)
                        .saturating_mul(2)
                        .saturating_add(SCORE_BYTES),
                )?,
                atlases: size(3)?,
                sprites: size(5)?,
            },
            |scene| {
                let profiles = configured_profiles(concat!(
                    "version=1\nkey 1 start=F1\n",
                    "button 1 start=rightshoulder\nmouse 1 start=right\n",
                ))
                .map_err(failure)?;
                let digits = load_digits(scene)?;
                check_scores(scene, &digits)?;
                check_bonus_numbers(scene, &digits)?;
                check_decimal_domain(&digits)?;
                check_localised_stations(scene, &profiles, reserved)
            },
        )
    }

    fn check_decimal_domain(digits: &TextAtlas) -> Result<(), RenderError> {
        let mut draws = Vec::with_capacity(3);
        for value in u8::MIN..=u8::MAX {
            draws.clear();
            queue_decimal(
                digits,
                u16::from(value),
                (72i32, 52i32),
                &mut draws,
            )?;
            let expected = value.to_string();
            if draws.len() != expected.len() {
                return Err(failure("incorrect decimal digit count"));
            }
            let mut right = 72i32;
            let valid = draws.iter().zip(expected.bytes().rev()).all(
                |(draw, glyph)| {
                    right = right.saturating_sub(i32::from(DIGIT_WIDTH));
                    let source_top = i32::from(glyph.saturating_sub(b'0'))
                        .saturating_mul(i32::from(DIGIT_HEIGHT));
                    draw.source.y == source_top
                        && draw.destination.x == right
                        && draw.destination.y == 52i32
                },
            );
            if !valid {
                return Err(failure("incorrect decimal digit placement"));
            }
        }
        Ok(())
    }

    fn check_bonus_numbers(
        scene: &mut SceneSession<'_, '_>,
        digits: &TextAtlas,
    ) -> Result<(), RenderError> {
        let mut draws = Vec::with_capacity(5);
        queue_centiseconds(digits, 1000, (175i32, 26i32), &mut draws)?;
        if draws.len() != 5 || !scene.present([0, 0, 0], &draws)? {
            return Err(failure("initial bonus clock not rendered"));
        }
        if !draws
            .iter()
            .zip([0i32, 0i32, 100i32, 0i32, 10i32])
            .all(|(draw, source_top)| draw.source.y == source_top)
        {
            return Err(failure("incorrect bonus clock glyph order"));
        }
        let initial = scene.readback()?;
        draws.clear();
        queue_centiseconds(digits, 2, (175i32, 26i32), &mut draws)?;
        if draws.len() != 4
            || !scene.present([0, 0, 0], &draws)?
            || scene.readback()? == initial
        {
            return Err(failure("final bonus clock not rendered"));
        }
        draws.clear();
        queue_decimal(digits, 64, (72i32, 52i32), &mut draws)?;
        if draws.len() != 2
            || !draws
                .iter()
                .zip([40i32, 60i32])
                .all(|(draw, source_top)| draw.source.y == source_top)
        {
            return Err(failure("incorrect bonus result glyph order"));
        }
        Ok(())
    }

    fn check_scores(
        scene: &mut SceneSession<'_, '_>,
        digits: &TextAtlas,
    ) -> Result<(), RenderError> {
        let mut draws = Vec::with_capacity(4);
        queue_score(digits, Player::Marge, (0, 0i32, false), &mut draws)?;
        if draws.len() != 1 {
            return Err(failure("zero score must be one glyph"));
        }
        if !scene.present([0, 0, 0], &draws)? {
            return Err(failure("zero score not rendered"));
        }
        let zero = scene.readback()?;
        draws.clear();
        queue_score(digits, Player::Marge, (1, 0i32, false), &mut draws)?;
        if !scene.present([0, 0, 0], &draws)? || scene.readback()? == zero {
            return Err(failure("score increment not rendered"));
        }
        draws.clear();
        queue_score(digits, Player::Lisa, (1_029, 0i32, true), &mut draws)?;
        if draws.len() != 4 || !scene.present([0, 0, 0], &draws)? {
            return Err(failure("four-digit score not rendered"));
        }
        if !draws
            .iter()
            .zip([
                (90i32, 306i32),
                (20i32, 300i32),
                (0i32, 294i32),
                (10i32, 288i32),
            ])
            .all(|(draw, (source_top, left))| {
                draw.source.y == source_top
                    && draw.destination.y == 28i32
                    && draw.destination.x == left
            })
        {
            return Err(failure("incorrect score digit order or placement"));
        }
        Ok(())
    }

    fn check_localised_stations(
        scene: &mut SceneSession<'_, '_>,
        profiles: &Profiles,
        reserved: [bool; 4],
    ) -> Result<(), RenderError> {
        let catalog = title_messages::catalogue()?;
        for (index, name, hints) in [
            (0usize, "P1 LISA", [
                "JOIN F1",
                "MOUSE RIGHT",
                "P rightshoulder",
            ]),
            (1usize, "J1 LISA", [
                "UNIRSE F1",
                "RAT\u{d3}N DERECHO",
                "C rightshoulder",
            ]),
        ] {
            let locale = title_messages::locale_tag(index)?;
            let mut hud = StationHud::load_localised(
                scene,
                profiles,
                reserved,
                StationText {
                    catalog: &catalog,
                    locale: &locale,
                },
            )?;
            check_remote(scene, &mut hud)?;
            let owned = hints.map(String::from);
            check_local(scene, &hud, &binding_plate(name, &owned))?;
        }
        Ok(())
    }
}
