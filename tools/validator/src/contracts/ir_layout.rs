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
//   - Canonical platform and hardware source directory ownership.
// - Must-Not:
//   - Implement host adapters or interpret gameplay.
// - Allows:
//   - Inputs: Authored paths in the repository inventory.
//   - Outputs: Diagnostics for source outside canonical owner directories.
//   - Side effects: None.
// - Split-When:
//   - Semantic name resolution needs an independent validator.
// - Merge-When:
//   - A stronger parsed policy covers equivalent host API checks.
// - Summary:
//   - Enforce one integration owner and its IR/backend layout.
// - Description:
//   - Reject new split integration owners and misplaced source files.
// - Usage:
//   - Invoked by the mandatory architecture contract runner.
// - Defaults:
//   - Require canonical owners and directory module roots in the IR.

use std::collections::BTreeSet;
use std::ffi::OsString;

use crate::files::{authored_extension, authored_prefix};

fn os_backend(name: &str) -> bool {
    matches!(
        name,
        "linux" | "windows" | "macos" | "android" | "web" | "ios" | "freebsd"
    )
}

fn device_family(name: &str) -> bool {
    matches!(
        name,
        "playstation"
            | "xbox"
            | "nintendo"
            | "keyboard"
            | "mouse"
            | "logitech"
            | "wheel"
            | "touch"
            | "hid"
            | "generic"
    )
}

fn backend_path(path: &str, allowed: fn(&str) -> bool) -> bool {
    path == "mod.rs"
        || path.split_once('/').is_some_and(|(family, remainder)| {
            allowed(family) && !remainder.is_empty()
        })
}

fn graphics_path(relative: &str) -> bool {
    let Some(("software", remainder)) = relative.split_once('/') else {
        return true;
    };
    let Some((_, capability)) = remainder.split_once('/') else {
        return true;
    };
    let Some(path) = capability.strip_prefix("graphics/") else {
        return true;
    };
    !path.split('/').any(|part| {
        ["audio", "audio.rs", "input", "input.rs", "sprites.rs"]
            .iter()
            .any(|name| part.eq_ignore_ascii_case(name))
    })
}

fn ambiguous_directory(part: &str) -> bool {
    ["host", "native"]
        .iter()
        .any(|name| part.eq_ignore_ascii_case(name))
}

fn owner_source(relative: &str) -> bool {
    if !graphics_path(relative) {
        return false;
    }
    if relative.split('/').any(ambiguous_directory) {
        return false;
    }
    if relative == "lib.rs" {
        return true;
    }
    let Some((directory, remainder)) = relative.split_once('/') else {
        return false;
    };
    match directory {
        "ir" => remainder == "mod.rs" || remainder.ends_with("/mod.rs"),
        "software" => backend_path(remainder, os_backend),
        "hardware" => backend_path(remainder, device_family),
        _ => false,
    }
}

fn path_allowed(path: &str) -> bool {
    let mut parts = path.split('/');
    if parts.next() != Some("crates") {
        return true;
    }
    let Some(owner) = parts.next() else {
        return true;
    };
    if owner == "hardware"
        || owner.starts_with("hardware-")
        || owner.starts_with("platform-")
    {
        return false;
    }
    if matches!(owner, "games" | "apps") {
        return !parts.any(ambiguous_directory);
    }
    if owner != "platform" {
        return true;
    }
    let relative = parts.collect::<Vec<_>>().join("/");
    if let Some(source) = relative.strip_prefix("src/") {
        return owner_source(source) && !source.ends_with("Cargo.toml");
    }
    matches!(relative.as_str(), "Cargo.toml" | "LICENSE" | "README.md")
}

fn directory_modules(authored: &[OsString], root: &str) -> Result<(), String> {
    let mut required = BTreeSet::new();
    for path in authored.iter().filter(|path| {
        authored_prefix(path, root)
            && authored_extension(path).is_some_and(|ext| ext == "rs")
    }) {
        let mut name = path.to_str().ok_or("invalid authored path")?;
        while let Some((directory, _)) = name.rsplit_once('/') {
            if directory == root {
                break;
            }
            let _inserted =
                required.insert(OsString::from(format!("{directory}/mod.rs")));
            name = directory;
        }
    }
    for module in required {
        if !authored.contains(&module) {
            return Err(format!(
                "missing directory module: {}",
                module.to_string_lossy()
            ));
        }
    }
    Ok(())
}

pub(super) fn validate(authored: &[OsString]) -> Result<(), String> {
    for path in authored {
        let name = path.to_string_lossy().replace('\\', "/");
        if !path_allowed(&name) {
            return Err(format!(
                "noncanonical integration owner or path: {name}"
            ));
        }
    }
    let root = "crates/platform/src";
    if !authored.iter().any(|path| authored_prefix(path, root)) {
        return Ok(());
    }
    for module in [
        "lib.rs",
        "ir/mod.rs",
        "ir/security/mod.rs",
        "ir/hardware/mod.rs",
        "software/mod.rs",
    ] {
        if !authored.contains(&OsString::from(format!("{root}/{module}"))) {
            return Err(format!("missing canonical module: {module}"));
        }
    }
    directory_modules(authored, root)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{path_allowed, validate};

    #[test]
    fn canonical_paths_are_admitted() {
        for path in [
            "crates/platform/src/lib.rs",
            "crates/platform/src/ir/security/mod.rs",
            "crates/platform/src/ir/audio/mod.rs",
            "crates/platform/src/ir/security/lan/host_binding/mod.rs",
            "crates/platform/src/ir/ui/scene/mod.rs",
            "crates/platform/src/software/linux/file.rs",
            "crates/platform/src/software/windows/input.rs",
            "crates/platform/src/software/linux/toolchain/mod.rs",
            "crates/platform/src/hardware/playstation/four.rs",
            "crates/platform/src/hardware/nintendo/pro_one.rs",
            "crates/games/test/src/lib.rs",
            "crates/games/test/src/runtime/mod.rs",
            "crates/platform/src/software/linux/audio/mod.rs",
            "crates/platform/src/software/linux/input/mod.rs",
            "crates/platform/src/software/linux/graphics/scene_renderer.rs",
            "tools/build/src/main.rs",
        ] {
            assert!(path_allowed(path), "{path}");
        }
    }

    #[test]
    fn misplaced_and_split_owners_are_rejected() {
        for path in [
            "crates/platform/src/file.rs",
            "crates/games/test/src/native/controller.rs",
            "crates/apps/test/src/host/window.rs",
            "crates/platform/src/software/linux/graphics/audio.rs",
            "crates/platform/src/software/linux/graphics/audio/mod.rs",
            "crates/platform/src/software/linux/graphics/input/mod.rs",
            "crates/platform/src/software/windows/graphics/input.rs",
            "crates/platform/src/software/linux/graphics/sprites.rs",
            "crates/platform/src/ir/audio.rs",
            "crates/platform/src/ir/security/audio.rs",
            "crates/platform/src/ir/ui/menu.rs",
            "crates/platform/src/ir/security/lan/host.rs",
            "crates/platform/src/ir/graphics/helper.rs",
            "crates/platform/src/ir/host/Cargo.toml",
            "crates/platform/src/ir.rs",
            "crates/platform/src/hardware/linux/input.rs",
            "crates/platform/src/hardware/xinput/mod.rs",
            "crates/platform/src/software/native/host/mod.rs",
            "crates/platform/src/ir/native/mod.rs",
            "crates/platform/src/ir/host/mod.rs",
            "crates/platform/src/software/linux/host/mod.rs",
            "crates/platform/src/software/playstation/four.rs",
            "crates/hardware/src/input.rs",
            "crates/hardware-audio/Cargo.toml",
            "crates/platform-windowing/src/lib.rs",
        ] {
            assert!(!path_allowed(path), "{path}");
        }
    }

    #[test]
    fn misplaced_capabilities_cannot_hide_under_nested_paths() {
        for path in [
            "crates/games/example/src/runtime/Native/input.rs",
            "crates/apps/example/src/runtime/HOST/file.rs",
            "crates/platform/src/software/linux/graphics/cache/audio/mod.rs",
            "crates/platform/src/software/windows/graphics/cache/input.rs",
            "crates/platform/src/software/linux/graphics/cache/sprites.rs",
            "crates/platform/src/ir/Native/toolchain.rs",
        ] {
            assert!(!path_allowed(path), "{path}");
        }
    }

    #[test]
    fn nested_ir_modules_require_directory_roots() {
        let root = "crates/platform/src";
        let mut paths = [
            "lib.rs",
            "ir/mod.rs",
            "ir/security/mod.rs",
            "ir/hardware/mod.rs",
            "software/mod.rs",
            "ir/security/audio/inner/mod.rs",
        ]
        .into_iter()
        .map(|path| OsString::from(format!("{root}/{path}")))
        .collect::<Vec<_>>();
        assert!(validate(&paths).is_err());
        paths.push(OsString::from(format!("{root}/ir/security/audio/mod.rs")));
        assert!(validate(&paths).is_ok());
    }

    #[test]
    fn ir_requires_security_and_bridge_roots() {
        let root = "crates/platform/src";
        let mut paths = vec![
            OsString::from(format!("{root}/lib.rs")),
            OsString::from(format!("{root}/ir/mod.rs")),
        ];
        assert!(validate(&paths).is_err());
        paths.push(OsString::from(format!("{root}/ir/security/mod.rs")));
        paths.push(OsString::from(format!("{root}/software/mod.rs")));
        assert!(validate(&paths).is_err());
        paths.push(OsString::from(format!("{root}/ir/hardware/mod.rs")));
        assert!(validate(&paths).is_ok());
        paths.push(OsString::from(format!(
            "{root}/hardware/playstation/four.rs"
        )));
        assert!(validate(&paths).is_err());
        paths.push(OsString::from(format!("{root}/hardware/mod.rs")));
        assert!(validate(&paths).is_err());
        paths.push(OsString::from(format!(
            "{root}/hardware/playstation/mod.rs"
        )));
        assert!(validate(&paths).is_ok());
    }
}
