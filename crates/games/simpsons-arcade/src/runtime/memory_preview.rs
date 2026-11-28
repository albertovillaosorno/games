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
//   - Path-independent event-driven title launch.
// - Must-Not:
//   - Open native or browser APIs, discover or interpret original media.
// - Allows:
//   - Inputs: Pinned canonical memory bundles and explicit launch options.
//   - Outputs: Live title frames and one terminal completion result.
//   - Side effects: Delegated window, input and callback registration.
// - Split-When:
//   - Independent preference admission is required.
// - Merge-When:
//   - Native and event-driven launch can share the same ownership contract.
// - Summary:
//   - Launch the existing title runtime from admitted memory assets.
// - Description:
//   - Keep the scene and fixed-step input state across host callbacks.
// - Usage:
//   - Select pinned canonical manifest hashes and an explicit memory store.
// - Defaults:
//   - No local paths, persistence or network access.

//! Event-driven title preview from a caller-provided memory snapshot.

use std::cell::Cell;
use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::rc::Rc;

use games::audio::MasterVolume;
use games::presentation::{DisplaySettings, LocaleTag, PresentationMode};
use games::session::LocalSession;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::graphics::{NativeWindow, frame_loop};
use games_platform::ir::input::InputStartup;
use games_platform::ir::storage::bundle::{BundleSource, MemoryBundleStore};
use games_platform::ir::storage::preferences::{
    ControlStore, DisplayStore, LocaleStore, PublicationPolicy, VolumeStore,
};

pub use super::audio::AudioSelection;
use super::title_messages::{LaunchFailure, LaunchNotice};
use super::{
    Controls, Interactive, OptionalSources, PreviewOptions, Scene, Sources,
    WindowSwitching, audio, backdrop, dimension, encounter, failure, launch,
    live_with_session, poses, preview_layout, profile_limit, scene_limits,
    scenery, size, smoke_language_menu, title_messages, world,
};

/// Explicit selection for the local event-driven preview.
#[derive(Clone, Copy, Debug)]
pub struct Selection<'text> {
    /// Four actor manifests in cabinet order.
    pub actors: [ContentHash; 4],
    /// Optional pinned attack samples.
    pub audio: Option<AudioSelection>,
    /// Initial presentation policy.
    pub display: DisplaySettings,
    /// Optional enemy approach and strike manifests.
    pub enemy: Option<[ContentHash; 2]>,
    /// Optional bounded pose catalogue text.
    pub poses: Option<&'text str>,
    /// Optional actor shadow manifest.
    pub shadow: Option<ContentHash>,
    /// Enable the existing local touch panel.
    pub touch_enabled: bool,
    /// Optional scrolling world manifest.
    pub world: Option<ContentHash>,
}

#[derive(Debug)]
pub(super) struct Preferences {
    pub(super) controls: Rc<launch::ResolvedControls>,
    pub(super) display: DisplaySettings,
    pub(super) display_store: Option<Rc<DisplayStore>>,
    pub(super) locale_index: usize,
    pub(super) locale_store: Option<Rc<LocaleStore>>,
    pub(super) publication: PublicationPolicy,
    pub(super) volume: MasterVolume,
    pub(super) volume_store: Option<Rc<VolumeStore>>,
}

impl Preferences {
    pub(super) fn transient(display: DisplaySettings) -> Self {
        Self {
            controls: Rc::new(launch::ResolvedControls::transient()),
            display,
            display_store: None,
            locale_index: 0,
            locale_store: None,
            publication: PublicationPolicy::Durable,
            volume: MasterVolume::Full,
            volume_store: None,
        }
    }
}

/// Select the existing scene lifetime contract for a finite diagnostic.
#[derive(Clone, Copy, Debug)]
pub enum DiagnosticScene {
    /// Synchronous borrowed rendering session.
    Borrowed,
    /// Owned event-driven rendering session.
    Resident,
}

struct Resident {
    art: Scene,
    scene: SceneSession<'static, 'static>,
    startup: InputStartup,
}

fn preview_options<'preferences>(
    selection: &Selection<'_>,
    preferences: &'preferences Preferences,
    source: &Sources<'_>,
) -> Result<PreviewOptions<'preferences>, RenderError> {
    Ok(PreviewOptions {
        audio_device: None,
        controls: &preferences.controls,
        display: preferences.display,
        hidden: false,
        locale_index: preferences.locale_index,
        locale_store: preferences.locale_store.as_ref(),
        measure: false,
        preferences: preferences.display_store.as_ref(),
        publication: preferences.publication,
        source_settings: audio::source_settings(
            source.audio.as_ref(),
            preferences.volume_store.as_deref(),
        )?,
        touch_enabled: selection.touch_enabled,
        volume: preferences.volume,
        volume_store: preferences.volume_store.as_ref(),
        window_switching: WindowSwitching::Fixed,
    })
}

/// Open the live title and yield to the host between bounded frame steps.
///
/// The memory snapshot is needed only during initialization. The same title
/// art, simulation, controls and session flow as native preview are used.
/// The host owns the frame schedule; input timestamps drive the fixed clock.
/// Completion follows the frame scheduler's ownership and error contract.
///
/// # Errors
/// Rejects invalid assets, unsupported capabilities or startup failures.
pub fn start<Complete>(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    complete: Complete,
) -> Result<(), RenderError>
where
    Complete: FnOnce(Result<(), RenderError>) + 'static,
{
    start_with_preferences(
        store,
        selection,
        &Preferences::transient(selection.display),
        move |_locale, result| complete(result),
    )
    .map_err(|failure| failure.error)
}

pub(super) fn start_with_preferences<Complete>(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    preferences: &Preferences,
    complete: Complete,
) -> Result<(), LaunchFailure>
where
    Complete: FnOnce(usize, Result<(), RenderError>) + 'static,
{
    let source = sources(store, selection)
        .map_err(|error| LaunchFailure::new(LaunchNotice::Assets, error))?;
    let options = preview_options(selection, preferences, &source)?;
    let Resident {
        mut art,
        mut scene,
        startup,
    } = open_scene(&source, options)?;
    let mut preview =
        Box::new(Interactive::for_scene(startup, options, &art, None)?);
    let mut playback = audio::open_optional(
        source.audio.as_ref(),
        None,
        options.source_settings,
        options.volume,
    )?;
    if let Some(output) = &mut playback {
        output.enable_output();
    }
    let active = Rc::new(Cell::new(preferences.locale_index));
    let selected = Rc::clone(&active);
    frame_loop::start(
        move |_timestamp| {
            let result =
                preview.step(&mut scene, (&mut art, playback.as_mut()), 0);
            active.set(preview.locale_index);
            result
        },
        move |result| complete(selected.get(), result),
    )
    .map_err(LaunchFailure::from)
}

fn sources<'source>(
    store: &'source MemoryBundleStore,
    selection: &Selection<'_>,
) -> Result<Sources<'source>, RenderError> {
    Sources::load(
        BundleSource::Memory(store),
        selection.actors,
        OptionalSources {
            audio: selection.audio,
            enemy: selection.enemy,
            poses: selection
                .poses
                .map(poses::Catalogue::parse)
                .transpose()?
                .unwrap_or_default(),
            shadow: selection.shadow,
            world: selection.world,
        },
    )
}

fn open_scene(
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
) -> Result<Resident, RenderError> {
    let reserved = [false; 4];
    let layout =
        preview_layout(options.touch_enabled, &options.controls.profiles)?;
    let limits = scene_limits(source, options, reserved)?;
    let mut window = NativeWindow::open(
        "The Simpsons Arcade",
        dimension(960)?,
        dimension(if options.touch_enabled {
            912
        } else {
            672
        })?,
        false,
    )?;
    window.apply_display_settings(options.display)?;
    let startup = window.input(size(reserved.len())?)?;
    let mut scene = window.into_scene(layout, limits)?;
    let art = prepare_art(&mut scene, source, options)?;
    Ok(Resident { art, scene, startup })
}

pub(super) fn verify_scenery(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    preferences: &Preferences,
) -> Result<(), RenderError> {
    let source = sources(store, selection)?;
    if !source.poses.has_scenery() && source.poses.backdrop_count() == 0 {
        return Err(failure("scenery smoke requires scenic or backdrop poses"));
    }
    let options = preview_options(selection, preferences, &source)?;
    let Resident { mut art, mut scene, .. } = open_scene(&source, options)?;
    scenery::smoke(&mut scene, &mut art)?;
    backdrop::smoke(&mut scene, &mut art, &source)?;
    if source.poses.stage_two_strip().is_some() {
        encounter::smoke_wrestler(
            &mut scene,
            &mut art,
            selection.touch_enabled,
        )?;
    }
    Ok(())
}

pub(super) fn verify_soundtrack<Complete>(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    preferences: &Preferences,
    complete: Complete,
) -> Result<(), RenderError>
where
    Complete: FnOnce(Result<(), RenderError>) + 'static,
{
    if selection
        .audio
        .is_none_or(|audio| audio.stage_two_music.is_none())
    {
        return Err(failure("soundtrack smoke requires second-stage audio"));
    }
    let source = sources(store, selection)?;
    let options = preview_options(selection, preferences, &source)?;
    let mut smoke = audio::open_optional(
        source.audio.as_ref(),
        None,
        options.source_settings,
        options.volume,
    )?
    .ok_or_else(|| failure("soundtrack smoke requires audio"))?
    .soundtrack_smoke()?;
    frame_loop::start(move |timestamp| smoke.step(timestamp), complete)
}

/// Exercise locale switching, renderer readbacks and isolated persistence.
///
/// The supplied location must be reserved for diagnostics. A valid existing
/// locale and control profile at that location are replaced by the fixture.
/// # Errors
/// Rejects unavailable rendering or storage, unexpected fallback, changed
/// unrelated settings, missing visible changes, or failed restoration.
pub fn verify_localisation(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    (location, publication): (&OsStr, PublicationPolicy),
    ownership: DiagnosticScene,
) -> Result<(), RenderError> {
    let preferences =
        localisation_preferences(location, selection.display, publication)?;
    let source = sources(store, selection)?;
    let options = preview_options(selection, &preferences, &source)?;
    match ownership {
        DiagnosticScene::Borrowed => {
            verify_borrowed(&source, options, location)
        },
        DiagnosticScene::Resident => {
            let Resident {
                mut art,
                mut scene,
                startup,
            } = open_scene(&source, options)?;
            verify_localised_scene(
                &mut scene,
                &mut art,
                startup,
                (options, location),
            )
        },
    }
}

fn verify_localised_scene(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    startup: InputStartup,
    (options, location): (PreviewOptions<'_>, &OsStr),
) -> Result<(), RenderError> {
    let mut preview = Interactive::for_scene(startup, options, art, None)?;
    let changed = verify_language_mode(scene, art, options)?;
    preview.persist_preferences(changed)?;
    let reopened = LocaleStore::open(location).map_err(failure)?;
    let (handle, index) =
        title_messages::restore_locale(Some(reopened)).map_err(failure)?;
    if handle.is_none() || index != changed.language_index() {
        return Err(failure("selected locale did not survive reopening"));
    }
    for mode in [
        PresentationMode::Adaptive,
        PresentationMode::AspectFit,
        PresentationMode::Fill,
        PresentationMode::Integer,
        PresentationMode::Stretch,
    ] {
        let _selected = verify_language_mode(scene, art, PreviewOptions {
            display: DisplaySettings {
                presentation: mode,
                ..options.display
            },
            locale_index: index,
            ..options
        })?;
    }
    Ok(())
}

fn localisation_preferences(
    location: &OsStr,
    display: DisplaySettings,
    publication: PublicationPolicy,
) -> Result<Preferences, RenderError> {
    let store = LocaleStore::open(location).map_err(failure)?;
    let unsupported = LocaleTag::parse("ja").map_err(failure)?;
    publication
        .confirm(store.store(&unsupported).map_err(failure)?)
        .map_err(failure)?;
    let (handle, index) =
        title_messages::restore_locale(Some(store)).map_err(failure)?;
    let admitted = handle.ok_or_else(|| failure("locale store unavailable"))?;
    if index != 0 || admitted.load().map_err(failure)? != Some(unsupported) {
        return Err(failure("locale fallback modified stored preference"));
    }
    let control_store =
        ControlStore::open(location, profile_limit().map_err(failure)?)
            .map_err(failure)?;
    publication
        .confirm(
            control_store
                .store("version=1\ntouch-players=1,3\n")
                .map_err(failure)?,
        )
        .map_err(failure)?;
    let controls =
        launch::ResolvedControls::from_store(control_store, publication)
            .map_err(failure)?;
    Ok(Preferences {
        controls: Rc::new(controls),
        display,
        display_store: Some(Rc::new(
            DisplayStore::open(location).map_err(failure)?,
        )),
        locale_index: index,
        locale_store: Some(Rc::new(admitted)),
        publication,
        volume: MasterVolume::Full,
        volume_store: Some(Rc::new(
            VolumeStore::open(location).map_err(failure)?,
        )),
    })
}

fn verify_language_mode(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    options: PreviewOptions<'_>,
) -> Result<LocalSession, RenderError> {
    let session = options.session([false; 4]);
    scene.apply_display_settings(session.effective_display())?;
    let mut simulation = live_with_session(false, session)?;
    let mut controls = Controls::with_smoke_profiles(
        options.touch_enabled,
        &options.controls.profiles,
    )?;
    smoke_language_menu(scene, art, &mut controls, &mut simulation)
}

fn prepare_art(
    scene: &mut SceneSession<'_, '_>,
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
) -> Result<Scene, RenderError> {
    scene.protect_bottom(NonZeroU32::new(
        scene
            .layout()
            .height()
            .get()
            .saturating_sub(world::BASE_HEIGHT),
    ))?;
    Scene::load(scene, source, options, [false; 4])
}

fn verify_borrowed(
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
    location: &OsStr,
) -> Result<(), RenderError> {
    let reserved = [false; 4];
    let layout =
        preview_layout(options.touch_enabled, &options.controls.profiles)?;
    let limits = scene_limits(source, options, reserved)?;
    let mut window = NativeWindow::open(
        "The Simpsons Arcade - localisation diagnostic",
        dimension(960)?,
        dimension(672)?,
        true,
    )?;
    window.apply_display_settings(options.display)?;
    let startup = window.input(size(reserved.len())?)?;
    window.with_scene(layout, limits, |scene| {
        let mut art = prepare_art(scene, source, options)?;
        verify_localised_scene(scene, &mut art, startup, (options, location))
    })
}
