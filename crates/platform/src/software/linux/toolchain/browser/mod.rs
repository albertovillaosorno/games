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
//   - Binding authored Wasm with the reviewed local release tool.
// - Must-Not:
//   - Download tools implicitly or bind imported original software.
// - Allows:
//   - Inputs: Trusted workspace, staging parent and compiled artifact.
//   - Outputs: Bounded generated browser bindings.
//   - Side effects: Private staging and fixed tool execution.
// - Split-When:
//   - Another binding tool requires independent execution semantics.
// - Merge-When:
//   - Another host adapter duplicates the same binding operation.
// - Summary:
//   - Verify and isolate the pinned binding executable before invocation.
// - Description:
//   - Snapshot authored input and only return the expected binding files.
// - Usage:
//   - Explicitly invoke the authored browser packaging capability.
// - Defaults:
//   - No network acquisition and no automatic reference execution.

use std::ffi::OsStr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::Path;
use std::process::Command;
use std::str;

use games_assets::ContentHash;

use crate::ir::security::toolchain::artifact::admit_executable;
use crate::ir::storage::file::snapshot;
use crate::ir::storage::staging::AuthoredStaging;
use crate::ir::storage::{PathKind, directory_entries};
use crate::ir::toolchain::{BrowserBindings, Target, ToolchainError};

const MAX_ARTIFACT: NonZeroU64 = NonZeroU64::MIN.saturating_add(33_554_431);
const MAX_SCRIPT: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const TOOL: &str = concat!(
    ".reference/tools/wasm-bindgen-release/0.2.127/",
    "wasm-bindgen-0.2.127-x86_64-unknown-linux-musl/wasm-bindgen",
);
// Executable member of the pinned official 0.2.127 release archive.
const TOOL_HASH: &str =
    "f36dbe1938e78a79ce34d0ab3700907555161e293954f6f35bf6785a2f0cf916";

const fn failure(diagnostic: String) -> ToolchainError {
    ToolchainError::Failed {
        operation: "browser binding",
        diagnostic,
    }
}

fn module(path: &OsStr) -> Result<Vec<u8>, ToolchainError> {
    let bytes = snapshot(path, MAX_ARTIFACT)
        .map_err(|error| failure(format!("module: {error:?}")))?;
    admit_executable(Target::WebAssembly, &bytes)
        .map_err(|error| failure(error.to_string()))?;
    Ok(bytes)
}

fn tool(root: &OsStr) -> Result<Vec<u8>, ToolchainError> {
    let path = Path::new(root).join(TOOL);
    let bytes = snapshot(path.as_os_str(), MAX_ARTIFACT).map_err(|error| {
        failure(format!(
            "extract the pinned wasm-bindgen release: {error:?}"
        ))
    })?;
    if ContentHash::calculate(&bytes).to_string() != TOOL_HASH {
        return Err(failure("unexpected binding tool identity".into()));
    }
    Ok(bytes)
}

pub(super) fn bind(
    root: &OsStr,
    parent: &OsStr,
    artifact: &OsStr,
) -> Result<BrowserBindings, ToolchainError> {
    let mut stage = AuthoredStaging::new(parent)
        .map_err(|error| failure(format!("private stage: {error:?}")))?;
    stage
        .write_file("bindgen", &tool(root)?, true)
        .map_err(|error| failure(format!("stage tool: {error:?}")))?;
    stage
        .write_file("input.wasm", &module(artifact)?, false)
        .map_err(|error| failure(format!("stage module: {error:?}")))?;
    let directory = Path::new(stage.path());
    let status = Command::new(directory.join("bindgen"))
        .current_dir(directory)
        .args([
            "--target",
            "web",
            "--no-typescript",
            "--out-name",
            "game",
            "--out-dir",
            ".",
            "input.wasm",
        ])
        .status()
        .map_err(|source| ToolchainError::Io {
            operation: "launch pinned wasm-bindgen",
            source,
        })?;
    if !status.success() {
        return Err(failure(status.to_string()));
    }
    let expected = ["bindgen", "game.js", "game_bg.wasm", "input.wasm"];
    let entries = directory_entries(
        directory.as_os_str(),
        NonZeroUsize::MIN.saturating_add(3),
    )
    .map_err(|error| failure(error.to_string()))?;
    if entries.len() != expected.len()
        || entries.iter().any(|entry| {
            entry.kind != PathKind::RegularFile
                || !entry
                    .name
                    .to_str()
                    .is_some_and(|name| expected.contains(&name))
        })
    {
        return Err(failure("unexpected binding output".into()));
    }
    let wasm = module(directory.join("game_bg.wasm").as_os_str())?;
    let javascript =
        snapshot(directory.join("game.js").as_os_str(), MAX_SCRIPT)
            .map_err(|error| failure(format!("script: {error:?}")))?;
    if javascript.is_empty()
        || str::from_utf8(&javascript).is_err()
        || javascript.contains(&0)
    {
        return Err(failure("invalid generated browser script".into()));
    }
    Ok(BrowserBindings { javascript, module: wasm })
}
