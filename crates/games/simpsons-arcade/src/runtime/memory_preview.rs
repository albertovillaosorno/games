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
//   - Independently owned audio or preference admission is required.
// - Merge-When:
//   - Native and event-driven launch can share the same ownership contract.
// - Summary:
//   - Launch the existing title runtime from admitted memory assets.
// - Description:
//   - Keep the scene and fixed-step input state across host callbacks.
// - Usage:
//   - Select pinned canonical manifest hashes and an explicit memory store.
// - Defaults:
//   - No local paths, audio, persistence or network access.

//! Event-driven title preview from a caller-provided memory snapshot.

use std::num::NonZeroU32;
use std::rc::Rc;

use games::audio::MasterVolume;
use games::presentation::DisplaySettings;
use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::{NativeWindow, frame_loop};
use games_platform::ir::storage::bundle::{BundleSource, MemoryBundleStore};

use super::{
    Interactive, OptionalSources, PreviewOptions, Scene, Sources,
    WindowSwitching, dimension, launch, poses, preview_layout, scene_limits,
    size, world,
};

/// Explicit selection for the silent local event-driven preview.
#[derive(Clone, Copy, Debug)]
pub struct Selection<'text> {
    /// Four actor manifests in cabinet order.
    pub actors: [ContentHash; 4],
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
    let source = Sources::load(
        BundleSource::Memory(store),
        selection.actors,
        OptionalSources {
            audio: None,
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
    let controls = Rc::new(launch::ResolvedControls::transient());
    let options = PreviewOptions {
        audio_device: None,
        controls: &controls,
        display: selection.display,
        hidden: false,
        measure: false,
        preferences: None,
        touch_enabled: selection.touch_enabled,
        volume: MasterVolume::Full,
        volume_store: None,
        window_switching: WindowSwitching::Fixed,
    };
    let reserved = [false; 4];
    let layout = preview_layout(options.touch_enabled, &controls.profiles)?;
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
    frame_loop::start(
        move |_timestamp| preview.step(&mut scene, (&mut art, None), 0),
        complete,
    )
}
