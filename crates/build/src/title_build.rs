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
use std::io::{Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const NATIVE_TARGET: &str = "x86_64-unknown-linux-gnu";
pub(crate) const PREVIEW_FEATURE: &str = "native-preview";

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

    const fn cargo_args(self) -> [&'static str; 11] {
        let package = match self {
            Self::SimpsonsArcade => "games-simpsons-arcade",
        };
        [
            "build",
            "--locked",
            "--release",
            "--target",
            NATIVE_TARGET,
            "--package",
            package,
            "--bin",
            self.binary(),
            "--features",
            PREVIEW_FEATURE,
        ]
    }

    pub(crate) fn compile(self, root: &Path) -> Result<PathBuf, String> {
        if !cfg!(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "gnu"
        )) {
            return Err(String::from("requires an x86-64 GNU/Linux host"));
        }
        let target = root.join("target");
        let status = Command::new("cargo")
            .current_dir(root)
            .args(self.cargo_args())
            .arg("--target-dir")
            .arg(&target)
            .status()
            .map_err(|error| format!("launch Cargo: {error}"))?;
        if !status.success() {
            return Err(format!("title compilation failed: {status}"));
        }
        let executable = target
            .join(NATIVE_TARGET)
            .join("release")
            .join(self.binary());
        if !executable.is_file() {
            return Err(String::from("compiled title executable is missing"));
        }
        Ok(executable)
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
            [target] if target == "linux-x86-64" => Ok(selected),
            _ => Err(String::from("expected optional target linux-x86-64")),
        }
    }

    /// Build the release executable using the selected host toolchain.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported host or an unsuccessful build.
    pub fn run(self) -> Result<(), String> {
        let executable = self.compile(&workspace_root()?)?;
        writeln!(stdout().lock(), "Executable: {}", executable.display())
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| format!("workspace path: {error}"))
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
