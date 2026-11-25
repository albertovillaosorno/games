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
//   - Optional touch-panel geometry and matching visual button symbols.
// - Must-Not:
//   - Interpret native events, advance simulation or import original media.
// - Allows:
//   - Inputs: Title input mapping and native scene atlas allocation.
//   - Outputs: Selected station controls and one static panel sprite.
//   - Side effects: Loads authored control art into the scene atlas.
// - Split-When:
//   - Another title needs shared control-panel presentation primitives.
// - Merge-When:
//   - Independent title input layouts duplicate the same geometry.
// - Summary:
//   - Visible controls outside the original arcade scene.
// - Description:
//   - Both button pixels and hit regions derive from one layout.
// - Usage:
//   - Install and draw only when touch controls are explicitly enabled.
// - Defaults:
//   - One touch-controlled station unless multiple seats are configured.

//! Optional native touch controls with matched button geometry.

use games::input::InputMapping;
use games::input::touch::{TouchLayout, TouchPoint, TouchZone};
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError, ui};

use super::{
    Action, CONTROLS, Control, Player, Profiles, dimension, size, world,
};

pub(super) const MAX_SCENE_HEIGHT: u16 = PANEL_TOP + PANEL_HEIGHT * 4;
const MAX_PANEL_BYTES: usize = 409_600;
const CONTACTS_PER_PANEL: usize = 8;
const PANEL_HEIGHT: u16 = 80;
const PANEL_TOP: u16 = 224;
const WIDTH: u16 = 320;

const BUTTONS: [PanelButton; 8] = [
    PanelButton::new(Control::Pause, [180, 4, 56, 32], [
        0b11011, 0b11011, 0b11011, 0b11011, 0b11011,
    ]),
    PanelButton::new(Control::Start, [240, 4, 56, 32], [
        0b10000, 0b11100, 0b11111, 0b11100, 0b10000,
    ]),
    PanelButton::new(Control::Primary, [180, 40, 56, 32], [
        0b01110, 0b10001, 0b11111, 0b10001, 0b10001,
    ]),
    PanelButton::new(Control::Secondary, [240, 40, 56, 32], [
        0b11110, 0b10001, 0b11110, 0b10001, 0b11110,
    ]),
    PanelButton::new(Control::Up, [44, 4, 36, 32], [
        0b00100, 0b01110, 0b10101, 0b00100, 0b00100,
    ]),
    PanelButton::new(Control::Left, [4, 40, 36, 32], [
        0b00100, 0b01000, 0b11111, 0b01000, 0b00100,
    ]),
    PanelButton::new(Control::Down, [44, 40, 36, 32], [
        0b00100, 0b00100, 0b10101, 0b01110, 0b00100,
    ]),
    PanelButton::new(Control::Right, [84, 40, 36, 32], [
        0b00100, 0b00010, 0b11111, 0b00010, 0b00100,
    ]),
];

#[derive(Clone, Copy, Debug)]
struct PanelButton {
    control: Control,
    glyph: [u8; 5],
    height: u16,
    left: u16,
    top: u16,
    width: u16,
}

impl PanelButton {
    fn bounds(self, offset: u16, height: u16) -> Option<[TouchPoint; 2]> {
        let top = self.top.checked_add(offset)?;
        let right = self.left.checked_add(self.width)?;
        let bottom = top.checked_add(self.height)?;
        Some([
            TouchPoint {
                x: boundary(self.left, WIDTH)?,
                y: boundary(top, height)?,
            },
            TouchPoint {
                x: boundary(right, WIDTH)?.checked_sub(1)?,
                y: boundary(bottom, height)?.checked_sub(1)?,
            },
        ])
    }

    fn color(self, horizontal: u16, vertical: u16) -> Option<[u8; 4]> {
        let local_x = horizontal.checked_sub(self.left)?;
        let local_y = vertical.checked_sub(self.top)?;
        if local_x >= self.width || local_y >= self.height {
            return None;
        }
        if local_x == 0
            || local_y == 0
            || local_x == self.width.saturating_sub(1)
            || local_y == self.height.saturating_sub(1)
        {
            return Some([113, 126, 151, 255]);
        }
        if self.symbol(local_x, local_y) {
            return Some([242, 245, 250, 255]);
        }
        Some([48, 57, 75, 255])
    }

    const fn new(control: Control, rect: [u16; 4], glyph: [u8; 5]) -> Self {
        let [left, top, width, height] = rect;
        Self {
            control,
            glyph,
            height,
            left,
            top,
            width,
        }
    }

    fn symbol(self, horizontal: u16, vertical: u16) -> bool {
        let Some(column) = horizontal
            .checked_sub(
                self.width.saturating_sub(15).checked_div(2).unwrap_or(0),
            )
            .and_then(|value| value.checked_div(3))
        else {
            return false;
        };
        let Some(row) = vertical
            .checked_sub(
                self.height.saturating_sub(15).checked_div(2).unwrap_or(0),
            )
            .and_then(|value| value.checked_div(3))
        else {
            return false;
        };
        ui::ink(&self.glyph, 5, column, row)
    }
}

fn boundary(pixel: u16, extent: u16) -> Option<u16> {
    let numerator = u32::from(pixel).checked_mul(u32::from(u16::MAX))?;
    u16::try_from(numerator.div_ceil(u32::from(extent))).ok()
}

/// Selected physical seats, in ascending order.
pub(super) fn seats(profiles: &Profiles) -> impl Iterator<Item = Player> {
    Player::ALL
        .into_iter()
        .zip(profiles.touch_players)
        .filter_map(|(player, selected)| selected.then_some(player))
}

pub(super) fn reserved(profiles: &Profiles, reserved: [bool; 4]) -> bool {
    profiles
        .touch_players
        .into_iter()
        .zip(reserved)
        .any(|(selected, remote)| selected && remote)
}

pub(super) fn extra_bytes(profiles: &Profiles) -> usize {
    seats(profiles).count().saturating_sub(1).saturating_mul(
        usize::from(WIDTH)
            .saturating_mul(usize::from(PANEL_HEIGHT))
            .saturating_mul(4),
    )
}

pub(super) fn height(profiles: &Profiles) -> Result<u16, RenderError> {
    let count = u16::try_from(seats(profiles).count())
        .map_err(|_error| RenderError::TooLarge)?;
    PANEL_TOP
        .checked_add(
            PANEL_HEIGHT
                .checked_mul(count)
                .ok_or(RenderError::TooLarge)?,
        )
        .ok_or(RenderError::TooLarge)
}

fn panel_top(index: usize) -> Result<u16, RenderError> {
    u16::try_from(index)
        .ok()
        .and_then(|row| row.checked_mul(PANEL_HEIGHT))
        .and_then(|row| PANEL_TOP.checked_add(row))
        .ok_or(RenderError::TooLarge)
}

pub(super) fn center(
    player: Player,
    control: Control,
    profiles: &Profiles,
) -> Result<TouchPoint, RenderError> {
    let index = seats(profiles)
        .position(|seat| seat == player)
        .ok_or(RenderError::InvalidSprite)?;
    let offset = panel_top(index)?;
    let height = height(profiles)?;
    let [min, max] = panel(profiles, player)?
        .iter()
        .find(|button| button.control == control)
        .and_then(|button| button.bounds(offset, height))
        .ok_or(RenderError::InvalidDimensions)?;
    Ok(TouchPoint {
        x: min.x.midpoint(max.x),
        y: min.y.midpoint(max.y),
    })
}

pub(super) fn install(
    mapping: &mut InputMapping<Action>,
    profiles: &Profiles,
) -> Result<(), RenderError> {
    let count = seats(profiles).count();
    let mut zones = Vec::with_capacity(count.saturating_mul(BUTTONS.len()));
    let height = height(profiles)?;
    for (row, player) in seats(profiles).enumerate() {
        let offset = panel_top(row)?;
        for button in panel(profiles, player)? {
            let id = u16::try_from(zones.len())
                .map_err(|_error| RenderError::TooLarge)?;
            let [min, max] = button
                .bounds(offset, height)
                .ok_or(RenderError::InvalidDimensions)?;
            let zone = TouchZone::new(id, min, max)
                .ok_or(RenderError::InvalidDimensions)?;
            mapping.bind_touch(id, Action {
                control: button.control,
                player,
            });
            zones.push(zone);
        }
    }
    mapping.set_touch_layout(TouchLayout::new(
        zones,
        size(count.saturating_mul(CONTACTS_PER_PANEL))?,
    ));
    Ok(())
}

pub(super) fn load(
    scene: &mut SceneSession<'_, '_>,
    profiles: &Profiles,
) -> Result<SpriteDraw, RenderError> {
    let count = seats(profiles).count();
    let panel_height = height(profiles)?.saturating_sub(PANEL_TOP);
    let layout = FrameLayout::new(
        dimension(u32::from(WIDTH))?,
        dimension(u32::from(panel_height))?,
        size(MAX_PANEL_BYTES)?,
    )?;
    let mut pixels = Vec::with_capacity(layout.bytes());
    for player in seats(profiles) {
        let buttons = panel(profiles, player)?;
        let label = if count > 1 {
            ["SEAT 1", "SEAT 2", "SEAT 3", "SEAT 4"]
                .get(player.index())
                .copied()
                .ok_or(RenderError::InvalidSprite)?
        } else {
            ""
        };
        raster_panel(&mut pixels, &buttons, label);
    }
    let atlas = scene.load_atlas(layout, &pixels)?;
    let source = SpriteRect {
        height: layout.height(),
        width: layout.width(),
        x: 0,
        y: 0,
    };
    Ok(SpriteDraw {
        atlas,
        destination: SpriteRect {
            y: i32::from(PANEL_TOP),
            ..source
        },
        flip_horizontal: false,
        flip_vertical: false,
        source,
    })
}

fn raster_panel(pixels: &mut Vec<u8>, buttons: &[PanelButton; 8], label: &str) {
    let line = ui::TextLine {
        color: [242, 245, 250, 255],
        scale: 2,
        text: label,
        top: 35,
    };
    for y in 0..PANEL_HEIGHT {
        for x in 0..WIDTH {
            let color = buttons
                .iter()
                .find_map(|button| button.color(x, y))
                .unwrap_or_else(|| panel_background(line, x, y));
            pixels.extend_from_slice(&color);
        }
    }
}

fn panel_background(
    line: ui::TextLine<'_>,
    horizontal: u16,
    vertical: u16,
) -> [u8; 4] {
    if !line.text.is_empty()
        && horizontal
            .checked_sub(120)
            .is_some_and(|local| line.pixel(local, vertical, 60))
    {
        line.color
    } else {
        [23, 27, 38, 255]
    }
}

fn panel(
    profiles: &Profiles,
    player: Player,
) -> Result<[PanelButton; 8], RenderError> {
    let slots = profiles
        .touch_slots
        .get(player.index())
        .ok_or(RenderError::InvalidSprite)?;
    let mut result = BUTTONS;
    for ((control, slot), button) in
        CONTROLS.into_iter().zip(slots).zip(&mut result)
    {
        let action = BUTTONS
            .iter()
            .find(|candidate| candidate.control == control)
            .ok_or(RenderError::InvalidSprite)?;
        let position = CONTROLS.get(*slot).ok_or(RenderError::InvalidSprite)?;
        let geometry = BUTTONS
            .iter()
            .find(|candidate| candidate.control == *position)
            .ok_or(RenderError::InvalidSprite)?;
        *button = PanelButton {
            control,
            glyph: action.glyph,
            ..*geometry
        };
    }
    Ok(result)
}
/// Convert full logical coordinates to the centered base-width panel.
pub(super) fn to_panel(point: TouchPoint, width: u32) -> Option<TouchPoint> {
    if width < u32::from(WIDTH) {
        return None;
    }
    let maximum = u64::from(u16::MAX);
    let offset =
        u64::from(world::centered_offset(width)).checked_mul(maximum)?;
    let scaled = u64::from(point.x).checked_mul(u64::from(width))?;
    let local = scaled.checked_sub(offset)?.checked_div(u64::from(WIDTH))?;
    Some(TouchPoint {
        x: u16::try_from(local).ok()?,
        ..point
    })
}

/// Project base-panel coordinates into the full logical scene.
pub(super) fn to_scene(point: TouchPoint, width: u32) -> Option<TouchPoint> {
    if width < u32::from(WIDTH) {
        return None;
    }
    let offset = u64::from(world::centered_offset(width))
        .checked_mul(u64::from(u16::MAX))?;
    let scaled = u64::from(point.x)
        .checked_mul(u64::from(WIDTH))?
        .checked_add(offset)?;
    let horizontal = scaled.div_ceil(u64::from(width));
    Some(TouchPoint {
        x: u16::try_from(horizontal).ok()?,
        ..point
    })
}

#[cfg(test)]
mod tests {
    use games::input::touch::{ContactId, ContactPhase};
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use games_platform_graphics::linux::RenderError;

    use super::super::DEFAULT_PROFILES;
    use super::{BUTTONS, TouchPoint, center, install, to_panel, to_scene};
    use crate::{Action, Player};

    #[test]
    fn centered_panel_projects_button_centers_and_rejects_side_space()
    -> Result<(), RenderError> {
        let mut mapping = InputMapping::new();
        install(&mut mapping, &DEFAULT_PROFILES)?;
        let mut router = InputRouter::new(mapping);
        let id = ContactId { device: 1, finger: 2 };
        for (width, button) in [320u32, 321, 362, 512]
            .into_iter()
            .flat_map(|width| BUTTONS.map(|button| (width, button)))
        {
            let center =
                center(Player::Marge, button.control, &DEFAULT_PROFILES)?;
            let surface = to_scene(center, width)
                .ok_or(RenderError::InvalidDimensions)?;
            let local = to_panel(surface, width)
                .ok_or(RenderError::InvalidDimensions)?;
            let action = Action {
                control: button.control,
                player: Player::Marge,
            };
            if router.handle(InputEvent::Contact {
                id,
                phase: ContactPhase::Down(local),
            }) != [LogicalAction { action, down: true }]
                || router.handle(InputEvent::Contact {
                    id,
                    phase: ContactPhase::Up,
                }) != [LogicalAction { action, down: false }]
            {
                return Err(RenderError::InvalidSprite);
            }
        }
        for x in [0, u16::MAX] {
            if to_panel(TouchPoint { x, y: 60_000 }, 362).is_some() {
                return Err(RenderError::InvalidDimensions);
            }
        }
        Ok(())
    }

    #[test]
    fn visible_buttons_route_to_one_station() -> Result<(), RenderError> {
        let mut mapping = InputMapping::new();
        install(&mut mapping, &DEFAULT_PROFILES)?;
        let mut router = InputRouter::new(mapping);
        let id = ContactId { device: 1, finger: 2 };
        for button in BUTTONS {
            let point =
                center(Player::Marge, button.control, &DEFAULT_PROFILES)?;
            let action = Action {
                control: button.control,
                player: Player::Marge,
            };
            if router.handle(InputEvent::Contact {
                id,
                phase: ContactPhase::Down(point),
            }) != [LogicalAction { action, down: true }]
                || router.handle(InputEvent::Contact {
                    id,
                    phase: ContactPhase::Up,
                }) != [LogicalAction { action, down: false }]
            {
                return Err(RenderError::Backend(format!(
                    "incorrect touch routing for {:?}",
                    button.control
                )));
            }
        }
        Ok(())
    }

    #[test]
    fn independent_panels_route_simultaneous_contacts_at_all_widths()
    -> Result<(), RenderError> {
        use super::super::ACTION_NAMES;

        let profiles = DEFAULT_PROFILES
            .configured(
                ACTION_NAMES,
                concat!(
                    "version=1\ntouch-players=4,1,3,2\n",
                    "touch 2 left=right\ntouch 2 right=left\n",
                ),
            )
            .map_err(RenderError::Backend)?;
        if super::height(&profiles)? != super::MAX_SCENE_HEIGHT {
            return Err(RenderError::InvalidDimensions);
        }
        let mut mapping = InputMapping::new();
        install(&mut mapping, &profiles)?;
        let mut router = InputRouter::new(mapping);
        for width in [320u32, 362, 512] {
            verify_panel_contacts(&mut router, &profiles, width)?;
        }
        Ok(())
    }

    fn verify_panel_contacts(
        router: &mut InputRouter<Action>,
        profiles: &super::Profiles,
        width: u32,
    ) -> Result<(), RenderError> {
        use crate::Control;
        for player in Player::ALL {
            let action = Action {
                control: Control::Right,
                player,
            };
            let center = center(player, action.control, profiles)?;
            let point = to_scene(center, width)
                .and_then(|surface| to_panel(surface, width))
                .ok_or(RenderError::InvalidDimensions)?;
            let id = ContactId {
                device: 1,
                finger: i64::try_from(player.index())
                    .map_err(|_error| RenderError::TooLarge)?,
            };
            if router.handle(InputEvent::Contact {
                id,
                phase: ContactPhase::Down(point),
            }) != [LogicalAction { action, down: true }]
            {
                return Err(RenderError::InvalidSprite);
            }
        }
        for player in Player::ALL.into_iter().rev() {
            let id = ContactId {
                device: 1,
                finger: i64::try_from(player.index())
                    .map_err(|_error| RenderError::TooLarge)?,
            };
            let action = Action {
                control: Control::Right,
                player,
            };
            if router.handle(InputEvent::Contact {
                id,
                phase: ContactPhase::Up,
            }) != [LogicalAction { action, down: false }]
            {
                return Err(RenderError::InvalidSprite);
            }
        }
        Ok(())
    }

    #[test]
    fn multi_panel_raster_labels_and_slot_geometry_match()
    -> Result<(), RenderError> {
        use super::super::ACTION_NAMES;
        use crate::Control;

        let profiles = DEFAULT_PROFILES
            .configured(ACTION_NAMES, "version=1\ntouch-players=1,3")
            .map_err(RenderError::Backend)?;
        let first = center(Player::Marge, Control::Primary, &profiles)?;
        let second = center(Player::Bart, Control::Primary, &profiles)?;
        if first.x != second.x
            || first.y >= second.y
            || center(Player::Homer, Control::Primary, &profiles).is_ok()
        {
            return Err(RenderError::InvalidDimensions);
        }
        let mut first_pixels = Vec::new();
        let mut second_pixels = Vec::new();
        let buttons = super::panel(&profiles, Player::Marge)?;
        super::raster_panel(&mut first_pixels, &buttons, "SEAT 1");
        super::raster_panel(&mut second_pixels, &buttons, "SEAT 3");
        if first_pixels == second_pixels
            || first_pixels.len() != 102_400
            || second_pixels.len() != 102_400
        {
            return Err(RenderError::InvalidLength);
        }
        Ok(())
    }

    #[test]
    fn swapped_touch_slots_move_symbols_and_route_selected_station()
    -> Result<(), RenderError> {
        use super::super::ACTION_NAMES;
        use super::panel;
        use crate::Control;

        let profiles = DEFAULT_PROFILES
            .configured(
                ACTION_NAMES,
                concat!(
                    "version=1\ntouch-player=3\n",
                    "touch 3 primary=secondary\ntouch 3 secondary=primary\n"
                ),
            )
            .map_err(RenderError::Backend)?;
        let mut mapping = InputMapping::new();
        install(&mut mapping, &profiles)?;
        let mut router = InputRouter::new(mapping);
        let point =
            center(Player::Marge, Control::Secondary, &DEFAULT_PROFILES)?;
        if center(Player::Bart, Control::Primary, &profiles)? != point {
            return Err(RenderError::InvalidDimensions);
        }
        let original = BUTTONS
            .iter()
            .find(|button| button.control == Control::Primary)
            .ok_or(RenderError::InvalidSprite)?;
        let buttons = panel(&profiles, Player::Bart)?;
        let remapped = buttons
            .iter()
            .find(|button| button.control == Control::Primary)
            .ok_or(RenderError::InvalidSprite)?;
        if remapped.glyph != original.glyph
            || remapped.bounds(super::PANEL_TOP, 304)
                == original.bounds(super::PANEL_TOP, 304)
        {
            return Err(RenderError::InvalidSprite);
        }
        let action = Action {
            control: Control::Primary,
            player: Player::Bart,
        };
        let id = ContactId { device: 1, finger: 2 };
        for (phase, down) in
            [(ContactPhase::Down(point), true), (ContactPhase::Up, false)]
        {
            if router.handle(InputEvent::Contact { id, phase })
                != [LogicalAction { action, down }]
            {
                return Err(RenderError::Backend(String::from(
                    "wrong remapped touch action",
                )));
            }
        }
        Ok(())
    }
}
