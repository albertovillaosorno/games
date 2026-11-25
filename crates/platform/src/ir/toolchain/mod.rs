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
//   - Portable authored toolchain requests and capability dispatch.
// - Must-Not:
//   - Execute subprocesses, inspect host paths or select an operating system.
// - Allows:
//   - Inputs: Admitted Cargo identifiers, targets and workspace path values.
//   - Outputs: Artifact paths, metadata, versions or typed failures.
//   - Side effects: Only explicit dispatch to the selected private adapter.
// - Split-When:
//   - A contributor capability needs independent admission and lifecycle.
// - Merge-When:
//   - Another contract owns identical toolchain requests and dispatch.
// - Summary:
//   - Host-independent contributor toolchain interface.
// - Description:
//   - Admit fixed requests before delegating native operations.
// - Usage:
//   - Request a selected target and an independently authored package.
// - Defaults:
//   - Locked requests and explicit unavailable capability failures.

//! Portable contributor toolchain requests and outcomes.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Error;

use super::security::toolchain::CargoArguments;
pub use super::security::toolchain::{ToolVersion, ValidationTool};

/// Authored executable build target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    /// GNU/Linux x86-64 distribution target.
    LinuxX86_64Gnu,
}

impl Target {
    /// Whether the selected toolchain supports this distribution target.
    #[must_use]
    pub fn available(self) -> bool {
        <() as ToolchainFactory>::available(self)
    }

    /// User-facing target identifier.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LinuxX86_64Gnu => "linux-x86-64",
        }
    }

    /// Cargo target triple and artifact subdirectory.
    #[must_use]
    pub const fn triple(self) -> &'static str {
        match self {
            Self::LinuxX86_64Gnu => "x86_64-unknown-linux-gnu",
        }
    }
}

/// Failure to execute or verify an authored toolchain operation.
#[derive(Debug)]
pub enum ToolchainError {
    /// The tool exited unsuccessfully.
    Failed {
        /// Operation that failed.
        operation: &'static str,
        /// Exit status or captured diagnostic.
        diagnostic: String,
    },
    /// The requested package, binary or feature was not an identifier.
    InvalidIdentifier,
    /// The host failed to start the requested operation.
    Io {
        /// Operation that could not start.
        operation: &'static str,
        /// Host error value.
        source: Error,
    },
    /// The tool did not produce the requested regular executable.
    MissingArtifact,
    /// The requested toolchain capability is unavailable.
    UnsupportedHost,
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { operation, diagnostic } => {
                write!(f, "{operation} failed: {diagnostic}")
            },
            Self::InvalidIdentifier => f.write_str(
                "expected Cargo package, binary and feature identifiers",
            ),
            Self::Io { operation, source } => {
                write!(f, "{operation}: {source}")
            },
            Self::MissingArtifact => {
                f.write_str("compiled title executable is missing")
            },
            Self::UnsupportedHost => {
                f.write_str("toolchain capability unavailable on this host")
            },
        }
    }
}

/// Private adapter for admitted workspace toolchain requests.
pub(crate) trait ToolchainFactory {
    fn available(target: Target) -> bool;
    fn compile(
        args: CargoArguments<'_>,
        root: &OsStr,
        target: Target,
    ) -> Result<OsString, ToolchainError>;
    fn metadata(
        args: CargoArguments<'_>,
        root: &OsStr,
        manifest: &OsStr,
    ) -> Result<Vec<u8>, ToolchainError>;
    fn probe_version(
        root: &OsStr,
        tool: ValidationTool,
    ) -> Result<ToolVersion, ToolchainError>;
    fn workspace_root() -> Result<OsString, ToolchainError>;
}

/// Fixed Cargo request for one independently authored workspace binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CargoTitle<'name> {
    args: CargoArguments<'name>,
    target: Target,
}

impl<'name> CargoTitle<'name> {
    /// Compile the selected authored executable using the local toolchain.
    ///
    /// # Errors
    /// Rejects unsupported hosts, Cargo failures and missing artifacts.
    pub fn compile(self, root: &OsStr) -> Result<OsString, ToolchainError> {
        self.compile_with::<()>(root)
    }

    fn compile_with<Factory>(
        self,
        root: &OsStr,
    ) -> Result<OsString, ToolchainError>
    where
        Factory: ToolchainFactory,
    {
        if !Factory::available(self.target) {
            return Err(ToolchainError::UnsupportedHost);
        }
        Factory::compile(self.args, root, self.target)
    }

    /// Fetch locked Cargo metadata for the same target and feature selection.
    ///
    /// # Errors
    /// Rejects unsupported hosts and unsuccessful toolchain operations.
    pub fn metadata(
        self,
        root: &OsStr,
        manifest: &OsStr,
    ) -> Result<Vec<u8>, ToolchainError> {
        self.metadata_with::<()>(root, manifest)
    }

    fn metadata_with<Factory>(
        self,
        root: &OsStr,
        manifest: &OsStr,
    ) -> Result<Vec<u8>, ToolchainError>
    where
        Factory: ToolchainFactory,
    {
        if !Factory::available(self.target) {
            return Err(ToolchainError::UnsupportedHost);
        }
        Factory::metadata(self.args, root, manifest)
    }

    /// Admit identifiers before constructing a fixed Cargo request.
    ///
    /// # Errors
    /// Rejects empty identifiers, option-like names and path components.
    pub fn new(
        package: &'name str,
        binary: &'name str,
        feature: &'name str,
        target: Target,
    ) -> Result<Self, ToolchainError> {
        let args =
            CargoArguments::new(package, binary, feature, target.triple())
                .map_err(|_error| ToolchainError::InvalidIdentifier)?;
        Ok(Self { args, target })
    }
}

/// Read the exact version of a known contributor tool.
///
/// # Errors
/// Returns an unavailable capability, process or unsuccessful-exit error.
pub fn probe_version(
    root: &OsStr,
    tool: ValidationTool,
) -> Result<ToolVersion, ToolchainError> {
    <() as ToolchainFactory>::probe_version(root, tool)
}

/// Resolve the canonical workspace containing the authored build tools.
///
/// # Errors
/// Returns unavailable capability or workspace-resolution errors.
pub fn workspace_root() -> Result<OsString, ToolchainError> {
    <() as ToolchainFactory>::workspace_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Recording<const AVAILABLE: bool>;

    impl<const AVAILABLE: bool> ToolchainFactory for Recording<AVAILABLE> {
        fn available(_target: Target) -> bool {
            AVAILABLE
        }

        fn compile(
            args: CargoArguments<'_>,
            root: &OsStr,
            target: Target,
        ) -> Result<OsString, ToolchainError> {
            if root != OsStr::new("workspace")
                || !args.build().contains(&target.triple())
                || !args.build().contains(&"native-preview")
            {
                return Err(ToolchainError::Failed {
                    operation: "compile fixture",
                    diagnostic: "unexpected request".into(),
                });
            }
            Ok(OsString::from(args.binary()))
        }

        fn metadata(
            args: CargoArguments<'_>,
            root: &OsStr,
            manifest: &OsStr,
        ) -> Result<Vec<u8>, ToolchainError> {
            if root != OsStr::new("workspace")
                || manifest != OsStr::new("manifest")
                || !args.metadata().contains(&"native-preview")
            {
                return Err(ToolchainError::Failed {
                    operation: "metadata fixture",
                    diagnostic: "unexpected request".into(),
                });
            }
            Ok(args.binary().as_bytes().to_vec())
        }

        fn probe_version(
            _root: &OsStr,
            _tool: ValidationTool,
        ) -> Result<ToolVersion, ToolchainError> {
            Err(ToolchainError::UnsupportedHost)
        }

        fn workspace_root() -> Result<OsString, ToolchainError> {
            Err(ToolchainError::UnsupportedHost)
        }
    }

    #[test]
    fn unavailable_target_never_dispatches() -> Result<(), String> {
        let title = admitted()?;
        let root = OsStr::new("workspace");
        let manifest = OsStr::new("manifest");
        if !matches!(
            title.compile_with::<Recording<false>>(root),
            Err(ToolchainError::UnsupportedHost)
        ) || !matches!(
            title.metadata_with::<Recording<false>>(root, manifest),
            Err(ToolchainError::UnsupportedHost)
        ) {
            return Err("unavailable adapter dispatched".into());
        }
        Ok(())
    }

    #[test]
    fn admitted_title_reuses_one_selection() -> Result<(), String> {
        let title = admitted()?;
        let root = OsStr::new("workspace");
        let manifest = OsStr::new("manifest");
        let executable = title
            .compile_with::<Recording<true>>(root)
            .map_err(|error| error.to_string())?;
        let metadata = title
            .metadata_with::<Recording<true>>(root, manifest)
            .map_err(|error| error.to_string())?;
        if executable != OsStr::new("example") || metadata != b"example" {
            return Err("inconsistent selected title".into());
        }
        Ok(())
    }

    fn admitted() -> Result<CargoTitle<'static>, String> {
        CargoTitle::new(
            "games-example",
            "example",
            "native-preview",
            Target::LinuxX86_64Gnu,
        )
        .map_err(|error| error.to_string())
    }
}
