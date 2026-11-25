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
//   - Native Cargo target admission and authored executable toolchain access.
// - Must-Not:
//   - Choose titles, interpret source media or execute imported binaries.
// - Allows:
//   - Inputs: Trusted workspace paths and admitted Cargo identifiers.
//   - Outputs: Authored executable paths, metadata bytes or typed failures.
//   - Side effects: Explicit Cargo invocations and artifact inspection.
// - Split-When:
//   - A separate host toolchain needs distinct process semantics.
// - Merge-When:
//   - Another platform service owns the same native Cargo operations.
// - Summary:
//   - Native Cargo build and metadata adapter.
// - Description:
//   - Centralises target availability and fixed toolchain argument layouts.
// - Usage:
//   - Request a known workspace package, binary, feature and target.
// - Defaults:
//   - Locked release builds; unsupported hosts fail before process launch.

//! Host toolchain operations for independently authored workspace artifacts.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fmt, io};

use crate::ir::security::toolchain::CargoArguments;
pub use crate::ir::security::toolchain::{ToolVersion, ValidationTool};

/// Target understood by the native Cargo adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    /// The currently supported native GNU/Linux x86-64 toolchain.
    LinuxX86_64Gnu,
}

impl Target {
    /// Whether the current host can build this target through this adapter.
    #[must_use]
    pub const fn available(self) -> bool {
        match self {
            Self::LinuxX86_64Gnu => cfg!(all(
                target_os = "linux",
                target_arch = "x86_64",
                target_env = "gnu"
            )),
        }
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

/// Failure to execute or verify an authored Cargo toolchain operation.
#[derive(Debug)]
pub enum ToolchainError {
    /// Cargo exited unsuccessfully with a diagnostic.
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
        /// Host error.
        source: io::Error,
    },
    /// Cargo did not produce the requested regular executable.
    MissingArtifact,
    /// The selected target is unavailable on this host.
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
                f.write_str("requires an x86-64 GNU/Linux host")
            },
        }
    }
}

/// Fixed Cargo request for a single independently authored workspace binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CargoTitle<'name> {
    args: CargoArguments<'name>,
    target: Target,
}

impl<'name> CargoTitle<'name> {
    const fn build_args(self) -> [&'name str; 11] {
        self.args.build()
    }

    /// Compile the selected authored executable using the local toolchain.
    /// # Errors
    /// Rejects unsupported hosts, Cargo failures and missing artifacts.
    pub fn compile(self, root: &Path) -> Result<PathBuf, ToolchainError> {
        self.require_host()?;
        let target = root.join("target");
        let status = Command::new("cargo")
            .current_dir(root)
            .args(self.build_args())
            .arg("--target-dir")
            .arg(&target)
            .status()
            .map_err(|source| ToolchainError::Io {
                operation: "launch Cargo",
                source,
            })?;
        if !status.success() {
            return Err(ToolchainError::Failed {
                operation: "title compilation",
                diagnostic: status.to_string(),
            });
        }
        let executable = self.executable_path(&target);
        if !executable.is_file() {
            return Err(ToolchainError::MissingArtifact);
        }
        Ok(executable)
    }

    fn executable_path(self, target_dir: &Path) -> PathBuf {
        target_dir
            .join(self.target.triple())
            .join("release")
            .join(self.args.binary())
    }

    /// Fetch locked Cargo metadata for the same target and feature selection.
    /// # Errors
    /// Rejects unsupported hosts and unsuccessful toolchain operations.
    pub fn metadata(
        self,
        root: &Path,
        manifest: &Path,
    ) -> Result<Vec<u8>, ToolchainError> {
        self.require_host()?;
        let output = Command::new("cargo")
            .current_dir(root)
            .args(self.metadata_args())
            .arg(manifest)
            .output()
            .map_err(|source| ToolchainError::Io {
                operation: "Cargo metadata",
                source,
            })?;
        if !output.status.success() {
            return Err(ToolchainError::Failed {
                operation: "Cargo metadata",
                diagnostic: String::from_utf8_lossy(&output.stderr)
                    .into_owned(),
            });
        }
        Ok(output.stdout)
    }

    const fn metadata_args(self) -> [&'name str; 9] {
        self.args.metadata()
    }

    /// Admit identifiers before constructing a fixed Cargo request.
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

    const fn require_host(self) -> Result<(), ToolchainError> {
        if self.target.available() {
            Ok(())
        } else {
            Err(ToolchainError::UnsupportedHost)
        }
    }
}

/// Read the exact version of a known development tool.
/// # Errors
/// Returns a process or unsuccessful-exit error.
pub fn probe_version(
    root: &Path,
    tool: ValidationTool,
) -> Result<ToolVersion, ToolchainError> {
    let (program, args) = tool.command();
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|source| ToolchainError::Io {
            operation: program,
            source,
        })?;
    if !output.status.success() {
        return Err(ToolchainError::Failed {
            operation: program,
            diagnostic: output.status.to_string(),
        });
    }
    Ok(ToolVersion {
        stderr: output.stderr,
        stdout: output.stdout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_request_rejects_options_and_paths() -> Result<(), String> {
        let target = Target::LinuxX86_64Gnu;
        for invalid in ["", "-option", "../game", "game/bin", "a b", "a,b"] {
            if CargoTitle::new(invalid, "game", "preview", target).is_ok()
                || CargoTitle::new("game", invalid, "preview", target).is_ok()
                || CargoTitle::new("game", "game", invalid, target).is_ok()
            {
                return Err(format!("unexpected identifier: {invalid}"));
            }
        }
        let _admitted = CargoTitle::new(
            "games-example",
            "example",
            "native-preview",
            target,
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn build_and_metadata_share_target_and_feature() -> Result<(), String> {
        let target = Target::LinuxX86_64Gnu;
        let title = CargoTitle::new("games-demo", "demo", "preview", target)
            .map_err(|error| error.to_string())?;
        let build = title.build_args();
        let metadata = title.metadata_args();
        for args in [build.as_slice(), metadata.as_slice()] {
            if !args.contains(&"--locked")
                || !args.contains(&target.triple())
                || !args.contains(&"preview")
            {
                return Err(String::from("inconsistent toolchain selection"));
            }
        }
        let expected = Path::new("target")
            .join(target.triple())
            .join("release/demo");
        if title.executable_path(Path::new("target")) != expected {
            return Err(String::from("incorrect selected artifact path"));
        }
        Ok(())
    }
}
