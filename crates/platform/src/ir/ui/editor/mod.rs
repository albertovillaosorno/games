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
//   - Presentation of shared next-launch digital control editing.
// - Must-Not:
//   - Change control mappings, route input, persist settings or select titles.
// - Allows:
//   - Inputs: Admitted profile, editor state and title-provided action names.
//   - Outputs: A static panel and bounded dynamic glyph draw commands.
//   - Side effects: Two bounded atlas uploads during explicit initialization.
// - Split-When:
//   - A reusable dynamic font layout needs an independent module.
// - Merge-When:
//   - Another shared menu duplicates this presentation mechanism.
// - Summary:
//   - Render selected station, action, binding and capture status.
// - Description:
//   - Upload a tiny printable glyph atlas once; draw dynamic labels without
//     further uploads or unbounded cached textures.
// - Usage:
//   - Reserve the reported bytes and sprites before loading the panel.
// - Defaults:
//   - Keyboard, gamepad and touch navigation with explicit persistence
//     feedback.

//! Portable presentation for the shared digital control editor.

mod messages;

use std::num::{NonZeroU32, NonZeroUsize};

use games::input::profile::editor::{
    AxisEditorView, AxisField, BindingFamily, ControlEditor, EditorStatus,
};
use games::input::profile::{DigitalBinding, DigitalProfiles, KeyNamer};
use games::presentation::MessageParameter;
pub use messages::{
    EditorMessages, editor_catalogue, editor_message_keys,
    register_editor_messages,
};
use messages::{number, text};

use crate::ir::frame::RenderError;
use crate::ir::scene::{AtlasUpload, SpriteDraw};
use crate::ir::ui::{PlateLayout, TextAtlas, TextLine, glyphs};

const GLYPH_WIDTH: u16 = 8;
const GLYPH_HEIGHT: u16 = 10;
const MIN_GLYPH_WIDTH: u16 = 4;
const MIN_GLYPH_HEIGHT: u16 = 5;
const GLYPH_BYTES: usize = 4 * 8 * 10 * glyphs::COUNT;
const DYNAMIC_LINES: usize = 5;
const INK: [u8; 4] = [248, 228, 168, 255];

/// Static editor panel and compact printable-glyph atlas.
#[derive(Clone, Debug)]
pub struct ControlEditorArt {
    glyphs: TextAtlas,
    messages: EditorMessages,
    panel: TextAtlas,
    width: u16,
}

/// Borrowed title bindings and selection to present in the editor.
#[derive(Clone, Copy, Debug)]
pub struct ControlEditorView<
    'profile,
    const PLAYERS: usize,
    const ACTIONS: usize,
> {
    /// Title action names, in profile order.
    pub actions: [&'profile str; ACTIONS],
    /// Current editor selection and acknowledgement.
    pub editor: ControlEditor,
    /// Native keyboard name resolver.
    pub key_name: KeyNamer,
    /// Current admitted next-launch bindings.
    pub profiles: &'profile DigitalProfiles<PLAYERS, ACTIONS>,
}

impl ControlEditorArt {
    /// Exact additional atlas byte budget for this panel and its glyphs.
    #[must_use]
    pub fn bytes(width: u16, height: u16) -> usize {
        usize::from(width)
            .saturating_mul(usize::from(height))
            .saturating_mul(4)
            .saturating_add(GLYPH_BYTES)
    }

    /// Upload the editor's bounded static artwork.
    ///
    /// # Errors
    /// Rejects clipped text, insufficient budget and invalid geometry.
    pub fn load<Upload>(
        scene: &mut Upload,
        spec: PlateLayout,
        touch_hints: Option<[&str; 2]>,
    ) -> Result<Self, RenderError>
    where
        Upload: AtlasUpload,
    {
        let messages = EditorMessages::english(spec.width)?;
        Self::load_localised(scene, spec, touch_hints, messages)
    }

    /// Upload the editor for a caller-selected, admitted catalogue locale.
    /// # Errors
    /// Rejects missing messages, clipped text or insufficient atlas budget.
    pub fn load_localised<Upload>(
        scene: &mut Upload,
        spec: PlateLayout,
        touch_hints: Option<[&str; 2]>,
        messages: EditorMessages,
    ) -> Result<Self, RenderError>
    where
        Upload: AtlasUpload,
    {
        if spec.max_bytes.get() < Self::bytes(spec.width, spec.height) {
            return Err(RenderError::TooLarge);
        }
        let panel_bytes = spec.max_bytes.get().saturating_sub(GLYPH_BYTES);
        let panel_budget =
            NonZeroUsize::new(panel_bytes).ok_or(RenderError::TooLarge)?;
        let panel = TextAtlas::load(
            scene,
            PlateLayout {
                max_bytes: panel_budget,
                ..spec
            },
            &[panel_lines(spec.width, touch_hints, &messages)?],
        )?;
        let labels = glyphs::labels().map(String::from).collect::<Vec<_>>();
        let lines = labels
            .iter()
            .map(|label| {
                vec![TextLine {
                    color: INK,
                    scale: 2,
                    text: label,
                    top: 0,
                }]
            })
            .collect::<Vec<_>>();
        let glyphs = TextAtlas::load(
            scene,
            PlateLayout {
                background: spec.background,
                height: GLYPH_HEIGHT,
                max_bytes: NonZeroUsize::new(GLYPH_BYTES)
                    .ok_or(RenderError::TooLarge)?,
                width: GLYPH_WIDTH,
            },
            &lines,
        )?;
        Ok(Self {
            glyphs,
            messages,
            panel,
            width: spec.width,
        })
    }

    /// Queue an editor frame from the selected admitted profile.
    ///
    /// # Errors
    /// Rejects a missing title action or invalid sprite geometry.
    pub fn queue<const PLAYERS: usize, const ACTIONS: usize>(
        &self,
        view: ControlEditorView<'_, PLAYERS, ACTIONS>,
        origin: (i32, i32),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let lines = if let Some(axis) = view.editor.axis_view(view.profiles) {
            axis_lines(axis, &view.actions, view.editor, &self.messages)?
        } else {
            digital_lines(view, &self.messages)?
        };
        draws.push(self.panel.draw(0, origin.0, origin.1)?);
        for (index, line) in lines.iter().enumerate() {
            let top = i32::try_from(index)
                .map_err(|_error| RenderError::InvalidDimensions)?
                .saturating_mul(18)
                .saturating_add(34);
            self.queue_line(
                line,
                (origin.0, origin.1.saturating_add(top)),
                draws,
            )?;
        }
        Ok(())
    }

    fn queue_line(
        &self,
        text: &str,
        origin: (i32, i32),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let count = glyphs::display(text).count();
        let (glyph_width, glyph_height) = if count
            .saturating_mul(usize::from(GLYPH_WIDTH))
            <= usize::from(self.width)
        {
            (GLYPH_WIDTH, GLYPH_HEIGHT)
        } else {
            (MIN_GLYPH_WIDTH, MIN_GLYPH_HEIGHT)
        };
        let extent = count
            .checked_mul(usize::from(glyph_width))
            .filter(|length| *length <= usize::from(self.width))
            .ok_or(RenderError::InvalidDimensions)?;
        let length = i32::try_from(extent)
            .map_err(|_error| RenderError::InvalidDimensions)?;
        let mut left = origin.0.saturating_add(
            i32::from(self.width)
                .saturating_sub(length)
                .checked_div(2)
                .unwrap_or_default(),
        );
        let top = origin.1.saturating_add(i32::from(
            GLYPH_HEIGHT
                .saturating_sub(glyph_height)
                .checked_div(2)
                .unwrap_or_default(),
        ));
        let width = NonZeroU32::new(u32::from(glyph_width))
            .ok_or(RenderError::InvalidDimensions)?;
        let height = NonZeroU32::new(u32::from(glyph_height))
            .ok_or(RenderError::InvalidDimensions)?;
        for character in glyphs::display(text) {
            if character != ' ' {
                let tile = glyphs::tile(character);
                let mut draw = self.glyphs.draw(tile, left, top)?;
                draw.destination.width = width;
                draw.destination.height = height;
                draws.push(draw);
            }
            left = left.saturating_add(i32::from(glyph_width));
        }
        Ok(())
    }

    /// Maximum number of sprites queued by the editor.
    #[must_use]
    pub fn sprites(width: u16) -> usize {
        usize::from(width.checked_div(MIN_GLYPH_WIDTH).unwrap_or_default())
            .saturating_mul(DYNAMIC_LINES)
            .saturating_add(1)
    }
}

fn axis_lines(
    view: AxisEditorView,
    actions: &[&str],
    editor: ControlEditor,
    messages: &EditorMessages,
) -> Result<[String; DYNAMIC_LINES], RenderError> {
    let AxisEditorView { axis, field, profile } = view;
    let name = |slot: Option<usize>| {
        slot.map_or_else(
            || messages.label("editor.none"),
            |index| {
                actions
                    .get(index)
                    .copied()
                    .ok_or(RenderError::InvalidSprite)
            },
        )
    };
    let mark = |selected: AxisField| {
        if field == Some(selected) {
            ">"
        } else {
            " "
        }
    };
    Ok([
        messages.format("editor.axis", &[
            player_parameter(editor)?,
            text("axis", axis.profile_name()),
        ])?,
        messages.format("editor.negative", &[
            text("mark", mark(AxisField::Negative)),
            text("action", name(profile.negative)?),
        ])?,
        messages.format("editor.positive", &[
            text("mark", mark(AxisField::Positive)),
            text("action", name(profile.positive)?),
        ])?,
        messages.format("editor.thresholds", &[
            text("press_mark", mark(AxisField::Press)),
            number("press", u64::from(profile.press)),
            text("release_mark", mark(AxisField::Release)),
            number("release", u64::from(profile.release)),
        ])?,
        messages
            .label(status_key(editor.status(), BindingFamily::Axis))?
            .to_owned(),
    ])
}

fn player_parameter(
    editor: ControlEditor,
) -> Result<MessageParameter<'static>, RenderError> {
    let player = u64::try_from(editor.station().saturating_add(1))
        .map_err(|_error| RenderError::InvalidSprite)?;
    Ok(number("player", player))
}

fn binding_name<'name>(
    binding: DigitalBinding,
    actions: &[&'name str],
    key_name: KeyNamer,
    messages: &'name EditorMessages,
) -> Result<&'name str, RenderError> {
    match binding {
        DigitalBinding::Key(key) => key.map_or_else(
            || messages.label("editor.none"),
            |value| Ok(key_name(value)),
        ),
        DigitalBinding::Mouse(button) => button.map_or_else(
            || messages.label("editor.none"),
            |value| Ok(value.profile_name()),
        ),
        DigitalBinding::Button(button) => Ok(button.profile_name()),
        DigitalBinding::TouchSlot(slot) => {
            actions.get(slot).copied().ok_or(RenderError::InvalidSprite)
        },
    }
}

const fn family_key(family: BindingFamily) -> &'static str {
    match family {
        BindingFamily::Axis => "editor.family.axis",
        BindingFamily::Controller => "editor.family.controller",
        BindingFamily::Keyboard => "editor.family.keyboard",
        BindingFamily::Mouse => "editor.family.mouse",
        BindingFamily::Touch => "editor.family.touch",
    }
}

fn digital_lines<const PLAYERS: usize, const ACTIONS: usize>(
    view: ControlEditorView<'_, PLAYERS, ACTIONS>,
    messages: &EditorMessages,
) -> Result<[String; DYNAMIC_LINES], RenderError> {
    let ControlEditorView {
        editor,
        profiles,
        actions,
        key_name,
    } = view;
    let action = actions
        .get(editor.action())
        .ok_or(RenderError::InvalidSprite)?;
    let binding = editor.current(profiles).ok_or(RenderError::InvalidSprite)?;
    let name = binding_name(binding, &actions, key_name, messages)?;
    let family = editor.family();
    let player = player_parameter(editor)?;
    let heading = if family == BindingFamily::Touch {
        let enabled = profiles
            .touch_players
            .get(editor.station())
            .ok_or(RenderError::InvalidSprite)?;
        let panel = messages.label(if *enabled {
            "editor.on"
        } else {
            "editor.off"
        })?;
        messages
            .format("editor.player_panel", &[player, text("panel", panel)])?
    } else {
        messages.format("editor.player", &[player])?
    };
    Ok([
        heading,
        messages.format("editor.action", &[text("action", action)])?,
        messages.format("editor.source", &[text(
            "source",
            messages.label(family_key(family))?,
        )])?,
        messages.format("editor.current", &[text("binding", name)])?,
        status_line(editor, &actions, messages)?,
    ])
}

fn status_line(
    editor: ControlEditor,
    actions: &[&str],
    messages: &EditorMessages,
) -> Result<String, RenderError> {
    let family = editor.family();
    match editor.status() {
        EditorStatus::Capturing if family == BindingFamily::Touch => {
            let slot = actions
                .get(editor.touch_slot())
                .ok_or(RenderError::InvalidSprite)?;
            messages.format("editor.next_slot", &[text("action", slot)])
        },
        EditorStatus::Reviewing(button) => messages.format(
            "editor.next_button",
            &[text("button", button.profile_name())],
        ),
        status @ (EditorStatus::Capturing
        | EditorStatus::Closing
        | EditorStatus::Ready
        | EditorStatus::Rejected
        | EditorStatus::Saved) => messages
            .label(status_key(status, family))
            .map(str::to_owned),
    }
}

const fn status_key(
    status: EditorStatus,
    family: BindingFamily,
) -> &'static str {
    match status {
        EditorStatus::Ready if matches!(family, BindingFamily::Touch) => {
            "editor.status.ready_touch"
        },
        EditorStatus::Ready => "editor.status.ready",
        EditorStatus::Closing => "editor.status.closing",
        EditorStatus::Capturing => match family {
            BindingFamily::Keyboard => "editor.status.keyboard",
            BindingFamily::Axis => "editor.status.axis",
            BindingFamily::Mouse => "editor.status.mouse",
            BindingFamily::Controller => "editor.status.controller",
            BindingFamily::Touch => "editor.status.touch",
        },
        EditorStatus::Saved => "editor.status.saved",
        EditorStatus::Rejected => "editor.status.rejected",
        EditorStatus::Reviewing(_) => "editor.status.reviewing",
    }
}

fn panel_lines<'text>(
    width: u16,
    touch_hints: Option<[&'text str; 2]>,
    messages: &'text EditorMessages,
) -> Result<Vec<TextLine<'text>>, RenderError> {
    let lines = [
        (8, messages.label("editor.title")?),
        (120, messages.label("editor.nav")?),
        (132, messages.label("editor.family_hint")?),
        (144, messages.label("editor.clear")?),
        (156, messages.label("editor.cancel")?),
    ];
    Ok(lines
        .into_iter()
        .chain(
            touch_hints
                .into_iter()
                .flatten()
                .zip([168, 180])
                .map(|(text, top)| (top, text)),
        )
        .map(|(top, text)| {
            TextLine {
                color: [220, 222, 232, 255],
                scale: 2,
                text,
                top,
            }
            .fit_width(width)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use games::input::physical::{ControllerButton, KeyboardKey};
    use games::input::profile::AxisProfile;
    use games::input::profile::editor::EditorNavigation;
    use games::presentation::{CatalogLimits, LocaleTag, TranslationCatalog};

    use super::*;
    use crate::ir::frame::FrameLayout;
    use crate::ir::scene::AtlasId;

    const PROFILE: DigitalProfiles<1, 2> = DigitalProfiles {
        axes: [[AxisProfile::UNBOUND; 6]],
        buttons: [[ControllerButton::South, ControllerButton::East]],
        keys: [[Some(KeyboardKey::LETTER_A), None]],
        mouse: [[None; 2]],
        touch_players: [true],
        touch_slots: [[0, 1]],
    };

    #[derive(Debug, Default)]
    struct Capture {
        bytes: usize,
        uploads: usize,
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
            let id = AtlasId(self.uploads);
            self.bytes = self.bytes.saturating_add(pixels.len());
            self.uploads = self.uploads.saturating_add(1);
            Ok(id)
        }
    }

    #[test]
    fn static_upload_and_bounded_dynamic_labels() {
        let mut capture = Capture::default();
        let spec = PlateLayout {
            background: [24, 22, 30, 255],
            height: 192,
            max_bytes: NonZeroUsize::new(ControlEditorArt::bytes(280, 192))
                .unwrap_or(NonZeroUsize::MIN),
            width: 280,
        };
        let result = ControlEditorArt::load(
            &mut capture,
            spec,
            Some([
                "TOUCH: ARROWS, B SOURCE, A EDIT",
                "TOUCH: START CLEAR, PAUSE BACK",
            ]),
        );
        let Ok(art) = result else {
            assert!(result.is_ok(), "{result:?}");
            return;
        };
        assert_eq!(capture.uploads, 2);
        assert_eq!(capture.bytes, spec.max_bytes.get());
        let mut draws = Vec::new();
        assert_eq!(
            art.queue_line("CURRENT: SPACE", (20i32, 50i32), &mut draws),
            Ok(())
        );
        assert_eq!(draws.len(), 13);
        assert!(draws.len() < ControlEditorArt::sprites(280));
        assert_eq!(capture.uploads, 2);
        assert!(
            art.queue_line(&"A".repeat(71), (0i32, 0i32), &mut draws)
                .is_err()
        );
    }

    #[test]
    fn dynamic_labels_share_spanish_glyphs_and_composition()
    -> Result<(), RenderError> {
        let mut capture = Capture::default();
        let spec = PlateLayout {
            background: [24, 22, 30, 255],
            height: 192,
            max_bytes: NonZeroUsize::new(ControlEditorArt::bytes(280, 192))
                .ok_or(RenderError::TooLarge)?,
            width: 280,
        };
        let art = ControlEditorArt::load(&mut capture, spec, None)?;
        let mut composed = Vec::new();
        let mut decomposed = Vec::new();
        art.queue_line("\u{bf}A\u{d1}O?", (0i32, 0i32), &mut composed)?;
        art.queue_line("\u{bf}AN\u{303}O?", (0i32, 0i32), &mut decomposed)?;
        let mut plain = Vec::new();
        art.queue_line("?ANO?", (0i32, 0i32), &mut plain)?;
        let mut unsupported = Vec::new();
        let mut replacement = Vec::new();
        art.queue_line("\u{5b57}", (0i32, 0i32), &mut unsupported)?;
        art.queue_line("\u{fffd}", (0i32, 0i32), &mut replacement)?;
        if composed.len() == 5
            && composed == decomposed
            && composed != plain
            && unsupported == replacement
            && capture.uploads == 2
        {
            Ok(())
        } else {
            Err(RenderError::InvalidSprite)
        }
    }

    fn catalogues() -> Result<Rc<TranslationCatalog>, RenderError> {
        let en = LocaleTag::parse("en")
            .map_err(|_error| RenderError::InvalidSprite)?;
        let es = LocaleTag::parse("es-MX")
            .map_err(|_error| RenderError::InvalidSprite)?;
        let mut catalogue =
            TranslationCatalog::new(en.clone(), CatalogLimits {
                locales: NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
                messages: NonZeroUsize::new(
                    editor_message_keys().count().saturating_mul(2),
                )
                .unwrap_or(NonZeroUsize::MIN),
                utf8_bytes: NonZeroUsize::new(8192)
                    .unwrap_or(NonZeroUsize::MIN),
            });
        for locale in [&en, &es] {
            if !register_editor_messages(&mut catalogue, locale)
                .map_err(|_error| RenderError::InvalidSprite)?
            {
                return Err(RenderError::InvalidSprite);
            }
        }
        if editor_message_keys().any(|key| {
            !catalogue.contains_exact(&en, key)
                || !catalogue.contains_exact(&es, key)
        }) {
            return Err(RenderError::InvalidSprite);
        }
        Ok(Rc::new(catalogue))
    }

    fn spanish() -> Result<EditorMessages, RenderError> {
        EditorMessages::new(
            catalogues()?,
            LocaleTag::parse("es-MX")
                .map_err(|_error| RenderError::InvalidSprite)?,
            NonZeroUsize::new(1120).unwrap_or(NonZeroUsize::MIN),
        )
    }

    fn key_name(_key: KeyboardKey) -> &'static str {
        "A"
    }

    fn view(editor: ControlEditor) -> ControlEditorView<'static, 1, 2> {
        ControlEditorView {
            actions: ["SALTO", "PAUSA"],
            editor,
            key_name,
            profiles: &PROFILE,
        }
    }

    #[test]
    fn translated_digital_lines_preserve_physical_binding_and_action()
    -> Result<(), RenderError> {
        let messages = spanish()?;
        let lines = digital_lines(view(ControlEditor::default()), &messages)?;
        if lines
            == [
                "JUGADOR 1",
                "ACCI\u{d3}N: SALTO",
                "FUENTE: TECLADO",
                "ACTUAL: A",
                "ENTER / A PARA CAMBIAR",
            ]
        {
            Ok(())
        } else {
            Err(RenderError::InvalidSprite)
        }
    }

    #[test]
    fn localized_editor_queues_all_families_at_title_dimensions()
    -> Result<(), RenderError> {
        let mut capture = Capture::default();
        let spec = PlateLayout {
            background: [24, 22, 30, 255],
            height: 192,
            max_bytes: NonZeroUsize::new(ControlEditorArt::bytes(280, 192))
                .ok_or(RenderError::TooLarge)?,
            width: 280,
        };
        let art = ControlEditorArt::load_localised(
            &mut capture,
            spec,
            None,
            spanish()?,
        )?;
        let mut editor = ControlEditor::default();
        let mut draws = Vec::new();
        for _ in 0..5u8 {
            draws.clear();
            art.queue(view(editor), (0i32, 0i32), &mut draws)?;
            if draws.is_empty()
                || draws.len() > ControlEditorArt::sprites(spec.width)
            {
                return Err(RenderError::InvalidSprite);
            }
            let _effect = editor.navigate(EditorNavigation::Family, &PROFILE);
        }
        if capture.bytes != spec.max_bytes.get() || capture.uploads != 2 {
            return Err(RenderError::InvalidLength);
        }
        Ok(())
    }

    #[test]
    fn editor_respects_caller_fallback_for_unknown_locale()
    -> Result<(), RenderError> {
        let catalogue = catalogues()?;
        let unknown = LocaleTag::parse("fr-CA")
            .map_err(|_error| RenderError::InvalidSprite)?;
        let messages =
            EditorMessages::new(catalogue, unknown, NonZeroUsize::MIN)?;
        if messages.label("editor.title")? == "EDIT CONTROLS - NEXT LAUNCH" {
            Ok(())
        } else {
            Err(RenderError::InvalidSprite)
        }
    }

    #[test]
    fn longer_confirmation_preserves_every_glyph_with_integer_fit()
    -> Result<(), RenderError> {
        let mut capture = Capture::default();
        let spec = PlateLayout {
            background: [24, 22, 30, 255],
            height: 192,
            max_bytes: NonZeroUsize::new(ControlEditorArt::bytes(280, 192))
                .ok_or(RenderError::TooLarge)?,
            width: 280,
        };
        let messages = spanish()?;
        let line = messages.format("editor.next_button", &[text(
            "button",
            ControllerButton::RightShoulder.profile_name(),
        )])?;
        let count =
            glyphs::display(&line).filter(|glyph| *glyph != ' ').count();
        let art = ControlEditorArt::load_localised(
            &mut capture,
            spec,
            None,
            messages,
        )?;
        let mut draws = Vec::new();
        art.queue_line(&line, (10i32, 20i32), &mut draws)?;
        let fits = draws.iter().all(|draw| {
            draw.destination.width.get() == 4
                && draw.destination.height.get() == 5
                && draw.destination.y == 22i32
                && draw.destination.x >= 10i32
                && draw.destination.x.saturating_add(4) <= 290i32
        });
        if draws.len() == count
            && fits
            && draws.len() <= ControlEditorArt::sprites(spec.width)
            && capture.uploads == 2
        {
            Ok(())
        } else {
            Err(RenderError::InvalidSprite)
        }
    }
}
