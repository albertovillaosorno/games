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

use std::num::NonZeroU32;
use std::rc::Rc;

use games::audio::MasterVolume;
use games::presentation::DisplaySettings;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::{NativeWindow, frame_loop};
use games_platform::ir::storage::bundle::{BundleSource, MemoryBundleStore};
use games_platform::ir::storage::preferences::{
    DisplayStore, PublicationPolicy, VolumeStore,
};

pub use super::audio::AudioSelection;
use super::{
    Interactive, OptionalSources, PreviewOptions, Scene, Sources,
    WindowSwitching, audio, dimension, launch, poses, preview_layout,
    scene_limits, size, world,
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
            publication: PublicationPolicy::Durable,
            volume: MasterVolume::Full,
            volume_store: None,
        }
    }
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
        complete,
    )
}

pub(super) fn start_with_preferences<Complete>(
    store: &MemoryBundleStore,
    selection: &Selection<'_>,
    preferences: &Preferences,
    complete: Complete,
) -> Result<(), RenderError>
where
    Complete: FnOnce(Result<(), RenderError>) + 'static,
{
    let source = Sources::load(
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
    )?;
    let options = preview_options(selection, preferences, &source)?;
    let reserved = [false; 4];
    let layout =
        preview_layout(options.touch_enabled, &preferences.controls.profiles)?;
    let limits = scene_limits(&source, options, reserved)?;
    let mut window = NativeWindow::open(
        "The Simpsons Arcade - movement prototype",
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
    scene.protect_bottom(NonZeroU32::new(
        layout.height().get().saturating_sub(world::BASE_HEIGHT),
    ))?;
    let mut art = Scene::load(&mut scene, &source, options, reserved)?;
    let mut preview = Interactive::for_scene(startup, options, &art, None)?;
    let mut playback = audio::open_optional(
        source.audio.as_ref(),
        None,
        options.source_settings,
        options.volume,
    )?;
    if let Some(output) = &mut playback {
        output.enable_output();
    }
    frame_loop::start(
        move |_timestamp| {
            preview.step(&mut scene, (&mut art, playback.as_mut()), 0)
        },
        complete,
    )
}
