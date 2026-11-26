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
//   - Windowed presentation, origin-local preferences and no remote access.

//! Browser title launch from a user-selected canonical directory.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::num::{NonZeroU64, NonZeroUsize};
use std::rc::Rc;
use std::str;

use games::presentation::{DisplaySettings, WindowMode};
use games_platform::ir::frame::RenderError;
use games_platform::ir::storage::bundle::MemoryBundleLimits;
use games_platform::ir::storage::bundle::directory::{
    DirectoryLimits, SelectedDirectory, select,
};
use games_platform::ir::storage::preferences::{
    ControlStore, DisplayStore, PreferenceError, PublicationPolicy, VolumeStore,
};
use games_platform::ir::storage::save::SaveError;

use super::launch::{
    AudioSelection as CatalogueAudio, Catalogue, ResolvedControls,
};
use super::memory_preview::{
    AudioSelection, Preferences, Selection, start_with_preferences,
};
use super::{failure, parse_hash, parse_hashes, profile_limit};

const CATALOGUE: &str = "launch.local";
const POSES: &str = "launch-poses.txt";
const PREFERENCES: &str = "simpsons-arcade";

type Completion = fn(Result<(), RenderError>);

#[derive(Debug)]
struct Stores {
    controls: ControlStore,
    display: DisplayStore,
    volume: VolumeStore,
}

impl Stores {
    fn load(
        self,
        defaults: DisplaySettings,
    ) -> Result<Preferences, RenderError> {
        let display = self.display.load().map_err(failure)?.unwrap_or(defaults);
        if display.window != WindowMode::Windowed {
            return Err(failure("browser requires windowed preferences"));
        }
        let volume = self.volume.load().map_err(failure)?.unwrap_or_default();
        let publication = PublicationPolicy::BestEffort;
        let controls = ResolvedControls::from_store(self.controls, publication)
            .map_err(failure)?;
        Ok(Preferences {
            controls: Rc::new(controls),
            display,
            display_store: Some(Rc::new(self.display)),
            publication,
            volume,
            volume_store: Some(Rc::new(self.volume)),
        })
    }

    fn open(
        namespace: &OsStr,
        maximum: NonZeroU64,
    ) -> Result<Self, PreferenceError> {
        Ok(Self {
            controls: ControlStore::open(namespace, maximum)?,
            display: DisplayStore::open(namespace)?,
            volume: VolumeStore::open(namespace)?,
        })
    }
}

/// Present the browser directory picker, then start the selected title.
///
/// The selected root must contain launch.local. Preferences are stored in
/// a fixed origin-local namespace when available. Asset bytes
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
    let preferences = preferences(options.display)?;
    start_with_preferences(
        directory.bundles(),
        &options,
        &preferences,
        complete,
    )
}

fn preferences(defaults: DisplaySettings) -> Result<Preferences, RenderError> {
    match Stores::open(
        OsStr::new(PREFERENCES),
        profile_limit().map_err(failure)?,
    ) {
        Ok(stores) => stores.load(defaults),
        Err(error) if unavailable(error) => {
            Ok(Preferences::transient(defaults))
        },
        Err(error) => Err(failure(error)),
    }
}

const fn unavailable(error: PreferenceError) -> bool {
    matches!(
        error,
        PreferenceError::Storage(SaveError::Io(
            ErrorKind::Unsupported | ErrorKind::PermissionDenied,
        ))
    )
}

#[cfg(test)]
mod tests {
    use games::presentation::PresentationMode;
    use games_assets::ContentHash;

    use super::*;

    #[test]
    fn only_unavailable_store_open_is_eligible_for_transient_fallback() {
        for error in [ErrorKind::Unsupported, ErrorKind::PermissionDenied] {
            assert!(unavailable(PreferenceError::Storage(SaveError::Io(
                error
            ))));
        }
        for error in [
            PreferenceError::InvalidPayload,
            PreferenceError::Storage(SaveError::UnsafeStorage),
            PreferenceError::Storage(SaveError::Io(ErrorKind::Other)),
        ] {
            assert!(!unavailable(error));
        }
    }

    #[test]
    fn browser_preferences_reuse_typed_slots_and_profile_admission()
    -> Result<(), RenderError> {
        use games::audio::MasterVolume;
        use games_platform::ir::storage::fixtures::TestRoot;

        use super::super::{DEFAULT_PROFILES, configured_profiles};

        let root = TestRoot::new().map_err(failure)?;
        let maximum = profile_limit().map_err(failure)?;
        let open = || Stores::open(root.location(), maximum).map_err(failure);
        let defaults = DisplaySettings {
            presentation: PresentationMode::Integer,
            window: WindowMode::Windowed,
        };
        let initial = open()?.load(defaults)?;
        if initial.display != defaults
            || initial.volume != MasterVolume::Full
            || initial.controls.profiles != DEFAULT_PROFILES
            || !initial.controls.can_reset()
            || initial.publication != PublicationPolicy::BestEffort
        {
            return Err(failure("unexpected initial browser preferences"));
        }
        let chosen = DisplaySettings {
            presentation: PresentationMode::Fill,
            window: WindowMode::Windowed,
        };
        let handles = open()?;
        let _display = handles.display.store(chosen).map_err(failure)?;
        let _volume =
            handles.volume.store(MasterVolume::Half).map_err(failure)?;
        let profile = "version=1\nkey 1 right=Q\n";
        let _controls = handles.controls.store(profile).map_err(failure)?;
        let loaded = handles.load(defaults)?;
        if loaded.display != chosen
            || loaded.volume != MasterVolume::Half
            || loaded.controls.profiles
                != configured_profiles(profile).map_err(failure)?
        {
            return Err(failure("stored browser preferences lost"));
        }
        loaded.controls.persist_defaults().map_err(failure)?;
        if open()?.load(defaults)?.controls.profiles != DEFAULT_PROFILES {
            return Err(failure("stored browser control reset lost"));
        }
        Ok(())
    }

    #[test]
    fn browser_preferences_reject_incompatible_saved_values()
    -> Result<(), RenderError> {
        use games_platform::ir::storage::fixtures::TestRoot;
        let root = TestRoot::new().map_err(failure)?;
        let maximum = profile_limit().map_err(failure)?;
        let open = || Stores::open(root.location(), maximum).map_err(failure);
        let defaults = DisplaySettings::default();
        let malformed = open()?;
        let _invalid = malformed
            .controls
            .store("version=invalid\n")
            .map_err(failure)?;
        if malformed.load(defaults).is_ok() {
            return Err(failure("invalid browser control profile accepted"));
        }
        let unsupported = open()?;
        let _valid =
            unsupported.controls.store("version=1\n").map_err(failure)?;
        let _fullscreen = unsupported
            .display
            .store(DisplaySettings {
                presentation: defaults.presentation,
                window: WindowMode::Fullscreen,
            })
            .map_err(failure)?;
        if unsupported.load(defaults).is_ok() {
            return Err(failure("unsupported saved fullscreen accepted"));
        }
        Ok(())
    }

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
