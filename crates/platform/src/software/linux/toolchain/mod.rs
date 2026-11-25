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
//   - Native Cargo target availability and authored toolchain execution.
// - Must-Not:
//   - Choose titles, interpret source media or execute imported binaries.
// - Allows:
//   - Inputs: Trusted workspace path values and admitted Cargo identifiers.
//   - Outputs: Authored executable paths, metadata bytes or typed failures.
//   - Side effects: Explicit Cargo invocations and artifact inspection.
// - Split-When:
//   - A separate host toolchain needs distinct process semantics.
// - Merge-When:
//   - Another platform service owns the same native Cargo operations.
// - Summary:
//   - Native Cargo build and metadata adapter.
// - Description:
//   - Executes only fixed, previously admitted toolchain requests.
// - Usage:
//   - Dispatch through the portable contributor toolchain capability.
// - Defaults:
//   - Locked release builds; unsupported targets fail before process launch.

//! Linux toolchain execution for independently authored workspace artifacts.

mod environment;
mod reference;
mod validation;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ir::security::toolchain::CargoArguments;
use crate::ir::toolchain::reference::ReferenceTool;
use crate::ir::toolchain::{
    Target, ToolVersion, ToolchainError, ToolchainFactory, ValidationTool,
};

impl ToolchainFactory for () {
    fn available(target: Target) -> bool {
        match target {
            Target::LinuxX86_64Gnu => {
                cfg!(all(target_arch = "x86_64", target_env = "gnu"))
            },
            Target::AndroidArm64
            | Target::LinuxArm64Gnu
            | Target::WebAssembly
            | Target::WindowsArm64Gnullvm
            | Target::WindowsX86_64Gnu => false,
        }
    }

    fn compile(
        args: CargoArguments<'_>,
        root: &OsStr,
        target: Target,
    ) -> Result<OsString, ToolchainError> {
        let workspace = Path::new(root);
        let target_dir = workspace.join("target");
        let status = Command::new("cargo")
            .current_dir(workspace)
            .args(args.build())
            .arg("--target-dir")
            .arg(&target_dir)
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
        let executable = executable_path(&target_dir, args, target);
        if !executable.is_file() {
            return Err(ToolchainError::MissingArtifact);
        }
        Ok(executable.into_os_string())
    }

    fn download_reference(
        tool: ReferenceTool,
    ) -> Result<OsString, ToolchainError> {
        let workspace = PathBuf::from(Self::workspace_root()?);
        reference::download(&workspace, tool)
    }

    fn metadata(
        args: CargoArguments<'_>,
        root: &OsStr,
        manifest: &OsStr,
    ) -> Result<Vec<u8>, ToolchainError> {
        let output = Command::new("cargo")
            .current_dir(Path::new(root))
            .args(args.metadata())
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

    fn probe_version(
        root: &OsStr,
        tool: ValidationTool,
    ) -> Result<ToolVersion, ToolchainError> {
        let (program, args) = tool.command();
        let output = Command::new(program)
            .current_dir(Path::new(root))
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

    fn workspace_root() -> Result<OsString, ToolchainError> {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map(PathBuf::into_os_string)
            .map_err(|source| ToolchainError::Io {
                operation: "workspace root",
                source,
            })
    }
}

fn executable_path(
    target_dir: &Path,
    args: CargoArguments<'_>,
    target: Target,
) -> PathBuf {
    target_dir
        .join(target.triple())
        .join("release")
        .join(args.binary())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_artifact_path_matches_cargo_layout() -> Result<(), String> {
        let target = Target::LinuxX86_64Gnu;
        let args = CargoArguments::new(
            "games-demo",
            "demo",
            "preview",
            target.triple(),
        )
        .map_err(|error| format!("{error:?}"))?;
        let expected = Path::new("target")
            .join(target.triple())
            .join("release/demo");
        if executable_path(Path::new("target"), args, target) != expected {
            return Err("unexpected Cargo artifact path".into());
        }
        Ok(())
    }
}
