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

use games::presentation::{DisplaySettings, PresentationMode};
use games_assets::ContentHash;
use games_platform_storage::file::snapshot;
use games_platform_storage::preferences::DisplayStore;

const FILE_NAME: &str = "simpsons-arcade.local";
const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(4095);
const USAGE: &str = concat!(
    "usage: simpsons-arcade [--smoke | --measure] [--touch] ",
    "[--display MODE] [--window MODE] [--settings DIR] [--controls FILE] ",
    "[--lan-host FILE] ",
    "[--config FILE | [--poses FILE] ",
    "[--enemy APPROACH_SHA STRIKE_SHA] [--audio SHA [--audio-mod SHA]] ",
    "[--world SHA] ",
    "PARENT MARGE_SHA HOMER_SHA BART_SHA LISA_SHA]\n",
    "Without an asset selection, read simpsons-arcade.local ",
    "beside the binary.\n",
    "usage: simpsons-arcade --lan-controller FILE"
);

type Arguments = Peekable<vec::IntoIter<OsString>>;
type ConfiguredAssets = (Assets, DisplaySettings, LocalPaths);
pub(super) type Fields<'text> = BTreeMap<&'text str, &'text str>;

#[derive(Debug, Default, Eq, PartialEq)]
struct LocalPaths {
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
        let enemy = match (
            values.remove("enemy_approach"),
            values.remove("enemy_strike"),
        ) {
            (None, None) => None,
            (Some(approach), Some(strike)) => {
                Some([hash(approach)?, hash(strike)?])
            },
            _ => return Err(String::from("both enemy hashes are required")),
        };
        let audio = AudioSelection::new(
            values.remove("audio").map(hash).transpose()?,
            values.remove("audio_mod").map(hash).transpose()?,
        )?;
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
                world,
            },
            display,
            LocalPaths { controls, settings },
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
}

#[derive(Debug)]
pub(super) struct Launch {
    pub(super) assets: Assets,
    pub(super) controls: Option<PathBuf>,
    pub(super) display: DisplaySettings,
    pub(super) host: Option<PathBuf>,
    pub(super) measure: bool,
    pub(super) preferences: Option<DisplayStore>,
    pub(super) smoke: bool,
    pub(super) touch_enabled: bool,
}

impl Launch {
    pub(super) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut remaining = args.into_iter().peekable();
        let smoke = flag(&mut remaining, "--smoke");
        let measure = measurement_flag(&mut remaining, smoke)?;
        let touch_enabled = flag(&mut remaining, "--touch");
        let mode = optional_value(&mut remaining, "--display")?;
        let window = optional_value(&mut remaining, "--window")?;
        let explicit_settings = optional_path(&mut remaining, "--settings")?;
        let explicit_controls = optional_path(&mut remaining, "--controls")?;
        let host = optional_path(&mut remaining, "--lan-host")?;
        if smoke && host.is_some() {
            return Err(String::from("smoke cannot open a LAN host"));
        }
        let (assets, defaults, paths) = if flag(&mut remaining, "--config") {
            let config = PathBuf::from(next(&mut remaining)?);
            if remaining.next().is_some() {
                return Err(String::from(USAGE));
            }
            Assets::open(&config)?
        } else if remaining.peek().is_none() {
            let executable = env::current_exe()
                .map_err(|error| format!("executable path: {error}"))?;
            Assets::open(&sibling_catalogue(&executable)?)?
        } else {
            (
                Assets::explicit(remaining)?,
                DisplaySettings::default(),
                LocalPaths::default(),
            )
        };
        let preferences = explicit_settings
            .or(paths.settings)
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
        let display = resolve_display(
            defaults,
            stored,
            [mode.as_deref(), window.as_deref()],
            touch_enabled,
        )?;

        Ok(Self {
            assets,
            controls: explicit_controls.or(paths.controls),
            display,
            host,
            measure,
            preferences,
            smoke,
            touch_enabled,
        })
    }
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
    use games::presentation::WindowMode;

    use super::*;

    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
             enemy_approach={DIGEST}\nenemy_strike={DIGEST}\n",
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
            world: Some("world".into()),
        };
        if !launch.smoke || !launch.touch_enabled || launch.assets != expected {
            return Err(String::from("incorrect explicit selection"));
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
}
