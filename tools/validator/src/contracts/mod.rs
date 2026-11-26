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

use std::ffi::OsStr;

use games_platform::ir::storage::strip_path_prefix;
use games_platform::ir::toolchain::validation::{self, Request};
use serde_json::Value;

use crate::todo::validate as validate_tasks;
use crate::{boundaries, clones, files, unified_ir};

/// A workspace package classified by its actual canonical directory.
#[derive(Debug)]
pub struct Package {
    /// Cargo package name.
    pub name: String,
    /// Whether this package must compile as a portable library.
    pub portable: bool,
    /// Whether its runtime graph must exclude source decoders.
    pub runtime: bool,
}

/// Validate the task inventory.
///
/// # Errors
///
/// Returns diagnostics if the task validator fails.
pub fn todo(root: &OsStr) -> Result<(), String> {
    validate_tasks(root)
}

fn manifest(root: &OsStr) -> Result<Value, String> {
    let source = files::read_text(root, OsStr::new("Cargo.toml"))?;
    let value = source
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    if !value.contains_key("workspace") {
        return Err(String::from("missing workspace manifest"));
    }
    let output = validation::run(root, Request::WorkspaceMetadata)
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err(String::from("Cargo metadata failed"));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| error.to_string())?;
    Ok(metadata)
}

fn classify(package: &Value, root: &OsStr) -> Result<Package, String> {
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .ok_or("missing workspace package name")?
        .to_owned();
    let manifest = package
        .get("manifest_path")
        .and_then(Value::as_str)
        .ok_or("missing package manifest path")?;
    let relative = strip_path_prefix(OsStr::new(manifest), root)
        .map_err(|error| error.to_string())?;
    let path = relative.to_str().ok_or("invalid package manifest path")?;
    let portable = path.starts_with("crates/");
    let runtime = path.starts_with("crates/games/")
        || path.starts_with("crates/apps/")
        || path.starts_with("crates/engine/");
    if portable {
        let targets = package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or("missing package targets")?;
        if !targets.iter().any(|target| {
            target
                .get("kind")
                .and_then(Value::as_array)
                .is_some_and(|kinds| {
                    kinds.iter().any(|kind| kind.as_str() == Some("lib"))
                })
        }) {
            return Err(format!("portable package requires a library: {name}"));
        }
    }
    Ok(Package { name, portable, runtime })
}

/// Validate workspace metadata and task records.
///
/// # Errors
///
/// Returns diagnostics if a required check fails.
pub fn validate(root: &OsStr) -> Result<Vec<Package>, String> {
    let metadata = manifest(root)?;
    if files::read_authored(root, OsStr::new("LICENSE"))?
        != files::read_authored(root, OsStr::new("crates/engine/LICENSE"))?
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
    let canonical = files::canonical(root)?;
    let mut inventory = packages
        .iter()
        .map(|package| classify(package, &canonical))
        .collect::<Result<Vec<_>, _>>()?;
    inventory.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use serde_json::json;

    use super::classify;

    #[test]
    fn new_titles_enter_portable_and_runtime_checks_automatically() {
        for folder in ["games", "apps"] {
            let manifest = format!("/repo/crates/{folder}/new/Cargo.toml");
            let package = json!({
                "name": "arbitrary-name",
                "manifest_path": manifest,
                "targets": [{"kind": ["lib"]}],
            });
            assert!(
                classify(&package, OsStr::new("/repo"))
                    .is_ok_and(|entry| entry.portable && entry.runtime)
            );
            let binary = json!({
                "name": "arbitrary-name",
                "manifest_path": manifest,
                "targets": [{"kind": ["bin"]}],
            });
            assert!(classify(&binary, OsStr::new("/repo")).is_err());
        }
        let tool = json!({
            "name": "tool", "manifest_path": "/repo/tools/new/Cargo.toml",
            "targets": [{"kind": ["bin"]}],
        });
        assert!(
            classify(&tool, OsStr::new("/repo"))
                .is_ok_and(|entry| !entry.portable && !entry.runtime)
        );
    }
}
