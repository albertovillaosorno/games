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
//   - Negative controls for source and dependency contracts.
// - Must-Not:
//   - Alter the real repository or depend on game media.
// - Allows:
//   - Inputs: Synthetic manifests and known authored headers.
//   - Outputs: Assertions about contract acceptance and rejection.
//   - Side effects: None.
// - Split-When:
//   - A separate architecture fixture responsibility arises.
// - Merge-When:
//   - Shared contract fixtures supersede these tests.
// - Summary:
//   - Architecture enforcement tests.
// - Description:
//   - Exercises missing ownership, inheritance and forbidden edges.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No original media or live repositories required.

use toml::Value;

use super::{Role, dependency, dependency_allowed, header, member};
use crate::fixtures::require;

#[test]
fn requires_meaningful_source_contracts() {
    let valid = include_str!("../../tools/validator/src/contracts.rs");
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
    let source = include_str!("../../crates/engine/Cargo.toml");
    let mut valid = Value::Table(
        source
            .parse::<toml::Table>()
            .map_err(|error| error.to_string())?,
    );
    let manifest = include_str!("../../Cargo.toml")
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
        ("title", "games/simpsons-arcade"),
        ("importer", "crates/importer"),
        ("tool", "tools/download"),
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
