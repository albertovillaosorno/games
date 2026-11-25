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
use std::ffi::OsString;
use std::iter::Peekable;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::{env, str, vec};

use games::audio::MasterVolume;
use games::presentation::{DisplaySettings, PresentationMode};
use games_assets::ContentHash;
use games_platform_storage::file::snapshot;
use games_platform_storage::preferences::{
    ControlStore, DisplayStore, VolumeStore,
};

use super::{
    ACTION_NAMES, DEFAULT_PROFILES, Profiles, profile_limit, read_profile,
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
    "usage: simpsons-arcade --list-audio-devices"
);

type Arguments = Peekable<vec::IntoIter<OsString>>;
type ConfiguredAssets = (Assets, DisplaySettings, LocalPaths);
type EnemyHashes = Option<[OsString; 2]>;
type VolumePreferences = (Option<VolumeStore>, MasterVolume);
pub(super) type Fields<'text> = BTreeMap<&'text str, &'text str>;

#[derive(Debug, Default, Eq, PartialEq)]
struct LocalPaths {
    audio_device: Option<String>,
    controls: Option<PathBuf>,
    settings: Option<PathBuf>,
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

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Assets {
    pub(super) actors: [OsString; 4],
    pub(super) audio: Option<AudioSelection>,
    pub(super) directory: PathBuf,
    pub(super) enemy: Option<[OsString; 2]>,
    pub(super) poses: Option<PathBuf>,
    pub(super) shadow: Option<OsString>,
    pub(super) world: Option<OsString>,
}

impl Assets {
    fn catalogue(path: &Path, text: &str) -> Result<ConfiguredAssets, String> {
        if u64::try_from(text.len()).map_or(true, |len| len > MAX_BYTES.get()) {
            return Err(String::from("launch catalogue exceeds byte limit"));
        }
        let base = path.parent().ok_or("invalid launch catalogue path")?;
        let mut values = fields(text)?;
        if values.remove("version") != Some("1") {
            return Err(String::from("expected launch catalogue version=1"));
        }
        let directory = base.join(required(&mut values, "parent")?);
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
        let poses = values.remove("poses").map(|value| base.join(value));
        let controls = values.remove("controls").map(|value| base.join(value));
        let settings = values.remove("settings").map(|value| base.join(value));
        let display = DisplaySettings::default()
            .configured(values.remove("display"), values.remove("window"))?;
        if !values.is_empty() {
            return Err(String::from("unknown launch catalogue field"));
        }
        Ok((
            Self {
                actors,
                audio,
                directory,
                enemy,
                poses,
                shadow,
                world,
            },
            display,
            LocalPaths {
                audio_device,
                controls,
                settings,
            },
        ))
    }

    fn explicit(mut args: Arguments) -> Result<Self, String> {
        let poses = if flag(&mut args, "--poses") {
            Some(PathBuf::from(next(&mut args)?))
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
            directory: directory.into(),
            enemy,
            poses,
            shadow,
            world,
        })
    }

    fn open(path: &Path) -> Result<ConfiguredAssets, String> {
        let bytes = snapshot(path, MAX_BYTES).map_err(|error| {
            format!("launch catalogue {}: {error:?}", path.display())
        })?;
        let text = str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        Self::catalogue(path, text)
    }

    fn select(mut remaining: Arguments) -> Result<ConfiguredAssets, String> {
        if flag(&mut remaining, "--config") {
            let config = PathBuf::from(next(&mut remaining)?);
            if remaining.next().is_some() {
                return Err(String::from(USAGE));
            }
            Self::open(&config)
        } else if remaining.peek().is_none() {
            let executable = env::current_exe()
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
    explicit: Option<PathBuf>,
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
    explicit: Option<PathBuf>,
    fallback: Option<PathBuf>,
    reset: bool,
    smoke: bool,
    store: Option<ControlStore>,
}

impl ControlSelection {
    fn open(
        options: ControlOptions,
        fallback: Option<PathBuf>,
        settings: Option<&Path>,
        smoke: bool,
    ) -> Result<Self, String> {
        let store = open_control_store(settings, options.reset)?;
        Ok(Self {
            explicit: options.explicit,
            fallback,
            reset: options.reset,
            smoke,
            store,
        })
    }

    pub(super) fn resolve(self) -> Result<ResolvedControls, String> {
        let Self {
            explicit,
            fallback,
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
        let profiles = DEFAULT_PROFILES.configured(ACTION_NAMES, &text)?;
        let [_, other @ ..] = profiles.keys;
        if other.iter().any(|[pause, ..]| pause.is_some()) {
            return Err("keyboard pause belongs to seat 1 only".into());
        }
        let pending = if update && !smoke {
            store.map(|preferences| (preferences, text))
        } else {
            None
        };
        Ok(ResolvedControls { pending, profiles })
    }
}

#[derive(Debug)]
pub(super) struct ResolvedControls {
    pending: Option<(ControlStore, String)>,
    pub(super) profiles: Profiles,
}

impl ResolvedControls {
    pub(super) fn persist(&self) -> Result<(), String> {
        if let Some((store, text)) = &self.pending {
            let receipt = store
                .store(text)
                .map_err(|error| format!("control preferences: {error:?}"))?;
            receipt.durability.map_err(|error| {
                format!("control preferences durability: {error:?}")
            })?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct ControllerLaunch {
    pub(super) catalogue: PathBuf,
    options: ControlOptions,
    settings: Option<PathBuf>,
}

impl ControllerLaunch {
    pub(super) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut remaining = args.into_iter().peekable();
        if !flag(&mut remaining, "--lan-controller") {
            return Err(String::from(USAGE));
        }
        let catalogue = PathBuf::from(next(&mut remaining)?);
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
        fallback: Option<&Path>,
        settings: Option<&Path>,
    ) -> Result<ResolvedControls, String> {
        let directory = self.settings.as_deref().or(settings);
        ControlSelection::open(
            self.options,
            fallback.map(Path::to_path_buf),
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
    pub(super) host: Option<PathBuf>,
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
        let display = resolve_display(
            defaults,
            stored,
            [mode.as_deref(), window.as_deref()],
            touch_enabled,
        )?;

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

fn load_volume(settings: Option<&Path>) -> Result<VolumePreferences, String> {
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
    settings: Option<&Path>,
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
    touch_enabled: bool,
) -> Result<DisplaySettings, String> {
    let mut base = stored.unwrap_or(defaults);
    if touch_enabled
        && stored.is_some()
        && base.presentation == PresentationMode::Fill
    {
        base.presentation = PresentationMode::AspectFit;
    }
    let display = base.configured(mode, window)?;
    if touch_enabled && display.presentation == PresentationMode::Fill {
        return Err(String::from(
            "fill can crop touch controls; choose another display mode",
        ));
    }
    Ok(display)
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

fn optional_path(
    args: &mut Arguments,
    name: &str,
) -> Result<Option<PathBuf>, String> {
    if flag(args, name) {
        Ok(Some(PathBuf::from(next(args)?)))
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

fn sibling_catalogue(executable: &Path) -> Result<PathBuf, String> {
    executable
        .parent()
        .map(|parent| parent.join(FILE_NAME))
        .ok_or_else(|| String::from("executable has no parent directory"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use games::presentation::WindowMode;
    use games_platform_storage::fixtures::TestRoot;

    use super::*;

    const FALLBACK_PROFILE: &str = "version=1\nkey 1 right=Q\n";
    const EXPLICIT_PROFILE: &str = "version=1\nkey 1 right=E\n";
    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[derive(Debug)]
    struct ProfileFixture(TestRoot);

    impl ProfileFixture {
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
                    self.0.path().join("launch.local").into_os_string(),
                ])
                .collect();
            Launch::parse(args)?.controls.resolve()
        }

        fn store(&self) -> Result<ControlStore, String> {
            ControlStore::open(self.0.path(), profile_limit()?)
                .map_err(|error| format!("{error:?}"))
        }

        fn write(&self, name: &str, text: &str) -> Result<(), String> {
            fs::write(self.0.path().join(name), text)
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
        let path = Path::new("/package/simpsons-arcade.local");
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
            directory: PathBuf::from("/package/../local assets"),
            enemy: Some([DIGEST.into(), DIGEST.into()]),
            poses: Some(PathBuf::from("/package/poses.txt")),
            shadow: Some(DIGEST.into()),
            world: Some(DIGEST.into()),
        };
        if assets != expected
            || display != DisplaySettings::default()
            || paths.controls != Some(PathBuf::from("/package/controls.txt"))
            || paths.settings != Some(PathBuf::from("/package/preferences"))
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
            fixture.0.path().join("launch.local").into_os_string(),
        ];
        if Launch::parse(args.clone().into())?.volume != MasterVolume::Full {
            return Err("wrong initial volume".into());
        }
        let volume = VolumeStore::open(fixture.0.path())
            .map_err(|error| format!("{error:?}"))?;
        let receipt = volume
            .store(MasterVolume::Half)
            .map_err(|error| format!("{error:?}"))?;
        receipt.durability.map_err(|error| format!("{error:?}"))?;
        let launch = Launch::parse(args.into())?;
        if launch.volume != MasterVolume::Half
            || launch.controls.resolve()?.profiles
                != DEFAULT_PROFILES
                    .configured(ACTION_NAMES, FALLBACK_PROFILE)?
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
        let config = || {
            [
                OsString::from("--config"),
                fixture.0.path().join("launch.local").into_os_string(),
            ]
        };
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
        if Assets::catalogue(Path::new("local"), &no_audio).is_ok()
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
        let path = Path::new("local");
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
            directory: PathBuf::from("parent"),
            enemy: Some(["approach".into(), "strike".into()]),
            poses: Some(PathBuf::from("poses")),
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
            Assets::catalogue(Path::new("local"), &catalogue())?;
        if assets.shadow.is_some() {
            return Err(String::from("shadow must be optional"));
        }
        Ok(())
    }

    #[test]
    fn passive_audio_replacement_requires_explicit_base_selection()
    -> Result<(), String> {
        let path = Path::new("local");
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
        let path = Path::new("local");
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
        if launch.host.as_deref() != Some(Path::new("host.local")) {
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
            sibling_catalogue(Path::new("/package/simpsons-arcade")),
            Ok(PathBuf::from("/package/simpsons-arcade.local"))
        );
        for args in [
            vec!["--config"],
            vec!["--config", "local", "extra"],
            vec!["--poses"],
            vec!["--touch", "--display", "fill", "parent", "m", "h", "b", "l"],
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
            resolve_display(defaults, Some(stored), [None, None], false),
            Ok(stored)
        );
        assert_eq!(
            resolve_display(
                defaults,
                Some(stored),
                [Some("adaptive"), None],
                false,
            ),
            Ok(DisplaySettings {
                presentation: PresentationMode::Adaptive,
                ..stored
            })
        );
        assert_eq!(
            resolve_display(defaults, None, [None, None], false),
            Ok(defaults)
        );
    }

    #[test]
    fn stored_fill_can_open_touch_without_accepting_explicit_fill() {
        let stored = DisplaySettings {
            presentation: PresentationMode::Fill,
            window: WindowMode::Fullscreen,
        };
        let defaults = DisplaySettings::default();
        assert_eq!(
            resolve_display(defaults, Some(stored), [None, None], true),
            Ok(DisplaySettings {
                presentation: PresentationMode::AspectFit,
                ..stored
            })
        );
        assert!(
            resolve_display(defaults, Some(stored), [Some("fill"), None], true)
                .is_err()
        );
    }
    #[test]
    fn controls_precedence_and_reset_survive_relaunch() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let expected = |text| DEFAULT_PROFILES.configured(ACTION_NAMES, text);
        let store = fixture.store()?;
        let stored = || store.load().map_err(|error| format!("{error:?}"));
        if fixture.resolve(&[])?.profiles != expected(FALLBACK_PROFILE)? {
            return Err("catalogue fallback not applied".into());
        }
        fixture.resolve(&[])?.persist()?;
        if stored()?.is_some() {
            return Err("catalogue fallback was unexpectedly persisted".into());
        }

        let explicit = fixture.0.path().join("explicit.txt");
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
    fn smoke_and_invalid_controls_cannot_replace_preferences()
    -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let store = fixture.store()?;
        let _receipt = store
            .store(FALLBACK_PROFILE)
            .map_err(|error| format!("{error:?}"))?;
        let explicit = fixture.0.path().join("explicit.txt");
        let path = explicit.to_str().ok_or("invalid test path")?;
        let smoke = fixture.resolve(&["--smoke", "--controls", path])?;
        if smoke.profiles
            != DEFAULT_PROFILES.configured(ACTION_NAMES, EXPLICIT_PROFILE)?
        {
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
    fn controller_controls_share_persistence_and_reset() -> Result<(), String> {
        let fixture = ProfileFixture::new()?;
        let fallback = fixture.0.path().join("fallback.txt");
        let explicit = fixture.0.path().join("explicit.txt");
        let path = explicit.to_str().ok_or("invalid test path")?;
        let select = |options: &[&str]| {
            let args = ["--lan-controller", "client.local"]
                .into_iter()
                .chain(options.iter().copied())
                .map(OsString::from)
                .collect();
            ControllerLaunch::parse(args)?
                .select_controls(Some(&fallback), Some(fixture.0.path()))
        };
        let expected = |text| DEFAULT_PROFILES.configured(ACTION_NAMES, text);
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
