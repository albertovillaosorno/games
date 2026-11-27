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
//   - Shared control-editor message keys and example locale payloads.
// - Must-Not:
//   - Select title languages, translate physical identities or route input.
// - Allows:
//   - Inputs: Admitted catalogue, selected locale and typed parameters.
//   - Outputs: Borrowed labels and bounded formatted text.
//   - Side effects: None beyond caller-owned catalogue admission.
// - Split-When:
//   - Additional locale payloads need separate data files.
// - Merge-When:
//   - Another shared catalogue owns these exact message identifiers.
// - Summary:
//   - Localise shared editor presentation using the engine catalogue.
// - Description:
//   - Offer English and Mexican Spanish payloads without imposing a language
//     list on titles; admit caller-supplied translations.
// - Usage:
//   - Load required keys for each title-supported locale, then resolve.
// - Defaults:
//   - Compatibility loader uses the original English wording.

use std::num::NonZeroUsize;
use std::rc::Rc;

use games::presentation::{
    CatalogError, CatalogLimits, LocaleTag, MessageArguments, MessageParameter,
    MessageValue, TranslationCatalog,
};

use crate::ir::frame::RenderError;

const EN: &str = concat!(
    "catalog-v1\n",
    "editor.title\tEDIT CONTROLS - NEXT LAUNCH\n",
    "editor.nav\tARROWS / DPAD: PLAYER AND ITEM\n",
    "editor.family_hint\tTAB / RB: SOURCE - ENTER / A: EDIT\n",
    "editor.clear\tDEL / X: CLEAR / RESET FIELD\n",
    "editor.cancel\tESC / B: CANCEL OR RETURN\n",
    "editor.player\tPLAYER {player}\n",
    "editor.player_panel\tPLAYER {player} - PANEL {panel}\n",
    "editor.on\tON\n",
    "editor.off\tOFF\n",
    "editor.action\tACTION: {action}\n",
    "editor.source\tSOURCE: {source}\n",
    "editor.current\tCURRENT: {binding}\n",
    "editor.none\tNONE\n",
    "editor.axis\tPLAYER {player} / AXIS: {axis}\n",
    "editor.negative\t{mark}NEGATIVE: {action}\n",
    "editor.positive\t{mark}POSITIVE: {action}\n",
    "editor.thresholds\t{press_mark}PRESS: {press} ",
    "{release_mark}RELEASE: {release}\n",
    "editor.next_slot\tNEXT SLOT: {action}\n",
    "editor.next_button\tNEXT: {button} - A/ENTER SAVE\n",
    "editor.family.keyboard\tKEYBOARD\n",
    "editor.family.axis\tCONTROLLER AXIS\n",
    "editor.family.mouse\tMOUSE\n",
    "editor.family.controller\tCONTROLLER\n",
    "editor.family.touch\tTOUCH SLOT\n",
    "editor.status.ready_touch\tCONFIRM SLOT / CLEAR PANEL\n",
    "editor.status.ready\tENTER / A TO CHANGE\n",
    "editor.status.closing\tCLOSING\n",
    "editor.status.keyboard\tPRESS A KEY\n",
    "editor.status.axis\tARROWS: FIELD / VALUE\n",
    "editor.status.mouse\tCLICK A MOUSE BUTTON\n",
    "editor.status.controller\tPRESS A GAMEPAD BUTTON\n",
    "editor.status.touch\tCHOOSE SLOT / CONFIRM\n",
    "editor.status.saved\tSAVED FOR NEXT LAUNCH\n",
    "editor.status.rejected\tCONFLICT OR INVALID BINDING\n",
    "editor.status.reviewing\tCONFIRM OR CANCEL BUTTON\n",
);

const ES_MX: &str = concat!(
    "catalog-v1\n",
    "editor.title\tEDITAR CONTROLES - PR\u{d3}XIMO INICIO\n",
    "editor.nav\tFLECHAS / CRUCETA: JUGADOR Y OPCI\u{d3}N\n",
    "editor.family_hint\tTAB / RB: FUENTE - ENTER / A: EDITAR\n",
    "editor.clear\tDEL / X: BORRAR / RESTABLECER\n",
    "editor.cancel\tESC / B: CANCELAR O VOLVER\n",
    "editor.player\tJUGADOR {player}\n",
    "editor.player_panel\tJUGADOR {player} - PANEL {panel}\n",
    "editor.on\tACTIVO\n",
    "editor.off\tINACTIVO\n",
    "editor.action\tACCI\u{d3}N: {action}\n",
    "editor.source\tFUENTE: {source}\n",
    "editor.current\tACTUAL: {binding}\n",
    "editor.none\tNINGUNO\n",
    "editor.axis\tJUGADOR {player} / EJE: {axis}\n",
    "editor.negative\t{mark}NEGATIVO: {action}\n",
    "editor.positive\t{mark}POSITIVO: {action}\n",
    "editor.thresholds\t{press_mark}PULSAR: {press} ",
    "{release_mark}SOLTAR: {release}\n",
    "editor.next_slot\tSIG. ESPACIO: {action}\n",
    "editor.next_button\tSIG.: {button} - A/ENTER GUARDAR\n",
    "editor.family.keyboard\tTECLADO\n",
    "editor.family.axis\tEJE DEL CONTROL\n",
    "editor.family.mouse\tMOUSE\n",
    "editor.family.controller\tCONTROL\n",
    "editor.family.touch\tESPACIO T\u{c1}CTIL\n",
    "editor.status.ready_touch\tCONFIRMAR ESPACIO / BORRAR PANEL\n",
    "editor.status.ready\tENTER / A PARA CAMBIAR\n",
    "editor.status.closing\tCERRANDO\n",
    "editor.status.keyboard\tPULSA UNA TECLA\n",
    "editor.status.axis\tFLECHAS: CAMPO / VALOR\n",
    "editor.status.mouse\tPULSA UN BOT\u{d3}N DEL MOUSE\n",
    "editor.status.controller\tPULSA UN BOT\u{d3}N DEL CONTROL\n",
    "editor.status.touch\tELIGE ESPACIO / CONFIRMA\n",
    "editor.status.saved\tGUARDADO PARA PR\u{d3}XIMO INICIO\n",
    "editor.status.rejected\tCONFLICTO O ASIGNACI\u{d3}N INV\u{c1}LIDA\n",
    "editor.status.reviewing\tCONFIRMAR O CANCELAR BOT\u{d3}N\n",
);

/// Shared catalogue and caller-selected locale for the editor.
#[derive(Clone, Debug)]
pub struct EditorMessages {
    catalogue: Rc<TranslationCatalog>,
    locale: LocaleTag,
    max_utf8_bytes: NonZeroUsize,
}

impl EditorMessages {
    pub(super) fn english(width: u16) -> Result<Self, RenderError> {
        let locale = LocaleTag::parse("en")
            .map_err(|_error| RenderError::InvalidSprite)?;
        let mut catalogue =
            TranslationCatalog::new(locale.clone(), CatalogLimits {
                locales: NonZeroUsize::MIN,
                messages: NonZeroUsize::new(editor_message_keys().count())
                    .ok_or(RenderError::InvalidSprite)?,
                utf8_bytes: NonZeroUsize::new(EN.len())
                    .ok_or(RenderError::InvalidSprite)?,
            });
        catalogue
            .load_locale(locale.as_str(), EN.as_bytes())
            .map_err(|_error| RenderError::InvalidSprite)?;
        Self::new(
            Rc::new(catalogue),
            locale,
            NonZeroUsize::new(usize::from(width).saturating_mul(4))
                .ok_or(RenderError::InvalidDimensions)?,
        )
    }

    pub(super) fn format(
        &self,
        key: &str,
        parameters: &[MessageParameter<'_>],
    ) -> Result<String, RenderError> {
        self.catalogue
            .format_message(&self.locale, key, MessageArguments {
                parameters,
                max_utf8_bytes: self.max_utf8_bytes,
            })
            .map_err(|_error| RenderError::InvalidSprite)
    }

    pub(super) fn label(&self, key: &str) -> Result<&str, RenderError> {
        self.catalogue
            .resolve_tag(&self.locale, key)
            .ok_or(RenderError::InvalidSprite)
    }

    /// Use an admitted catalogue and explicit locale and message capacity.
    ///
    /// # Errors
    ///
    /// Rejects missing required messages after the catalogue's fallback.
    pub fn new(
        catalogue: Rc<TranslationCatalog>,
        locale: LocaleTag,
        max_utf8_bytes: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        if editor_message_keys()
            .any(|key| catalogue.resolve_tag(&locale, key).is_none())
        {
            return Err(RenderError::InvalidSprite);
        }
        Ok(Self {
            catalogue,
            locale,
            max_utf8_bytes,
        })
    }
}

/// Optional built-in editor catalogue for a title-selected language.
#[must_use]
pub fn editor_catalogue(locale: &LocaleTag) -> Option<&'static [u8]> {
    match locale.as_str() {
        "en" => Some(EN.as_bytes()),
        "es-MX" => Some(ES_MX.as_bytes()),
        _ => None,
    }
}

/// Exact shared editor keys that each title-supported locale must supply.
pub fn editor_message_keys() -> impl Iterator<Item = &'static str> {
    EN.lines()
        .skip(1)
        .filter_map(|line| line.split_once('\t').map(|(key, _text)| key))
}

/// Register an available shared editor translation for an exact locale.
/// Returns false when a caller must supply its own resource.
/// # Errors
/// Rejects malformed, duplicate or over-budget resources.
pub fn register_editor_messages(
    catalogue: &mut TranslationCatalog,
    locale: &LocaleTag,
) -> Result<bool, CatalogError> {
    let Some(bytes) = editor_catalogue(locale) else {
        return Ok(false);
    };
    catalogue.load_locale(locale.as_str(), bytes)?;
    Ok(true)
}

pub(super) const fn text<'text>(
    name: &'text str,
    value: &'text str,
) -> MessageParameter<'text> {
    MessageParameter {
        name,
        value: MessageValue::Text(value),
    }
}

pub(super) const fn number(name: &str, value: u64) -> MessageParameter<'_> {
    MessageParameter {
        name,
        value: MessageValue::Unsigned(value),
    }
}
