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
//   - Explicit native launch argument and local asset catalogue admission.
// - Must-Not:
//   - Search for original media, infer hashes or open canonical payloads.
// - Allows:
//   - Inputs: Command-line arguments or one bounded local launch file.
//   - Outputs: Selected asset identities, paths and presentation flags.
//   - Side effects: Read the explicitly selected or executable-adjacent file.
// - Split-When:
//   - Multiple titles need an identical launch catalogue contract.
// - Merge-When:
//   - Another title module parses the same launch options.
// - Summary:
//   - Resolve a pinned asset selection without repeating long hashes.
// - Description:
//   - Relative paths are anchored to the catalogue, not the working directory.
// - Usage:
//   - Supply legacy explicit arguments, --config FILE or a sibling file.
// - Defaults:
//   - No original media discovery, recursive scans or network access.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::iter::Peekable;
use std::num::NonZeroU64;
use std::{str, vec};

use games::audio::MasterVolume;
use games::input::physical::{ControllerAxis, ControllerButton, PointerButton};
use games::input::profile::{AxisProfile, DigitalBinding};
use games::presentation::DisplaySettings;
use games_assets::ContentHash;
use games_platform::ir::input::{key_from_name, key_name};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::preferences::{
    ControlStore, DisplayStore, PublicationPolicy, VolumeStore,
};
use games_platform::ir::storage::{executable_path, join_path, parent_path};

use super::{
    ACTION_NAMES, DEFAULT_PROFILES, MAX_PROFILE_BYTES, Profiles,
    configured_profiles, profile_limit, read_profile,
    validate_control_bindings,
};

const DEFAULT_PROFILE: &str = "version=1\n";
const FILE_NAME: &str = "simpsons-arcade.local";
const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(4095);
const USAGE: &str = concat!(
    "usage: simpsons-arcade [--smoke | --measure] [--touch] ",
    "[--display MODE] [--window MODE] [--audio-device NAME] ",
    "[--settings DIR] ",
    "[--controls FILE | --reset-controls] ",
    "[--lan-host FILE] ",
    "[--config FILE | [--poses FILE] ",
    "[--enemy APPROACH_SHA STRIKE_SHA] [--audio SHA [--audio-mod SHA]] ",
    "[--shadow SHA] [--world SHA] ",
    "PARENT MARGE_SHA HOMER_SHA BART_SHA LISA_SHA]\n",
    "Without an asset selection, read simpsons-arcade.local ",
    "beside the binary.\n",
    "usage: simpsons-arcade --lan-controller FILE [--settings DIR] ",
    "[--controls FILE | --reset-controls]\n",
    "usage: simpsons-arcade --list-audio-devices\n",
    "usage: simpsons-arcade --edit-control DIR SEAT ACTION ",
    "key|mouse|button|touch BINDING"
);

type AdmittedEdit = (Profiles, String);
type Arguments = Peekable<vec::IntoIter<OsString>>;
type ConfiguredAssets = (Assets, DisplaySettings, LocalPaths);
type EnemyHashes = Option<[OsString; 2]>;
type VolumePreferences = (Option<VolumeStore>, MasterVolume);
pub(super) type Fields<'text> = BTreeMap<&'text str, &'text str>;

#[derive(Debug, Default, Eq, PartialEq)]
struct LocalPaths {
    audio_device: Option<String>,
    controls: Option<OsString>,
    settings: Option<OsString>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct AudioSelection {
    pub(super) base: OsString,
    pub(super) replacement: Option<OsString>,
}

impl AudioSelection {
    fn explicit(args: &mut Arguments) -> Result<Option<Self>, String> {
        let base = flag(args, "--audio").then(|| next(args)).transpose()?;
        let replacement =
            flag(args, "--audio-mod").then(|| next(args)).transpose()?;
        Self::new(base, replacement)
    }

    fn new(
        base: Option<OsString>,
        replacement: Option<OsString>,
    ) -> Result<Option<Self>, String> {
        match (base, replacement) {
            (Some(digest), changes) => Ok(Some(Self {
                base: digest,
                replacement: changes,
            })),
            (None, None) => Ok(None),
            (None, Some(_)) => {
                Err(String::from("audio replacement requires a base bundle"))
            },
        }
    }
}

/// Host-independent launch fields, before resolving any selected paths.
#[derive(Debug)]
pub(super) struct Catalogue<'text> {
    pub(super) actors: [OsString; 4],
    pub(super) audio: Option<AudioSelection>,
    pub(super) audio_device: Option<String>,
    pub(super) controls: Option<&'text str>,
    pub(super) display: DisplaySettings,
    pub(super) enemy: EnemyHashes,
    pub(super) parent: &'text str,
    pub(super) poses: Option<&'text str>,
    pub(super) settings: Option<&'text str>,
    pub(super) shadow: Option<OsString>,
    pub(super) world: Option<OsString>,
}

impl<'text> Catalogue<'text> {
    pub(super) fn parse(text: &'text str) -> Result<Self, String> {
        if u64::try_from(text.len()).map_or(true, |len| len > MAX_BYTES.get()) {
            return Err(String::from("launch catalogue exceeds byte limit"));
        }
        let mut values = fields(text)?;
        if values.remove("version") != Some("1") {
            return Err(String::from("expected launch catalogue version=1"));
        }
        let parent = required(&mut values, "parent")?;
        let actors = [
            hash(required(&mut values, "marge")?)?,
            hash(required(&mut values, "homer")?)?,
            hash(required(&mut values, "bart")?)?,
            hash(required(&mut values, "lisa")?)?,
        ];
        let enemy = enemy_hashes(&mut values)?;
        let audio = AudioSelection::new(
            values.remove("audio").map(hash).transpose()?,
            values.remove("audio_mod").map(hash).transpose()?,
        )?;
        let audio_device = selected_audio_device(
            values
                .remove("audio_device")
                .map(str::to_owned)
                .map(validate_audio_device)
                .transpose()?,
            audio.is_some(),
        )?;
        let shadow = values.remove("shadow").map(hash).transpose()?;
        let world = values.remove("world").map(hash).transpose()?;
        let poses = values.remove("poses");
        let controls = values.remove("controls");
        let settings = values.remove("settings");
        let display = DisplaySettings::default()
            .configured(values.remove("display"), values.remove("window"))?;
        if !values.is_empty() {
            return Err(String::from("unknown launch catalogue field"));
        }
        Ok(Self {
            actors,
            audio,
            audio_device,
            controls,
            display,
            enemy,
            parent,
            poses,
            settings,
            shadow,
            world,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Assets {
    pub(super) actors: [OsString; 4],
    pub(super) audio: Option<AudioSelection>,
    pub(super) directory: OsString,
    pub(super) enemy: Option<[OsString; 2]>,
    pub(super) poses: Option<OsString>,
    pub(super) shadow: Option<OsString>,
    pub(super) world: Option<OsString>,
}

impl Assets {
    fn catalogue(path: &OsStr, text: &str) -> Result<ConfiguredAssets, String> {
        let fields = Catalogue::parse(text)?;
        let base = parent_path(path)
            .map_err(|error| format!("launch catalogue path: {error}"))?;
        Ok((
            Self {
                actors: fields.actors,
                audio: fields.audio,
                directory: resolve_local(base, fields.parent)?,
                enemy: fields.enemy,
                poses: fields
                    .poses
                    .map(|value| resolve_local(base, value))
                    .transpose()?,
                shadow: fields.shadow,
                world: fields.world,
            },
            fields.display,
            LocalPaths {
                audio_device: fields.audio_device,
                controls: fields
                    .controls
                    .map(|value| resolve_local(base, value))
                    .transpose()?,
                settings: fields
                    .settings
                    .map(|value| resolve_local(base, value))
                    .transpose()?,
            },
        ))
    }

    fn explicit(mut args: Arguments) -> Result<Self, String> {
        let poses = if flag(&mut args, "--poses") {
            Some(next(&mut args)?)
        } else {
            None
        };
        let enemy = if flag(&mut args, "--enemy") {
            Some([next(&mut args)?, next(&mut args)?])
        } else {
            None
        };
        let audio = AudioSelection::explicit(&mut args)?;
        let shadow = flag(&mut args, "--shadow")
            .then(|| next(&mut args))
            .transpose()?;
        let world = if flag(&mut args, "--world") {
            Some(next(&mut args)?)
        } else {
            None
        };
        let [directory, marge, homer, bart, lisa] =
            <[OsString; 5]>::try_from(args.collect::<Vec<_>>())
                .map_err(|_error| String::from(USAGE))?;
        Ok(Self {
            actors: [marge, homer, bart, lisa],
            audio,
            directory,
            enemy,
            poses,
            shadow,
            world,
        })
    }

    fn open(path: &OsStr) -> Result<ConfiguredAssets, String> {
        let bytes = snapshot(path, MAX_BYTES).map_err(|error| {
            format!("launch catalogue {}: {error:?}", path.to_string_lossy())
        })?;
        let text = str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        Self::catalogue(path, text)
    }

    fn select(mut remaining: Arguments) -> Result<ConfiguredAssets, String> {
        if flag(&mut remaining, "--config") {
            let config = next(&mut remaining)?;
            if remaining.next().is_some() {
                return Err(String::from(USAGE));
            }
            Self::open(&config)
        } else if remaining.peek().is_none() {
            let executable = executable_path()
                .map_err(|error| format!("executable path: {error}"))?;
            Self::open(&sibling_catalogue(&executable)?)
        } else {
            Ok((
                Self::explicit(remaining)?,
                DisplaySettings::default(),
                LocalPaths::default(),
            ))
        }
    }
}

#[derive(Debug)]
struct ControlOptions {
    explicit: Option<OsString>,
    reset: bool,
}

impl ControlOptions {
    fn parse(args: &mut Arguments, smoke: bool) -> Result<Self, String> {
        let explicit = optional_path(args, "--controls")?;
        let reset = flag(args, "--reset-controls");
        if reset && (smoke || explicit.is_some()) {
            return Err(String::from(
                "reset controls is exclusive with smoke and explicit controls",
            ));
        }
        Ok(Self { explicit, reset })
    }
}

#[derive(Debug)]
pub(super) struct ControlSelection {
    explicit: Option<OsString>,
    fallback: Option<OsString>,
    publication: PublicationPolicy,
    reset: bool,
    smoke: bool,
    store: Option<ControlStore>,
}

impl ControlSelection {
    fn open(
        options: ControlOptions,
        fallback: Option<OsString>,
        settings: Option<&OsStr>,
        smoke: bool,
    ) -> Result<Self, String> {
        let store = open_control_store(settings, options.reset)?;
        Ok(Self {
            explicit: options.explicit,
            fallback,
            publication: PublicationPolicy::Durable,
            reset: options.reset,
            smoke,
            store,
        })
    }

    pub(super) fn resolve(self) -> Result<ResolvedControls, String> {
        let Self {
            explicit,
            fallback,
            publication,
            reset,
            smoke,
            store,
        } = self;
        let update = reset || explicit.is_some();
        let text = if reset {
            DEFAULT_PROFILE.to_owned()
        } else if let Some(path) = explicit {
            read_profile(&path)?
        } else if let Some(stored) = store
            .as_ref()
            .map(ControlStore::load)
            .transpose()
            .map_err(|error| format!("control preferences: {error:?}"))?
            .flatten()
        {
            stored
        } else if let Some(path) = fallback {
            read_profile(&path)?
        } else {
            DEFAULT_PROFILE.to_owned()
        };
        let profiles = configured_profiles(&text)?;
        let persistent_store = if smoke {
            None
        } else {
            store
        };
        let pending = (update && persistent_store.is_some()).then_some(text);
        Ok(ResolvedControls {
            pending,
            profiles,
            publication,
            store: persistent_store,
        })
    }
}

#[derive(Debug)]
pub(super) struct ResolvedControls {
    pending: Option<String>,
    pub(super) profiles: Profiles,
    publication: PublicationPolicy,
    store: Option<ControlStore>,
}

impl ResolvedControls {
    pub(super) const fn can_reset(&self) -> bool {
        self.store.is_some()
    }

    pub(super) fn from_store(
        store: ControlStore,
        publication: PublicationPolicy,
    ) -> Result<Self, String> {
        ControlSelection {
            explicit: None,
            fallback: None,
            publication,
            reset: false,
            smoke: false,
            store: Some(store),
        }
        .resolve()
    }

    pub(super) fn persist(&self) -> Result<(), String> {
        if let (Some(store), Some(text)) = (&self.store, &self.pending) {
            Self::store_profile(store, text, self.publication)?;
        }
        Ok(())
    }

    pub(super) fn persist_axis_edit(
        &self,
        current: &Profiles,
        selection: (usize, ControllerAxis),
        profile: AxisProfile,
    ) -> Result<Option<Profiles>, String> {
        self.persist_candidate(
            current
                .edited_axis(selection.0, selection.1, profile)
                .ok()
                .as_ref(),
        )
    }

    fn persist_candidate(
        &self,
        proposed: Option<&Profiles>,
    ) -> Result<Option<Profiles>, String> {
        let store = self.store.as_ref().ok_or("control store unavailable")?;
        let Some(profile) = proposed else {
            return Ok(None);
        };
        let Ok((candidate, encoded)) = admit_candidate(profile) else {
            return Ok(None);
        };
        Self::store_profile(store, &encoded, self.publication)?;
        Ok(Some(candidate))
    }

    pub(super) fn persist_defaults(&self) -> Result<(), String> {
        let store = self.store.as_ref().ok_or("control store unavailable")?;
        Self::store_profile(store, DEFAULT_PROFILE, self.publication)
    }

    pub(super) fn persist_edit(
        &self,
        current: &Profiles,
        selection: (usize, usize),
        binding: DigitalBinding,
    ) -> Result<Option<Profiles>, String> {
        self.persist_candidate(
            current
                .edited(selection.0, selection.1, binding)
                .ok()
                .as_ref(),
        )
    }

    fn store_profile(
        store: &ControlStore,
        text: &str,
        publication: PublicationPolicy,
    ) -> Result<(), String> {
        let receipt = store
            .store(text)
            .map_err(|error| format!("control preferences: {error:?}"))?;
        publication.confirm(receipt).map_err(|error| {
            format!("control preferences durability: {error:?}")
        })
    }

    pub(super) const fn transient() -> Self {
        Self {
            pending: None,
            profiles: DEFAULT_PROFILES,
            publication: PublicationPolicy::Durable,
            store: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct ControllerLaunch {
    pub(super) catalogue: OsString,
    options: ControlOptions,
    settings: Option<OsString>,
}

impl ControllerLaunch {
    pub(super) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut remaining = args.into_iter().peekable();
        if !flag(&mut remaining, "--lan-controller") {
            return Err(String::from(USAGE));
        }
        let catalogue = next(&mut remaining)?;
        let settings = optional_path(&mut remaining, "--settings")?;
        let options = ControlOptions::parse(&mut remaining, false)?;
        if remaining.next().is_some() {
            return Err(String::from(USAGE));
        }
        Ok(Self {
            catalogue,
            options,
            settings,
        })
    }

    pub(super) fn select_controls(
        self,
        fallback: Option<&OsStr>,
        settings: Option<&OsStr>,
    ) -> Result<ResolvedControls, String> {
        let directory = self.settings.as_deref().or(settings);
        ControlSelection::open(
            self.options,
            fallback.map(OsStr::to_os_string),
            directory,
            false,
        )?
        .resolve()
    }
}

#[derive(Debug)]
pub(super) struct Launch {
    pub(super) assets: Assets,
    pub(super) audio_device: Option<String>,
    pub(super) controls: ControlSelection,
    pub(super) display: DisplaySettings,
    pub(super) host: Option<OsString>,
    pub(super) measure: bool,
    pub(super) preferences: Option<DisplayStore>,
    pub(super) smoke: bool,
    pub(super) touch_enabled: bool,
    pub(super) volume: MasterVolume,
    pub(super) volume_store: Option<VolumeStore>,
}

impl Launch {
    pub(super) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut remaining = args.into_iter().peekable();
        let smoke = flag(&mut remaining, "--smoke");
        let measure = measurement_flag(&mut remaining, smoke)?;
        let touch_enabled = flag(&mut remaining, "--touch");
        let mode = optional_value(&mut remaining, "--display")?;
        let window = optional_value(&mut remaining, "--window")?;
        let device = optional_value(&mut remaining, "--audio-device")?
            .map(validate_audio_device)
            .transpose()?;
        let explicit_settings = optional_path(&mut remaining, "--settings")?;
        let control_options = ControlOptions::parse(&mut remaining, smoke)?;
        let host = optional_path(&mut remaining, "--lan-host")?;
        if smoke && host.is_some() {
            return Err(String::from("smoke cannot open a LAN host"));
        }
        let (assets, defaults, paths) = Assets::select(remaining)?;
        let audio_device = selected_audio_device(
            device.or(paths.audio_device),
            assets.audio.is_some(),
        )?;
        let settings = explicit_settings.or(paths.settings);
        let preferences = settings
            .as_deref()
            .map(DisplayStore::open)
            .transpose()
            .map_err(|error| format!("display store: {error:?}"))?;
        let stored = preferences
            .as_ref()
            .map(DisplayStore::load)
            .transpose()
            .map_err(|error| format!("display preferences: {error:?}"))?
            .flatten();
        let (volume_store, volume) = load_volume(settings.as_deref())?;
        let display = resolve_display(defaults, stored, [
            mode.as_deref(),
            window.as_deref(),
        ])?;

        Ok(Self {
            assets,
            audio_device,
            controls: ControlSelection::open(
                control_options,
                paths.controls,
                settings.as_deref(),
                smoke,
            )?,
            display,
            host,
            measure,
            preferences,
            smoke,
            touch_enabled,
            volume,
            volume_store,
        })
    }
}

pub(super) fn edit_control(args: &[OsString]) -> Result<(), String> {
    let [flag, directory, seat, action, kind, name] = args else {
        return Err(USAGE.into());
    };
    if flag != "--edit-control" {
        return Err(USAGE.into());
    }
    let station = seat
        .to_str()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|value| value.checked_sub(1))
        .filter(|value| *value < DEFAULT_PROFILES.keys.len())
        .ok_or("invalid control seat")?;
    let action_slot = action_index(action.to_str().ok_or("invalid action")?)?;
    let binding = edit_binding(
        kind.to_str().ok_or("invalid binding kind")?,
        name.to_str().ok_or("invalid binding name")?,
    )?;
    let store = ControlStore::open(directory, profile_limit()?)
        .map_err(|error| format!("control preferences: {error:?}"))?;
    let stored = store
        .load()
        .map_err(|error| format!("control preferences: {error:?}"))?;
    let current =
        configured_profiles(stored.as_deref().unwrap_or(DEFAULT_PROFILE))?;
    let (_candidate, encoded) =
        admit_edit(&current, station, action_slot, binding)?;
    ResolvedControls::store_profile(
        &store,
        &encoded,
        PublicationPolicy::Durable,
    )
}

fn admit_edit(
    current: &Profiles,
    station: usize,
    action: usize,
    binding: DigitalBinding,
) -> Result<AdmittedEdit, String> {
    admit_candidate(&current.edited(station, action, binding)?)
}

fn admit_candidate(candidate: &Profiles) -> Result<AdmittedEdit, String> {
    validate_control_bindings(candidate)?;
    let encoded = candidate.encoded(
        ACTION_NAMES,
        MAX_PROFILE_BYTES,
        (key_name, key_from_name),
    )?;
    Ok((*candidate, encoded))
}

fn action_index(name: &str) -> Result<usize, String> {
    ACTION_NAMES
        .iter()
        .position(|action| *action == name)
        .ok_or_else(|| "unknown control action".into())
}

fn edit_binding(kind: &str, name: &str) -> Result<DigitalBinding, String> {
    let invalid = || "unknown control binding".to_owned();
    match kind {
        "button" => ControllerButton::from_profile_name(name)
            .map(DigitalBinding::Button)
            .ok_or_else(invalid),
        "key" if name == "none" => Ok(DigitalBinding::Key(None)),
        "key" => key_from_name(name)
            .map(|key| DigitalBinding::Key(Some(key)))
            .ok_or_else(invalid),
        "mouse" if name == "none" => Ok(DigitalBinding::Mouse(None)),
        "mouse" => PointerButton::from_profile_name(name)
            .map(|button| DigitalBinding::Mouse(Some(button)))
            .ok_or_else(invalid),
        "touch" => action_index(name).map(DigitalBinding::TouchSlot),
        _ => Err(invalid()),
    }
}

fn enemy_hashes(values: &mut Fields<'_>) -> Result<EnemyHashes, String> {
    match (
        values.remove("enemy_approach"),
        values.remove("enemy_strike"),
    ) {
        (None, None) => Ok(None),
        (Some(approach), Some(strike)) => {
            Ok(Some([hash(approach)?, hash(strike)?]))
        },
        _ => Err(String::from("both enemy hashes are required")),
    }
}

fn selected_audio_device(
    device: Option<String>,
    has_audio: bool,
) -> Result<Option<String>, String> {
    if device.is_some() && !has_audio {
        return Err("audio device requires an audio bundle".into());
    }
    Ok(device.filter(|name| name != "default"))
}

fn load_volume(settings: Option<&OsStr>) -> Result<VolumePreferences, String> {
    let store = settings
        .map(VolumeStore::open)
        .transpose()
        .map_err(|error| format!("volume store: {error:?}"))?;
    let volume = store
        .as_ref()
        .map(VolumeStore::load)
        .transpose()
        .map_err(|error| format!("volume preferences: {error:?}"))?
        .flatten()
        .unwrap_or_default();
    Ok((store, volume))
}

fn validate_audio_device(name: String) -> Result<String, String> {
    if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control)
    {
        return Err("invalid audio device name".into());
    }
    Ok(name)
}

fn open_control_store(
    settings: Option<&OsStr>,
    reset_controls: bool,
) -> Result<Option<ControlStore>, String> {
    if reset_controls && settings.is_none() {
        return Err(String::from(
            "reset controls requires a settings directory",
        ));
    }
    settings
        .map(|directory| {
            ControlStore::open(directory, profile_limit()?)
                .map_err(|error| format!("control store: {error:?}"))
        })
        .transpose()
}

fn resolve_display(
    defaults: DisplaySettings,
    stored: Option<DisplaySettings>,
    [mode, window]: [Option<&str>; 2],
) -> Result<DisplaySettings, String> {
    stored
        .unwrap_or(defaults)
        .configured(mode, window)
        .map_err(String::from)
}

pub(super) fn fields(text: &str) -> Result<Fields<'_>, String> {
    let mut values = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (name, value) =
            line.split_once('=').ok_or("expected launch field=value")?;
        if name.is_empty()
            || value.is_empty()
            || value.chars().any(char::is_control)
            || values.insert(name, value).is_some()
        {
            return Err(String::from("invalid or duplicate launch field"));
        }
    }
    Ok(values)
}

fn flag(args: &mut Arguments, expected: &str) -> bool {
    if args.peek().is_some_and(|value| value == expected) {
        let _flag = args.next();
        true
    } else {
        false
    }
}

fn hash(text: &str) -> Result<OsString, String> {
    let _digest: ContentHash = text
        .parse()
        .map_err(|error| format!("launch hash: {error:?}"))?;
    Ok(text.into())
}

fn measurement_flag(args: &mut Arguments, smoke: bool) -> Result<bool, String> {
    let measure = flag(args, "--measure");
    if smoke && measure {
        return Err(String::from("smoke and live measurement are exclusive"));
    }
    Ok(measure)
}

fn next(args: &mut Arguments) -> Result<OsString, String> {
    args.next().ok_or_else(|| String::from(USAGE))
}

pub(super) fn optional_catalogue_path(
    base: &OsStr,
    values: &mut Fields<'_>,
    key: &str,
) -> Result<Option<OsString>, String> {
    values
        .remove(key)
        .map(|value| resolve_local(base, value))
        .transpose()
}

fn optional_path(
    args: &mut Arguments,
    name: &str,
) -> Result<Option<OsString>, String> {
    if flag(args, name) {
        Ok(Some(next(args)?))
    } else {
        Ok(None)
    }
}

fn optional_value(
    args: &mut Arguments,
    name: &str,
) -> Result<Option<String>, String> {
    if flag(args, name) {
        next(args)?
            .into_string()
            .map(Some)
            .map_err(|_error| String::from(USAGE))
    } else {
        Ok(None)
    }
}

pub(super) fn required<'text>(
    values: &mut Fields<'text>,
    key: &str,
) -> Result<&'text str, String> {
    values
        .remove(key)
        .ok_or_else(|| format!("missing launch field: {key}"))
}

pub(super) fn resolve_local(
    base: &OsStr,
    value: &str,
) -> Result<OsString, String> {
    join_path(base, value).map_err(|error| format!("local path: {error}"))
}

fn sibling_catalogue(executable: &OsStr) -> Result<OsString, String> {
    let parent = parent_path(executable)
        .map_err(|error| format!("executable parent: {error}"))?;
    resolve_local(parent, FILE_NAME)
}

#[cfg(test)]
mod tests {

    use games::presentation::{PresentationMode, WindowMode};
    use games_platform::ir::storage::fixtures::TestRoot;

    use super::*;

    const FALLBACK_PROFILE: &str = "version=1\nkey 1 right=Q\n";
    const EXPLICIT_PROFILE: &str = "version=1\nkey 1 right=E\n";
    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[derive(Debug)]
    struct ProfileFixture(TestRoot);

    impl ProfileFixture {
        fn edit(
            &self,
            seat: &str,
            action: &str,
            binding: (&str, &str),
        ) -> Result<(), String> {
            edit_control(&[
                "--edit-control".into(),
                self.0.location().to_owned(),
                seat.into(),
                action.into(),
                binding.0.into(),
                binding.1.into(),
            ])
        }

        fn new() -> Result<Self, String> {
            let fixture =
                Self(TestRoot::new().map_err(|error| error.to_string())?);
            fixture.write(
                "launch.local",
                &format!("{}controls=fallback.txt\nsettings=.\n", catalogue()),
            )?;
            fixture.write("fallback.txt", FALLBACK_PROFILE)?;
            fixture.write("explicit.txt", EXPLICIT_PROFILE)?;
            Ok(fixture)
        }

        fn resolve(&self, flags: &[&str]) -> Result<ResolvedControls, String> {
            let args = flags
                .iter()
                .map(OsString::from)
                .chain([
                    OsString::from("--config"),
                    resolve_local(self.0.location(), "launch.local")?,
                ])
                .collect();
            Launch::parse(args)?.controls.resolve()
        }

        fn store(&self) -> Result<ControlStore, String> {
            ControlStore::open(self.0.location(), profile_limit()?)
                .map_err(|error| format!("{error:?}"))
        }

        fn write(&self, name: &str, text: &str) -> Result<(), String> {
            self.0
                .write_file(name, text.as_bytes())
                .map_err(|error| error.to_string())
        }
    }

    fn catalogue() -> String {
        format!(
            "version=1\nparent=../local assets\nmarge={DIGEST}\n\
             homer={DIGEST}\nbart={DIGEST}\nlisa={DIGEST}\n"
        )
    }

    #[test]
    fn catalogue_anchors_paths_to_its_own_directory() -> Result<(), String> {
        let path = OsStr::new("/package/simpsons-arcade.local");
        let text = format!(
            "{}poses=poses.txt\ncontrols=controls.txt\n\
             settings=preferences\naudio={DIGEST}\nworld={DIGEST}\n\
             enemy_approach={DIGEST}\nenemy_strike={DIGEST}\n\
             shadow={DIGEST}\n",
            catalogue()
        );
        let (assets, display, paths) = Assets::catalogue(path, &text)?;
        let expected = Assets {
            actors: [DIGEST, DIGEST, DIGEST, DIGEST].map(OsString::from),
            audio: Some(AudioSelection {
                base: DIGEST.into(),
                replacement: None,
            }),
            directory: OsString::from("/package/../local assets"),
            enemy: Some([DIGEST.into(), DIGEST.into()]),
            poses: Some(OsString::from("/package/poses.txt")),
            shadow: Some(DIGEST.into()),
            world: Some(DIGEST.into()),
        };
        if assets != expected
            || display != DisplaySettings::default()
            || paths.controls != Some(OsString::from("/package/controls.txt"))
            || paths.settings != Some(OsString::from("/package/preferences"))
        {
            return Err(String::from("incorrect catalogue selection"));
        }
        Ok(())
    }

    #[test]
    fn launch_restores_master_volume_without_overwriting_controls()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let args = [
            OsString::from("--config"),
            resolve_local(fixture.0.location(), "launch.local")?,
        ];
        if Launch::parse(args.clone().into())?.volume != MasterVolume::Full {
            return Err("wrong initial volume".into());
        }
        let volume = VolumeStore::open(fixture.0.location())
            .map_err(|error| format!("{error:?}"))?;
        let receipt = volume
            .store(MasterVolume::Half)
            .map_err(|error| format!("{error:?}"))?;
        receipt.durability.map_err(|error| format!("{error:?}"))?;
        let launch = Launch::parse(args.into())?;
        if launch.volume != MasterVolume::Half
            || launch.controls.resolve()?.profiles
                != configured_profiles(FALLBACK_PROFILE)?
        {
            return Err("volume or controls not restored".into());
        }
        Ok(())
    }

    #[test]
    fn audio_device_selection_is_overridable() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        fixture.write(
            "launch.local",
            &format!("{}audio={DIGEST}\naudio_device=Speakers\n", catalogue()),
        )?;
        let config_path = resolve_local(fixture.0.location(), "launch.local")?;
        let config = || [OsString::from("--config"), config_path.clone()];
        let selected = Launch::parse(config().into())?;
        if selected.audio_device.as_deref() != Some("Speakers") {
            return Err("wrong configured device".into());
        }
        let default_args = ["--audio-device", "default"]
            .map(OsString::from)
            .into_iter()
            .chain(config())
            .collect();
        if Launch::parse(default_args)?.audio_device.is_some() {
            return Err("default output override ignored".into());
        }
        let args = ["--audio-device", "Headphones"]
            .map(OsString::from)
            .into_iter()
            .chain(config())
            .collect();
        if Launch::parse(args)?.audio_device.as_deref() != Some("Headphones") {
            return Err("named output override ignored".into());
        }
        let no_audio = format!("{}audio_device=Speakers\n", catalogue());
        if Assets::catalogue(OsStr::new("local"), &no_audio).is_ok()
            || validate_audio_device(String::new()).is_ok()
            || validate_audio_device("x\0y".into()).is_ok()
            || validate_audio_device("x".repeat(513)).is_ok()
        {
            return Err("invalid audio device accepted".into());
        }
        Ok(())
    }

    #[test]
    fn catalogue_rejects_ambiguous_or_incomplete_fields() {
        let path = OsStr::new("local");
        let base = catalogue();
        for suffix in [
            "version=1\n",
            "marge=duplicate\n",
            "unknown=value\n",
            "enemy_approach=invalid\n",
            "poses=\n",
            "audio=not-a-hash\n",
            "audio_mod=not-a-hash\n",
            "shadow=not-a-hash\n",
            "world=not-a-hash\n",
        ] {
            assert!(
                Assets::catalogue(path, &format!("{base}{suffix}")).is_err()
            );
        }
        for invalid in [
            base.replace("version=1", "version=2"),
            base.replace("parent=../local assets\n", ""),
            base.replace(&format!("lisa={DIGEST}\n"), ""),
            format!("{base}{}", " ".repeat(4096)),
        ] {
            assert!(Assets::catalogue(path, &invalid).is_err());
        }
        assert!(Assets::catalogue(path, &base).is_ok());
    }

    #[test]
    fn explicit_arguments_preserve_legacy_selection() -> Result<(), String> {
        let args = [
            "--smoke", "--touch", "--poses", "poses", "--enemy", "approach",
            "strike", "--audio", "audio", "--world", "world", "parent", "m",
            "h", "b", "l",
        ];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        let expected = Assets {
            actors: ["m", "h", "b", "l"].map(OsString::from),
            audio: Some(AudioSelection {
                base: "audio".into(),
                replacement: None,
            }),
            directory: OsString::from("parent"),
            enemy: Some(["approach".into(), "strike".into()]),
            poses: Some(OsString::from("poses")),
            shadow: None,
            world: Some("world".into()),
        };
        if !launch.smoke || !launch.touch_enabled || launch.assets != expected {
            return Err(String::from("incorrect explicit selection"));
        }
        Ok(())
    }

    #[test]
    fn explicit_shadow_selection_is_optional() -> Result<(), String> {
        let args = ["--shadow", "mask", "parent", "m", "h", "b", "l"];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        if launch.assets.shadow != Some("mask".into()) {
            return Err(String::from("explicit shadow missing"));
        }
        let (assets, _, _) =
            Assets::catalogue(OsStr::new("local"), &catalogue())?;
        if assets.shadow.is_some() {
            return Err(String::from("shadow must be optional"));
        }
        Ok(())
    }

    #[test]
    fn passive_audio_replacement_requires_explicit_base_selection()
    -> Result<(), String> {
        let path = OsStr::new("local");
        let base = catalogue();
        let replacement = format!("{base}audio_mod={DIGEST}\n");
        if Assets::catalogue(path, &replacement).is_ok() {
            return Err(String::from("replacement without base accepted"));
        }
        let (configured, _, _) =
            Assets::catalogue(path, &format!("{replacement}audio={DIGEST}\n"))?;
        let selected = configured.audio.ok_or("missing audio selection")?;
        if selected.replacement != Some(DIGEST.into()) {
            return Err(String::from("catalogue lost replacement"));
        }
        let invalid = ["--audio-mod", "mod", "parent", "m", "h", "b", "l"];
        if Launch::parse(invalid.map(OsString::from).into()).is_ok() {
            return Err(String::from("explicit replacement without base"));
        }
        let valid = [
            "--audio",
            "base",
            "--audio-mod",
            "mod",
            "parent",
            "m",
            "h",
            "b",
            "l",
        ];
        let launch = Launch::parse(valid.map(OsString::from).into())?;
        if launch.assets.audio
            != Some(AudioSelection {
                base: "base".into(),
                replacement: Some("mod".into()),
            })
        {
            return Err(String::from("explicit replacement lost"));
        }
        Ok(())
    }

    #[test]
    fn launch_display_values_are_optional_strict_and_overridable()
    -> Result<(), String> {
        let path = OsStr::new("local");
        let text =
            format!("{}display=integer\nwindow=fullscreen\n", catalogue());
        let (_, settings, _) = Assets::catalogue(path, &text)?;
        let expected = DisplaySettings {
            presentation: PresentationMode::Integer,
            window: WindowMode::Fullscreen,
        };
        if settings != expected
            || settings.configured(Some("stretch"), None)?.window
                != expected.window
        {
            return Err(String::from("incorrect catalogue display settings"));
        }
        let args = [
            "--display",
            "fill",
            "--window",
            "windowed",
            "parent",
            "m",
            "h",
            "b",
            "l",
        ];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        if launch.display.presentation != PresentationMode::Fill
            || launch.display.window != WindowMode::Windowed
        {
            return Err(String::from("incorrect explicit display settings"));
        }
        for suffix in ["display=unknown\n", "window=unknown\n"] {
            if Assets::catalogue(path, &format!("{}{suffix}", catalogue()))
                .is_ok()
            {
                return Err(String::from("invalid display setting accepted"));
            }
        }
        Ok(())
    }

    #[test]
    fn live_measurement_is_explicit_and_not_a_smoke_mode() -> Result<(), String>
    {
        let args = ["--measure", "parent", "m", "h", "b", "l"];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        if !launch.measure || launch.smoke {
            return Err(String::from("live measurement flag lost"));
        }
        let conflicting =
            ["--smoke", "--measure", "parent", "m", "h", "b", "l"];
        if Launch::parse(conflicting.map(OsString::from).into()).is_ok() {
            return Err(String::from("conflicting measurement flags accepted"));
        }
        Ok(())
    }

    #[test]
    fn hosting_requires_explicit_selection_and_rejects_smoke()
    -> Result<(), String> {
        let args = ["--lan-host", "host.local", "parent", "m", "h", "b", "l"];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        if launch.host.as_deref() != Some(OsStr::new("host.local")) {
            return Err(String::from("host selection lost"));
        }
        let invalid = [
            "--smoke",
            "--lan-host",
            "host.local",
            "parent",
            "m",
            "h",
            "b",
            "l",
        ];
        if Launch::parse(invalid.map(OsString::from).into()).is_ok() {
            return Err(String::from("smoke unexpectedly opened host"));
        }
        Ok(())
    }

    #[test]
    fn implicit_path_is_executable_adjacent() {
        assert_eq!(
            sibling_catalogue(OsStr::new("/package/simpsons-arcade")),
            Ok(OsString::from("/package/simpsons-arcade.local"))
        );
        for args in [
            vec!["--config"],
            vec!["--config", "local", "extra"],
            vec!["--poses"],
            vec!["parent", "m", "h", "b"],
        ] {
            assert!(
                Launch::parse(args.into_iter().map(OsString::from).collect())
                    .is_err()
            );
        }
    }

    #[test]
    fn preferences_precedence_preserves_each_explicit_override() {
        let defaults = DisplaySettings {
            presentation: PresentationMode::Integer,
            window: WindowMode::Windowed,
        };
        let stored = DisplaySettings {
            presentation: PresentationMode::Stretch,
            window: WindowMode::Fullscreen,
        };
        assert_eq!(
            resolve_display(defaults, Some(stored), [None, None]),
            Ok(stored)
        );
        assert_eq!(
            resolve_display(defaults, Some(stored), [Some("adaptive"), None],),
            Ok(DisplaySettings {
                presentation: PresentationMode::Adaptive,
                ..stored
            })
        );
        assert_eq!(resolve_display(defaults, None, [None, None]), Ok(defaults));
    }

    #[test]
    fn stored_and_explicit_fill_remain_available_with_touch()
    -> Result<(), String> {
        let stored = DisplaySettings {
            presentation: PresentationMode::Fill,
            window: WindowMode::Fullscreen,
        };
        let defaults = DisplaySettings::default();
        if resolve_display(defaults, Some(stored), [None, None])? != stored {
            return Err(String::from("stored Fill preference lost"));
        }
        let args =
            ["--touch", "--display", "fill", "parent", "m", "h", "b", "l"];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        if !launch.touch_enabled
            || launch.display.presentation != PresentationMode::Fill
        {
            return Err(String::from("explicit Fill with touch rejected"));
        }
        Ok(())
    }
    #[test]
    fn controls_precedence_and_reset_survive_relaunch() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let expected = configured_profiles;
        let store = fixture.store()?;
        let stored = || store.load().map_err(|error| format!("{error:?}"));
        if fixture.resolve(&[])?.profiles != expected(FALLBACK_PROFILE)? {
            return Err("catalogue fallback not applied".into());
        }
        fixture.resolve(&[])?.persist()?;
        if stored()?.is_some() {
            return Err("catalogue fallback was unexpectedly persisted".into());
        }

        let explicit = resolve_local(fixture.0.location(), "explicit.txt")?;
        let path = explicit.to_str().ok_or("invalid test path")?;
        let resolved = fixture.resolve(&["--controls", path])?;
        if resolved.profiles != expected(EXPLICIT_PROFILE)?
            || stored()?.is_some()
        {
            return Err("explicit profile not selected without writing".into());
        }
        resolved.persist()?;
        if stored()?.as_deref() != Some(EXPLICIT_PROFILE)
            || fixture.resolve(&[])?.profiles != resolved.profiles
        {
            return Err("saved controls did not override catalogue".into());
        }

        let reset = fixture.resolve(&["--reset-controls"])?;
        if reset.profiles != DEFAULT_PROFILES {
            return Err("reset did not select default controls".into());
        }
        reset.persist()?;
        if stored()?.as_deref() != Some(DEFAULT_PROFILE)
            || fixture.resolve(&[])?.profiles != DEFAULT_PROFILES
        {
            return Err("reset did not survive relaunch".into());
        }
        Ok(())
    }

    #[test]
    fn interactive_edits_keep_current_profile_and_save_only_valid_candidates()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let resolved = fixture.resolve(&[])?;
        let original = resolved.profiles;
        let right = action_index("right")?;
        let primary = action_index("primary")?;
        let pause = action_index("pause")?;
        let candidate = resolved
            .persist_edit(
                &original,
                (0, right),
                DigitalBinding::Key(key_from_name("E")),
            )?
            .ok_or("valid key edit rejected")?;
        let next = resolved
            .persist_edit(
                &candidate,
                (0, primary),
                DigitalBinding::Mouse(Some(PointerButton::Right)),
            )?
            .ok_or("valid mouse edit rejected")?;
        let store = fixture.store()?;
        let before = store.load().map_err(|error| format!("{error:?}"))?;
        if resolved
            .persist_edit(&next, (0, pause), DigitalBinding::Key(None))?
            .is_some()
            || resolved
                .persist_edit(
                    &next,
                    (1, right),
                    DigitalBinding::Key(key_from_name("E")),
                )?
                .is_some()
        {
            return Err("invalid edit was admitted".into());
        }
        if store.load().map_err(|error| format!("{error:?}"))? != before
            || fixture.resolve(&[])?.profiles != next
            || resolved.profiles != original
        {
            return Err("edit changed an active or admitted profile".into());
        }
        Ok(())
    }

    #[test]
    fn axis_edit_round_trips_and_rejects_invalid_candidates()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let resolved = fixture.resolve(&[])?;
        let original = resolved.profiles;
        let profile = AxisProfile {
            negative: Some(action_index("left")?),
            positive: Some(action_index("right")?),
            press: 20_000,
            release: 8_000,
        };
        let next = resolved
            .persist_axis_edit(&original, (2, ControllerAxis::RightX), profile)?
            .ok_or("valid axis edit rejected")?;
        let expected =
            original.edited_axis(2, ControllerAxis::RightX, profile)?;
        if next != expected
            || fixture.resolve(&[])?.profiles != expected
            || resolved.profiles != original
        {
            return Err(
                "axis edit modified unrelated or active mappings".into()
            );
        }
        verify_invalid_axis_edit(&fixture, &resolved, &next, profile)
    }

    fn verify_invalid_axis_edit(
        fixture: &ProfileFixture,
        resolved: &ResolvedControls,
        next: &Profiles,
        profile: AxisProfile,
    ) -> Result<(), String> {
        let store = fixture.store()?;
        let before = store.load().map_err(|error| format!("{error:?}"))?;
        if resolved
            .persist_axis_edit(next, (2, ControllerAxis::TriggerLeft), profile)?
            .is_some()
            || resolved
                .persist_axis_edit(
                    next,
                    (2, ControllerAxis::RightX),
                    AxisProfile {
                        release: profile.press,
                        ..profile
                    },
                )?
                .is_some()
            || resolved
                .persist_axis_edit(next, (4, ControllerAxis::RightX), profile)?
                .is_some()
            || store.load().map_err(|error| format!("{error:?}"))? != before
        {
            return Err(
                "invalid axis edit changed the persisted profile".into()
            );
        }
        Ok(())
    }

    #[test]
    fn offline_edits_persist_across_relaunch() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let mut expected = DEFAULT_PROFILES;
        for (seat, action, kind, name, binding) in [
            (
                "1",
                "right",
                "key",
                "Q",
                DigitalBinding::Key(key_from_name("Q")),
            ),
            (
                "1",
                "primary",
                "mouse",
                "right",
                DigitalBinding::Mouse(Some(PointerButton::Right)),
            ),
            (
                "2",
                "primary",
                "button",
                "b",
                DigitalBinding::Button(ControllerButton::East),
            ),
            (
                "2",
                "primary",
                "touch",
                "secondary",
                DigitalBinding::TouchSlot(action_index("secondary")?),
            ),
        ] {
            fixture.edit(seat, action, (kind, name))?;
            let index = seat
                .parse::<usize>()
                .map_err(|error| error.to_string())?
                .checked_sub(1)
                .ok_or("invalid seat")?;
            expected =
                expected.edited(index, action_index(action)?, binding)?;
            if fixture.resolve(&[])?.profiles != expected {
                return Err("edited binding did not survive relaunch".into());
            }
        }
        Ok(())
    }

    #[test]
    fn offline_edits_reject_conflicts() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        fixture.edit("1", "right", ("key", "Q"))?;
        fixture.edit("1", "primary", ("mouse", "right"))?;
        let store = fixture.store()?;
        let before = store.load().map_err(|error| format!("{error:?}"))?;
        for (seat, action, kind, name) in [
            ("0", "right", "key", "Q"),
            ("5", "right", "key", "Q"),
            ("1", "pause", "key", "none"),
            ("2", "pause", "key", "E"),
            ("2", "right", "key", "Q"),
            ("2", "primary", "mouse", "right"),
            ("1", "unknown", "key", "E"),
            ("1", "right", "touch", "unknown"),
        ] {
            if fixture.edit(seat, action, (kind, name)).is_ok() {
                return Err("invalid edit was accepted".into());
            }
        }
        if before != store.load().map_err(|error| format!("{error:?}"))? {
            return Err("rejected edit changed saved controls".into());
        }
        Ok(())
    }

    #[test]
    fn reloaded_edits_route_physical_inputs() -> Result<(), String> {
        use games::input::InputEvent;

        use super::super::Controls;

        let fixture = ProfileFixture::new()?;
        fixture.edit("1", "right", ("key", "Q"))?;
        fixture.edit("1", "primary", ("mouse", "right"))?;
        fixture.edit("2", "right", ("button", "x"))?;
        fixture.edit("1", "right", ("touch", "left"))?;
        let profiles = fixture.resolve(&[])?.profiles;
        let mut controls = Controls::with_profiles(true, &profiles)
            .map_err(|error| format!("{error:?}"))?;
        for device in [1, 2] {
            let attached = controls
                .actions(InputEvent::Attached { device })
                .map_err(|error| format!("{error:?}"))?;
            if !attached.is_empty() {
                return Err("attachment generated an unexpected edge".into());
            }
        }
        verify_reloaded_edges(&mut controls)
    }

    fn verify_reloaded_edges(
        controls: &mut super::super::Controls,
    ) -> Result<(), String> {
        use games::input::touch::{ContactId, ContactPhase};
        use games::input::{InputEvent, LogicalAction};

        use super::super::{Action, Control, Player, touch};

        let point =
            touch::center(Player::Marge, Control::Left, &DEFAULT_PROFILES)
                .map_err(|error| format!("{error:?}"))?;
        let key = key_from_name("Q").ok_or("missing key")?;
        let id = ContactId { device: 1, finger: 1 };
        let mut verify =
            |events: [InputEvent; 2], player, control| -> Result<(), String> {
                for (event, down) in events.into_iter().zip([true, false]) {
                    let expected = [LogicalAction {
                        action: Action { control, player },
                        down,
                    }];
                    let actual = controls
                        .actions(event)
                        .map_err(|error| format!("{error:?}"))?;
                    (actual == expected)
                        .then_some(())
                        .ok_or("reloaded profile routed the wrong action")?;
                }
                Ok(())
            };
        verify(
            [true, false]
                .map(|down| InputEvent::Key { code: key.code(), down }),
            Player::Marge,
            Control::Right,
        )?;
        verify(
            [true, false].map(|down| InputEvent::Mouse {
                code: PointerButton::Right.code(),
                down,
            }),
            Player::Marge,
            Control::Primary,
        )?;
        verify(
            [true, false].map(|down| InputEvent::Button {
                device: 2,
                code: ControllerButton::West.code(),
                down,
            }),
            Player::Homer,
            Control::Right,
        )?;
        verify(
            [ContactPhase::Down(point), ContactPhase::Up]
                .map(|phase| InputEvent::Contact { id, phase }),
            Player::Marge,
            Control::Right,
        )
    }

    #[test]
    fn next_launch_reset_preserves_current_controls() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let resolved = fixture.resolve(&[])?;
        if !resolved.can_reset()
            || resolved.profiles != configured_profiles(FALLBACK_PROFILE)?
        {
            return Err("control reset unavailable or fallback lost".into());
        }
        resolved.persist_defaults()?;
        if fixture.resolve(&[])?.profiles != DEFAULT_PROFILES
            || resolved.profiles != configured_profiles(FALLBACK_PROFILE)?
        {
            return Err("next-launch reset changed current controls".into());
        }
        let smoke = fixture.resolve(&["--smoke"])?;
        if smoke.can_reset() || smoke.persist_defaults().is_ok() {
            return Err("smoke can reset controls".into());
        }
        Ok(())
    }

    #[test]
    fn smoke_and_invalid_controls_cannot_replace_preferences()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let store = fixture.store()?;
        let _receipt = store
            .store(FALLBACK_PROFILE)
            .map_err(|error| format!("{error:?}"))?;
        let explicit = resolve_local(fixture.0.location(), "explicit.txt")?;
        let path = explicit.to_str().ok_or("invalid test path")?;
        let smoke = fixture.resolve(&["--smoke", "--controls", path])?;
        if smoke.profiles != configured_profiles(EXPLICIT_PROFILE)? {
            return Err("smoke did not select explicit controls".into());
        }
        smoke.persist()?;
        fixture.write("explicit.txt", "version=1\nkey 1 right=Z\n")?;
        let invalid = ["--reset-controls", "parent", "m", "h", "b", "l"];
        if fixture.resolve(&["--controls", path]).is_ok()
            || fixture.resolve(&["--smoke", "--reset-controls"]).is_ok()
            || fixture
                .resolve(&["--controls", path, "--reset-controls"])
                .is_ok()
            || Launch::parse(invalid.map(OsString::from).into()).is_ok()
        {
            return Err("invalid control selection accepted".into());
        }
        if store
            .load()
            .map_err(|error| format!("{error:?}"))?
            .as_deref()
            != Some(FALLBACK_PROFILE)
        {
            return Err("invalid selection replaced saved controls".into());
        }
        Ok(())
    }

    #[test]
    fn invalid_keyboard_pause_cannot_replace_persisted_controls()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let store = fixture.store()?;
        let _receipt = store
            .store(FALLBACK_PROFILE)
            .map_err(|error| format!("{error:?}"))?;
        let explicit = resolve_local(fixture.0.location(), "explicit.txt")?;
        let path = explicit.to_str().ok_or("invalid test path")?;
        for profile in [
            "version=1\nkey 1 pause=none\n",
            "version=1\nkey 1 pause=F1\n",
            "version=1\nkey 2 pause=F1\n",
        ] {
            fixture.write("explicit.txt", profile)?;
            if fixture.resolve(&["--controls", path]).is_ok()
                || fixture.resolve(&[])?.profiles
                    != configured_profiles(FALLBACK_PROFILE)?
            {
                return Err("invalid pause replaced persisted controls".into());
            }
        }
        Ok(())
    }

    #[test]
    fn controller_controls_share_persistence_and_reset() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let fallback = resolve_local(fixture.0.location(), "fallback.txt")?;
        let explicit = resolve_local(fixture.0.location(), "explicit.txt")?;
        let path = explicit.to_str().ok_or("invalid test path")?;
        let select = |options: &[&str]| {
            let args = ["--lan-controller", "client.local"]
                .into_iter()
                .chain(options.iter().copied())
                .map(OsString::from)
                .collect();
            ControllerLaunch::parse(args)?
                .select_controls(Some(&fallback), Some(fixture.0.location()))
        };
        let expected = configured_profiles;
        if select(&[])?.profiles != expected(FALLBACK_PROFILE)? {
            return Err("controller fallback not applied".into());
        }
        let explicit_selection = select(&["--controls", path])?;
        if explicit_selection.profiles != expected(EXPLICIT_PROFILE)? {
            return Err("controller explicit profile not applied".into());
        }
        explicit_selection.persist()?;
        if select(&[])?.profiles != explicit_selection.profiles {
            return Err("controller saved profile not applied".into());
        }
        select(&["--reset-controls"])?.persist()?;
        if select(&[])?.profiles != DEFAULT_PROFILES {
            return Err("controller reset did not persist".into());
        }
        if select(&["--controls", path, "--reset-controls"]).is_ok()
            || select(&["--smoke"]).is_ok()
        {
            return Err("invalid controller controls accepted".into());
        }
        Ok(())
    }
}
