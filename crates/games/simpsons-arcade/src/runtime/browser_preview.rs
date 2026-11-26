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
//   - Explicit canonical directory selection for the browser title.
// - Must-Not:
//   - Resolve host paths, discover media or implement title behavior.
// - Allows:
//   - Inputs: A selected canonical directory and local launch catalogue.
//   - Outputs: A live title preview and one terminal completion outcome.
//   - Side effects: Delegated picker, asset transfer and title presentation.
// - Split-When:
//   - Other launch mechanisms require independent selection semantics.
// - Merge-When:
//   - An existing title launcher accepts the same directory selection.
// - Summary:
//   - Open the locally prepared title without sending assets to a server.
// - Description:
//   - Reuse the native catalogue parser and the canonical memory loader.
// - Usage:
//   - Select the migrated assets directory containing launch.local.
// - Defaults:
//   - Windowed presentation, transient controls and no remote access.

//! Browser title launch from a user-selected canonical directory.

use std::num::{NonZeroU64, NonZeroUsize};
use std::str;

use games::presentation::WindowMode;
use games_platform::ir::frame::RenderError;
use games_platform::ir::storage::bundle::MemoryBundleLimits;
use games_platform::ir::storage::bundle::directory::{
    DirectoryLimits, SelectedDirectory, select,
};

use super::launch::{AudioSelection as CatalogueAudio, Catalogue};
use super::memory_preview::{AudioSelection, Selection, start as start_memory};
use super::{failure, parse_hash, parse_hashes};

const CATALOGUE: &str = "launch.local";
const POSES: &str = "launch-poses.txt";

type Completion = fn(Result<(), RenderError>);

/// Present the browser directory picker, then start the selected title.
///
/// The selected root must contain launch.local. Native path resolution and
/// persistent preferences are not available in this launcher. Asset bytes
/// are held only for initialization and the live title owns the loaded art.
/// # Errors
/// Rejects unavailable selection, invalid catalogue or startup failure.
pub fn start(
    touch_enabled: bool,
    complete: Completion,
) -> Result<(), RenderError> {
    select(limits(), &[CATALOGUE, POSES], move |result| {
        let opened = result.map_err(failure).and_then(|directory| {
            start_selected(&directory, touch_enabled, complete)
        });
        if let Err(error) = opened {
            complete(Err(error));
        }
    })
    .map_err(failure)
}

const fn limits() -> DirectoryLimits {
    DirectoryLimits {
        bundles: MemoryBundleLimits {
            bytes: NonZeroU64::MIN.saturating_add(134_217_727),
            manifests: NonZeroUsize::MIN.saturating_add(1023),
            objects: NonZeroUsize::MIN.saturating_add(4095),
        },
        files: NonZeroUsize::MIN.saturating_add(8191),
        manifest: NonZeroU64::MIN.saturating_add(262_143),
        object: NonZeroU64::MIN.saturating_add(16_777_215),
        sidecar: NonZeroU64::MIN.saturating_add(18_431),
    }
}

fn audio_selection(
    selection: &CatalogueAudio,
) -> Result<AudioSelection, RenderError> {
    Ok(AudioSelection {
        base: parse_hash(&selection.base)?,
        replacement: selection
            .replacement
            .as_deref()
            .map(parse_hash)
            .transpose()?,
    })
}

fn selection<'text>(
    catalogue: &Catalogue<'_>,
    poses: Option<&'text str>,
    touch_enabled: bool,
) -> Result<Selection<'text>, RenderError> {
    if catalogue.parent != "." {
        return Err(failure("browser catalogue parent must be ."));
    }
    if catalogue.controls.is_some() || catalogue.settings.is_some() {
        return Err(failure("browser cannot resolve native preferences"));
    }
    if catalogue.audio_device.is_some() {
        return Err(failure("browser cannot select a native audio device"));
    }
    if catalogue.display.window != WindowMode::Windowed {
        return Err(failure("browser requires windowed presentation"));
    }
    let pose_text = match (catalogue.poses, poses) {
        (None, _) => None,
        (Some(POSES), Some(text)) => Some(text),
        (Some(POSES), None) => {
            return Err(failure("missing browser pose catalogue"));
        },
        (Some(_), _) => {
            return Err(failure("browser poses must use launch-poses.txt"));
        },
    };
    Ok(Selection {
        actors: parse_hashes(&catalogue.actors)?,
        audio: catalogue.audio.as_ref().map(audio_selection).transpose()?,
        display: catalogue.display,
        enemy: catalogue.enemy.as_ref().map(parse_hashes).transpose()?,
        poses: pose_text,
        shadow: catalogue.shadow.as_deref().map(parse_hash).transpose()?,
        touch_enabled,
        world: catalogue.world.as_deref().map(parse_hash).transpose()?,
    })
}

fn start_selected(
    directory: &SelectedDirectory,
    touch_enabled: bool,
    complete: Completion,
) -> Result<(), RenderError> {
    let bytes = directory
        .sidecar(CATALOGUE)
        .ok_or_else(|| failure("missing launch.local"))?;
    let catalogue = Catalogue::parse(str::from_utf8(bytes).map_err(failure)?)
        .map_err(failure)?;
    let pose_text = catalogue
        .poses
        .and_then(|name| directory.sidecar(name))
        .map(str::from_utf8)
        .transpose()
        .map_err(failure)?;
    let options = selection(&catalogue, pose_text, touch_enabled)?;
    start_memory(directory.bundles(), &options, complete)
}

#[cfg(test)]
mod tests {
    use games::presentation::PresentationMode;
    use games_assets::ContentHash;

    use super::*;

    fn catalogue(parent: &str, extra: &str) -> String {
        let hash = ContentHash::calculate(b"fixture");
        format!(
            "version=1\nparent={parent}\nmarge={hash}\nhomer={hash}\n\
             bart={hash}\nlisa={hash}\n{extra}"
        )
    }

    #[test]
    fn shared_catalogue_preserves_browser_pinned_selection()
    -> Result<(), RenderError> {
        let hash = ContentHash::calculate(b"fixture");
        let text = catalogue(
            ".",
            &format!(
                "poses=launch-poses.txt\ndisplay=integer\n\
                 audio={hash}\naudio_mod={hash}\n"
            ),
        );
        let fields = Catalogue::parse(&text).map_err(failure)?;
        let result = selection(&fields, Some("pose fixture"), true)?;
        if result.actors != [hash; 4]
            || result.audio.is_none_or(|audio| {
                audio.base != hash || audio.replacement != Some(hash)
            })
            || result.display.presentation != PresentationMode::Integer
            || result.poses != Some("pose fixture")
            || !result.touch_enabled
        {
            return Err(failure("browser selection differs from catalogue"));
        }
        Ok(())
    }

    #[test]
    fn rejects_native_paths_and_missing_poses() -> Result<(), RenderError> {
        for (parent, extra) in [
            ("..", ""),
            ("/assets", ""),
            (".", "controls=profile.txt\n"),
            (".", "settings=.\n"),
            (".", "window=fullscreen\n"),
            (".", "poses=../poses.txt\n"),
            (".", "poses=launch-poses.txt\n"),
        ] {
            let text = catalogue(parent, extra);
            let fields = Catalogue::parse(&text).map_err(failure)?;
            if selection(&fields, None, false).is_ok() {
                return Err(failure("unsupported browser catalogue accepted"));
            }
        }
        Ok(())
    }
}
