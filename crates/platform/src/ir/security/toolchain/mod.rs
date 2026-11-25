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
//   - Admission and fixed argument layouts for authored Cargo requests.
// - Must-Not:
//   - Execute programs, inspect paths or select an operating system.
// - Allows:
//   - Inputs: Package, binary and feature identifiers; selected target.
//   - Outputs: Validated arguments or explicit identifier rejection.
//   - Side effects: None.
// - Split-When:
//   - A different build tool needs a distinct request grammar.
// - Merge-When:
//   - Another shared contract owns identical Cargo argument admission.
// - Summary:
//   - Admit toolchain requests before native process dispatch.
// - Description:
//   - Lock the package, binary, features and target across operations.
// - Usage:
//   - Supply identifiers and a target from the trusted host catalogue.
// - Defaults:
//   - Reject empty names, option-like names and path components.

//! Host-independent authored Cargo argument admission.

/// The request contains an invalid package, binary or feature identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidIdentifier;

/// Admitted fixed Cargo request arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CargoArguments<'name> {
    binary: &'name str,
    feature: &'name str,
    package: &'name str,
    target: &'static str,
}

impl<'name> CargoArguments<'name> {
    /// Admitted artifact binary identifier.
    #[must_use]
    pub const fn binary(self) -> &'name str {
        self.binary
    }

    /// Locked release-build arguments for the selected binary.
    #[must_use]
    pub const fn build(self) -> [&'name str; 11] {
        [
            "build",
            "--locked",
            "--release",
            "--target",
            self.target,
            "--package",
            self.package,
            "--bin",
            self.binary,
            "--features",
            self.feature,
        ]
    }

    /// Locked metadata arguments for the same target and feature selection.
    #[must_use]
    pub const fn metadata(self) -> [&'name str; 9] {
        [
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            self.target,
            "--features",
            self.feature,
            "--manifest-path",
        ]
    }

    /// Admit names before constructing the fixed request.
    ///
    /// The target comes from the trusted host catalogue, not user arguments.
    ///
    /// # Errors
    /// Rejects invalid package, binary or feature names.
    pub fn new(
        package: &'name str,
        binary: &'name str,
        feature: &'name str,
        target: &'static str,
    ) -> Result<Self, InvalidIdentifier> {
        if ![package, binary, feature, target]
            .into_iter()
            .all(identifier)
        {
            return Err(InvalidIdentifier);
        }
        Ok(Self {
            binary,
            feature,
            package,
            target,
        })
    }
}

fn identifier(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_arguments_reject_options_and_paths_in_every_field() {
        for invalid in ["", "-option", "../game", "game/bin", "a b", "a,b"] {
            assert_eq!(
                CargoArguments::new(invalid, "game", "preview", "target"),
                Err(InvalidIdentifier)
            );
            assert_eq!(
                CargoArguments::new("game", invalid, "preview", "target"),
                Err(InvalidIdentifier)
            );
            assert_eq!(
                CargoArguments::new("game", "game", invalid, "target"),
                Err(InvalidIdentifier)
            );
            assert_eq!(
                CargoArguments::new("game", "game", "preview", invalid),
                Err(InvalidIdentifier)
            );
        }
    }

    #[test]
    fn requests_reuse_exact_admitted_target_and_feature() -> Result<(), String>
    {
        let args = CargoArguments::new(
            "games-example",
            "example",
            "native-preview",
            "selected-target",
        )
        .map_err(|error| format!("{error:?}"))?;
        let build = args.build();
        let metadata = args.metadata();
        if build
            != [
                "build",
                "--locked",
                "--release",
                "--target",
                "selected-target",
                "--package",
                "games-example",
                "--bin",
                "example",
                "--features",
                "native-preview",
            ]
            || metadata
                != [
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                    "--filter-platform",
                    "selected-target",
                    "--features",
                    "native-preview",
                    "--manifest-path",
                ]
            || args.binary() != "example"
        {
            return Err(String::from("admitted request changed"));
        }
        Ok(())
    }
}
