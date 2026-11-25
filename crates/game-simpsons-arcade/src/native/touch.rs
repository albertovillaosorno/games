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
//   - Outputs: Digital Marge controls and one static panel sprite.
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
//   - One touch-controlled station; other input devices remain usable.

//! Optional native touch controls with matched button geometry.

use std::num::NonZeroUsize;

use games::input::InputMapping;
use games::input::touch::{TouchLayout, TouchPoint, TouchZone};
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};

use super::{Action, Control, Player, dimension, glyph, size};

pub(super) const SCENE_HEIGHT: u16 = 304;
const PANEL_HEIGHT: u16 = 80;
const PANEL_TOP: u16 = 224;
const WIDTH: u16 = 320;

const BUTTONS: [PanelButton; 8] = [
    PanelButton::new(Control::Coin, [180, 4, 56, 32], [
        0b01110, 0b10001, 0b10101, 0b10001, 0b01110,
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
    fn bounds(self) -> Option<[TouchPoint; 2]> {
        let top = self.top.checked_add(PANEL_TOP)?;
        let right = self.left.checked_add(self.width)?;
        let bottom = top.checked_add(self.height)?;
        Some([
            TouchPoint {
                x: boundary(self.left, WIDTH)?,
                y: boundary(top, SCENE_HEIGHT)?,
            },
            TouchPoint {
                x: boundary(right, WIDTH)?.checked_sub(1)?,
                y: boundary(bottom, SCENE_HEIGHT)?.checked_sub(1)?,
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
        glyph::ink(&self.glyph, 5, column, row)
    }
}

fn boundary(pixel: u16, extent: u16) -> Option<u16> {
    let numerator = u32::from(pixel).checked_mul(u32::from(u16::MAX))?;
    u16::try_from(numerator.div_ceil(u32::from(extent))).ok()
}

pub(super) fn center(control: Control) -> Result<TouchPoint, RenderError> {
    let [min, max] = BUTTONS
        .iter()
        .find(|button| button.control == control)
        .and_then(|button| button.bounds())
        .ok_or(RenderError::InvalidDimensions)?;
    Ok(TouchPoint {
        x: min.x.midpoint(max.x),
        y: min.y.midpoint(max.y),
    })
}

pub(super) fn install(
    mapping: &mut InputMapping<Action>,
) -> Result<(), RenderError> {
    let mut zones = Vec::with_capacity(BUTTONS.len());
    for (index, button) in BUTTONS.into_iter().enumerate() {
        let id =
            u16::try_from(index).map_err(|_error| RenderError::TooLarge)?;
        let [min, max] =
            button.bounds().ok_or(RenderError::InvalidDimensions)?;
        let zone = TouchZone::new(id, min, max)
            .ok_or(RenderError::InvalidDimensions)?;
        mapping.bind_touch(id, Action {
            control: button.control,
            player: Player::Marge,
        });
        zones.push(zone);
    }
    mapping.set_touch_layout(TouchLayout::new(
        zones,
        NonZeroUsize::new(8).ok_or(RenderError::TooLarge)?,
    ));
    Ok(())
}

pub(super) fn load(
    scene: &mut SceneSession<'_, '_>,
) -> Result<SpriteDraw, RenderError> {
    let layout = FrameLayout::new(
        dimension(u32::from(WIDTH))?,
        dimension(u32::from(PANEL_HEIGHT))?,
        size(102_400)?,
    )?;
    let mut pixels = Vec::with_capacity(layout.bytes());
    for y in 0..PANEL_HEIGHT {
        for x in 0..WIDTH {
            let color = BUTTONS
                .iter()
                .find_map(|button| button.color(x, y))
                .unwrap_or([23, 27, 38, 255]);
            pixels.extend_from_slice(&color);
        }
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

#[cfg(test)]
mod tests {
    use games::input::touch::{ContactId, ContactPhase};
    use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use games_platform_graphics::linux::RenderError;

    use super::{BUTTONS, center, install};
    use crate::{Action, Player};

    #[test]
    fn visible_buttons_route_to_one_station() -> Result<(), RenderError> {
        let mut mapping = InputMapping::new();
        install(&mut mapping)?;
        let mut router = InputRouter::new(mapping);
        let id = ContactId { device: 1, finger: 2 };
        for button in BUTTONS {
            let point = center(button.control)?;
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
}
