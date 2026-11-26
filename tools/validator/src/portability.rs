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
//   - Strict platform isolation without per-file exceptions.
// - Must-Not:
//   - Implement platform access or interpret game rules.
// - Allows:
//   - Inputs: Authored Rust tokens and manifests.
//   - Outputs: Contract diagnostics with affected file paths.
//   - Side effects: Read-only repository access.
// - Split-When:
//   - A distinct policy cannot share source or dependency parsing.
// - Merge-When:
//   - The existing contract runner covers the same validation.
// - Summary:
//   - Platform isolation enforcement.
// - Description:
//   - Rejects all target and concrete backend coupling outside adapters.
// - Usage:
//   - Invoked by cargo validate via the contract runner.
// - Defaults:
//   - Any detected coupling is an error.
//
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use crate::boundaries::{dependency_tables, host_independent};
use crate::clones::tokens;
use crate::{files, host_api};

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

fn wire_layout(
    words: &[&str],
    index: usize,
    word: &str,
) -> Option<&'static str> {
    if matches!(word, "to_ne_bytes" | "from_ne_bytes") {
        return Some("native-endian-bytes");
    }
    if !matches!(word, "usize" | "isize") {
        return None;
    }
    let tail = words.get(index.checked_add(1)?..)?;
    let ([":", ":", path @ ..] | [">", ":", ":", path @ ..]) = tail else {
        return None;
    };
    let member = match path {
        ["r", "#", member, ..] | [member, ..] => *member,
        [] => return None,
    };
    matches!(
        member,
        "BITS"
            | "to_le_bytes"
            | "from_le_bytes"
            | "to_be_bytes"
            | "from_be_bytes"
    )
    .then_some("pointer-width-layout")
}

fn wire_layout_counts(words: &[&str], result: &mut Counts) {
    for (index, word) in words.iter().copied().enumerate() {
        if let Some(rule) = wire_layout(words, index, word) {
            record(result, rule);
        }
    }
}

fn rust(source: &str) -> Counts {
    let parsed = tokens(source);
    let words: Vec<_> =
        parsed.iter().map(|token| token.text.as_str()).collect();
    let mut result = Counts::new();
    wire_layout_counts(&words, &mut result);
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
        if os(word)
            && matches!(prev, Some(":" | "(" | "," | "{" | "mod" | "use"))
        {
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

fn source_counts(name: &str, source: &str) -> Result<Counts, String> {
    let mut counts = rust(source);
    if neutral_ir(name)
        || host_independent(name)
        || name.starts_with("crates/platform/src/hardware/")
    {
        for violation in host_api::findings(source)
            .map_err(|error| format!("{name}: {error}"))?
        {
            record(&mut counts, &format!("host:{violation}"));
        }
    }
    Ok(counts)
}

fn inventory(
    root: &OsStr,
    authored: &[OsString],
    workspace: &toml::Table,
) -> Result<Inventory, String> {
    let mut found = Inventory::new();
    for path in authored {
        let name = path.to_string_lossy().replace('\\', "/");
        if !(name.starts_with("crates/") || name.starts_with("tools/"))
            || adapter(&name)
        {
            continue;
        }
        let is_rust =
            files::authored_extension(path).is_some_and(|ext| ext == "rs");
        let is_manifest =
            files::authored_leaf(path).is_some_and(|ext| ext == "Cargo.toml");
        if !is_rust && !is_manifest {
            continue;
        }
        let text = files::read_text(root, path)?;
        let counts = if is_rust {
            source_counts(&name, &text)?
        } else {
            manifest(&text, workspace)?
        };
        if !counts.is_empty() {
            let _previous = found.insert(name, counts);
        }
    }
    Ok(found)
}

fn reject_coupling(found: &Inventory) -> Result<(), String> {
    if found.is_empty() {
        return Ok(());
    }
    let details = serde_json::to_string_pretty(found)
        .map_err(|error| error.to_string())?;
    Err(format!("platform coupling outside adapters:\n{details}"))
}

/// Reject every occurrence of platform coupling outside adapters.
/// # Errors
/// Reports unreadable sources, invalid manifests or any coupling.
pub fn validate(
    root: &OsStr,
    authored: &[OsString],
    workspace: &toml::Table,
) -> Result<(), String> {
    reject_coupling(&inventory(root, authored, workspace)?)
}

/// Check the actual repository before any cached validation can pass.
/// # Errors
/// Rejects unreadable metadata, authored inventory or platform coupling.
pub fn preflight(root: &OsStr) -> Result<(), String> {
    let source = files::read_text(root, OsStr::new("Cargo.toml"))?;
    let document = source
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    let workspace = document
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get("dependencies"))
        .and_then(toml::Value::as_table)
        .ok_or("missing workspace dependencies")?;
    let authored = files::authored(root)?;
    validate(root, &authored, workspace)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{
        Counts, Inventory, adapter, manifest, neutral_ir, reject_coupling,
        rust, source_counts, validate,
    };
    use crate::fixtures::{directory, require};

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
    fn title_host_io_is_rejected_but_opaque_values_are_allowed()
    -> Result<(), String> {
        for name in [
            "crates/games/example/src/lib.rs",
            "crates/apps/example/src/main.rs",
        ] {
            let rejected =
                source_counts(name, "use std::fs; use std::path::Path;")?;
            require(
                rejected.contains_key("host:std::fs"),
                "title filesystem access",
            )?;
            require(
                rejected.contains_key("host:std::path"),
                "title native path access",
            )?;
            let admitted = source_counts(
                name,
                "use std::ffi::OsStr; use std::io::Error;",
            )?;
            require(admitted.is_empty(), "title opaque path and error values")?;
        }
        Ok(())
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
    fn wire_layout_must_not_depend_on_native_integer_representation() {
        for source in [
            "let _ = value.to_ne_bytes();",
            "let _ = u32::from_ne_bytes(bytes);",
            "let _ = u32::r#from_ne_bytes(bytes);",
            "macro_rules! decode { () => { u32::from_ne_bytes(bytes) }; }",
        ] {
            assert_eq!(rust(source).get("native-endian-bytes"), Some(&1));
        }
        for source in [
            "let _ = usize::BITS;",
            "let _ = isize::from_be_bytes(bytes);",
            "let _ = core::primitive::usize::to_le_bytes(value);",
            "let _ = <usize>::r#to_be_bytes(value);",
        ] {
            assert_eq!(rust(source).get("pointer-width-layout"), Some(&1));
        }
        let admitted = concat!(
            "let _ = u32::from_le_bytes(bytes);",
            "let _ = u64::to_be_bytes(value);",
            "let _ = usize::try_from(value);",
            "let _ = usize::MAX;",
            "let _ = \"usize::BITS and from_ne_bytes\";",
            "/* isize::from_be_bytes(bytes) */",
        );
        assert!(rust(admitted).is_empty());
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
    fn real_inventory_rejects_previously_allowed_source() -> Result<(), String>
    {
        let root_owner = directory()?;
        let root = root_owner.location();
        let source = OsString::from("crates/games/example/src/lib.rs");
        root_owner
            .create_directories("crates/games/example/src")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file(
                "crates/games/example/src/lib.rs",
                b"#[cfg(unix)] fn run() {}",
            )
            .map_err(|error| error.to_string())?;
        let result = validate(root, &[source], &toml::Table::new());
        require(
            result.is_err_and(|error| error.contains("os:unix")),
            "strict inventory must reject actual authored OS coupling",
        )
    }

    #[test]
    fn linux_entrypoints_and_tools_never_receive_allowances()
    -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        let authored = [
            "crates/games/example/src/lib.rs",
            "crates/games/example/src/main.rs",
            "tools/build/src/main.rs",
            "tools/validator/src/validate.rs",
        ]
        .map(OsString::from);
        for relative in &authored {
            root_owner
                .create_directories(
                    relative
                        .to_str()
                        .and_then(|name| {
                            name.rsplit_once('/').map(|(parent, _)| parent)
                        })
                        .ok_or("fixture parent")?,
                )
                .map_err(|error| error.to_string())?;
            let predicate = concat!(
                "#[cfg(all(target_os = \"linux\",",
                " target_env = \"gnu\"))] fn run() {}"
            );
            root_owner
                .write_file(
                    relative.to_str().ok_or("fixture path")?,
                    predicate.as_bytes(),
                )
                .map_err(|error| error.to_string())?;
        }
        let result = validate(root, &authored, &toml::Table::new());
        let error = result.err().ok_or("accepted target coupling")?;
        for path in authored {
            require(
                error.contains(path.to_string_lossy().as_ref()),
                "missing path",
            )?;
        }
        Ok(())
    }

    #[test]
    fn host_and_target_findings_are_reported_together() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        let paths = [
            "crates/platform/src/ir/example/mod.rs",
            "crates/platform/src/lib.rs",
            "tools/build/src/main.rs",
        ]
        .map(OsString::from);
        for (relative, source) in paths.iter().zip([
            "use std::fs;",
            "#[cfg(unix)] mod backend;",
            "#[cfg(target_os = \"linux\")] fn run() {}",
        ]) {
            root_owner
                .create_directories(
                    relative
                        .to_str()
                        .and_then(|name| {
                            name.rsplit_once('/').map(|(parent, _)| parent)
                        })
                        .ok_or("fixture parent")?,
                )
                .map_err(|error| error.to_string())?;
            root_owner
                .write_file(
                    relative.to_str().ok_or("fixture path")?,
                    source.as_bytes(),
                )
                .map_err(|error| error.to_string())?;
        }
        let result = validate(root, &paths, &toml::Table::new());
        let error = result.err().ok_or("accepted portability findings")?;
        for diagnostic in ["host:std::fs", "os:unix", "target:target_os"] {
            require(error.contains(diagnostic), diagnostic)?;
        }
        Ok(())
    }

    #[test]
    fn portable_source_cannot_embed_host_files_or_environment()
    -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        let authored = [
            "crates/engine/src/lib.rs",
            "crates/importer/src/lib.rs",
            "crates/platform/src/ir/example/mod.rs",
            "crates/platform/src/hardware/example/mod.rs",
            "tools/download/src/download.rs",
            "tools/validator/src/validate.rs",
            "tools/future-tool/src/main.rs",
            "crates/platform/src/software/linux/example/mod.rs",
        ]
        .map(OsString::from);
        for relative in &authored {
            root_owner
                .create_directories(
                    relative
                        .to_str()
                        .and_then(|name| {
                            name.rsplit_once('/').map(|(parent, _)| parent)
                        })
                        .ok_or("fixture parent")?,
                )
                .map_err(|error| error.to_string())?;
            root_owner
                .write_file(
                    relative.to_str().ok_or("fixture path")?,
                    br#"
                const SECRET: &str = env!("TOKEN");
                const BYTES: &[u8] = include_bytes!("asset.zip");
                "#,
                )
                .map_err(|error| error.to_string())?;
        }
        let result = validate(root, &authored, &toml::Table::new());
        let error = result.err().ok_or("accepted compile-time host inputs")?;
        for relative in authored.iter().take(7) {
            require(
                error.contains(relative.to_string_lossy().as_ref()),
                "missing portable path",
            )?;
        }
        for diagnostic in [
            "host:compile-time environment macro",
            "host:compile-time file macro",
        ] {
            require(error.contains(diagnostic), diagnostic)?;
        }
        require(
            !error.contains("software/linux/example"),
            "software adapter remains the host boundary",
        )
    }

    #[test]
    fn zero_coupling_is_the_only_admitted_inventory() -> Result<(), String> {
        require(
            reject_coupling(&Inventory::new()).is_ok(),
            "empty inventory",
        )?;
        for count in [1, 2, 100] {
            let found = Inventory::from([(
                "old.rs".to_owned(),
                Counts::from([("os:linux".to_owned(), count)]),
            )]);
            require(
                reject_coupling(&found).is_err_and(|error| {
                    error.contains("old.rs") && error.contains("os:linux")
                }),
                "previously admitted coupling must also fail",
            )?;
        }
        Ok(())
    }
}
