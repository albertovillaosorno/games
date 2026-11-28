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
use games_platform::ir::process::{write_diagnostic, write_status};
use games_platform::ir::storage::bundle::directory::{
    DirectoryLimits, DirectoryRequest, DirectorySelectionError,
    ManifestRequest, SelectedDirectory, SidecarSnapshot, select,
};
use games_platform::ir::storage::bundle::{BundleError, MemoryBundleLimits};
use games_platform::ir::storage::preferences::{
    ControlStore, DisplayStore, LocaleStore, PreferenceError,
    PublicationPolicy, VolumeStore,
};
use games_platform::ir::storage::save::SaveError;

use super::launch::{
    AudioSelection as CatalogueAudio, Catalogue, ResolvedControls,
};
use super::memory_preview::{
    AudioSelection, Preferences, Selection, start_with_preferences,
    verify_scenery, verify_soundtrack,
};
use super::title_messages::{self, LaunchFailure, LaunchNotice, launch_label};
use super::{
    art_bundle_limits, audio, failure, parse_hash, parse_hashes, poses,
    profile_limit, world,
};

const CATALOGUE: &str = "launch.local";
const POSES: &str = "launch-poses.txt";
const PREFERENCES: &str = "simpsons-arcade";

type Completion = Rc<dyn Fn(usize, Result<(), LaunchFailure>)>;
type SmokeCompletion = fn(Result<(), RenderError>);
type Start = fn(
    &SelectedDirectory,
    bool,
    &Preferences,
    &Completion,
) -> Result<(), LaunchFailure>;

#[derive(Debug)]
struct Stores {
    controls: ControlStore,
    display: DisplayStore,
    locale: LocaleStore,
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
        let (locale_store, locale_index) =
            title_messages::restore_locale(Some(self.locale))
                .map_err(failure)?;
        let publication = PublicationPolicy::BestEffort;
        let controls = ResolvedControls::from_store(self.controls, publication)
            .map_err(failure)?;
        Ok(Preferences {
            controls: Rc::new(controls),
            display,
            display_store: Some(Rc::new(self.display)),
            locale_index,
            locale_store: locale_store.map(Rc::new),
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
            locale: LocaleStore::open(namespace)?,
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
pub fn start(touch_enabled: bool) -> Result<(), RenderError> {
    let locale = picker_locale(OsStr::new(PREFERENCES));
    let result =
        select_with(touch_enabled, locale, start_selected, Rc::new(report));
    match &result {
        Ok(()) => report_status(locale, LaunchNotice::Choose),
        Err(rejected) => report_failure(locale, rejected),
    }
    result.map_err(|rejected| rejected.error)
}

/// Check selected scene tracks through the resident renderer.
/// With second-stage art, also drive the existing routed handoff smoke.
/// Unlike live play, this explicit diagnostic runs a finite readback suite.
/// # Errors
/// Rejects absent scene tracks, invalid assets or failed verification.
pub fn scenery_smoke(
    touch_enabled: bool,
    complete: SmokeCompletion,
) -> Result<(), RenderError> {
    select_smoke(touch_enabled, smoke_selected, complete)
}

/// Verify host-consumed second-stage soundtrack playback and pause.
/// # Errors
/// Rejects missing assets, playback failures or an exceeded time budget.
pub fn soundtrack_smoke(complete: SmokeCompletion) -> Result<(), RenderError> {
    select_smoke(false, soundtrack_selected, complete)
}

fn select_smoke(
    touch_enabled: bool,
    start: Start,
    complete: SmokeCompletion,
) -> Result<(), RenderError> {
    let locale = picker_locale(OsStr::new(PREFERENCES));
    select_with(
        touch_enabled,
        locale,
        start,
        Rc::new(move |_index, result| {
            complete(result.map_err(|rejected| rejected.error));
        }),
    )
    .map_err(|rejected| rejected.error)
}

fn select_with(
    touch_enabled: bool,
    locale: usize,
    start: Start,
    complete: Completion,
) -> Result<(), LaunchFailure> {
    let prompt = launch_label(locale, LaunchNotice::Choose)?;
    select(
        DirectoryRequest {
            limits: limits(),
            prompt: &prompt,
            sidecars: &[CATALOGUE, POSES],
        },
        move |sidecars| {
            let options = selection_from(sidecars, touch_enabled)?;
            let requests = manifest_requests(&options)?;
            let admitted = preferences(options.display)?;
            Ok((requests, admitted))
        },
        move |result| match result {
            Ok((directory, admitted)) => {
                let index = admitted.locale_index;
                if let Err(rejected) =
                    start(&directory, touch_enabled, &admitted, &complete)
                {
                    complete(index, Err(rejected));
                }
            },
            Err(rejected) => complete(locale, Err(directory_failure(rejected))),
        },
    )
    .map_err(bundle_failure)
}

fn picker_locale(namespace: &OsStr) -> usize {
    let stored = LocaleStore::open(namespace)
        .and_then(|store| store.load())
        .ok()
        .flatten();
    title_messages::locale_index(stored.as_ref())
}

fn bundle_failure(error: BundleError) -> LaunchFailure {
    let notice = match error {
        BundleError::Io(ErrorKind::Interrupted) => LaunchNotice::Cancelled,
        BundleError::Io(ErrorKind::Unsupported) => LaunchNotice::Unsupported,
        BundleError::Asset(_)
        | BundleError::Io(_)
        | BundleError::MissingKey
        | BundleError::UnsafeStorage => LaunchNotice::Assets,
    };
    LaunchFailure::new(notice, failure(error))
}

fn directory_failure(
    rejected: DirectorySelectionError<RenderError>,
) -> LaunchFailure {
    match rejected {
        DirectorySelectionError::Bundle(error) => bundle_failure(error),
        DirectorySelectionError::Selection(error) => {
            LaunchFailure::new(LaunchNotice::Configuration, error)
        },
    }
}

fn report(index: usize, result: Result<(), LaunchFailure>) {
    match result {
        Ok(()) => report_status(index, LaunchNotice::Ended),
        Err(rejected) => report_failure(index, &rejected),
    }
}

fn report_status(index: usize, notice: LaunchNotice) {
    match launch_label(index, notice) {
        Ok(label) => {
            let _reported = write_status(format_args!("{label}"));
        },
        Err(error) => {
            let _reported = write_diagnostic(format_args!("{error:?}"));
        },
    }
}

fn report_failure(index: usize, rejected: &LaunchFailure) {
    if rejected.notice == LaunchNotice::Cancelled {
        report_status(index, LaunchNotice::Cancelled);
        return;
    }
    let detail = format!("browser-preview: failed: {:?}", rejected.error);
    let message = title_messages::launch_error(index, rejected.notice, &detail);
    let _reported = write_diagnostic(format_args!("{message}"));
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
        sidecar: poses::MAX_CATALOGUE_BYTES,
    }
}

fn audio_selection(
    selection: &CatalogueAudio,
) -> Result<AudioSelection, RenderError> {
    AudioSelection::from_local(selection)
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

fn selection_from(
    sidecars: &SidecarSnapshot,
    touch_enabled: bool,
) -> Result<Selection<'_>, RenderError> {
    let bytes = sidecars
        .get(CATALOGUE)
        .ok_or_else(|| failure("missing launch.local"))?;
    let catalogue = Catalogue::parse(str::from_utf8(bytes).map_err(failure)?)
        .map_err(failure)?;
    let pose_text = catalogue
        .poses
        .and_then(|name| sidecars.get(name))
        .map(str::from_utf8)
        .transpose()
        .map_err(failure)?;
    selection(&catalogue, pose_text, touch_enabled)
}

fn append_audio_requests(
    requests: &mut Vec<ManifestRequest>,
    audio: &AudioSelection,
) -> Result<(), RenderError> {
    requests.push(ManifestRequest {
        digest: audio.base,
        limits: audio::base_limits()?,
    });
    if let Some(digest) = audio.music {
        requests.push(ManifestRequest {
            digest,
            limits: audio::music_limits()?,
        });
    }
    if let Some(hashes) = audio.bonus_music {
        let limits = audio::bonus_music_limits()?;
        requests
            .extend(hashes.map(|digest| ManifestRequest { digest, limits }));
    }
    if let Some(hashes) = audio.handoff_music {
        let limits = audio::handoff_music_limits()?;
        requests
            .extend(hashes.map(|digest| ManifestRequest { digest, limits }));
    }
    if let Some(digest) = audio.stage_two_music {
        requests.push(ManifestRequest {
            digest,
            limits: audio::stage_two_music_limits()?,
        });
    }
    if let Some(digest) = audio.cemetery_landing {
        requests.push(ManifestRequest {
            digest,
            limits: audio::cemetery_landing_limits()?,
        });
    }
    if let Some(digest) = audio.cemetery_music {
        requests.push(ManifestRequest {
            digest,
            limits: audio::cemetery_music_limits()?,
        });
    }
    if let Some(digest) = audio.replacement {
        requests.push(ManifestRequest {
            digest,
            limits: audio::replacement_limits()?,
        });
    }
    Ok(())
}

fn manifest_requests(
    selection: &Selection<'_>,
) -> Result<Vec<ManifestRequest>, RenderError> {
    let catalogue = selection
        .poses
        .map(poses::Catalogue::parse)
        .transpose()?
        .unwrap_or_default();
    let art = art_bundle_limits()?;
    let strip_limits = world::bundle_limits()?;
    let mut requests: Vec<_> = selection
        .actors
        .into_iter()
        .chain(selection.enemy.into_iter().flatten())
        .chain(selection.shadow)
        .chain(catalogue.digests())
        .map(|digest| ManifestRequest {
            digest,
            limits: if Some(digest) == catalogue.stage_two_strip() {
                strip_limits
            } else {
                art
            },
        })
        .collect();
    if let Some(digest) = selection.world {
        requests.push(ManifestRequest {
            digest,
            limits: strip_limits,
        });
    }
    if let Some(audio) = selection.audio {
        append_audio_requests(&mut requests, &audio)?;
    }
    Ok(requests)
}

fn start_selected(
    directory: &SelectedDirectory,
    touch_enabled: bool,
    admitted: &Preferences,
    complete: &Completion,
) -> Result<(), LaunchFailure> {
    let options = selection_from(directory.sidecars(), touch_enabled).map_err(
        |error| LaunchFailure::new(LaunchNotice::Configuration, error),
    )?;
    let callback = Rc::clone(complete);
    start_with_preferences(
        directory.bundles(),
        &options,
        admitted,
        move |index, result| {
            callback(index, result.map_err(LaunchFailure::from));
        },
    )?;
    report_status(admitted.locale_index, LaunchNotice::Ready);
    Ok(())
}

fn smoke_selected(
    directory: &SelectedDirectory,
    touch_enabled: bool,
    admitted: &Preferences,
    complete: &Completion,
) -> Result<(), LaunchFailure> {
    let options = selection_from(directory.sidecars(), touch_enabled)?;
    verify_scenery(directory.bundles(), &options, admitted)?;
    complete(admitted.locale_index, Ok(()));
    Ok(())
}

fn soundtrack_selected(
    directory: &SelectedDirectory,
    touch_enabled: bool,
    admitted: &Preferences,
    complete: &Completion,
) -> Result<(), LaunchFailure> {
    let options = selection_from(directory.sidecars(), touch_enabled)?;
    let callback = Rc::clone(complete);
    let index = admitted.locale_index;
    verify_soundtrack(
        directory.bundles(),
        &options,
        admitted,
        move |result| {
            callback(index, result.map_err(LaunchFailure::from));
        },
    )?;
    Ok(())
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
    use games::audio::MasterVolume;
    use games::presentation::{LocaleTag, PresentationMode};
    use games_assets::{ContentHash, ManifestLimits};

    use super::*;
    type ManifestExpectation = (ContentHash, usize, ManifestLimits);

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
    fn picker_language_uses_stored_locale_without_rewriting()
    -> Result<(), RenderError> {
        use games_platform::ir::storage::fixtures::TestRoot;

        let root = TestRoot::new().map_err(failure)?;
        let store = LocaleStore::open(root.location()).map_err(failure)?;
        if picker_locale(root.location()) != 0 {
            return Err(failure("unexpected picker default"));
        }
        for (tag, expected) in [("es-MX", 1usize), ("ja", 0usize)] {
            let locale = LocaleTag::parse(tag).map_err(failure)?;
            PublicationPolicy::Durable
                .confirm(store.store(&locale).map_err(failure)?)
                .map_err(failure)?;
            if picker_locale(root.location()) != expected
                || store.load().map_err(failure)? != Some(locale)
            {
                return Err(failure("picker changed stored preference"));
            }
        }
        Ok(())
    }

    #[test]
    fn directory_rejection_preserves_category_and_technical_detail()
    -> Result<(), RenderError> {
        let cancelled = bundle_failure(BundleError::Io(ErrorKind::Interrupted));
        if cancelled.notice != LaunchNotice::Cancelled
            || launch_label(1, cancelled.notice)?
                != "Se cancel\u{f3} la selecci\u{f3}n de carpeta."
        {
            return Err(failure("cancelled directory treated as a failure"));
        }
        let missing = directory_failure(DirectorySelectionError::Bundle(
            BundleError::MissingKey,
        ));
        let detail = format!("{:?}", missing.error);
        let message = title_messages::launch_error(1, missing.notice, &detail);
        if missing.notice != LaunchNotice::Assets
            || !message.starts_with("Revisa los recursos can\u{f3}nicos")
            || !message.ends_with(&detail)
            || !detail.contains("MissingKey")
        {
            return Err(failure(message));
        }
        let malformed = RenderError::Backend(String::from("invalid sidecar"));
        let rejected = directory_failure(DirectorySelectionError::Selection(
            malformed.clone(),
        ));
        if rejected.notice != LaunchNotice::Configuration
            || rejected.error != malformed
        {
            return Err(failure("configuration rejection lost detail"));
        }
        Ok(())
    }

    #[test]
    fn browser_preferences_reuse_typed_slots_and_profile_admission()
    -> Result<(), RenderError> {
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
            || initial.locale_index != 0
            || initial.locale_store.is_none()
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
        let spanish = LocaleTag::parse("es-MX").map_err(failure)?;
        let _locale = handles.locale.store(&spanish).map_err(failure)?;
        let _volume =
            handles.volume.store(MasterVolume::Half).map_err(failure)?;
        let profile = "version=1\nkey 1 right=Q\n";
        let _controls = handles.controls.store(profile).map_err(failure)?;
        let loaded = handles.load(defaults)?;
        if loaded.display != chosen
            || loaded.locale_index != 1
            || loaded.locale_store.is_none()
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
    fn admitted_preferences_survive_later_storage_changes()
    -> Result<(), RenderError> {
        use games_platform::ir::storage::fixtures::TestRoot;

        let root = TestRoot::new().map_err(failure)?;
        let maximum = profile_limit().map_err(failure)?;
        let open = || Stores::open(root.location(), maximum).map_err(failure);
        let defaults = DisplaySettings::default();
        let admitted = open()?.load(defaults)?;
        let changed = open()?;
        let _unsupported = changed
            .display
            .store(DisplaySettings {
                presentation: defaults.presentation,
                window: WindowMode::Fullscreen,
            })
            .map_err(failure)?;
        if admitted.display != defaults || open()?.load(defaults).is_ok() {
            return Err(failure("admitted preference snapshot changed"));
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
                 audio={hash}\naudio_mod={hash}\nmusic={hash}\n\
                 effects_volume=25\nmusic_volume=75\n"
            ),
        );
        let fields = Catalogue::parse(&text).map_err(failure)?;
        let result = selection(&fields, Some("pose fixture"), true)?;
        if result.actors != [hash; 4]
            || result.audio.is_none_or(|audio| {
                audio.base != hash
                    || audio.replacement != Some(hash)
                    || audio.music != Some(hash)
                    || audio.effects_volume != MasterVolume::Quarter
                    || audio.music_volume != MasterVolume::ThreeQuarters
            })
            || result.display.presentation != PresentationMode::Integer
            || result.poses != Some("pose fixture")
            || !result.touch_enabled
        {
            return Err(failure("browser selection differs from catalogue"));
        }
        Ok(())
    }

    fn has_manifest_requests(
        requests: &[ManifestRequest],
        digest: ContentHash,
        count: usize,
        limit: ManifestLimits,
    ) -> bool {
        requests
            .iter()
            .filter(|request| {
                request.digest == digest
                    && request.limits.manifest_bytes == limit.manifest_bytes
                    && request.limits.payload_bytes == limit.payload_bytes
                    && request.limits.records == limit.records
                    && request.limits.total_bytes == limit.total_bytes
            })
            .count()
            == count
    }

    fn pose_fixture(
        pose: ContentHash,
        scenic: ContentHash,
        background: ContentHash,
    ) -> String {
        ["marge", "homer", "bart", "lisa"]
            .into_iter()
            .flat_map(|name| {
                ["attack", "jump"]
                    .map(|kind| format!("{name} {kind} 0 {pose}\n"))
            })
            .chain(
                (8u16..=328).step_by(8).map(|tick| {
                    format!("scenic-pair {tick} 248 52 {scenic}\n")
                }),
            )
            .chain(
                super::super::backdrop::KEYS
                    .into_iter()
                    .map(|tick| format!("backdrop {tick} {background}\n")),
            )
            .collect()
    }

    fn handoff_fixture() -> [ContentHash; 4] {
        [
            b"defeat".as_slice(),
            b"victory",
            b"transition",
            b"instructions",
        ]
        .map(ContentHash::calculate)
    }

    fn verify_handoff_requests(
        requests: &[ManifestRequest],
        handoff: [ContentHash; 4],
    ) -> Result<(), RenderError> {
        for digest in handoff {
            if !has_manifest_requests(
                requests,
                digest,
                1,
                audio::handoff_music_limits()?,
            ) {
                return Err(failure("missing handoff manifest"));
            }
        }
        Ok(())
    }

    #[test]
    fn manifest_selection_includes_every_art_and_audio_role()
    -> Result<(), RenderError> {
        let actor = ContentHash::calculate(b"actor");
        let enemy = ContentHash::calculate(b"enemy");
        let shadow = ContentHash::calculate(b"shadow");
        let world = ContentHash::calculate(b"world");
        let pose = ContentHash::calculate(b"pose");
        let scenic = ContentHash::calculate(b"scenic");
        let background = ContentHash::calculate(b"backdrop");
        let base = ContentHash::calculate(b"base");
        let replacement = ContentHash::calculate(b"replacement");
        let music = ContentHash::calculate(b"music");
        let pump = ContentHash::calculate(b"pump");
        let results = ContentHash::calculate(b"results");
        let stage_two_music = ContentHash::calculate(b"stage two music");
        let cemetery_landing = ContentHash::calculate(b"cemetery landing");
        let cemetery_music = ContentHash::calculate(b"cemetery music");
        let poses = pose_fixture(pose, scenic, background);
        let selection = Selection {
            actors: [actor; 4],
            audio: Some(AudioSelection {
                base,
                bonus_music: Some([pump, results]),
                cemetery_landing: Some(cemetery_landing),
                cemetery_music: Some(cemetery_music),
                effects_volume: MasterVolume::Full,
                handoff_music: Some(handoff_fixture()),
                music: Some(music),
                music_volume: MasterVolume::Full,
                replacement: Some(replacement),
                stage_two_music: Some(stage_two_music),
            }),
            display: DisplaySettings::default(),
            enemy: Some([enemy; 2]),
            poses: Some(&poses),
            shadow: Some(shadow),
            touch_enabled: false,
            world: Some(world),
        };
        let requests = manifest_requests(&selection)?;
        verify_manifest_request_set(&requests, &[
            (actor, 4, art_bundle_limits()?),
            (enemy, 2, art_bundle_limits()?),
            (shadow, 1, art_bundle_limits()?),
            (world, 1, world::bundle_limits()?),
            (pose, 8, art_bundle_limits()?),
            (scenic, 41, art_bundle_limits()?),
            (background, 15, art_bundle_limits()?),
            (base, 1, audio::base_limits()?),
            (music, 1, audio::music_limits()?),
            (pump, 1, audio::bonus_music_limits()?),
            (results, 1, audio::bonus_music_limits()?),
            (replacement, 1, audio::replacement_limits()?),
            (stage_two_music, 1, audio::stage_two_music_limits()?),
            (cemetery_landing, 1, audio::cemetery_landing_limits()?),
            (cemetery_music, 1, audio::cemetery_music_limits()?),
        ])?;
        verify_handoff_requests(&requests, handoff_fixture())?;
        verify_invalid_pose(&selection)
    }

    fn verify_manifest_request_set(
        requests: &[ManifestRequest],
        expected: &[ManifestExpectation],
    ) -> Result<(), RenderError> {
        for &(digest, count, limit) in expected {
            if !has_manifest_requests(requests, digest, count, limit) {
                return Err(failure(
                    "missing or incorrectly bounded browser manifest",
                ));
            }
        }
        Ok(())
    }

    fn verify_invalid_pose(
        selection: &Selection<'_>,
    ) -> Result<(), RenderError> {
        if manifest_requests(&Selection {
            poses: Some("invalid"),
            ..*selection
        })
        .is_ok()
        {
            return Err(failure("invalid pose catalogue admitted"));
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
