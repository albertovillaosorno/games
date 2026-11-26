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
//   - Keyboard and gamepad navigation with explicit persistence feedback.

//! Portable presentation for the shared digital control editor.

use std::num::NonZeroUsize;

use games::input::profile::editor::{
    BindingFamily, ControlEditor, EditorStatus,
};
use games::input::profile::{DigitalBinding, DigitalProfiles, KeyNamer};

use crate::ir::frame::RenderError;
use crate::ir::scene::{AtlasUpload, SpriteDraw};
use crate::ir::ui::{PlateLayout, TextAtlas, TextLine};

const GLYPH_WIDTH: u16 = 8;
const GLYPH_HEIGHT: u16 = 10;
const GLYPH_COUNT: usize = 95;
const GLYPH_BYTES: usize = 4 * 8 * 10 * GLYPH_COUNT;
const DYNAMIC_LINES: usize = 5;
const INK: [u8; 4] = [248, 228, 168, 255];

/// Static editor panel and compact printable-glyph atlas.
#[derive(Clone, Copy, Debug)]
pub struct ControlEditorArt {
    glyphs: TextAtlas,
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
            &[panel_lines(spec.width)],
        )?;
        let labels = (b' '..=b'~')
            .map(|byte| String::from(char::from(byte)))
            .collect::<Vec<_>>();
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
        let ControlEditorView {
            editor,
            profiles,
            actions,
            key_name,
        } = view;
        let action = actions
            .get(editor.action())
            .ok_or(RenderError::InvalidSprite)?;
        let binding =
            editor.current(profiles).ok_or(RenderError::InvalidSprite)?;
        let name = binding_name(binding, &actions, key_name)
            .ok_or(RenderError::InvalidSprite)?;
        let status = status_label(editor.status(), editor.family());
        let lines = [
            format!("PLAYER {}", editor.station().saturating_add(1)),
            format!("ACTION: {action}"),
            format!("SOURCE: {}", editor.family().label()),
            format!("CURRENT: {name}"),
            match editor.status() {
                EditorStatus::Capturing
                    if editor.family() == BindingFamily::Touch =>
                {
                    let slot = actions
                        .get(editor.touch_slot())
                        .ok_or(RenderError::InvalidSprite)?;
                    format!("NEXT SLOT: {slot}")
                },
                EditorStatus::Reviewing(button) => {
                    format!("NEXT: {} - A/ENTER SAVE", button.profile_name())
                },
                EditorStatus::Capturing
                | EditorStatus::Ready
                | EditorStatus::Rejected
                | EditorStatus::Saved => status.to_owned(),
            },
        ];
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
        let capacity = usize::from(
            self.width.checked_div(GLYPH_WIDTH).unwrap_or_default(),
        );
        let count = text.chars().count();
        if count > capacity {
            return Err(RenderError::InvalidDimensions);
        }
        let length = i32::try_from(count)
            .map_err(|_error| RenderError::InvalidDimensions)?
            .saturating_mul(i32::from(GLYPH_WIDTH));
        let mut left = origin.0.saturating_add(
            i32::from(self.width)
                .saturating_sub(length)
                .checked_div(2)
                .unwrap_or_default(),
        );
        for character in text.chars() {
            if character != ' ' {
                let glyph = u8::try_from(character)
                    .ok()
                    .filter(|byte| (b' '..=b'~').contains(byte))
                    .unwrap_or(b'?');
                let tile = usize::from(glyph.saturating_sub(b' '));
                draws.push(self.glyphs.draw(tile, left, origin.1)?);
            }
            left = left.saturating_add(i32::from(GLYPH_WIDTH));
        }
        Ok(())
    }

    /// Maximum number of sprites queued by the editor.
    #[must_use]
    pub fn sprites(width: u16) -> usize {
        usize::from(width.checked_div(GLYPH_WIDTH).unwrap_or_default())
            .saturating_mul(DYNAMIC_LINES)
            .saturating_add(1)
    }
}

fn binding_name<'name>(
    binding: DigitalBinding,
    actions: &[&'name str],
    key_name: KeyNamer,
) -> Option<&'name str> {
    Some(match binding {
        DigitalBinding::Key(key) => key.map_or("NONE", key_name),
        DigitalBinding::Mouse(button) => {
            button.map_or("NONE", |value| value.profile_name())
        },
        DigitalBinding::Button(button) => button.profile_name(),
        DigitalBinding::TouchSlot(slot) => *actions.get(slot)?,
    })
}

const fn status_label(
    status: EditorStatus,
    family: BindingFamily,
) -> &'static str {
    match status {
        EditorStatus::Ready => "ENTER TO CHANGE",
        EditorStatus::Capturing => match family {
            BindingFamily::Keyboard => "PRESS A KEY",
            BindingFamily::Mouse => "CLICK A MOUSE BUTTON",
            BindingFamily::Controller => "PRESS A GAMEPAD BUTTON",
            BindingFamily::Touch => "CHOOSE SLOT / ENTER",
        },
        EditorStatus::Saved => "SAVED FOR NEXT LAUNCH",
        EditorStatus::Rejected => "CONFLICT OR INVALID BINDING",
        EditorStatus::Reviewing(_) => "CONFIRM OR CANCEL BUTTON",
    }
}

fn panel_lines(width: u16) -> Vec<TextLine<'static>> {
    [
        (8, "EDIT CONTROLS - NEXT LAUNCH"),
        (130, "ARROWS / DPAD: PLAYER AND ACTION"),
        (146, "TAB / RB: SOURCE - ENTER / A: EDIT"),
        (162, "DEL / X: CLEAR KEY OR MOUSE"),
        (178, "ESC / B: CANCEL OR RETURN"),
    ]
    .into_iter()
    .map(|(top, text)| {
        TextLine {
            color: [220, 222, 232, 255],
            scale: 2,
            text,
            top,
        }
        .fit_width(width)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::frame::FrameLayout;
    use crate::ir::scene::AtlasId;

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
        let result = ControlEditorArt::load(&mut capture, spec);
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
            art.queue_line(&"A".repeat(36), (0i32, 0i32), &mut draws)
                .is_err()
        );
    }
}
