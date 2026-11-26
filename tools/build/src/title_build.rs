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
//   - Selection and compilation of independently authored title binaries.
// - Must-Not:
//   - Compile, execute or distribute original game software or assets.
// - Allows:
//   - Inputs: An admitted title identifier and an optional target identifier.
//   - Outputs: The compiled native executable path or a build error.
//   - Side effects: Invokes Cargo for a selected workspace package.
// - Split-When:
//   - Target-specific packaging needs its own independent workflow.
// - Merge-When:
//   - Another host tool duplicates title selection or compilation.
// - Summary:
//   - Build only the selected title and its required native features.
// - Description:
//   - Fixed package and target selections never become arbitrary Cargo flags.
// - Usage:
//   - games-build build simpsons-arcade [linux-x86-64]
// - Defaults:
//   - Native GNU/Linux x86-64 release executable, no asset publication.

//! Host-side selection of authored title executables.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
use std::str;

use games_platform::ir::storage::file::snapshot_secret;
use games_platform::ir::storage::join_path;
use games_platform::ir::storage::layout::Layout;
use games_platform::ir::toolchain::{self, CargoTitle, Target};

pub(crate) const NATIVE_TARGET: Target = Target::LinuxX86_64Gnu;
const PREVIEW_FEATURE: &str = "native-preview";
const BROWSER_FEATURE: &str = "browser-preview";
const BROWSER_BINARY: &str = "simpsons-arcade-browser-preview";
pub(crate) const BROWSER_MODULE: &str = "game_bg.wasm";
pub(crate) const BROWSER_SCRIPT: &str = "game.js";
pub(crate) const BROWSER_INDEX: &str = "index.html";

/// Buildable title in the current workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Title {
    /// Four-player arcade movement preview.
    SimpsonsArcade,
}

impl Title {
    pub(crate) const fn binary(self) -> &'static str {
        match self {
            Self::SimpsonsArcade => "simpsons-arcade",
        }
    }

    pub(crate) const fn manifest(self) -> &'static str {
        match self {
            Self::SimpsonsArcade => "crates/games/simpsons-arcade/Cargo.toml",
        }
    }

    /// Admit a known title and the optional native target.
    ///
    /// # Errors
    ///
    /// Rejects unknown titles, targets and extra arguments.
    pub fn parse(title: &str, args: &[OsString]) -> Result<Self, String> {
        let selected = match title {
            "simpsons-arcade" => Self::SimpsonsArcade,
            _ => return Err(format!("unsupported title: {title}")),
        };
        match args {
            [] => Ok(selected),
            [target] if target == NATIVE_TARGET.name() => Ok(selected),
            [target]
                if Target::portable_checks()
                    .iter()
                    .any(|candidate| target == candidate.name()) =>
            {
                Err(format!(
                    "target {} is unavailable for distribution; core only",
                    target.to_string_lossy()
                ))
            },
            _ => Err(String::from("expected optional target linux-x86-64")),
        }
    }

    pub(crate) fn request(self) -> Result<CargoTitle<'static>, String> {
        let package = match self {
            Self::SimpsonsArcade => "games-simpsons-arcade",
        };
        CargoTitle::new(package, self.binary(), PREVIEW_FEATURE, NATIVE_TARGET)
            .map_err(|error| error.to_string())
    }
}

/// A selected authored title and one of its supported distributions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Distribution {
    target: Target,
    title: Title,
}

impl Distribution {
    pub(crate) const fn binary(self) -> &'static str {
        if self.is_browser() {
            BROWSER_MODULE
        } else {
            self.title.binary()
        }
    }

    pub(crate) const fn is_browser(self) -> bool {
        matches!(self.target, Target::WebAssembly)
    }

    pub(crate) const fn native(title: Title) -> Self {
        Self {
            target: NATIVE_TARGET,
            title,
        }
    }

    pub(crate) fn new(title: Title, target: Target) -> Result<Self, String> {
        if target == NATIVE_TARGET {
            Ok(Self::native(title))
        } else if target == Target::WebAssembly {
            Ok(Self { target, title })
        } else {
            Err(format!(
                "target {} is unavailable for distribution; core only",
                target.name()
            ))
        }
    }

    /// Admit a title and optional supported target identifier.
    /// # Errors
    /// Rejects unknown titles, targets and additional arguments.
    pub fn parse(title: &str, args: &[OsString]) -> Result<Self, String> {
        let selected = Title::parse(title, &[])?;
        let target = match args {
            [] => NATIVE_TARGET,
            [name] => Target::portable_checks()
                .into_iter()
                .find(|target| name == target.name())
                .ok_or(
                    "expected optional target linux-x86-64 or webassembly",
                )?,
            _ => return Err("expected one optional target".into()),
        };
        Self::new(selected, target)
    }

    pub(crate) fn request(self) -> Result<CargoTitle<'static>, String> {
        if !self.is_browser() {
            return self.title.request();
        }
        let package = match self.title {
            Title::SimpsonsArcade => "games-simpsons-arcade",
        };
        CargoTitle::new(package, BROWSER_BINARY, BROWSER_FEATURE, self.target)
            .map_err(|error| error.to_string())
    }

    /// Compile the selected release artifact.
    /// # Errors
    /// Rejects unsupported hosts and unsuccessful builds.
    pub fn run(self) -> Result<(), String> {
        let executable = self
            .request()?
            .compile(&workspace_root()?)
            .map_err(|error| error.to_string())?;
        let kind = if self.is_browser() {
            "Unbound Wasm module"
        } else {
            "Executable"
        };
        toolchain::write_line(&format!(
            "{kind}: {}",
            executable.to_string_lossy()
        ))
        .map_err(|error| error.to_string())
    }

    pub(crate) const fn target(self) -> Target {
        self.target
    }

    pub(crate) const fn title(self) -> Title {
        self.title
    }
}

pub(crate) fn local_path(
    base: &OsStr,
    child: &str,
) -> Result<OsString, String> {
    join_path(base, child).map_err(|error| error.to_string())
}

pub(crate) fn local_layout(root: &OsStr) -> Result<Layout, String> {
    let maximum = NonZeroU64::MIN.saturating_add(65_535);
    let config = snapshot_secret(&local_path(root, ".env")?, maximum)
        .map_err(|error| format!("local configuration: {error:?}"))?;
    let text =
        str::from_utf8(&config).map_err(|_error| "invalid .env encoding")?;
    Layout::parse(text).map_err(str::to_owned)
}

pub(crate) fn workspace_root() -> Result<OsString, String> {
    toolchain::workspace_root().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_title_defaults_to_native_and_rejects_unsupported_targets() {
        let title = "simpsons-arcade";
        assert_eq!(Title::parse(title, &[]), Ok(Title::SimpsonsArcade));
        assert_eq!(
            Title::parse(title, &["linux-x86-64".into()]),
            Ok(Title::SimpsonsArcade)
        );
        for target in Target::portable_checks() {
            if target == NATIVE_TARGET {
                continue;
            }
            let name = target.name();
            assert_eq!(
                Title::parse(title, &[name.into()]),
                Err(format!(
                    "target {name} is unavailable for distribution; core only"
                ))
            );
        }
        for target in ["wasm", "--workspace", "../target"] {
            assert!(Title::parse(title, &[target.into()]).is_err());
        }
        assert!(Title::parse("../simpsons-arcade", &[]).is_err());
        assert!(
            Title::parse(title, &["linux-x86-64".into(), "extra".into()])
                .is_err()
        );
    }
}
