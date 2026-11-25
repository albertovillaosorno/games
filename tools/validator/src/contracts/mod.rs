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
//   - Composition of workspace and task validation.
// - Must-Not:
//   - Own gameplay acceptance or modify the roadmap.
// - Allows:
//   - Inputs: Repository root and workspace manifests.
//   - Outputs: Validation success or diagnostic.
//   - Side effects: Read-only access and Cargo metadata execution.
// - Split-When:
//   - Independent checks acquire distinct policy.
// - Merge-When:
//   - A shared contract runner replaces this composition.
// - Summary:
//   - Workspace and task contract entry point.
// - Description:
//   - Checks workspace metadata and delegates task validation.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Reject invalid metadata or failed task checks.
//

mod ir_layout;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::todo::validate as validate_tasks;
use crate::{boundaries, clones, files, unified_ir};

/// Validate the task inventory.
///
/// # Errors
///
/// Returns diagnostics if the task validator fails.
pub fn todo(root: &Path) -> Result<(), String> {
    validate_tasks(root)
}

fn manifest(root: &Path) -> Result<Value, String> {
    let source = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| error.to_string())?;
    let value = source
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    if !value.contains_key("workspace") {
        return Err(String::from("missing workspace manifest"));
    }
    let output = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--no-deps", "--locked", "--format-version", "1"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from("Cargo metadata failed"));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| error.to_string())?;
    Ok(metadata)
}

/// Validate workspace metadata and task records.
///
/// # Errors
///
/// Returns diagnostics if a required check fails.
pub fn validate(root: &Path) -> Result<Vec<String>, String> {
    let metadata = manifest(root)?;
    if fs::read(root.join("LICENSE")).map_err(|error| error.to_string())?
        != fs::read(root.join("crates/engine/LICENSE"))
            .map_err(|error| error.to_string())?
    {
        return Err(String::from("engine licence differs from root licence"));
    }
    let authored = files::authored(root)?;
    ir_layout::validate(&authored)?;
    unified_ir::validate(root, &authored)?;
    boundaries::validate(root, &metadata)?;
    clones::validate(root)?;
    todo(root)?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("missing workspace package inventory")?;
    let mut names = packages
        .iter()
        .map(|package| {
            package
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| String::from("missing workspace package name"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}
