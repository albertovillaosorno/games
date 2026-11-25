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

use std::ffi::OsString;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::str;

use games_platform::ir::storage::file::snapshot_secret;
use games_platform::ir::storage::layout::Layout;
use games_platform::ir::toolchain::{self, CargoTitle, Target};

pub(crate) const NATIVE_TARGET: Target = Target::LinuxX86_64Gnu;
const PREVIEW_FEATURE: &str = "native-preview";

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

    pub(crate) fn compile(self, root: &Path) -> Result<PathBuf, String> {
        self.request()?
            .compile(root.as_os_str())
            .map(PathBuf::from)
            .map_err(|error| error.to_string())
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

    /// Build the release executable using the selected host toolchain.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported host or an unsuccessful build.
    pub fn run(self) -> Result<(), String> {
        let executable = self.compile(&workspace_root()?)?;
        toolchain::write_line(&format!("Executable: {}", executable.display()))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn local_layout(root: &Path) -> Result<Layout, String> {
    let maximum = NonZeroU64::MIN.saturating_add(65_535);
    let config = snapshot_secret(root.join(".env").as_os_str(), maximum)
        .map_err(|error| format!("local configuration: {error:?}"))?;
    let text =
        str::from_utf8(&config).map_err(|_error| "invalid .env encoding")?;
    Layout::parse(text).map_err(str::to_owned)
}

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    toolchain::workspace_root()
        .map(PathBuf::from)
        .map_err(|error| error.to_string())
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
        for target in ["android-arm64", "wasm", "--workspace", "../target"] {
            assert!(Title::parse(title, &[target.into()]).is_err());
        }
        assert!(Title::parse("../simpsons-arcade", &[]).is_err());
        assert!(
            Title::parse(title, &["linux-x86-64".into(), "extra".into()])
                .is_err()
        );
    }
}
