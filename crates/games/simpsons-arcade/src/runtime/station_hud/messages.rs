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
//   - Authored station HUD message keys and title-selected translations.
// - Must-Not:
//   - Change station identity, input routing or countdown rules.
// - Allows:
//   - Inputs: Shared catalogue, selected locale and typed parameters.
//   - Outputs: Bounded station labels and formatted prompts.
//   - Side effects: None beyond admission to the caller's catalogue.
// - Split-When:
//   - Separate authored data files become necessary.
// - Merge-When:
//   - Title catalogue resources move to a common data owner.
// - Summary:
//   - Localise station presentation without changing gameplay.
// - Description:
//   - Preserve physical bindings and character identities as parameters.
// - Usage:
//   - Register station messages for each title-supported locale.
// - Defaults:
//   - The title selects an explicitly admitted catalogue.

use std::num::NonZeroUsize;

use games::presentation::{
    CatalogError, LocaleTag, MessageArguments, MessageParameter, MessageValue,
    TranslationCatalog,
};
use games_platform::ir::frame::RenderError;

macro_rules! messages {
    ($($key:literal => $english:literal, $spanish:literal;)*) => {
        const ENGLISH: &str =
            concat!("catalog-v1\n", $($key, "\t", $english, "\n",)*);
        const SPANISH: &str =
            concat!("catalog-v1\n", $($key, "\t", $spanish, "\n",)*);
        pub(in crate::runtime) const STATION_MESSAGE_KEYS: &[&str] =
            &[$($key,)*];
    };
}

messages! {
    "station.binding" => "{kind} {binding}", "{kind} {binding}";
    "station.character" => "P{seat} {name}", "J{seat} {name}";
    "station.join" => "JOIN", "UNIRSE";
    "station.join.none" => "JOIN NONE", "SIN TECLA";
    "station.keypad" => "KP", "NUM";
    "station.mouse" => "MOUSE", "RAT\u{d3}N";
    "station.offline" => "OFFLINE", "SIN CONEXI\u{d3}N";
    "station.pad" => "PAD", "CONTROL";
    "station.profile" => "PROFILE", "PERFIL";
    "station.rejoin" => "REJOIN{seconds}", "VOLVER {seconds}";
    "station.remote.offline" => "REMOTE OFFLINE", "REMOTO SIN RED";
    "station.remote.start" => "REMOTE START", "INICIO REMOTO";
    "station.short.key" => "K", "T";
    "station.short.pad" => "P", "C";
}

/// Borrow the title-owned catalogue for a station atlas.
#[derive(Clone, Copy, Debug)]
pub(in crate::runtime) struct StationText<'text> {
    pub catalog: &'text TranslationCatalog,
    pub locale: &'text LocaleTag,
}

impl<'text> StationText<'text> {
    pub(super) fn format(
        self,
        key: &str,
        parameters: &[MessageParameter<'_>],
    ) -> Result<String, RenderError> {
        self.catalog
            .format_message(self.locale, key, MessageArguments {
                max_utf8_bytes: NonZeroUsize::MIN.saturating_add(127),
                parameters,
            })
            .map_err(|error| {
                RenderError::Backend(format!("station {key}: {error:?}"))
            })
    }

    pub(super) fn literal(self, key: &str) -> Result<&'text str, RenderError> {
        self.catalog.resolve_tag(self.locale, key).ok_or_else(|| {
            RenderError::Backend(format!("missing station message: {key}"))
        })
    }
}

pub(in crate::runtime) fn register_station_messages(
    catalog: &mut TranslationCatalog,
    locale: &LocaleTag,
) -> Result<bool, CatalogError> {
    let data = match locale.as_str() {
        "en" => ENGLISH,
        "es-MX" => SPANISH,
        _ => return Ok(false),
    };
    catalog.load_locale(locale.as_str(), data.as_bytes())?;
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
