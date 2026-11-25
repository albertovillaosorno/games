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

use crate::clones::tokens;

const LEGACY: &str = include_str!("portability-baseline.json");

type Counts = BTreeMap<String, usize>;
type Inventory = BTreeMap<String, Counts>;

fn adapter(path: &str) -> bool {
    matches!(
        path.split('/').nth(1),
        Some(
            "platform"
                | "platform-audio"
                | "platform-graphics"
                | "platform-storage"
        )
    )
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

fn manifest(source: &str) -> Result<Counts, String> {
    let document = source
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    let mut result = Counts::new();
    if let Some(targets) =
        document.get("target").and_then(toml::Value::as_table)
    {
        for name in targets.keys() {
            record(&mut result, &format!("target-table:{name}"));
        }
    }
    for group in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(entries) =
            document.get(group).and_then(toml::Value::as_table)
        {
            for name in entries.keys().filter(|name| sdk(name)) {
                record(&mut result, &format!("sdk:{name}"));
            }
        }
    }
    Ok(result)
}

fn inventory(root: &Path, authored: &[PathBuf]) -> Result<Inventory, String> {
    let mut found = Inventory::new();
    for path in authored {
        let name = path.to_string_lossy().replace('\\', "/");
        if !name.starts_with("crates/")
            || adapter(&name)
            || name == "crates/validator/src/portability.rs"
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
            rust(&text)
        } else {
            manifest(&text)?
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

/// Reject newly introduced platform-specific coupling outside adapters.
///
/// # Errors
/// Reports unreadable sources, invalid baseline or additional coupling.
pub fn validate(root: &Path, authored: &[PathBuf]) -> Result<(), String> {
    let legacy: Inventory =
        serde_json::from_str(LEGACY).map_err(|error| error.to_string())?;
    let found = inventory(root, authored)?;
    let violations = excess(&found, &legacy);
    if violations.is_empty() {
        return Ok(());
    }
    let details = serde_json::to_string_pretty(&violations)
        .map_err(|error| error.to_string())?;
    Err(format!(
        "new platform coupling outside adapters:\n{details}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{Counts, Inventory, adapter, excess, manifest, rust};

    #[test]
    fn neutral_code_and_comments_are_admitted() {
        assert!(
            rust("fn run() { /* sdl2::x */ let _ = \"linux\"; }").is_empty()
        );
        assert!(adapter("crates/platform-graphics/src/linux/input.rs"));
        assert!(!adapter("crates/game-linux/src/main.rs"));
    }

    #[test]
    fn direct_platform_dependencies_are_detected() {
        let actual = rust(concat!(
            "#[cfg(target_os = \"windows\")] use std :: os :: windows::fs;",
            "use sdl2 :: event; use games_platform_graphics :: linux :: ui;",
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
            manifest("[target.'cfg(unix)'.dependencies]\nsdl2 = \"1\"")
                .is_ok_and(|cargo| !cargo.is_empty())
        );
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
