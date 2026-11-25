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
//   - Authored source ownership and workspace dependency checks.
// - Must-Not:
//   - Change member manifests or define game-specific behaviour.
// - Allows:
//   - Inputs: Source headers, manifests and workspace dependency metadata.
//   - Outputs: Contract diagnostics with affected file paths.
//   - Side effects: Read-only repository access.
// - Split-When:
//   - A distinct policy cannot share source or dependency parsing.
// - Merge-When:
//   - The existing contract runner covers the same validation.
// - Summary:
//   - Boundary and dependency direction enforcement.
// - Description:
//   - Guards source ownership and shared-to-title dependency barriers.
// - Usage:
//   - Invoked by cargo validate via the contract runner.
// - Defaults:
//   - Reject missing ownership, local lint overrides and reverse edges.
//
use std::fs;
use std::path::Path;

use toml::Value;

use crate::files;

const HEADER_KEYS: &[&str] = &[
    "Owns:",
    "Must-Not:",
    "Allows:",
    "Inputs:",
    "Outputs:",
    "Side effects:",
    "Split-When:",
    "Merge-When:",
    "Summary:",
    "Description:",
    "Usage:",
    "Defaults:",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Importer,
    Platform,
    Runtime,
    Shared,
    Title,
    Tool,
}

fn role(path: &str) -> Option<Role> {
    let mut parts = path.split('/');
    let parent = parts.next()?;
    let name = parts.next()?;
    if name.is_empty() || matches!(name, "." | "..") {
        return None;
    }
    match parent {
        "tools" => Some(Role::Tool),
        "games" | "apps" => Some(Role::Title),
        "crates" => Some(match name {
            "engine" | "runtime" => Role::Runtime,
            "importer" => Role::Importer,
            "platform" => Role::Platform,
            _ => Role::Shared,
        }),
        _ => None,
    }
}

const fn dependency_allowed(owner: Role, target: Role) -> bool {
    if matches!(
        owner,
        Role::Shared | Role::Runtime | Role::Importer | Role::Platform
    ) && matches!(target, Role::Title | Role::Tool)
    {
        return false;
    }
    !matches!(
        (owner, target),
        (Role::Title, Role::Title)
            | (
                Role::Shared,
                Role::Runtime | Role::Importer | Role::Platform
            )
            | (Role::Runtime | Role::Platform, Role::Importer)
            | (Role::Importer, Role::Runtime | Role::Platform)
    )
}

fn code_path(path: &Path) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    extension.is_some_and(|value| {
        [
            "rs", "sh", "wgsl", "glsl", "vert", "frag", "comp", "hlsl", "c",
            "h", "cc", "cpp", "js", "ts", "tsx", "jsx",
        ]
        .contains(&value)
    }) || path.starts_with(".githooks")
}

fn comment(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("//")
        .or_else(|| trimmed.strip_prefix('#'))
        .map(str::trim)
}

fn header(text: &str) -> Result<(), String> {
    let mut comments = Vec::new();
    for line in text.lines().take(90) {
        if let Some(content) = comment(line) {
            comments.push(content);
        } else if !line.trim().is_empty() {
            break;
        }
    }
    let start = comments
        .iter()
        .position(|line| *line == "Boundary-Contract:")
        .ok_or("missing Boundary-Contract header")?;
    for key in HEADER_KEYS {
        let label = format!("- {key}");
        let mut found = comments
            .iter()
            .skip(start.saturating_add(1))
            .skip_while(|line| !line.starts_with(&label));
        let value = found
            .next()
            .copied()
            .ok_or_else(|| format!("missing Boundary-Contract {key}"))?;
        let next = found.next().copied().unwrap_or_default();
        let empty = value.strip_prefix(&label).is_some_and(|rest| {
            rest.trim().is_empty()
                && (next.is_empty()
                    || (key != &"Allows:"
                        && HEADER_KEYS.iter().any(|candidate| {
                            next.starts_with(&format!("- {candidate}"))
                        })))
        });
        if empty {
            return Err(format!("empty Boundary-Contract {key}"));
        }
    }
    Ok(())
}

fn sources(root: &Path) -> Result<(), String> {
    for path in files::authored(root)? {
        if code_path(&path) {
            let content = fs::read_to_string(root.join(&path))
                .map_err(|error| error.to_string())?;
            header(&content)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn table<'value>(
    value: &'value Value,
    key: &str,
) -> Result<&'value toml::Table, String> {
    value
        .get(key)
        .and_then(Value::as_table)
        .ok_or_else(|| format!("missing or invalid {key} table"))
}

fn direct_groups(value: &Value) -> Result<Vec<&toml::Table>, String> {
    let mut groups = Vec::new();
    for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = value.get(name) {
            groups.push(
                dependencies
                    .as_table()
                    .ok_or_else(|| format!("invalid {name} table"))?,
            );
        }
    }
    Ok(groups)
}

fn dependency_tables(value: &Value) -> Result<Vec<&toml::Table>, String> {
    let mut groups = direct_groups(value)?;
    if let Some(targets) = value.get("target") {
        let entries =
            targets.as_table().ok_or("invalid target dependencies")?;
        for target in entries.values() {
            groups.extend(direct_groups(target)?);
        }
    }
    Ok(groups)
}

fn dependency(
    owner: Role,
    name: &str,
    value: &Value,
    dependencies: &toml::Table,
) -> Result<(), String> {
    if value.get("workspace").and_then(Value::as_bool) != Some(true) {
        return Err(format!("member dependency {name} must be inherited"));
    }
    let reference = dependencies
        .get(name)
        .ok_or_else(|| format!("dependency not in workspace: {name}"))?;
    if let Some(path) = reference.get("path").and_then(Value::as_str) {
        if path.split('/').count() != 2 {
            return Err(format!("invalid workspace path: {path}"));
        }
        let target = role(path)
            .ok_or_else(|| format!("invalid workspace path: {path}"))?;
        if !dependency_allowed(owner, target) {
            return Err(format!("forbidden dependency direction: {name}"));
        }
    }
    Ok(())
}

fn member(
    owner: Role,
    manifest: &Value,
    dependencies: &toml::Table,
) -> Result<(), String> {
    let package = table(manifest, "package")?;
    for key in [
        "version",
        "edition",
        "license",
        "repository",
        "keywords",
        "categories",
    ] {
        if package
            .get(key)
            .and_then(|value| value.get("workspace"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(format!("package {key} must inherit from workspace"));
        }
    }
    let lints = table(manifest, "lints")?;
    if lints.len() != 1 || lints.get("workspace") != Some(&Value::Boolean(true))
    {
        return Err(String::from(
            "member lints must inherit without overrides",
        ));
    }
    for group in dependency_tables(manifest)? {
        for (name, value) in group {
            dependency(owner, name, value, dependencies)?;
        }
    }
    Ok(())
}

fn workspace_lints(workspace: &toml::Table) -> Result<(), String> {
    let lints = workspace
        .get("lints")
        .and_then(Value::as_table)
        .ok_or("missing workspace lint policy")?;
    for group in ["rust", "clippy"] {
        if !lints.get(group).is_some_and(Value::is_table) {
            return Err(format!("missing workspace {group} lints"));
        }
    }
    Ok(())
}

fn manifests(root: &Path) -> Result<(), String> {
    let root_manifest = Value::Table(
        fs::read_to_string(root.join("Cargo.toml"))
            .map_err(|error| error.to_string())?
            .parse::<toml::Table>()
            .map_err(|error| error.to_string())?,
    );
    let workspace = table(&root_manifest, "workspace")?;
    let dependencies = workspace
        .get("dependencies")
        .and_then(Value::as_table)
        .ok_or("missing workspace dependencies")?;
    workspace_lints(workspace)?;
    for group in ["crates", "tools", "games", "apps"] {
        let directory = root.join(group);
        if !directory.exists() {
            continue;
        }
        for entry in
            fs::read_dir(directory).map_err(|error| error.to_string())?
        {
            let item = entry.map_err(|error| error.to_string())?;
            let file_type =
                item.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                return Err(String::from(
                    "symlink in workspace package directory",
                ));
            }
            if !file_type.is_dir() {
                continue;
            }
            let manifest_path = item.path().join("Cargo.toml");
            if !manifest_path.exists() {
                continue;
            }
            let relative = manifest_path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?;
            let name = relative.to_string_lossy().replace('\\', "/");
            let member_role = role(&name).ok_or_else(|| {
                format!("unrecognised workspace member: {name}")
            })?;
            let content = fs::read_to_string(&manifest_path)
                .map_err(|error| error.to_string())?;
            let value = Value::Table(
                content
                    .parse::<toml::Table>()
                    .map_err(|error| format!("{name}: {error}"))?,
            );
            member(member_role, &value, dependencies)
                .map_err(|error| format!("{name}: {error}"))?;
        }
    }
    Ok(())
}

/// Check source ownership and dependency boundary contracts.
///
/// # Errors
///
/// Rejects missing source contracts, lint overrides and forbidden edges.
pub(super) fn validate(root: &Path) -> Result<(), String> {
    sources(root)?;
    manifests(root)
}

#[cfg(test)]
#[path = "../../../tests/validator/boundaries.rs"]
mod tests;
