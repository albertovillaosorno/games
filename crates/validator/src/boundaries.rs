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
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
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
        "crates" => Some(match name {
            "engine" | "runtime" => Role::Runtime,
            "importer" => Role::Importer,
            "platform" => Role::Platform,
            "validator" | "download" => Role::Tool,
            _ if name.starts_with("game-") || name.starts_with("app-") => {
                Role::Title
            },
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

fn source_layout(path: &Path) -> bool {
    if path.extension().is_none_or(|ext| ext != "rs") {
        return true;
    }
    let mut parts = path.components();
    parts
        .next()
        .is_some_and(|part| part.as_os_str() == "crates")
        && parts.next().is_some()
        && parts.next().is_some_and(|part| part.as_os_str() != "tests")
}

fn sources(
    root: &Path,
    members: &BTreeSet<PathBuf>,
    authored: &[PathBuf],
) -> Result<(), String> {
    for path in authored {
        if !source_layout(path) {
            return Err(format!(
                "Rust source outside crate: {}",
                path.display()
            ));
        }
        if path.extension().is_some_and(|ext| ext == "rs") {
            let package = path.components().take(2).collect::<PathBuf>();
            if !members.contains(&package.join("Cargo.toml")) {
                return Err(format!(
                    "Rust source outside workspace member: {}",
                    path.display()
                ));
            }
        }
        if code_path(path) {
            let content = fs::read_to_string(root.join(path))
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

fn target_source(
    root: &Path,
    directory: &Path,
    target: &JsonValue,
    authored: &[PathBuf],
) -> Result<(), String> {
    let source = target
        .get("src_path")
        .and_then(JsonValue::as_str)
        .ok_or("missing Cargo target source path")?;
    let relative = Path::new(source)
        .strip_prefix(root)
        .map_err(|error| format!("Cargo target outside repository: {error}"))?;
    if !relative.starts_with(directory)
        || relative.extension().is_none_or(|ext| ext != "rs")
        || authored
            .binary_search_by(|path| path.as_path().cmp(relative))
            .is_err()
    {
        return Err(format!(
            "Cargo target outside owned authored source: {source}"
        ));
    }
    Ok(())
}

fn package_manifest(
    root: &Path,
    package: &JsonValue,
    authored: &[PathBuf],
) -> Result<PathBuf, String> {
    let manifest = package
        .get("manifest_path")
        .and_then(JsonValue::as_str)
        .ok_or("missing package manifest path")?;
    let path = Path::new(manifest)
        .strip_prefix(root)
        .map_err(|error| format!("workspace member outside root: {error}"))?;
    let directory = path.parent().ok_or("missing package directory")?;
    if directory.parent() != Some(Path::new("crates"))
        || directory.file_name().is_none()
        || path.file_name().is_none_or(|name| name != "Cargo.toml")
    {
        return Err(format!(
            "workspace member outside direct crates: {manifest}"
        ));
    }
    let targets = package
        .get("targets")
        .and_then(JsonValue::as_array)
        .ok_or("missing package targets")?;
    for target in targets {
        let kinds = target
            .get("kind")
            .and_then(JsonValue::as_array)
            .ok_or("missing target kind")?;
        if kinds.iter().any(|kind| kind.as_str() == Some("test")) {
            return Err(format!(
                "separate integration test target: {manifest}"
            ));
        }
        target_source(root, directory, target, authored)?;
    }
    Ok(path.to_path_buf())
}

fn metadata_members(
    root: &Path,
    metadata: &JsonValue,
    authored: &[PathBuf],
) -> Result<BTreeSet<PathBuf>, String> {
    let ids = metadata
        .get("workspace_members")
        .and_then(JsonValue::as_array)
        .ok_or("missing workspace members")?;
    let mut remaining = ids
        .iter()
        .map(|id| id.as_str().ok_or("invalid workspace member ID"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if remaining.is_empty() || remaining.len() != ids.len() {
        return Err(String::from("empty or duplicate workspace members"));
    }
    let packages = metadata
        .get("packages")
        .and_then(JsonValue::as_array)
        .ok_or("missing workspace packages")?;
    let mut manifests = BTreeSet::new();
    for package in packages {
        let id = package
            .get("id")
            .and_then(JsonValue::as_str)
            .ok_or("missing package ID")?;
        if !remaining.remove(id) {
            return Err(format!("unexpected workspace package: {id}"));
        }
        if !manifests.insert(package_manifest(root, package, authored)?) {
            return Err(format!("duplicate workspace package path: {id}"));
        }
    }
    if !remaining.is_empty() {
        return Err(String::from("workspace package metadata incomplete"));
    }
    Ok(manifests)
}

fn workspace_lints(workspace: &toml::Table) -> Result<(), String> {
    let lints = workspace
        .get("lints")
        .and_then(Value::as_table)
        .ok_or("missing workspace lint policy")?;
    let rust = lints
        .get("rust")
        .and_then(Value::as_table)
        .ok_or("missing workspace rust lints")?;
    if !lints.get("clippy").is_some_and(Value::is_table) {
        return Err(String::from("missing workspace clippy lints"));
    }
    for (lint, required) in [
        ("unsafe_code", "forbid"),
        ("warnings", "deny"),
        ("unfulfilled_lint_expectations", "deny"),
    ] {
        if rust.get(lint).and_then(Value::as_str) != Some(required) {
            return Err(format!("workspace {lint} must be {required}"));
        }
    }
    Ok(())
}

fn manifests(root: &Path, members: &BTreeSet<PathBuf>) -> Result<(), String> {
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
    let directory = root.join("crates");
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let item = entry.map_err(|error| error.to_string())?;
        let file_type = item.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err(String::from("symlink in workspace package directory"));
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
        if !members.contains(relative) {
            return Err(format!(
                "crate excluded from workspace: {}",
                relative.display()
            ));
        }
        if fs::symlink_metadata(&manifest_path)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err(format!("symlink manifest: {}", relative.display()));
        }
        let name = relative.to_string_lossy().replace('\\', "/");
        let member_role = role(&name)
            .ok_or_else(|| format!("unrecognised workspace member: {name}"))?;
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
    Ok(())
}

/// Check source ownership and dependency boundary contracts.
///
/// # Errors
///
/// Rejects missing source contracts, lint overrides and forbidden edges.
pub fn validate(root: &Path, metadata: &JsonValue) -> Result<(), String> {
    let canonical = root.canonicalize().map_err(|error| error.to_string())?;
    let authored = files::authored(root)?;
    let members = metadata_members(&canonical, metadata, &authored)?;
    sources(root, &members, &authored)?;
    manifests(root, &members)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use serde_json::{Value as JsonValue, json};
    use toml::Value;

    use super::{
        Role, dependency, dependency_allowed, header, member, metadata_members,
        source_layout, workspace_lints,
    };
    use crate::fixtures::require;

    #[test]
    fn requires_colocated_rust_sources() {
        for path in
            ["crates/engine/src/lib.rs", "crates/validator/src/hooks.rs"]
        {
            assert!(source_layout(Path::new(path)));
        }
        for path in [
            "tests/engine/input.rs",
            "tools/download/src/main.rs",
            "crates/engine/tests/input.rs",
        ] {
            assert!(!source_layout(Path::new(path)));
        }
    }

    #[test]
    fn requires_meaningful_source_contracts() {
        let valid = include_str!("contracts.rs");
        assert!(header(valid).is_ok(), "normal source header");
        assert!(header("pub fn call() {}\n").is_err(), "missing header");
        let missing = valid.replacen("- Must-Not:", "- Not-Allowed:", 1);
        assert!(header(&missing).is_err(), "missing required key");
        let empty = valid.replacen(
            "//   - Own gameplay acceptance or modify the roadmap.",
            "//",
            1,
        );
        assert!(header(&empty).is_err(), "empty ownership responsibility");
    }

    #[test]
    fn blocks_reverse_dependencies_and_mixed_adapters() {
        for (owner, target) in [
            (Role::Shared, Role::Tool),
            (Role::Shared, Role::Title),
            (Role::Runtime, Role::Importer),
            (Role::Importer, Role::Platform),
            (Role::Importer, Role::Runtime),
            (Role::Platform, Role::Importer),
            (Role::Title, Role::Title),
        ] {
            assert!(!dependency_allowed(owner, target));
        }
        for (owner, target) in [
            (Role::Tool, Role::Shared),
            (Role::Title, Role::Runtime),
            (Role::Importer, Role::Shared),
            (Role::Runtime, Role::Shared),
        ] {
            assert!(dependency_allowed(owner, target));
        }
    }

    #[test]
    fn members_must_inherit_lints_and_dependencies() -> Result<(), String> {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/engine/Cargo.toml"
        ));
        let mut valid = Value::Table(
            source
                .parse::<toml::Table>()
                .map_err(|error| error.to_string())?,
        );
        let manifest = include_str!("../../../Cargo.toml")
            .parse::<toml::Table>()
            .map_err(|error| error.to_string())?;
        let deps = manifest
            .get("workspace")
            .and_then(Value::as_table)
            .and_then(|table| table.get("dependencies"))
            .and_then(Value::as_table)
            .ok_or("missing workspace dependency fixture")?;
        require(
            member(Role::Runtime, &valid, deps).is_ok(),
            "valid member inheritance",
        )?;
        let no_lints = source
            .replace("[lints]\nworkspace = true", "[lints]\nworkspace = false");
        let disabled_lints = Value::Table(
            no_lints
                .parse::<toml::Table>()
                .map_err(|error| error.to_string())?,
        );
        require(
            member(Role::Runtime, &disabled_lints, deps).is_err(),
            "lint inheritance must not be disabled",
        )?;
        let lints = valid.get_mut("lints").ok_or("lints missing")?;
        let previous = lints
            .as_table_mut()
            .ok_or("invalid lint table")?
            .insert(String::from("rust"), Value::Table(toml::Table::new()));
        require(
            previous.is_none(),
            "lint override was not initially present",
        )?;
        require(
            member(Role::Runtime, &valid, deps).is_err(),
            "local lint overrides must be rejected",
        )?;
        let source_with_dep = format!(
            "{source}\n[dev-dependencies]\nother = {{ workspace = true }}\n"
        );
        let unknown_dependency = Value::Table(
            source_with_dep
                .parse::<toml::Table>()
                .map_err(|error| error.to_string())?,
        );
        require(
            member(Role::Runtime, &unknown_dependency, deps).is_err(),
            "dependencies must be centrally declared",
        )
    }

    #[test]
    fn dependency_records_enforce_all_edge_types() -> Result<(), String> {
        let mut workspace = toml::Table::new();
        for (name, path) in [
            ("other", "crates/persistence"),
            ("title", "crates/game-simpsons-arcade"),
            ("importer", "crates/importer"),
            ("tool", "crates/download"),
        ] {
            let value = Value::Table(toml::Table::from_iter([(
                String::from("path"),
                Value::String(String::from(path)),
            )]));
            let previous = workspace.insert(String::from(name), value);
            require(previous.is_none(), "duplicate fixture dependency")?;
        }
        let inherited = Value::Table(toml::Table::from_iter([(
            String::from("workspace"),
            Value::Boolean(true),
        )]));
        require(
            dependency(Role::Tool, "other", &inherited, &workspace).is_ok(),
            "tools can use shared crates",
        )?;
        for (owner, target) in [
            (Role::Shared, "title"),
            (Role::Shared, "tool"),
            (Role::Runtime, "importer"),
            (Role::Title, "title"),
        ] {
            require(
                dependency(owner, target, &inherited, &workspace).is_err(),
                "forbidden workspace dependency direction",
            )?;
        }
        require(
            dependency(Role::Tool, "missing", &inherited, &workspace).is_err(),
            "undeclared dependency",
        )?;
        require(
            dependency(
                Role::Tool,
                "other",
                &Value::String(String::from("*")),
                &workspace,
            )
            .is_err(),
            "direct versions bypass workspace inheritance",
        )
    }

    fn metadata_fixture(path: &str, target_kind: &str) -> JsonValue {
        let source = Path::new(path).with_file_name("src/lib.rs");
        json!({
            "workspace_members": ["member"],
            "packages": [{
                "id": "member",
                "manifest_path": path,
                "targets": [{"kind": [target_kind], "src_path": source}]
            }]
        })
    }

    #[test]
    fn rejects_workspace_members_outside_the_direct_crates_layout() {
        let root = Path::new("repo");
        let authored = [PathBuf::from("crates/engine/src/lib.rs")];
        let valid = metadata_fixture("repo/crates/engine/Cargo.toml", "lib");
        assert_eq!(
            metadata_members(root, &valid, &authored),
            Ok(BTreeSet::from([PathBuf::from("crates/engine/Cargo.toml")]))
        );
        for path in [
            "other/crates/engine/Cargo.toml",
            "repo/tools/example/Cargo.toml",
            "repo/crates/engine/other/Cargo.toml",
            "repo/crates/../Cargo.toml",
        ] {
            assert!(
                metadata_members(
                    root,
                    &metadata_fixture(path, "lib"),
                    &authored
                )
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_separate_test_targets_and_incomplete_metadata() {
        let root = Path::new("repo");
        let authored = [PathBuf::from("crates/engine/src/lib.rs")];
        let integration =
            metadata_fixture("repo/crates/engine/Cargo.toml", "test");
        assert!(metadata_members(root, &integration, &authored).is_err());
        let incomplete = json!({
            "workspace_members": ["member"],
            "packages": []
        });
        assert!(metadata_members(root, &incomplete, &authored).is_err());
    }

    #[test]
    fn rejects_custom_targets_outside_the_owning_crate() {
        let root = Path::new("repo");
        let authored = [
            PathBuf::from("crates/engine/src/lib.rs"),
            PathBuf::from("crates/platform/src/lib.rs"),
        ];
        for source in [
            "repo/crates/platform/src/lib.rs",
            "repo/crates/engine/assets/ignored.rs",
            "repo/.reference/private.rs",
        ] {
            let metadata = json!({
                "workspace_members": ["member"],
                "packages": [{
                    "id": "member",
                    "manifest_path": "repo/crates/engine/Cargo.toml",
                    "targets": [{"kind": ["bin"], "src_path": source}]
                }]
            });
            assert!(metadata_members(root, &metadata, &authored).is_err());
        }
    }

    #[test]
    fn requires_absolute_workspace_safety_lints() -> Result<(), String> {
        let source = include_str!("../../../Cargo.toml");
        let workspace = |manifest: &str| {
            let value = manifest
                .parse::<toml::Table>()
                .map_err(|error| error.to_string())?;
            value
                .get("workspace")
                .and_then(Value::as_table)
                .cloned()
                .ok_or_else(|| String::from("missing fixture workspace"))
        };
        require(workspace_lints(&workspace(source)?).is_ok(), "root policy")?;
        for (from, to) in [
            ("unsafe_code = \"forbid\"", "unsafe_code = \"deny\""),
            ("warnings = \"deny\"", "warnings = \"warn\""),
            ("unfulfilled_lint_expectations = \"deny\"", ""),
        ] {
            let altered = source.replacen(from, to, 1);
            require(altered != source, "fixture must change")?;
            require(
                workspace_lints(&workspace(&altered)?).is_err(),
                "absolute lint policy cannot be weakened",
            )?;
        }
        Ok(())
    }
}
