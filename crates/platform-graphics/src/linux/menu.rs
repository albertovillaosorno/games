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
//   - Native presentation of shared local session menu choices.
// - Must-Not:
//   - Own title admission, gameplay, input mapping or persistent settings.
// - Allows:
//   - Inputs: Shared session state and title-provided heading and hints.
//   - Outputs: One static menu sprite with the selected item emphasized.
//   - Side effects: Bounded atlas upload during explicit initialization.
// - Split-When:
//   - Settings menus require independent dynamic layout or editing.
// - Merge-When:
//   - A native adapter duplicates shared menu presentation.
// - Summary:
//   - Render modern free-play and pause menus through one native adapter.
// - Description:
//   - Pre-rasterize each selection so menu navigation adds no per-frame uploads
//     or allocations.
// - Usage:
//   - Load with logical dimensions and input hints, then draw by session.
// - Defaults:
//   - Opaque neutral plate, warm highlight and compact readable lettering.

//! Native presentation of shared session menu state.

use games::session::{LocalSession, MenuItem, SessionPhase};

use super::RenderError;
use super::sprites::{SceneSession, SpriteDraw};
use super::ui::{PlateLayout, TextAtlas, TextLine};

const BACKGROUND: [u8; 4] = [24, 22, 30, 255];
const TEXT: [u8; 4] = [220, 222, 232, 255];
const SELECTED: [u8; 4] = [248, 228, 168, 255];

/// Title-owned text, without duplicating the shared menu choices.
#[derive(Clone, Copy, Debug)]
pub struct MenuContent<'text> {
    /// Explanatory lines for the main menu.
    pub main_hints: &'text [&'text str],
    /// Explanatory lines for the pause menu.
    pub pause_hints: &'text [&'text str],
    /// Visible title of the current game.
    pub title: &'text str,
}

/// Pre-rasterized native presentation of the shared free-play menus.
#[derive(Clone, Copy, Debug)]
pub struct MenuArt {
    atlas: TextAtlas,
}

impl MenuArt {
    /// Draw the menu at the specified logical origin when one is open.
    ///
    /// # Errors
    /// Rejects a selection not represented in the uploaded atlas.
    pub fn draw(
        &self,
        session: LocalSession,
        left: i32,
        top: i32,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        let Some(selected) = session.selected() else {
            return Ok(None);
        };
        let offset = match session.phase() {
            SessionPhase::MainMenu => 0,
            SessionPhase::Paused => SessionPhase::MainMenu.items().len(),
            SessionPhase::Playing | SessionPhase::Exited => {
                return Err(RenderError::InvalidSprite);
            },
        };
        let index = session
            .items()
            .iter()
            .position(|item| *item == selected)
            .ok_or(RenderError::InvalidSprite)?;
        self.atlas
            .draw(offset.saturating_add(index), left, top)
            .map(Some)
    }

    /// Upload a static menu atlas using a caller-provided logical panel size.
    ///
    /// # Errors
    /// Rejects text that cannot fit, invalid dimensions or excessive bytes.
    pub fn load(
        scene: &mut SceneSession<'_, '_>,
        spec: PlateLayout,
        content: MenuContent<'_>,
    ) -> Result<Self, RenderError> {
        let mut plates = Vec::new();
        for phase in [SessionPhase::MainMenu, SessionPhase::Paused] {
            for selected in phase.items().iter().copied() {
                plates.push(plate(spec.width, content, phase, selected));
            }
        }
        Ok(Self {
            atlas: TextAtlas::load(
                scene,
                PlateLayout {
                    background: BACKGROUND,
                    ..spec
                },
                &plates,
            )?,
        })
    }
}

const fn label(item: MenuItem, selected: bool) -> &'static str {
    match (item, selected) {
        (MenuItem::Play, true) => "> PLAY",
        (MenuItem::Play, false) => "PLAY",
        (MenuItem::Resume, true) => "> RESUME",
        (MenuItem::Resume, false) => "RESUME",
        (MenuItem::MainMenu, true) => "> MAIN MENU",
        (MenuItem::MainMenu, false) => "MAIN MENU",
        (MenuItem::Exit, true) => "> EXIT",
        (MenuItem::Exit, false) => "EXIT",
    }
}

fn plate(
    width: u16,
    content: MenuContent<'_>,
    phase: SessionPhase,
    selected: MenuItem,
) -> Vec<TextLine<'_>> {
    let (subtitle, hints) = if phase == SessionPhase::MainMenu {
        ("FREE PLAY", content.main_hints)
    } else {
        ("PAUSED", content.pause_hints)
    };
    let mut lines = vec![
        TextLine {
            color: SELECTED,
            scale: 3,
            text: content.title,
            top: 8,
        }
        .fit_width(width),
        TextLine {
            color: TEXT,
            scale: 2,
            text: subtitle,
            top: 28,
        }
        .fit_width(width),
    ];
    let mut top = 48u16;
    for item in phase.items().iter().copied() {
        let highlighted = item == selected;
        lines.push(
            TextLine {
                color: if highlighted {
                    SELECTED
                } else {
                    TEXT
                },
                scale: 2,
                text: label(item, highlighted),
                top,
            }
            .fit_width(width),
        );
        top = top.saturating_add(16);
    }
    top = top.saturating_add(12);
    for hint in hints.iter().copied() {
        lines.push(
            TextLine {
                color: TEXT,
                scale: 2,
                text: hint,
                top,
            }
            .fit_width(width),
        );
        top = top.saturating_add(12);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_menu_choice_has_exactly_one_visible_highlight() {
        let content = MenuContent {
            main_hints: &["START TO PLAY"],
            pause_hints: &["BACK TO RESUME"],
            title: "GAME",
        };
        for phase in [SessionPhase::MainMenu, SessionPhase::Paused] {
            for item in phase.items().iter().copied() {
                let lines = plate(280, content, phase, item);
                let mut choices =
                    lines.iter().filter(|line| line.text.starts_with('>'));
                assert_eq!(
                    choices.next().map(|line| (line.text, line.color)),
                    Some((label(item, true), SELECTED))
                );
                assert!(choices.next().is_none());
            }
        }
    }
}
