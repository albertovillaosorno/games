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
//   - Platform isolation and portable-capability migration ceilings.
// - Must-Not:
//   - Implement platform access or interpret game rules.
// - Allows:
//   - Inputs: Authored Rust tokens, manifests and recorded migration debt.
//   - Outputs: Contract diagnostics with affected file paths.
//   - Side effects: Read-only repository access.
// - Split-When:
//   - A distinct policy cannot share source or dependency parsing.
// - Merge-When:
//   - The existing contract runner covers the same validation.
// - Summary:
//   - Platform isolation enforcement.
// - Description:
//   - Rejects new target and concrete backend coupling outside adapters.
// - Usage:
//   - Invoked by cargo validate via the contract runner.
// - Defaults:
//   - Reject new coupling; existing exceptions must shrink to zero.
//
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::boundaries::{dependency_tables, host_independent};
use crate::clones::tokens;
use crate::host_api;

const LEGACY: &str = include_str!("portability-baseline.json");

type Counts = BTreeMap<String, usize>;
type Inventory = BTreeMap<String, Counts>;

fn neutral_ir(path: &str) -> bool {
    let mut components = path.split('/');
    components.next() == Some("crates")
        && matches!(components.next(), Some("platform" | "hardware"))
        && components.next() == Some("src")
        && matches!(components.next(), Some("ir" | "ir.rs"))
}

fn adapter(path: &str) -> bool {
    path == "crates/platform/Cargo.toml"
        || path == "crates/platform/src/lib.rs"
        || path.starts_with("crates/platform/src/software/")
}

fn record(counts: &mut Counts, rule: &str) {
    let count = counts.entry(rule.to_owned()).or_default();
    *count = count.saturating_add(1);
}

fn sdk(token: &str) -> bool {
    matches!(
        token,
        "sdl2"
            | "sdl3"
            | "nix"
            | "libc"
            | "windows"
            | "winapi"
            | "winit"
            | "wgpu"
            | "ash"
            | "glow"
            | "cpal"
            | "gilrs"
            | "x11"
            | "wayland_client"
            | "objc2"
            | "jni"
            | "ndk"
            | "web_sys"
            | "js_sys"
            | "wasm_bindgen"
    )
}

fn os(token: &str) -> bool {
    matches!(
        token,
        "linux"
            | "windows"
            | "unix"
            | "android"
            | "macos"
            | "ios"
            | "freebsd"
            | "wasm32"
            | "x86_64"
            | "aarch64"
    )
}

fn rust(source: &str) -> Counts {
    let parsed = tokens(source);
    let words: Vec<_> =
        parsed.iter().map(|token| token.text.as_str()).collect();
    let mut result = Counts::new();
    for (index, word) in words.iter().copied().enumerate() {
        let next = words.get(index.saturating_add(1)).copied();
        let prev = index.checked_sub(1).and_then(|i| words.get(i)).copied();
        if matches!(
            word,
            "target_os"
                | "target_arch"
                | "target_env"
                | "target_family"
                | "target_vendor"
                | "target_feature"
                | "target_endian"
                | "target_pointer_width"
                | "target_abi"
                | "target_has_atomic"
        ) && matches!(next, Some("="))
        {
            let value = words
                .get(index.saturating_add(2))
                .copied()
                .unwrap_or_default();
            record(&mut result, &format!("target:{word}:{value}"));
        }
        if os(word) && matches!(prev, Some(":" | "(" | "," | "mod" | "use")) {
            record(&mut result, &format!("os:{word}"));
        }
        if sdk(word) && matches!(next, Some(":" | ";")) {
            record(&mut result, &format!("sdk:{word}"));
        }
        if word == "os" && prev == Some(":") && next == Some(":") {
            record(&mut result, "os-api");
        }
    }
    result
}

fn sdk_dependency(
    name: &str,
    entry: &toml::Value,
    workspace: &toml::Table,
) -> Result<Option<String>, String> {
    let specification = if entry.get("workspace").and_then(toml::Value::as_bool)
        == Some(true)
    {
        workspace
            .get(name)
            .ok_or_else(|| format!("dependency not in workspace: {name}"))?
    } else {
        entry
    };
    let package = specification
        .get("package")
        .and_then(toml::Value::as_str)
        .unwrap_or(name)
        .replace('-', "_");
    Ok(sdk(&package).then_some(package))
}

fn manifest(source: &str, workspace: &toml::Table) -> Result<Counts, String> {
    let document = toml::Value::Table(
        source
            .parse::<toml::Table>()
            .map_err(|error| error.to_string())?,
    );
    let mut result = Counts::new();
    if let Some(targets) =
        document.get("target").and_then(toml::Value::as_table)
    {
        for name in targets.keys() {
            record(&mut result, &format!("target-table:{name}"));
        }
    }
    for entries in dependency_tables(&document)? {
        for (name, entry) in entries {
            if let Some(package) = sdk_dependency(name, entry, workspace)? {
                record(&mut result, &format!("sdk:{package}"));
            }
        }
    }
    Ok(result)
}

fn inventory(
    root: &Path,
    authored: &[PathBuf],
    workspace: &toml::Table,
) -> Result<Inventory, String> {
    let mut found = Inventory::new();
    for path in authored {
        let name = path.to_string_lossy().replace('\\', "/");
        if !(name.starts_with("crates/") || name.starts_with("tools/"))
            || adapter(&name)
            || name == "tools/validator/src/portability.rs"
        {
            continue;
        }
        let is_rust = path.extension().is_some_and(|ext| ext == "rs");
        let is_manifest =
            path.file_name().is_some_and(|ext| ext == "Cargo.toml");
        if !is_rust && !is_manifest {
            continue;
        }
        let text = fs::read_to_string(root.join(path))
            .map_err(|error| error.to_string())?;
        let counts = if is_rust {
            if neutral_ir(&name) || host_independent(&name) {
                host_api::validate(&text)
                    .map_err(|error| format!("{name}: {error}"))?;
            }
            rust(&text)
        } else {
            manifest(&text, workspace)?
        };
        if !counts.is_empty() {
            let _previous = found.insert(name, counts);
        }
    }
    Ok(found)
}

fn excess(found: &Inventory, legacy: &Inventory) -> Inventory {
    let mut violations = Inventory::new();
    for (path, counts) in found {
        for (rule, count) in counts {
            let ceiling = legacy
                .get(path)
                .and_then(|rules| rules.get(rule))
                .copied()
                .unwrap_or_default();
            if *count > ceiling {
                let _previous = violations
                    .entry(path.clone())
                    .or_default()
                    .insert(rule.clone(), *count);
            }
        }
    }
    violations
}

fn compare(found: &Inventory, legacy: &Inventory) -> Result<(), String> {
    let violations = excess(found, legacy);
    if !violations.is_empty() {
        let details = serde_json::to_string_pretty(&violations)
            .map_err(|error| error.to_string())?;
        return Err(format!(
            "new platform coupling outside adapters:\n{details}"
        ));
    }
    if found == legacy {
        return Ok(());
    }
    let details = serde_json::to_string_pretty(found)
        .map_err(|error| error.to_string())?;
    Err(format!(
        "stale platform baseline; use the exact current inventory:\n{details}"
    ))
}

/// Require an exact inventory of remaining platform migration debt.
///
/// # Errors
/// Reports unreadable sources, invalid or stale baseline, or new coupling.
pub fn validate(
    root: &Path,
    authored: &[PathBuf],
    workspace: &toml::Table,
) -> Result<(), String> {
    let legacy: Inventory =
        serde_json::from_str(LEGACY).map_err(|error| error.to_string())?;
    let found = inventory(root, authored, workspace)?;
    compare(&found, &legacy)
}

#[cfg(test)]
mod tests {
    use super::{
        Counts, Inventory, adapter, compare, excess, manifest, neutral_ir, rust,
    };
    use crate::fixtures::require;

    #[test]
    fn neutral_code_and_comments_are_admitted() {
        assert!(
            rust("fn run() { /* sdl2::x */ let _ = \"linux\"; }").is_empty()
        );
        assert!(adapter("crates/platform/src/software/linux/input.rs"));
        assert!(!adapter("crates/game-linux/src/main.rs"));
    }

    #[test]
    fn owning_ir_does_not_inherit_adapter_exemption() {
        for path in [
            "crates/platform/src/ir/network.rs",
            "crates/hardware/src/ir/controller.rs",
            "crates/platform/src/ir.rs",
        ] {
            assert!(neutral_ir(path));
            assert!(!adapter(path));
        }
        assert!(!neutral_ir("crates/platform/src/software/linux/network.rs"));
        let coupling = rust("use crate::linux::native;");
        assert_eq!(coupling.get("os:linux"), Some(&1));
    }

    #[test]
    fn direct_platform_dependencies_are_detected() {
        let actual = rust(concat!(
            "#[cfg(target_os = \"windows\")] use std :: os :: windows::fs;",
            "use sdl2 :: event; use games_platform :: linux :: input;",
        ));
        for rule in [
            "target:target_os:\"windows\"",
            "os:windows",
            "sdk:sdl2",
            "os:linux",
            "os-api",
        ] {
            assert!(actual.contains_key(rule), "{rule}");
        }
        assert!(
            manifest(
                "[target.'cfg(unix)'.dependencies]\nsdl2 = \"1\"",
                &toml::Table::new(),
            )
            .is_ok_and(|cargo| cargo.get("sdk:sdl2") == Some(&1))
        );
    }

    #[test]
    fn sdk_aliases_in_all_dependency_groups() -> Result<(), String> {
        let workspace = concat!(
            "input = { package = 'sdl2', version = '1' }\n",
            "browser = { package = 'web-sys', version = '1' }\n",
            "ordinary = '1'\n",
            "unused = { package = 'wgpu', version = '1' }\n",
        )
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
        let counts = manifest(
            concat!(
                "[dependencies]\n",
                "input.workspace = true\n",
                "ordinary.workspace = true\n",
                "[dev-dependencies]\n",
                "input.workspace = true\n",
                "[build-dependencies]\n",
                "vendor = { package = 'libc', version = '1' }\n",
                "[target.'cfg(unix)'.dev-dependencies]\n",
                "browser.workspace = true\n",
                "[target.'cfg(unix)'.build-dependencies]\n",
                "graphics = { package = 'wayland-client', version = '1' }\n",
            ),
            &workspace,
        )?;
        require(counts.get("sdk:sdl2") == Some(&2), "SDL aliases")?;
        for name in ["sdk:libc", "sdk:web_sys", "sdk:wayland_client"] {
            require(counts.get(name) == Some(&1), name)?;
        }
        require(!counts.contains_key("sdk:wgpu"), "unused workspace SDK")?;
        require(counts.len() == 5, "expected exact inventory")?;
        require(
            manifest("[dependencies]\nmissing.workspace = true", &workspace)
                .is_err(),
            "missing inherited package",
        )
    }

    #[test]
    fn migration_reductions_must_update_baseline() -> Result<(), String> {
        let prior = Inventory::from([(
            "old.rs".to_owned(),
            Counts::from([("os:linux".to_owned(), 2)]),
        )]);
        let current = Inventory::from([(
            "old.rs".to_owned(),
            Counts::from([("os:linux".to_owned(), 1)]),
        )]);
        require(compare(&prior, &prior).is_ok(), "equal inventory")?;
        let stale = compare(&current, &prior);
        require(
            stale.is_err_and(|error| error.contains("stale platform baseline")),
            "reject stale allowance",
        )?;
        require(compare(&current, &current).is_ok(), "admit reduction")?;
        require(
            compare(&Inventory::new(), &current).is_err(),
            "remove obsolete entries",
        )?;
        require(compare(&prior, &current).is_err(), "reject reintroduction")
    }

    #[test]
    fn migration_ceiling_cannot_expand() {
        let legacy = Inventory::from([(
            "old.rs".to_owned(),
            Counts::from([("os:linux".to_owned(), 1)]),
        )]);
        assert!(excess(&legacy, &legacy).is_empty());
        let found = Inventory::from([(
            "new.rs".to_owned(),
            Counts::from([("os:linux".to_owned(), 1)]),
        )]);
        assert_eq!(excess(&found, &legacy), found);
    }
}
