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

use games_assets::ContentHash;
use games_platform_storage::file::snapshot;

const FILE_NAME: &str = "simpsons-arcade.local";
const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(4095);
const USAGE: &str = concat!(
    "usage: simpsons-arcade [--smoke] [--touch] ",
    "[--config FILE | [--poses FILE] ",
    "[--enemy APPROACH_SHA STRIKE_SHA] [--audio SHA] ",
    "PARENT MARGE_SHA HOMER_SHA BART_SHA LISA_SHA]\n",
    "Without an asset selection, read simpsons-arcade.local beside the binary."
);

type Arguments = Peekable<vec::IntoIter<OsString>>;
type Fields<'text> = BTreeMap<&'text str, &'text str>;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Assets {
    pub(super) actors: [OsString; 4],
    pub(super) audio: Option<OsString>,
    pub(super) directory: PathBuf,
    pub(super) enemy: Option<[OsString; 2]>,
    pub(super) poses: Option<PathBuf>,
}

impl Assets {
    fn catalogue(path: &Path, text: &str) -> Result<Self, String> {
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
        let audio = values.remove("audio").map(hash).transpose()?;
        let poses = values.remove("poses").map(|value| base.join(value));
        if !values.is_empty() {
            return Err(String::from("unknown launch catalogue field"));
        }
        Ok(Self {
            actors,
            audio,
            directory,
            enemy,
            poses,
        })
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
        let audio = if flag(&mut args, "--audio") {
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
        })
    }

    fn open(path: &Path) -> Result<Self, String> {
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
    pub(super) smoke: bool,
    pub(super) touch_enabled: bool,
}

impl Launch {
    pub(super) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut remaining = args.into_iter().peekable();
        let smoke = flag(&mut remaining, "--smoke");
        let touch_enabled = flag(&mut remaining, "--touch");
        let assets = if flag(&mut remaining, "--config") {
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
            Assets::explicit(remaining)?
        };
        Ok(Self {
            assets,
            smoke,
            touch_enabled,
        })
    }
}

fn fields(text: &str) -> Result<Fields<'_>, String> {
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

fn next(args: &mut Arguments) -> Result<OsString, String> {
    args.next().ok_or_else(|| String::from(USAGE))
}

fn required<'text>(
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
            "{}poses=poses.txt\naudio={DIGEST}\n\
             enemy_approach={DIGEST}\nenemy_strike={DIGEST}\n",
            catalogue()
        );
        let assets = Assets::catalogue(path, &text)?;
        let expected = Assets {
            actors: [DIGEST, DIGEST, DIGEST, DIGEST].map(OsString::from),
            audio: Some(DIGEST.into()),
            directory: PathBuf::from("/package/../local assets"),
            enemy: Some([DIGEST.into(), DIGEST.into()]),
            poses: Some(PathBuf::from("/package/poses.txt")),
        };
        if assets != expected {
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
            "strike", "--audio", "audio", "parent", "m", "h", "b", "l",
        ];
        let launch = Launch::parse(args.map(OsString::from).into())?;
        let expected = Assets {
            actors: ["m", "h", "b", "l"].map(OsString::from),
            audio: Some("audio".into()),
            directory: PathBuf::from("parent"),
            enemy: Some(["approach".into(), "strike".into()]),
            poses: Some(PathBuf::from("poses")),
        };
        if !launch.smoke || !launch.touch_enabled || launch.assets != expected {
            return Err(String::from("incorrect explicit selection"));
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
            vec!["parent", "m", "h", "b"],
        ] {
            assert!(
                Launch::parse(args.into_iter().map(OsString::from).collect())
                    .is_err()
            );
        }
    }
}
