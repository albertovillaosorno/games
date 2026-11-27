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
//   - Shared menu message identities and optional built-in translations.
// - Must-Not:
//   - Select a title's locales, fallback, gameplay or settings.
// - Allows:
//   - Inputs: Caller-selected catalogue, locale and named parameters.
//   - Outputs: Admitted messages or explicit rendering errors.
//   - Side effects: Bounded catalogue registration.
// - Split-When:
//   - Menu message resources require independently loaded catalogues.
// - Merge-When:
//   - A shared UI message registry replaces independent registration.
// - Summary:
//   - Localise shared menu text through the engine catalogue.
// - Description:
//   - Register available built-in resources without choosing a title locale.
// - Usage:
//   - Admit the requested resources and supply MenuText to menu loading.
// - Defaults:
//   - No implicit title locale or fallback selection.

use std::num::NonZeroUsize;

use games::presentation::{
    CatalogError, CatalogLimits, LocaleTag, MessageArguments, MessageParameter,
    TranslationCatalog,
};

use crate::ir::frame::RenderError;

macro_rules! messages {
    ($($key:literal => $en:literal, $es:literal;)*) => {
        /// Required stable keys for complete shared menu translation.
        pub const MENU_MESSAGE_KEYS: &[&str] = &[$($key),*];
        const ENGLISH: &str = concat!("catalog-v1\n",
            $($key, "\t", $en, "\n",)*);
        const MEXICAN_SPANISH: &str = concat!("catalog-v1\n",
            $($key, "\t", $es, "\n",)*);
    };
}

messages! {
    "menu.audio" => "AUDIO", "AUDIO";
    "menu.audio.all" => "MASTER APPLIES TO ALL SOURCES",
        "GENERAL APLICA A TODOS LOS SONIDOS";
    "menu.audio.hint" => "PRIMARY CHANGES LEVEL - PAUSE RETURNS",
        "PRINCIPAL CAMBIA NIVEL - PAUSA REGRESA";
    "menu.back" => "BACK", "VOLVER";
    "menu.control.defaults" => "DEFAULTS NEXT LAUNCH",
        "VALORES AL REINICIAR";
    "menu.control.edit" => "EDIT CONTROLS NEXT LAUNCH",
        "EDITAR CONTROLES AL REINICIAR";
    "menu.control.reset" => "RESET CONTROLS NEXT LAUNCH",
        "RESTABLECER CONTROLES AL REINICIAR";
    "menu.display" => "DISPLAY", "PANTALLA";
    "menu.display.adaptive" => "adaptive", "adaptable";
    "menu.display.aspect-fit" => "aspect-fit", "proporci\u{f3}n";
    "menu.display.fill" => "fill", "relleno";
    "menu.display.integer" => "integer", "entera";
    "menu.display.stretch" => "stretch", "estirada";
    "menu.display.value" => "DISPLAY: {mode}", "PANTALLA: {mode}";
    "menu.effects" => "EFFECTS", "EFECTOS";
    "menu.effects.value" => "EFFECTS: {percent}%", "EFECTOS: {percent}%";
    "menu.exit" => "EXIT", "SALIR";
    "menu.free-play" => "FREE PLAY", "JUEGO LIBRE";
    "menu.main" => "MAIN MENU", "MEN\u{da} PRINCIPAL";
    "menu.master.value" => "MASTER: {percent}%", "GENERAL: {percent}%";
    "menu.music" => "MUSIC", "M\u{da}SICA";
    "menu.music.value" => "MUSIC: {percent}%", "M\u{da}SICA: {percent}%";
    "menu.paused" => "PAUSED", "EN PAUSA";
    "menu.play" => "PLAY", "JUGAR";
    "menu.reset" => "RESET CONTROLS", "RESTABLECER CONTROLES";
    "menu.resume" => "RESUME", "CONTINUAR";
    "menu.settings" => "SETTINGS", "AJUSTES";
    "menu.volume" => "VOLUME", "VOLUMEN";
    "menu.window" => "WINDOW", "VENTANA";
    "menu.window.fullscreen" => "fullscreen", "pantalla completa";
    "menu.window.value" => "WINDOW: {mode}", "VENTANA: {mode}";
    "menu.window.windowed" => "windowed", "ventana";
}

/// Title-selected shared catalogue view for one menu atlas.
#[derive(Clone, Copy, Debug)]
pub struct MenuText<'text> {
    /// Catalogue containing shared message keys.
    pub catalog: &'text TranslationCatalog,
    /// Locale used to resolve menu messages.
    pub locale: &'text LocaleTag,
}

type OwnedMenuCatalog = (LocaleTag, TranslationCatalog);

impl<'text> MenuText<'text> {
    pub(super) fn format(
        self,
        key: &str,
        parameter: MessageParameter<'_>,
        limit: NonZeroUsize,
    ) -> Result<String, RenderError> {
        self.catalog
            .format_message(self.locale, key, MessageArguments {
                max_utf8_bytes: limit,
                parameters: &[parameter],
            })
            .map_err(|error| {
                RenderError::Backend(format!("menu {key}: {error:?}"))
            })
    }

    pub(super) fn literal(self, key: &str) -> Result<&'text str, RenderError> {
        self.catalog.resolve_tag(self.locale, key).ok_or_else(|| {
            RenderError::Backend(format!("missing menu message: {key}"))
        })
    }
}

pub(super) fn english_catalog() -> Result<OwnedMenuCatalog, RenderError> {
    let locale =
        LocaleTag::parse("en").map_err(|_error| RenderError::InvalidSprite)?;
    let limits = CatalogLimits {
        locales: NonZeroUsize::MIN,
        messages: NonZeroUsize::new(MENU_MESSAGE_KEYS.len())
            .ok_or(RenderError::InvalidSprite)?,
        utf8_bytes: NonZeroUsize::new(ENGLISH.len())
            .ok_or(RenderError::InvalidSprite)?,
    };
    let mut catalog = TranslationCatalog::new(locale.clone(), limits);
    let _registered =
        register_menu_messages(&mut catalog, &locale).map_err(|error| {
            RenderError::Backend(format!("menu catalogue: {error:?}"))
        })?;
    Ok((locale, catalog))
}

/// Register an available shared menu translation for an exact locale.
///
/// Returns false for locales without a built-in resource; callers may load
/// their own translation instead. Does not change catalogue fallback.
///
/// # Errors
/// Rejects malformed, duplicate or over-budget resources.
pub fn register_menu_messages(
    catalog: &mut TranslationCatalog,
    locale: &LocaleTag,
) -> Result<bool, CatalogError> {
    let data = match locale.as_str() {
        "en" => ENGLISH,
        "es-MX" => MEXICAN_SPANISH,
        _ => return Ok(false),
    };
    catalog.load_locale(locale.as_str(), data.as_bytes())?;
    Ok(true)
}
