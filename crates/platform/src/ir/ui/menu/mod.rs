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
//   - Portable presentation of shared local session menu choices.
// - Must-Not:
//   - Own title admission, gameplay, input mapping or persistent settings.
// - Allows:
//   - Inputs: Shared session state and title-provided heading and hints.
//   - Outputs: Static menu background and highlighted row sprites.
//   - Side effects: Bounded atlas upload during explicit initialization.
// - Split-When:
//   - Settings menus require independent dynamic layout or editing.
// - Merge-When:
//   - Another module duplicates shared menu presentation.
// - Summary:
//   - Render modern free-play and pause menus through one shared contract.
// - Description:
//   - Pre-rasterize backgrounds and rows so navigation adds no per-frame
//     uploads or allocations.
// - Usage:
//   - Load with logical dimensions and input hints, then draw by session.
// - Defaults:
//   - Opaque neutral plate, warm highlight and compact readable lettering.

//! Portable presentation of shared session menu state.

use std::num::NonZeroUsize;

use games::audio::MasterVolume;
use games::presentation::{DisplaySettings, PresentationMode, WindowMode};
use games::session::{ControlReset, LocalSession, MenuItem, SessionPhase};

use crate::ir::frame::RenderError;
use crate::ir::scene::{AtlasUpload, SpriteDraw};
use crate::ir::ui::{PlateLayout, TextAtlas, TextLine};

const PHASES: [SessionPhase; 3] = [
    SessionPhase::MainMenu,
    SessionPhase::Paused,
    SessionPhase::Settings,
];
const ROW_HEIGHT: u16 = 16;
const ROW_TOP: u16 = 48;
const ROWS: [Row; 21] = [
    Row::Choice(MenuItem::Back),
    Row::Choice(MenuItem::Exit),
    Row::Choice(MenuItem::MainMenu),
    Row::Choice(MenuItem::Play),
    Row::Choice(MenuItem::Resume),
    Row::Choice(MenuItem::Settings),
    Row::Presentation(PresentationMode::Adaptive),
    Row::Presentation(PresentationMode::AspectFit),
    Row::Presentation(PresentationMode::Fill),
    Row::Presentation(PresentationMode::Integer),
    Row::Presentation(PresentationMode::Stretch),
    Row::Volume(MasterVolume::Muted),
    Row::Volume(MasterVolume::Quarter),
    Row::Volume(MasterVolume::Half),
    Row::Volume(MasterVolume::ThreeQuarters),
    Row::Volume(MasterVolume::Full),
    Row::Window(WindowMode::Fullscreen),
    Row::Window(WindowMode::Windowed),
    Row::ControlReset(false),
    Row::ControlReset(true),
    Row::Choice(MenuItem::EditControls),
];
const SELECTED: [u8; 4] = [248, 228, 168, 255];
const TEXT: [u8; 4] = [220, 222, 232, 255];

/// Title-owned text, without duplicating the shared menu choices.
#[derive(Clone, Copy, Debug)]
pub struct MenuContent<'text> {
    /// Whether the host offers a next-launch control reset.
    pub control_reset: bool,
    /// Explanatory lines for the main menu.
    pub main_hints: &'text [&'text str],
    /// Explanatory lines for the pause menu.
    pub pause_hints: &'text [&'text str],
    /// Explanatory lines for display settings.
    pub settings_hints: &'text [&'text str],
    /// Visible title of the current game.
    pub title: &'text str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Row {
    Choice(MenuItem),
    ControlReset(bool),
    Presentation(PresentationMode),
    Volume(MasterVolume),
    Window(WindowMode),
}

impl Row {
    const fn for_item(
        item: MenuItem,
        display: DisplaySettings,
        volume: MasterVolume,
        control_reset: ControlReset,
    ) -> Self {
        match item {
            MenuItem::Display => Self::Presentation(display.presentation),
            MenuItem::ResetControls => Self::ControlReset(matches!(
                control_reset,
                ControlReset::Requested
            )),
            MenuItem::Volume => Self::Volume(volume),
            MenuItem::Window => Self::Window(display.window),
            MenuItem::Back
            | MenuItem::EditControls
            | MenuItem::Exit
            | MenuItem::MainMenu
            | MenuItem::Play
            | MenuItem::Resume
            | MenuItem::Settings => Self::Choice(item),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Choice(item) => match item {
                MenuItem::Back => "BACK",
                MenuItem::Display => "DISPLAY",
                MenuItem::EditControls => "EDIT CONTROLS NEXT LAUNCH",
                MenuItem::Exit => "EXIT",
                MenuItem::MainMenu => "MAIN MENU",
                MenuItem::Play => "PLAY",
                MenuItem::ResetControls => "RESET CONTROLS",
                MenuItem::Resume => "RESUME",
                MenuItem::Settings => "SETTINGS",
                MenuItem::Volume => "VOLUME",
                MenuItem::Window => "WINDOW",
            }
            .to_owned(),
            Self::ControlReset(false) => {
                "RESET CONTROLS NEXT LAUNCH".to_owned()
            },
            Self::ControlReset(true) => "DEFAULTS NEXT LAUNCH".to_owned(),
            Self::Presentation(mode) => format!("DISPLAY: {}", mode.as_str()),
            Self::Volume(level) => format!("VOLUME: {}%", level.percent()),
            Self::Window(mode) => format!("WINDOW: {}", mode.as_str()),
        }
    }

    fn tile(self, selected: bool) -> Option<usize> {
        ROWS.iter().position(|row| *row == self).map(|index| {
            index
                .saturating_mul(2)
                .saturating_add(usize::from(selected))
        })
    }
}

/// Portable menu backgrounds and compact rows shared across all selections.
#[derive(Clone, Copy, Debug)]
pub struct MenuArt {
    control_reset: bool,
    panels: TextAtlas,
    rows: TextAtlas,
}

impl MenuArt {
    /// Upload static menu backgrounds and row variants under one byte budget.
    ///
    /// # Errors
    /// Rejects clipped text, invalid dimensions or excessive bytes.
    pub fn load<Upload>(
        scene: &mut Upload,
        spec: PlateLayout,
        content: MenuContent<'_>,
    ) -> Result<Self, RenderError>
    where
        Upload: AtlasUpload,
    {
        let panel_bytes = usize::from(spec.width)
            .saturating_mul(usize::from(spec.height))
            .saturating_mul(PHASES.len())
            .saturating_mul(4);
        let row_budget = spec
            .max_bytes
            .get()
            .checked_sub(panel_bytes)
            .and_then(NonZeroUsize::new)
            .ok_or(RenderError::TooLarge)?;
        let panels = PHASES.map(|phase| panel(spec.width, content, phase));
        let labels = ROWS
            .into_iter()
            .take_while(|row| {
                content.control_reset
                    || !matches!(
                        row,
                        Row::ControlReset(_)
                            | Row::Choice(MenuItem::EditControls)
                    )
            })
            .flat_map(|row| {
                let label = row.label();
                let highlighted = format!("> {label}");
                [label, highlighted]
            })
            .collect::<Vec<_>>();
        let rows = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                row_plate(spec.width, label, index.checked_rem(2) == Some(1))
            })
            .collect::<Vec<_>>();
        Ok(Self {
            control_reset: content.control_reset,
            panels: TextAtlas::load(scene, spec, &panels)?,
            rows: TextAtlas::load(
                scene,
                PlateLayout {
                    height: ROW_HEIGHT,
                    max_bytes: row_budget,
                    ..spec
                },
                &rows,
            )?,
        })
    }

    /// Queue the visible menu into a caller-owned draw list.
    ///
    /// # Errors
    /// Rejects a phase or selection not represented in the loaded atlases.
    pub fn queue(
        &self,
        session: LocalSession,
        origin: (i32, i32),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let Some(selected) = session.selected() else {
            return Ok(());
        };
        if session.phase() == SessionPhase::Settings
            && self.control_reset
                == matches!(session.control_reset(), ControlReset::Disabled)
        {
            return Err(RenderError::InvalidSprite);
        }
        let (left, top) = origin;
        let panel = PHASES
            .iter()
            .position(|phase| *phase == session.phase())
            .ok_or(RenderError::InvalidSprite)?;
        draws.push(self.panels.draw(panel, left, top)?);
        for (index, item) in session.items().iter().copied().enumerate() {
            let tile = Row::for_item(
                item,
                session.display(),
                session.volume(),
                session.control_reset(),
            )
            .tile(item == selected)
            .ok_or(RenderError::InvalidSprite)?;
            let offset = i32::try_from(index)
                .map_err(|_error| RenderError::InvalidDimensions)?
                .saturating_mul(i32::from(ROW_HEIGHT));
            let row_top = top
                .saturating_add(i32::from(ROW_TOP))
                .saturating_add(offset);
            draws.push(self.rows.draw(tile, left, row_top)?);
        }
        Ok(())
    }
}

fn panel(
    width: u16,
    content: MenuContent<'_>,
    phase: SessionPhase,
) -> Vec<TextLine<'_>> {
    let (subtitle, hints) = match phase {
        SessionPhase::MainMenu => ("FREE PLAY", content.main_hints),
        SessionPhase::Paused => ("PAUSED", content.pause_hints),
        SessionPhase::Settings => ("SETTINGS", content.settings_hints),
        SessionPhase::Controls
        | SessionPhase::Playing
        | SessionPhase::Exited => ("", &[][..]),
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
    let count = u16::try_from(phase.items(content.control_reset).len())
        .unwrap_or(u16::MAX);
    let mut top = ROW_TOP
        .saturating_add(count.saturating_mul(ROW_HEIGHT))
        .saturating_add(12);
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

fn row_plate(width: u16, label: &str, selected: bool) -> Vec<TextLine<'_>> {
    vec![
        TextLine {
            color: if selected {
                SELECTED
            } else {
                TEXT
            },
            scale: 2,
            text: label,
            top: 2,
        }
        .fit_width(width),
    ]
}

#[cfg(test)]
mod tests {
    use games::session::{MenuInput, SessionEffect};

    use super::*;
    use crate::ir::frame::FrameLayout;
    use crate::ir::scene::AtlasId;

    #[derive(Debug, Default)]
    struct Capture {
        layouts: Vec<FrameLayout>,
    }

    impl AtlasUpload for Capture {
        fn upload_atlas(
            &mut self,
            layout: FrameLayout,
            pixels: &[u8],
        ) -> Result<AtlasId, RenderError> {
            if layout.bytes() != pixels.len() {
                return Err(RenderError::InvalidLength);
            }
            let id = AtlasId(self.layouts.len());
            self.layouts.push(layout);
            Ok(id)
        }
    }

    #[test]
    fn every_phase_choice_has_both_row_variants_for_all_display_modes() {
        for phase in PHASES {
            for item in phase.items(true).iter().copied() {
                verify_rows(item);
            }
        }
    }

    fn verify_rows(item: MenuItem) {
        for presentation in PresentationMode::ALL {
            for window in WindowMode::ALL {
                verify_volume_rows(item, DisplaySettings {
                    presentation,
                    window,
                });
            }
        }
    }

    fn verify_volume_rows(item: MenuItem, display: DisplaySettings) {
        for volume in MasterVolume::ALL {
            let row =
                Row::for_item(item, display, volume, ControlReset::Available);
            let plain = row.tile(false);
            assert!(plain.is_some());
            assert_eq!(
                row.tile(true),
                plain.map(|index| index.saturating_add(1))
            );
        }
    }

    #[test]
    fn setting_rows_reflect_the_requested_value() {
        let display = DisplaySettings {
            presentation: PresentationMode::Integer,
            window: WindowMode::Fullscreen,
        };
        assert_eq!(
            Row::for_item(
                MenuItem::Display,
                display,
                MasterVolume::Full,
                ControlReset::Disabled
            )
            .label(),
            "DISPLAY: integer"
        );
        assert_eq!(
            Row::for_item(
                MenuItem::Window,
                display,
                MasterVolume::Full,
                ControlReset::Disabled
            )
            .label(),
            "WINDOW: fullscreen"
        );
        assert_eq!(
            Row::for_item(
                MenuItem::Volume,
                display,
                MasterVolume::Half,
                ControlReset::Disabled
            )
            .label(),
            "VOLUME: 50%"
        );
    }

    #[test]
    fn menu_uploads_once_and_selection_changes_only_sprite_commands() {
        let mut capture = Capture::default();
        let loaded = MenuArt::load(
            &mut capture,
            PlateLayout {
                background: [0, 0, 0, 255],
                height: 176,
                max_bytes: NonZeroUsize::new(1_000_000)
                    .unwrap_or(NonZeroUsize::MIN),
                width: 160,
            },
            MenuContent {
                control_reset: true,
                main_hints: &["START"],
                pause_hints: &["RESUME"],
                settings_hints: &["BACK"],
                title: "TEST",
            },
        );
        assert!(loaded.is_ok());
        let Ok(art) = loaded else {
            return;
        };
        assert_eq!(capture.layouts.len(), 2);
        verify_menu_commands(art);
        verify_reset_commands(art);
        assert_eq!(capture.layouts.len(), 2);
    }

    fn select_reset() -> LocalSession {
        let mut session = LocalSession::default().with_control_reset(true);
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        for _ in 0u8..3 {
            assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        }
        session
    }

    fn verify_reset_commands(art: MenuArt) {
        let mut session = select_reset();
        let mut initial = Vec::new();
        assert_eq!(art.queue(session, (0i32, 0i32), &mut initial), Ok(()));
        assert_eq!(initial.len(), 7);
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        let mut requested = Vec::new();
        assert_eq!(art.queue(session, (0i32, 0i32), &mut requested), Ok(()));
        assert_eq!(initial.get(..4), requested.get(..4));
        assert_ne!(initial.get(4), requested.get(4));
        assert_eq!(initial.get(5), requested.get(5));
    }

    fn verify_menu_commands(art: MenuArt) {
        let mut session =
            LocalSession::with_display(DisplaySettings::default(), false)
                .with_control_reset(true);
        let origin = (4i32, 7i32);
        let mut initial = Vec::new();
        assert_eq!(art.queue(session, origin, &mut initial), Ok(()));
        assert_eq!(initial.len(), session.items().len().saturating_add(1));
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        let mut moved = Vec::new();
        assert_eq!(art.queue(session, origin, &mut moved), Ok(()));
        assert_eq!(initial.first(), moved.first());
        assert_ne!(initial.get(1), moved.get(1));
        assert_ne!(initial.get(2), moved.get(2));
        assert_eq!(initial.get(3..), moved.get(3..));
        verify_play_pause(art, session, &initial);
    }

    fn verify_play_pause(
        art: MenuArt,
        mut session: LocalSession,
        initial: &[SpriteDraw],
    ) {
        let origin = (4i32, 7i32);
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        let mut moved = Vec::new();
        assert_eq!(art.queue(session, origin, &mut moved), Ok(()));
        assert_eq!(moved.len(), 0);
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(art.queue(session, origin, &mut moved), Ok(()));
        assert_eq!(moved.len(), session.items().len().saturating_add(1));
        assert_ne!(initial.first(), moved.first());
    }
}
