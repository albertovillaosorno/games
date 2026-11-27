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

mod messages;
use std::num::NonZeroUsize;

use games::audio::{AudioSource, MasterVolume, SourceSettings};
use games::presentation::{
    MessageParameter, MessageValue, PresentationMode, WindowMode,
};
use games::session::{ControlReset, LocalSession, MenuItem, SessionPhase};
pub use messages::{MENU_MESSAGE_KEYS, MenuText, register_menu_messages};

use self::messages::english_catalog;
use crate::ir::frame::RenderError;
use crate::ir::scene::{AtlasUpload, SpriteDraw};
use crate::ir::ui::{PlateLayout, TextAtlas, TextLine};

const PHASES: [SessionPhase; 4] = [
    SessionPhase::Audio,
    SessionPhase::MainMenu,
    SessionPhase::Paused,
    SessionPhase::Settings,
];
const ROW_HEIGHT: u16 = 16;
const ROW_TOP: u16 = 48;
const ROWS: [Row; 32] = [
    Row::Choice(MenuItem::Audio),
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
    Row::SourceVolume(AudioSource::Effects, MasterVolume::Muted),
    Row::SourceVolume(AudioSource::Effects, MasterVolume::Quarter),
    Row::SourceVolume(AudioSource::Effects, MasterVolume::Half),
    Row::SourceVolume(AudioSource::Effects, MasterVolume::ThreeQuarters),
    Row::SourceVolume(AudioSource::Effects, MasterVolume::Full),
    Row::SourceVolume(AudioSource::Music, MasterVolume::Muted),
    Row::SourceVolume(AudioSource::Music, MasterVolume::Quarter),
    Row::SourceVolume(AudioSource::Music, MasterVolume::Half),
    Row::SourceVolume(AudioSource::Music, MasterVolume::ThreeQuarters),
    Row::SourceVolume(AudioSource::Music, MasterVolume::Full),
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
    /// Admitted source settings determine which controls are visible.
    pub sources: SourceSettings,
    /// Visible title of the current game.
    pub title: &'text str,
    /// Whether the host supports changing window mode.
    pub window_switching: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Row {
    Choice(MenuItem),
    ControlReset(bool),
    Presentation(PresentationMode),
    SourceVolume(AudioSource, MasterVolume),
    Volume(MasterVolume),
    Window(WindowMode),
}

impl Row {
    const fn available(
        self,
        control_reset: bool,
        sources: SourceSettings,
    ) -> bool {
        match self {
            Self::ControlReset(_) | Self::Choice(MenuItem::EditControls) => {
                control_reset
            },
            Self::SourceVolume(source, _) => sources.get(source).is_some(),
            Self::Choice(_)
            | Self::Presentation(_)
            | Self::Volume(_)
            | Self::Window(_) => true,
        }
    }

    const fn for_item(item: MenuItem, session: LocalSession) -> Self {
        let display = session.display();
        let volume = session.volume();
        let control_reset = session.control_reset();
        let sources = session.source_settings();
        match item {
            MenuItem::Display => Self::Presentation(display.presentation),
            MenuItem::ResetControls => Self::ControlReset(matches!(
                control_reset,
                ControlReset::Requested
            )),
            MenuItem::Volume => Self::Volume(volume),
            MenuItem::EffectsVolume => Self::SourceVolume(
                AudioSource::Effects,
                sources.levels().effects,
            ),
            MenuItem::MusicVolume => {
                Self::SourceVolume(AudioSource::Music, sources.levels().music)
            },
            MenuItem::Window => Self::Window(display.window),
            MenuItem::Audio
            | MenuItem::Back
            | MenuItem::EditControls
            | MenuItem::Exit
            | MenuItem::MainMenu
            | MenuItem::Play
            | MenuItem::Resume
            | MenuItem::Settings => Self::Choice(item),
        }
    }

    fn label(
        self,
        text: MenuText<'_>,
        limit: NonZeroUsize,
    ) -> Result<String, RenderError> {
        match self {
            Self::Choice(item) => {
                text.literal(choice_key(item)).map(str::to_owned)
            },
            Self::ControlReset(false) => {
                text.literal("menu.control.reset").map(str::to_owned)
            },
            Self::ControlReset(true) => {
                text.literal("menu.control.defaults").map(str::to_owned)
            },
            Self::Presentation(mode) => text.format(
                "menu.display.value",
                MessageParameter {
                    name: "mode",
                    value: MessageValue::Text(
                        text.literal(presentation_key(mode))?,
                    ),
                },
                limit,
            ),
            Self::SourceVolume(source, level) => volume_label(
                text,
                match source {
                    AudioSource::Effects => "menu.effects.value",
                    AudioSource::Music => "menu.music.value",
                },
                level,
                limit,
            ),
            Self::Volume(level) => {
                volume_label(text, "menu.master.value", level, limit)
            },
            Self::Window(mode) => text.format(
                "menu.window.value",
                MessageParameter {
                    name: "mode",
                    value: MessageValue::Text(text.literal(match mode {
                        WindowMode::Fullscreen => "menu.window.fullscreen",
                        WindowMode::Windowed => "menu.window.windowed",
                    })?),
                },
                limit,
            ),
        }
    }

    fn tile(
        self,
        selected: bool,
        control_reset: bool,
        sources: SourceSettings,
    ) -> Option<usize> {
        available_rows(control_reset, sources)
            .position(|row| row == self)
            .map(|index| {
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
    sources: SourceSettings,
    window_switching: bool,
}

impl MenuArt {
    /// Exact atlas bytes for the requested layout and available menu rows.
    #[must_use]
    pub fn bytes(
        width: u16,
        height: u16,
        control_reset: bool,
        sources: SourceSettings,
    ) -> usize {
        let rows = available_rows(control_reset, sources).count();
        usize::from(width).saturating_mul(4).saturating_mul(
            usize::from(height)
                .saturating_mul(PHASES.len())
                .saturating_add(
                    usize::from(ROW_HEIGHT)
                        .saturating_mul(rows)
                        .saturating_mul(2),
                ),
        )
    }

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
        let (locale, catalog) = english_catalog()?;
        Self::load_localised(scene, spec, content, MenuText {
            catalog: &catalog,
            locale: &locale,
        })
    }

    /// Upload translated menu art using a caller-selected locale.
    ///
    /// # Errors
    /// Rejects missing or invalid messages, clipped text or excess bytes.
    pub fn load_localised<Upload>(
        scene: &mut Upload,
        spec: PlateLayout,
        content: MenuContent<'_>,
        text: MenuText<'_>,
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
        let panels = PHASES
            .into_iter()
            .map(|phase| panel(spec.width, content, phase, text))
            .collect::<Result<Vec<_>, _>>()?;
        let limit =
            NonZeroUsize::new(usize::from(spec.width).saturating_mul(4))
                .ok_or(RenderError::InvalidDimensions)?;
        let mut labels = Vec::new();
        for row in available_rows(content.control_reset, content.sources) {
            let label = row.label(text, limit)?;
            let highlighted = format!("> {label}");
            labels.extend([label, highlighted]);
        }
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
            sources: content.sources,
            window_switching: content.window_switching,
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
            && (self.control_reset
                == matches!(session.control_reset(), ControlReset::Disabled)
                || self.window_switching != session.window_switching())
        {
            return Err(RenderError::InvalidSprite);
        }
        if session.phase() == SessionPhase::Audio {
            let sources = session.source_settings();
            if sources.effects.is_some() != self.sources.effects.is_some()
                || sources.music.is_some() != self.sources.music.is_some()
            {
                return Err(RenderError::InvalidSprite);
            }
        }
        let (left, top) = origin;
        let panel = PHASES
            .iter()
            .position(|phase| *phase == session.phase())
            .ok_or(RenderError::InvalidSprite)?;
        draws.push(self.panels.draw(panel, left, top)?);
        for (index, item) in session.items().iter().copied().enumerate() {
            let tile = Row::for_item(item, session)
                .tile(item == selected, self.control_reset, self.sources)
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

fn available_rows(
    control_reset: bool,
    sources: SourceSettings,
) -> impl Iterator<Item = Row> {
    ROWS.into_iter()
        .filter(move |row| row.available(control_reset, sources))
}

fn panel<'text>(
    width: u16,
    content: MenuContent<'text>,
    phase: SessionPhase,
    text: MenuText<'text>,
) -> Result<Vec<TextLine<'text>>, RenderError> {
    let audio_hints = [
        text.literal("menu.audio.hint")?,
        text.literal("menu.audio.all")?,
    ];
    let (subtitle, hints) = match phase {
        SessionPhase::Audio => (text.literal("menu.audio")?, &audio_hints[..]),
        SessionPhase::MainMenu => {
            (text.literal("menu.free-play")?, content.main_hints)
        },
        SessionPhase::Paused => {
            (text.literal("menu.paused")?, content.pause_hints)
        },
        SessionPhase::Settings => {
            (text.literal("menu.settings")?, content.settings_hints)
        },
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
    let count = u16::try_from(
        phase
            .items_with_sources(
                content.control_reset,
                content.window_switching,
                content.sources,
            )
            .len(),
    )
    .unwrap_or(u16::MAX);
    append_hints(&mut lines, width, hints, count);
    Ok(lines)
}

fn append_hints<'text>(
    lines: &mut Vec<TextLine<'text>>,
    width: u16,
    hints: &[&'text str],
    count: u16,
) {
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
}

const fn choice_key(item: MenuItem) -> &'static str {
    match item {
        MenuItem::Audio => "menu.audio",
        MenuItem::Back => "menu.back",
        MenuItem::Display => "menu.display",
        MenuItem::EffectsVolume => "menu.effects",
        MenuItem::EditControls => "menu.control.edit",
        MenuItem::Exit => "menu.exit",
        MenuItem::MainMenu => "menu.main",
        MenuItem::MusicVolume => "menu.music",
        MenuItem::Play => "menu.play",
        MenuItem::ResetControls => "menu.reset",
        MenuItem::Resume => "menu.resume",
        MenuItem::Settings => "menu.settings",
        MenuItem::Volume => "menu.volume",
        MenuItem::Window => "menu.window",
    }
}

const fn presentation_key(mode: PresentationMode) -> &'static str {
    match mode {
        PresentationMode::Adaptive => "menu.display.adaptive",
        PresentationMode::AspectFit => "menu.display.aspect-fit",
        PresentationMode::Fill => "menu.display.fill",
        PresentationMode::Integer => "menu.display.integer",
        PresentationMode::Stretch => "menu.display.stretch",
    }
}

fn volume_label(
    text: MenuText<'_>,
    key: &str,
    level: MasterVolume,
    limit: NonZeroUsize,
) -> Result<String, RenderError> {
    text.format(
        key,
        MessageParameter {
            name: "percent",
            value: MessageValue::Text(level.percent()),
        },
        limit,
    )
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
    use games::presentation::{
        CatalogLimits, DisplaySettings, LocaleTag, TranslationCatalog,
    };
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
            let row = Row::for_item(
                item,
                LocalSession::with_preferences(display, volume, false)
                    .with_control_reset(true),
            );
            let plain = row.tile(false, true, SourceSettings::default());
            assert!(plain.is_some());
            assert_eq!(
                row.tile(true, true, SourceSettings::default()),
                plain.map(|index| index.saturating_add(1))
            );
        }
    }

    #[test]
    fn built_in_menu_catalogues_are_complete_and_localised() {
        let parsed_en = LocaleTag::parse("en");
        let parsed_es = LocaleTag::parse("es-MX");
        assert!(parsed_en.is_ok() && parsed_es.is_ok());
        let (Ok(en), Ok(es)) = (parsed_en, parsed_es) else {
            return;
        };
        let mut catalog = TranslationCatalog::new(en.clone(), CatalogLimits {
            locales: NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
            messages: NonZeroUsize::new(128).unwrap_or(NonZeroUsize::MIN),
            utf8_bytes: NonZeroUsize::new(8192).unwrap_or(NonZeroUsize::MIN),
        });
        assert_eq!(register_menu_messages(&mut catalog, &en), Ok(true));
        assert_eq!(register_menu_messages(&mut catalog, &es), Ok(true));
        assert_eq!(catalog.first_missing(&en, MENU_MESSAGE_KEYS), None);
        assert_eq!(catalog.first_missing(&es, MENU_MESSAGE_KEYS), None);
        let text = MenuText {
            catalog: &catalog,
            locale: &es,
        };
        let limit = NonZeroUsize::new(256).unwrap_or(NonZeroUsize::MIN);
        assert_eq!(
            Row::Presentation(PresentationMode::AspectFit).label(text, limit),
            Ok("PANTALLA: proporci\u{f3}n".to_owned())
        );
        assert_eq!(
            Row::SourceVolume(AudioSource::Music, MasterVolume::Half)
                .label(text, limit),
            Ok("M\u{da}SICA: 50%".to_owned())
        );
        let mut capture = Capture::default();
        assert!(
            MenuArt::load_localised(
                &mut capture,
                PlateLayout {
                    background: [0, 0, 0, 255],
                    height: 192,
                    max_bytes: NonZeroUsize::new(2_000_000)
                        .unwrap_or(NonZeroUsize::MIN),
                    width: 280,
                },
                MenuContent {
                    control_reset: true,
                    main_hints: &["JUGAR"],
                    pause_hints: &["CONTINUAR"],
                    settings_hints: &["VOLVER"],
                    sources: SourceSettings::default(),
                    title: "PRUEBA",
                    window_switching: true,
                },
                text,
            )
            .is_ok()
        );
        assert_eq!(capture.layouts.len(), 2);
    }

    #[test]
    fn setting_rows_reflect_the_requested_value() {
        let loaded = english_catalog();
        assert!(loaded.is_ok());
        let Ok((locale, catalog)) = loaded else {
            return;
        };
        let text = MenuText {
            catalog: &catalog,
            locale: &locale,
        };
        let limit = NonZeroUsize::new(256).unwrap_or(NonZeroUsize::MIN);
        let display = DisplaySettings {
            presentation: PresentationMode::Integer,
            window: WindowMode::Fullscreen,
        };
        assert_eq!(
            Row::for_item(
                MenuItem::Display,
                LocalSession::with_preferences(
                    display,
                    MasterVolume::Full,
                    false
                ),
            )
            .label(text, limit),
            Ok("DISPLAY: integer".to_owned())
        );
        assert_eq!(
            Row::for_item(
                MenuItem::Window,
                LocalSession::with_preferences(
                    display,
                    MasterVolume::Full,
                    false
                ),
            )
            .label(text, limit),
            Ok("WINDOW: fullscreen".to_owned())
        );
        assert_eq!(
            Row::for_item(
                MenuItem::Volume,
                LocalSession::with_preferences(
                    display,
                    MasterVolume::Half,
                    false
                ),
            )
            .label(text, limit),
            Ok("MASTER: 50%".to_owned())
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
                sources: SourceSettings::default(),
                title: "TEST",
                window_switching: true,
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

    #[test]
    fn audio_atlas_budget_and_row_changes_match_admitted_sources() {
        let sources = SourceSettings {
            effects: Some(MasterVolume::Full),
            music: Some(MasterVolume::Half),
        };
        let mut capture = Capture::default();
        let bytes = MenuArt::bytes(160, 176, false, sources);
        let loaded = MenuArt::load(
            &mut capture,
            PlateLayout {
                background: [0, 0, 0, 255],
                height: 176,
                max_bytes: NonZeroUsize::new(bytes)
                    .unwrap_or(NonZeroUsize::MIN),
                width: 160,
            },
            MenuContent {
                control_reset: false,
                main_hints: &["START"],
                pause_hints: &["RESUME"],
                settings_hints: &["BACK"],
                sources,
                title: "TEST",
                window_switching: true,
            },
        );
        assert!(loaded.is_ok());
        let Ok(art) = loaded else {
            return;
        };
        assert_eq!(
            capture
                .layouts
                .iter()
                .map(|layout| layout.bytes())
                .sum::<usize>(),
            bytes
        );
        verify_audio_commands(art, sources);
        assert_eq!(capture.layouts.len(), 2);
    }

    fn select_effects(sources: SourceSettings) -> LocalSession {
        let mut session = LocalSession::default().with_source_settings(sources);
        for input in [
            MenuInput::Next,
            MenuInput::Confirm,
            MenuInput::Next,
            MenuInput::Next,
            MenuInput::Confirm,
            MenuInput::Next,
        ] {
            assert_eq!(session.input(input), SessionEffect::None);
        }
        assert_eq!(session.selected(), Some(MenuItem::EffectsVolume));
        session
    }

    fn verify_audio_commands(art: MenuArt, sources: SourceSettings) {
        let mut session = select_effects(sources);
        let mut initial = Vec::new();
        assert_eq!(art.queue(session, (0i32, 0i32), &mut initial), Ok(()));
        assert_eq!(initial.len(), 5);
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        let mut changed = Vec::new();
        assert_eq!(art.queue(session, (0i32, 0i32), &mut changed), Ok(()));
        assert_eq!(initial.get(..2), changed.get(..2));
        assert_ne!(initial.get(2), changed.get(2));
        assert_eq!(initial.get(3..), changed.get(3..));
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
