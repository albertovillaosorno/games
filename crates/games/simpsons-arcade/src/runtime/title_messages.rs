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
//   - Simpsons Arcade authored menu text and supported locale identities.
// - Must-Not:
//   - Own locale parsing, rendering, input routing or original assets.
// - Allows:
//   - Inputs: Explicit title locale and shared catalogue resources.
//   - Outputs: Complete authored menu catalogue and locale labels.
//   - Side effects: Bounded catalogue admission.
// - Split-When:
//   - Substantial independent authored text surfaces need separate modules.
// - Merge-When:
//   - Shared resource admission removes duplicate title-specific logic.
// - Summary:
//   - Declare the title's selected languages without changing gameplay.
// - Description:
//   - Retain original English title identity and translate modern menu UI.
// - Usage:
//   - Select a locale index from the title list and resolve stable keys.
// - Defaults:
//   - English is the default and final fallback for this title.

use std::num::NonZeroUsize;
use std::rc::Rc;

use games::input::physical::{KeyboardKey, PointerButton};
use games::presentation::{
    CatalogLimits, LocaleTag, MessageArguments, MessageParameter, MessageValue,
    TranslationCatalog,
};
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::menu::{
    MENU_MESSAGE_KEYS, MenuArt, MenuContent, MenuText, register_menu_messages,
};
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::preferences::{LocaleStore, PreferenceError};
use games_platform::ir::ui::PlateLayout;
use games_platform::ir::ui::editor::{
    ControlEditorArt, EditorMessages, editor_message_keys,
    register_editor_messages,
};

use super::{
    MENU_HEIGHT, MENU_WIDTH, PreviewOptions, Profiles, WindowSwitching,
    failure, key_label, size, station_hud, touch,
};

macro_rules! messages {
    ($($key:literal => $en:literal, $es:literal;)*) => {
        const KEYS: &[&str] = &[$($key),*];
        const EN: &str = concat!("catalog-v1\n", $($key, "\t", $en, "\n",)*);
        const ES: &str = concat!("catalog-v1\n", $($key, "\t", $es, "\n",)*);
    };
}

pub(super) const LOCALES: [(&str, &str); 2] =
    [("en", "English"), ("es-MX", "Espa\u{f1}ol")];

messages! {
    "title.bonus.instructions" => "TAP ATTACK OR JUMP TO PUMP",
        "PULSA ATAQUE O SALTO PARA INFLAR";
    "title.bonus.ready" => "READY", "LISTOS";
    "title.bonus.pumping" => "GO - TAP TO PUMP",
        "YA - PULSA PARA INFLAR";
    "title.bonus.results" => "RESULTS - RANK / SCORE",
        "RESULTADOS - PUESTO / PUNTOS";
    "title.bonus.complete" => "BONUS COMPLETE",
        "BONO COMPLETADO";
    "title.bonus.stage_two" => "STAGE 2", "ETAPA 2";
    "title.action.pause" => "PAUSE", "PAUSA";
    "title.action.start" => "START", "INICIO";
    "title.action.primary" => "PRIMARY", "PRINCIPAL";
    "title.action.secondary" => "SECONDARY", "SECUNDARIO";
    "title.action.left" => "LEFT", "IZQUIERDA";
    "title.action.right" => "RIGHT", "DERECHA";
    "title.action.up" => "UP", "ARRIBA";
    "title.action.down" => "DOWN", "ABAJO";
    "title.menu.heading" => "THE SIMPSONS ARCADE", "THE SIMPSONS ARCADE";
    "title.menu.local.choose" => "LEFT/RIGHT CHOOSE YOUR CHARACTER",
        "IZQUIERDA/DERECHA ELIGE PERSONAJE";
    "title.menu.local.join" => "PRESS YOUR START CONTROL TO JOIN",
        "PULSA INICIO PARA UNIRTE";
    "title.menu.local.touch" => "GAMEPAD / TOUCH: USE JOIN CONTROL",
        "CONTROL / T\u{c1}CTIL: PULSA UNIRTE";
    "title.menu.host.choose" => "LOCAL LEFT/RIGHT CHOOSE CHARACTER",
        "LOCAL: IZQUIERDA/DERECHA ELIGE";
    "title.menu.host.join" => "LOCAL START OPENS HOSTED GAME",
        "INICIO LOCAL ABRE LA PARTIDA";
    "title.menu.host.remote" => "REMOTE START JOINS DURING PLAY",
        "INICIO REMOTO UNE DURANTE EL JUEGO";
    "title.menu.confirm" => "UP/DOWN SELECT - PRIMARY CONFIRM",
        "ARRIBA/ABAJO ELIGE - PRINCIPAL CONFIRMA";
    "title.settings.change" => "PRIMARY CHANGES VALUE - PAUSE RETURNS",
        "PRINCIPAL CAMBIA VALOR - PAUSA REGRESA";
    "title.settings.world" => "ADAPTIVE EXTENDS STREET WHEN IT FITS",
        "ADAPTABLE AMPL\u{cd}A CALLE SI CABE";
    "title.settings.no-world" => "NO WORLD STRIP - ADAPTIVE ASPECT FIT",
        "SIN CALLE EXTRA - AJUSTA PROPORCI\u{d3}N";
    "title.settings.smoke" => "SMOKE DOES NOT SAVE SETTINGS",
        "PRUEBA NO GUARDA AJUSTES";
    "title.settings.session" => "SETTINGS ARE SESSION ONLY",
        "AJUSTES SOLO DE ESTA SESI\u{d3}N";
    "title.settings.controls" => "CONTROL EDITS APPLY NEXT LAUNCH",
        "CONTROLES CAMBIAN AL REINICIAR";
    "title.settings.saved" => "DISPLAY, VOLUME AND LANGUAGE SAVED",
        "PANTALLA, VOLUMEN E IDIOMA GUARDADOS";
    "title.settings.locale-session" => "LANGUAGE IS SESSION ONLY",
        "IDIOMA SOLO DE ESTA SESI\u{d3}N";
    "title.binding.mouse" => "MOUSE {button}", "RAT\u{d3}N {button}";
    "title.binding.mouse.left" => "LEFT", "IZQUIERDO";
    "title.binding.mouse.right" => "RIGHT", "DERECHO";
    "title.binding.mouse.middle" => "MIDDLE", "CENTRAL";
    "title.binding.mouse.x1" => "X1", "X1";
    "title.binding.mouse.x2" => "X2", "X2";
    "title.binding.none" => "NONE", "NINGUNO";
    "title.binding.profile" => "PROFILE", "PERFIL";
    "title.pause.horizontal" => "P{player} LEFT: {left} RIGHT: {right}",
        "J{player} IZQ: {left} DER: {right}";
    "title.pause.vertical" => "UP: {up} DOWN: {down}",
        "ARRIBA: {up} ABAJO: {down}";
    "title.pause.combat" => "ATTACK: {attack} JUMP: {jump}",
        "ATAQUE: {attack} SALTO: {jump}";
    "title.pause.resume" => "{pause} OR START RESUMES - JOIN BELOW",
        "{pause} O INICIO CONTIN\u{da}A - UNIRSE ABAJO";
    "title.touch.seat" => "SEAT {seat}", "PUESTO {seat}";
    "title.controller.heading" => "REMOTE CONTROLLER", "CONTROL REMOTO";
    "title.controller.watch" =>
        "WATCH THE HOST SCREEN",
        "MIRA LA PANTALLA DEL ANFITRI\u{d3}N";
    "title.controller.leave" =>
        "CLOSE WINDOW TO LEAVE",
        "CIERRA LA VENTANA PARA SALIR";
    "title.controller.touch" =>
        "TOUCH BELOW - DRAG OFF TO RELEASE",
        "T\u{c1}CTIL ABAJO - DESLIZA PARA SOLTAR";
    "title.controller.attack" => "ATTACK", "ATAQUE";
    "title.controller.jump" => "JUMP", "SALTO";
    "title.controller.no-axis" =>
        "NO REMOTE AXIS BINDINGS",
        "SIN EJES REMOTOS ASIGNADOS";
    "title.controller.binding" =>
        "{action}: {key}{mouse} / {button}",
        "{action}: {key}{mouse} / {button}";
    "title.controller.axis" =>
        "{axis}: -{negative} +{positive}",
        "{axis}: -{negative} +{positive}";
    "title.controller.error.timeout" =>
        "LAN connection timed out",
        "Tiempo de conexi\u{f3}n LAN agotado";
    "title.controller.error.frame" =>
        "Invalid LAN frame; use matching host and controller builds",
        "Trama LAN inv\u{e1}lida; usa versiones compatibles";
    "title.controller.error.queue" =>
        "Remote input queue full",
        "Cola de control remoto llena";
    "title.controller.error.lost" =>
        "LAN connection lost",
        "Conexi\u{f3}n LAN perdida";
    "title.controller.error.transport" =>
        "LAN controller transport failed",
        "Fall\u{f3} el transporte LAN del control";
    "title.controller.error.relaunch" =>
        "{context} ({kind}); re-launch to rejoin",
        "{context} ({kind}); reinicia para volver";
    "title.controller.caption" =>
        "The Simpsons Arcade - remote controller",
        "The Simpsons Arcade - control remoto";
    "title.launch.choose" =>
        "Select the canonical assets directory: ",
        "Selecciona la carpeta can\u{f3}nica de recursos: ";
    "title.launch.ready" => "Preview ready.", "Vista previa lista.";
    "title.launch.ended" => "browser-preview: ended",
        "Termin\u{f3} la vista previa.";
    "title.launch.cancelled" => "Directory selection cancelled.",
        "Se cancel\u{f3} la selecci\u{f3}n de carpeta.";
    "title.launch.configuration" =>
        "Check the launch configuration and preferences.",
        "Revisa la configuraci\u{f3}n de inicio y las preferencias.";
    "title.launch.assets" => "Check the selected canonical assets.",
        "Revisa los recursos can\u{f3}nicos seleccionados.";
    "title.launch.controls" => "Check the control preferences.",
        "Revisa las preferencias de control.";
    "title.launch.host" => "LAN host", "Anfitri\u{f3}n LAN";
    "title.launch.connection" => "Check the LAN configuration.",
        "Revisa la configuraci\u{f3}n de la red LAN.";
    "title.launch.presentation" => "Could not open or continue the preview.",
        "No se pudo abrir o continuar la vista previa.";
    "title.launch.unsupported" => "Preview unavailable on this platform.",
        "Vista previa no disponible en esta plataforma.";
    "title.control.touch.first" => "TOUCH: ARROWS, B SOURCE, A EDIT",
        "T\u{c1}CTIL: FLECHAS, B TIPO, A EDITA";
    "title.control.touch.second" => "TOUCH: START CLEAR, PAUSE BACK",
        "T\u{c1}CTIL: INICIO BORRA, PAUSA REGRESA";
}

/// Authored launch notices; technical identifiers and details stay intact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LaunchNotice {
    Assets,
    Cancelled,
    Choose,
    Configuration,
    Connection,
    Controls,
    Ended,
    Host,
    Presentation,
    Ready,
    Unsupported,
}

impl LaunchNotice {
    const fn key(self) -> &'static str {
        match self {
            Self::Assets => "title.launch.assets",
            Self::Cancelled => "title.launch.cancelled",
            Self::Choose => "title.launch.choose",
            Self::Configuration => "title.launch.configuration",
            Self::Connection => "title.launch.connection",
            Self::Controls => "title.launch.controls",
            Self::Ended => "title.launch.ended",
            Self::Host => "title.launch.host",
            Self::Presentation => "title.launch.presentation",
            Self::Ready => "title.launch.ready",
            Self::Unsupported => "title.launch.unsupported",
        }
    }
}

#[derive(Debug)]
pub(super) struct LaunchFailure {
    pub(super) error: RenderError,
    pub(super) notice: LaunchNotice,
}

impl LaunchFailure {
    pub(super) const fn new(notice: LaunchNotice, error: RenderError) -> Self {
        Self { error, notice }
    }
}

impl From<RenderError> for LaunchFailure {
    fn from(error: RenderError) -> Self {
        Self::new(LaunchNotice::Presentation, error)
    }
}

/// Native and browser preference admission share this result.
pub(super) type LocalePreference = (Option<LocaleStore>, usize);

/// Title menu/editor atlas admission for a single runtime configuration.
#[derive(Clone, Copy)]
struct ArtOptions<'profiles> {
    preview: PreviewOptions<'profiles>,
    reserved: [bool; 4],
    world_available: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ArtLimits {
    pub(super) atlas_bytes: usize,
    pub(super) atlases: usize,
    pub(super) sprites: usize,
}

/// Resident artwork for one declared presentation language.
#[derive(Debug)]
pub(super) struct LocaleArt {
    catalog: Rc<TranslationCatalog>,
    pub(super) editor: Option<ControlEditorArt>,
    pub(super) hud: station_hud::StationHud,
    locale: LocaleTag,
    pub(super) menu: MenuArt,
    pub(super) panel: Option<SpriteDraw>,
}

impl LocaleArt {
    pub(super) fn actions(&self) -> Result<[&str; 8], RenderError> {
        editor_actions(&self.catalog, &self.locale)
    }

    fn load(
        scene: &mut SceneSession<'_, '_>,
        catalog: Rc<TranslationCatalog>,
        index: usize,
        options: ArtOptions<'_>,
    ) -> Result<Self, RenderError> {
        let locale = locale_tag(index)?;
        let text = TitleText {
            catalog: &catalog,
            locale: &locale,
        };
        let (_tag, name) =
            LOCALES.get(index).ok_or(RenderError::InvalidSprite)?;
        let menu = load_menu(scene, name, options, text)?;
        let editor = load_editor(scene, options, &catalog, &locale)?;
        let hud = station_hud::StationHud::load_localised(
            scene,
            &options.preview.controls.profiles,
            options.reserved,
            station_hud::StationText {
                catalog: &catalog,
                locale: &locale,
            },
        )?;
        let panel = if options.preview.touch_enabled
            && touch::seats(&options.preview.controls.profiles).count() > 1
        {
            Some(touch::load(
                scene,
                &options.preview.controls.profiles,
                touch::PanelRole::Cabinet,
                Some(text),
            )?)
        } else {
            None
        };
        Ok(Self {
            catalog,
            editor,
            hud,
            locale,
            menu,
            panel,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct TitleText<'text> {
    pub(super) catalog: &'text TranslationCatalog,
    pub(super) locale: &'text LocaleTag,
}

impl<'text> TitleText<'text> {
    pub(super) fn label(self, key: &str) -> Result<&'text str, RenderError> {
        self.catalog
            .resolve_tag(self.locale, key)
            .ok_or(RenderError::InvalidSprite)
    }

    fn labels<const COUNT: usize>(
        self,
        keys: [&str; COUNT],
    ) -> Result<[&'text str; COUNT], RenderError> {
        let mut labels = [""; COUNT];
        for (slot, key) in labels.iter_mut().zip(keys) {
            *slot = self.label(key)?;
        }
        Ok(labels)
    }

    pub(super) fn render(
        self,
        key: &str,
        parameters: &[MessageParameter<'_>],
    ) -> Result<String, RenderError> {
        self.catalog
            .format_message(self.locale, key, MessageArguments {
                max_utf8_bytes: size(
                    usize::from(MENU_WIDTH).saturating_mul(4),
                )?,
                parameters,
            })
            .map_err(failure)
    }
}

pub(super) fn launch_label(
    index: usize,
    notice: LaunchNotice,
) -> Result<String, RenderError> {
    let catalog = catalogue()?;
    let locale = locale_tag(index)?;
    TitleText {
        catalog: &catalog,
        locale: &locale,
    }
    .label(notice.key())
    .map(String::from)
}

// Preserve the exact developer detail, including paths and machine codes.
// A broken authored catalogue must not mask the original startup failure.
pub(super) fn launch_error(
    index: usize,
    notice: LaunchNotice,
    detail: &str,
) -> String {
    launch_label(index, notice).map_or_else(
        |_error| detail.to_owned(),
        |summary| format!("{summary}\n{detail}"),
    )
}

pub(super) fn catalogue() -> Result<Rc<TranslationCatalog>, RenderError> {
    let fallback =
        LocaleTag::parse("en").map_err(|_error| RenderError::InvalidSprite)?;
    let mut catalog = TranslationCatalog::new(fallback, CatalogLimits {
        locales: NonZeroUsize::new(LOCALES.len())
            .ok_or(RenderError::InvalidSprite)?,
        messages: size(
            LOCALES.len().saturating_mul(
                KEYS.len()
                    .saturating_add(MENU_MESSAGE_KEYS.len())
                    .saturating_add(editor_message_keys().count())
                    .saturating_add(station_hud::STATION_MESSAGE_KEYS.len()),
            ),
        )?,
        utf8_bytes: size(32_768)?,
    });
    for ((tag, _label), payload) in LOCALES.into_iter().zip([EN, ES]) {
        let locale = LocaleTag::parse(tag)
            .map_err(|_error| RenderError::InvalidSprite)?;
        let menu = register_menu_messages(&mut catalog, &locale)
            .map_err(|_error| RenderError::InvalidSprite)?;
        let editor = register_editor_messages(&mut catalog, &locale)
            .map_err(|_error| RenderError::InvalidSprite)?;
        let station =
            station_hud::register_station_messages(&mut catalog, &locale)
                .map_err(failure)?;
        catalog
            .load_locale(tag, payload.as_bytes())
            .map_err(|_error| RenderError::InvalidSprite)?;
        if !menu
            || !editor
            || !station
            || catalog
                .first_missing(&locale, station_hud::STATION_MESSAGE_KEYS)
                .is_some()
            || catalog.first_missing(&locale, KEYS).is_some()
            || catalog.first_missing(&locale, MENU_MESSAGE_KEYS).is_some()
            || editor_message_keys()
                .any(|key| !catalog.contains_exact(&locale, key))
        {
            return Err(RenderError::InvalidSprite);
        }
    }
    Ok(Rc::new(catalog))
}

/// Resolve a stored locale against the title's declared list. The title
/// selects English as index zero; unavailable requests use that default.
pub(super) fn locale_index(requested: Option<&LocaleTag>) -> usize {
    let Some(locale) = requested else {
        return 0;
    };
    locale
        .candidates()
        .find_map(|candidate| {
            LOCALES.iter().position(|(tag, _label)| *tag == candidate)
        })
        .unwrap_or(0)
}

pub(super) fn locale_tag(index: usize) -> Result<LocaleTag, RenderError> {
    let (tag, _label) = LOCALES.get(index).ok_or(RenderError::InvalidSprite)?;
    LocaleTag::parse(tag).map_err(|_error| RenderError::InvalidSprite)
}

// Display-only labels, in the same logical order as ACTION_NAMES. Never
// pass these translated values to profile parsing or persistent bindings.
fn editor_actions<'catalog>(
    catalog: &'catalog TranslationCatalog,
    locale: &LocaleTag,
) -> Result<[&'catalog str; 8], RenderError> {
    let label = |key| {
        catalog
            .resolve_tag(locale, key)
            .ok_or(RenderError::InvalidSprite)
    };
    Ok([
        label("title.action.pause")?,
        label("title.action.start")?,
        label("title.action.primary")?,
        label("title.action.secondary")?,
        label("title.action.left")?,
        label("title.action.right")?,
        label("title.action.up")?,
        label("title.action.down")?,
    ])
}

pub(super) fn art_limits(
    options: PreviewOptions<'_>,
    reserved: [bool; 4],
) -> ArtLimits {
    let reset = options.can_reset_controls(reserved);
    let menu = MenuArt::bytes_with_language(
        MENU_WIDTH,
        MENU_HEIGHT,
        reset,
        options.source_settings,
    );
    let (editor_bytes, editor_atlases, editor_sprites) = if reset {
        (
            ControlEditorArt::bytes(MENU_WIDTH, MENU_HEIGHT),
            2usize,
            ControlEditorArt::sprites(MENU_WIDTH).saturating_add(2),
        )
    } else {
        (0, 0, 0)
    };
    ArtLimits {
        atlas_bytes: menu
            .saturating_add(editor_bytes)
            .saturating_add(station_hud::atlas_bytes(reserved))
            .saturating_mul(LOCALES.len()),
        atlases: 3usize
            .saturating_add(editor_atlases)
            .saturating_mul(LOCALES.len()),
        sprites: editor_sprites.saturating_add(1),
    }
}

pub(super) fn load_art(
    scene: &mut SceneSession<'_, '_>,
    world_available: bool,
    options: PreviewOptions<'_>,
    reserved: [bool; 4],
) -> Result<Vec<LocaleArt>, RenderError> {
    let catalog = catalogue()?;
    let config = ArtOptions {
        preview: options,
        reserved,
        world_available,
    };
    (0..LOCALES.len())
        .map(|index| LocaleArt::load(scene, Rc::clone(&catalog), index, config))
        .collect()
}

/// Retain incompatible data untouched and allow transient language choices.
/// Valid but unsupported tags use the title default without writing a save.
pub(super) fn restore_locale(
    store: Option<LocaleStore>,
) -> Result<LocalePreference, PreferenceError> {
    let Some(handle) = store else {
        return Ok((None, 0));
    };
    match handle.load() {
        Ok(locale) => Ok((Some(handle), locale_index(locale.as_ref()))),
        Err(PreferenceError::InvalidPayload) => Ok((None, 0)),
        Err(error) => Err(error),
    }
}

fn plate(bytes: usize) -> Result<PlateLayout, RenderError> {
    Ok(PlateLayout {
        background: [24, 22, 30, 255],
        height: MENU_HEIGHT,
        max_bytes: size(bytes)?,
        width: MENU_WIDTH,
    })
}

fn load_editor(
    scene: &mut SceneSession<'_, '_>,
    options: ArtOptions<'_>,
    catalog: &Rc<TranslationCatalog>,
    locale: &LocaleTag,
) -> Result<Option<ControlEditorArt>, RenderError> {
    if !options.preview.can_reset_controls(options.reserved) {
        return Ok(None);
    }
    let text = TitleText { catalog, locale };
    let hints = options
        .preview
        .touch_enabled
        .then(|| {
            text.labels([
                "title.control.touch.first",
                "title.control.touch.second",
            ])
        })
        .transpose()?;
    let messages = EditorMessages::new(
        Rc::clone(catalog),
        locale.clone(),
        size(usize::from(MENU_WIDTH).saturating_mul(4))?,
    )?;
    ControlEditorArt::load_localised(
        scene,
        plate(ControlEditorArt::bytes(MENU_WIDTH, MENU_HEIGHT))?,
        hints,
        messages,
    )
    .map(Some)
}

fn load_menu(
    scene: &mut SceneSession<'_, '_>,
    name: &str,
    options: ArtOptions<'_>,
    text: TitleText<'_>,
) -> Result<MenuArt, RenderError> {
    let hints = pause_hints(
        &options.preview.controls.profiles,
        options.reserved,
        text,
    )?;
    let pause = hints.each_ref().map(String::as_str);
    let reset = options.preview.can_reset_controls(options.reserved);
    let main = text.labels(main_menu_keys(options.reserved.contains(&true)))?;
    let settings = text.labels([
        "title.settings.change",
        adaptive_key(options.world_available),
        settings_key(options.preview, reset),
    ])?;
    MenuArt::load_localised(
        scene,
        plate(MenuArt::bytes_with_language(
            MENU_WIDTH,
            MENU_HEIGHT,
            reset,
            options.preview.source_settings,
        ))?,
        MenuContent {
            control_reset: reset,
            main_hints: &main,
            pause_hints: &pause,
            settings_hints: &settings,
            sources: options.preview.source_settings,
            title: text.label("title.menu.heading")?,
            window_switching: options.preview.window_switching
                == WindowSwitching::Supported,
        },
        MenuText {
            catalog: text.catalog,
            language_name: Some(name),
            locale: text.locale,
        },
    )
}

const fn adaptive_key(world_available: bool) -> &'static str {
    if world_available {
        "title.settings.world"
    } else {
        "title.settings.no-world"
    }
}

const fn settings_key(
    options: PreviewOptions<'_>,
    editor: bool,
) -> &'static str {
    if options.hidden {
        "title.settings.smoke"
    } else if options.preferences.is_none() {
        "title.settings.session"
    } else if options.locale_store.is_none() {
        "title.settings.locale-session"
    } else if editor {
        "title.settings.controls"
    } else {
        "title.settings.saved"
    }
}

const fn main_menu_keys(hosted: bool) -> [&'static str; 4] {
    if hosted {
        [
            "title.menu.host.choose",
            "title.menu.host.join",
            "title.menu.host.remote",
            "title.menu.confirm",
        ]
    } else {
        [
            "title.menu.local.choose",
            "title.menu.local.join",
            "title.menu.local.touch",
            "title.menu.confirm",
        ]
    }
}

pub(super) const fn mouse_key(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Left => "title.binding.mouse.left",
        PointerButton::Middle => "title.binding.mouse.middle",
        PointerButton::Right => "title.binding.mouse.right",
        PointerButton::X1 => "title.binding.mouse.x1",
        PointerButton::X2 => "title.binding.mouse.x2",
    }
}

pub(super) const fn parameter<'text>(
    name: &'text str,
    value: &'text str,
) -> MessageParameter<'text> {
    MessageParameter {
        name,
        value: MessageValue::Text(value),
    }
}

fn desktop_hint(
    key: Option<KeyboardKey>,
    mouse: Option<PointerButton>,
    text: TitleText<'_>,
) -> Result<String, RenderError> {
    let keyboard = key
        .map(|code| {
            let label = key_label(code);
            // Two long key and mouse combinations must fit a single pause line.
            if label.len() > 13 {
                text.label("title.binding.profile")
            } else {
                Ok(label)
            }
        })
        .transpose()?;
    let pointer = mouse
        .map(|button| {
            text.render("title.binding.mouse", &[parameter(
                "button",
                text.label(mouse_key(button))?,
            )])
        })
        .transpose()?;
    match (keyboard, pointer) {
        (Some(label), Some(button)) => Ok(format!("{label}/{button}")),
        (None, Some(button)) => Ok(button),
        (Some(label), None) => Ok(String::from(label)),
        (None, None) => text.label("title.binding.none").map(String::from),
    }
}

fn pause_hints(
    profiles: &Profiles,
    reserved: [bool; 4],
    text: TitleText<'_>,
) -> Result<[String; 4], RenderError> {
    let index = reserved.iter().position(|remote| !remote).unwrap_or(0);
    let [pause, ..] = profiles.keys.first().copied().unwrap_or([None; 8]);
    let [_, _, primary, secondary, left, right, up, down] =
        profiles.keys.get(index).copied().unwrap_or([None; 8]);
    let [mouse_pause, _, attack, jump, m_left, m_right, m_up, m_down] =
        profiles.mouse.get(index).copied().unwrap_or([None; 8]);
    Ok([
        text.render("title.pause.horizontal", &[
            MessageParameter {
                name: "player",
                value: MessageValue::Unsigned(
                    u64::try_from(index.saturating_add(1)).map_err(failure)?,
                ),
            },
            parameter("left", &desktop_hint(left, m_left, text)?),
            parameter("right", &desktop_hint(right, m_right, text)?),
        ])?,
        text.render("title.pause.vertical", &[
            parameter("up", &desktop_hint(up, m_up, text)?),
            parameter("down", &desktop_hint(down, m_down, text)?),
        ])?,
        text.render("title.pause.combat", &[
            parameter("attack", &desktop_hint(primary, attack, text)?),
            parameter("jump", &desktop_hint(secondary, jump, text)?),
        ])?,
        text.render("title.pause.resume", &[parameter(
            "pause",
            &desktop_hint(pause, mouse_pause, text)?,
        )])?,
    ])
}

#[cfg(test)]
mod tests {
    use games::audio::{MasterVolume, SourceSettings};
    use games::presentation::DisplaySettings;
    use games_platform::ir::storage::fixtures::TestRoot;
    use games_platform::ir::storage::preferences::{
        DisplayStore, PublicationPolicy,
    };
    use games_platform::ir::ui::{TextLine, first_missing_glyph};

    use super::super::{DEFAULT_PROFILES, launch, save_locale};
    use super::*;

    fn require(condition: bool) -> Result<(), RenderError> {
        condition.then_some(()).ok_or(RenderError::InvalidSprite)
    }

    #[test]
    fn title_locale_selection_uses_exact_or_admitted_parent() {
        for (tag, expected) in [
            ("en", 0usize),
            ("en-US", 0usize),
            ("es-MX", 1usize),
            ("es-MX-1996", 1usize),
            ("es-ES", 0usize),
            ("ja", 0usize),
        ] {
            let parsed = LocaleTag::parse(tag);
            assert!(parsed.is_ok());
            assert_eq!(locale_index(parsed.as_ref().ok()), expected);
        }
        assert_eq!(locale_index(None), 0usize);
    }

    #[test]
    fn declared_message_templates_have_compact_font_coverage()
    -> Result<(), RenderError> {
        let catalog = catalogue()?;
        for (index, (_tag, name)) in LOCALES.iter().enumerate() {
            let locale = locale_tag(index)?;
            let text = TitleText {
                catalog: &catalog,
                locale: &locale,
            };
            require(first_missing_glyph(name).is_none())?;
            let keys = KEYS
                .iter()
                .chain(MENU_MESSAGE_KEYS)
                .chain(station_hud::STATION_MESSAGE_KEYS)
                .copied()
                .chain(editor_message_keys());
            for key in keys {
                require(first_missing_glyph(text.label(key)?).is_none())?;
            }
        }
        Ok(())
    }

    #[test]
    fn every_declared_locale_has_shared_and_title_menu_messages()
    -> Result<(), RenderError> {
        let catalog = catalogue()?;
        let en = locale_tag(0)?;
        let es = locale_tag(1)?;
        require(
            editor_actions(&catalog, &en)?
                == [
                    "PAUSE",
                    "START",
                    "PRIMARY",
                    "SECONDARY",
                    "LEFT",
                    "RIGHT",
                    "UP",
                    "DOWN",
                ],
        )?;
        require(
            editor_actions(&catalog, &es)?
                == [
                    "PAUSA",
                    "INICIO",
                    "PRINCIPAL",
                    "SECUNDARIO",
                    "IZQUIERDA",
                    "DERECHA",
                    "ARRIBA",
                    "ABAJO",
                ],
        )
    }

    #[test]
    fn menu_hints_follow_actual_launch_capabilities() -> Result<(), RenderError>
    {
        let catalog = catalogue()?;
        let en = locale_tag(0)?;
        let text = TitleText {
            catalog: &catalog,
            locale: &en,
        };
        require(text.label(adaptive_key(true))?.contains("EXTENDS STREET"))?;
        require(text.label(adaptive_key(false))?.contains("ASPECT FIT"))?;
        let hosted = text.labels(main_menu_keys(true))?;
        require(hosted.contains(&"LOCAL START OPENS HOSTED GAME"))?;
        require(hosted.contains(&"REMOTE START JOINS DURING PLAY"))?;
        let local = text.labels(main_menu_keys(false))?;
        require(local.contains(&"PRESS YOUR START CONTROL TO JOIN"))?;
        require(!local.iter().any(|hint| hint.contains("REMOTE")))
    }

    #[test]
    fn settings_hint_follows_persistence_availability()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let display =
            Rc::new(DisplayStore::open(root.location()).map_err(failure)?);
        let locale =
            Rc::new(LocaleStore::open(root.location()).map_err(failure)?);
        let controls = Rc::new(launch::ResolvedControls::transient());
        let mut options = PreviewOptions {
            audio_device: None,
            controls: &controls,
            display: DisplaySettings::default(),
            hidden: true,
            locale_index: 0,
            locale_store: Some(&locale),
            measure: false,
            preferences: Some(&display),
            publication: PublicationPolicy::Durable,
            source_settings: SourceSettings::default(),
            touch_enabled: false,
            volume: MasterVolume::Full,
            volume_store: None,
            window_switching: WindowSwitching::Supported,
        };
        require(settings_key(options, false) == "title.settings.smoke")?;
        options.hidden = false;
        require(settings_key(options, true) == "title.settings.controls")?;
        require(settings_key(options, false) == "title.settings.saved")?;
        options.locale_store = None;
        require(
            settings_key(options, false) == "title.settings.locale-session",
        )?;
        options.preferences = None;
        require(settings_key(options, false) == "title.settings.session")
    }

    #[test]
    fn pause_hints_include_mouse_only_and_combined_desktop_bindings()
    -> Result<(), RenderError> {
        let catalog = catalogue()?;
        let en = locale_tag(0)?;
        let es = locale_tag(1)?;
        let mut profiles = DEFAULT_PROFILES;
        let [_, keys, ..] = &mut profiles.keys;
        let [_, mouse, ..] = &mut profiles.mouse;
        let [_, _, _, jump, left, ..] = keys;
        *jump = None;
        *left = None;
        *mouse = [
            Some(PointerButton::X1),
            None,
            None,
            Some(PointerButton::Middle),
            Some(PointerButton::Right),
            Some(PointerButton::X2),
            None,
            None,
        ];
        let [first, ..] = &mut profiles.mouse;
        *first = [None; 8];
        let reserved = [true, false, false, false];
        let english = TitleText {
            catalog: &catalog,
            locale: &en,
        };
        let [horizontal, vertical, combat, resume] =
            pause_hints(&profiles, reserved, english)?;
        require(horizontal == "P2 LEFT: MOUSE RIGHT RIGHT: Right/MOUSE X2")?;
        require(vertical == "UP: Up DOWN: Down")?;
        require(combat == "ATTACK: F JUMP: MOUSE MIDDLE")?;
        require(
            resume.contains("MOUSE X1") && resume.contains("START RESUMES"),
        )?;
        let spanish = TitleText {
            catalog: &catalog,
            locale: &es,
        };
        let [
            spanish_horizontal,
            spanish_vertical,
            spanish_combat,
            spanish_resume,
        ] = pause_hints(&profiles, reserved, spanish)?;
        require(
            spanish_horizontal
                == "J2 IZQ: RAT\u{d3}N DERECHO DER: Right/RAT\u{d3}N X2",
        )?;
        require(spanish_vertical == "ARRIBA: Up ABAJO: Down")?;
        require(spanish_combat == "ATAQUE: F SALTO: RAT\u{d3}N CENTRAL")?;
        require(
            spanish_resume.contains("RAT\u{d3}N X1")
                && spanish_resume.contains("INICIO CONTIN\u{da}A"),
        )
    }

    #[test]
    fn pause_hints_fit_extended_keyboard_and_mouse_labels()
    -> Result<(), RenderError> {
        let catalog = catalogue()?;
        for index in 0..LOCALES.len() {
            let locale = locale_tag(index)?;
            let text = TitleText {
                catalog: &catalog,
                locale: &locale,
            };
            let binding = desktop_hint(
                Some(KeyboardKey::from_code(100)),
                Some(PointerButton::Middle),
                text,
            )?;
            require(binding.contains("IntlBackslash/"))?;
            let hint = text.render("title.pause.horizontal", &[
                MessageParameter {
                    name: "player",
                    value: MessageValue::Unsigned(4),
                },
                parameter("left", &binding),
                parameter("right", &binding),
            ])?;
            require(
                TextLine {
                    color: [255; 4],
                    scale: 2,
                    text: &hint,
                    top: 0,
                }
                .fit_width(MENU_WIDTH)
                .scale
                    > 0,
            )?;
        }
        Ok(())
    }

    #[test]
    fn locale_preferences_fallback_without_rewriting_valid_unknown_tag()
    -> Result<(), RenderError> {
        let root = TestRoot::new().map_err(failure)?;
        let open = || LocaleStore::open(root.location()).map_err(failure);
        let (_, initial) = restore_locale(Some(open()?)).map_err(failure)?;
        require(initial == 0)?;
        let unknown = LocaleTag::parse("ja").map_err(failure)?;
        let receipt = open()?.store(&unknown).map_err(failure)?;
        PublicationPolicy::Durable
            .confirm(receipt)
            .map_err(failure)?;
        let (store, fallback) =
            restore_locale(Some(open()?)).map_err(failure)?;
        require(fallback == 0)?;
        require(open()?.load().map_err(failure)? == Some(unknown))?;
        save_locale(store.as_ref(), 1, PublicationPolicy::Durable)?;
        let (_, restored) = restore_locale(Some(open()?)).map_err(failure)?;
        require(restored == 1)
    }
}
