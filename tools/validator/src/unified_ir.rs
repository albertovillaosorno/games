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
//   - Unified IR consumer and physical device protocol boundaries.
// - Must-Not:
//   - Implement device protocols or resolve arbitrary name bindings.
// - Allows:
//   - Inputs: Authored Rust sources and owned path inventory.
//   - Outputs: Diagnostics for hardware host access and backend imports.
//   - Side effects: None.
// - Split-When:
//   - Semantic name resolution needs an independent validator.
// - Merge-When:
//   - A stronger parsed policy covers equivalent host API checks.
// - Summary:
//   - Keep physical device translation independent of host software.
// - Description:
//   - Reject direct host APIs and concrete integration imports.
// - Usage:
//   - Invoked by the architecture contract runner.
// - Defaults:
//   - Allow neutral IR and reject new concrete backend access.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};

use crate::clones::tokens;
use crate::{files, host_api};

fn native_word(word: &str) -> bool {
    matches!(
        word,
        "linux"
            | "windows"
            | "unix"
            | "android"
            | "macos"
            | "ios"
            | "freebsd"
            | "wasm32"
            | "aarch64"
            | "x86_64"
            | "xinput"
            | "sdl2"
            | "sdl3"
            | "nix"
            | "libc"
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
            | "target_os"
            | "target_arch"
            | "target_env"
            | "target_family"
            | "target_feature"
            | "target_vendor"
            | "target_pointer_width"
            | "target_endian"
            | "target_abi"
            | "target_has_atomic"
    )
}

fn graphics_vendor(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "nvidia"
            | "nvapi"
            | "cuda"
            | "amd"
            | "amdgpu"
            | "radeon"
            | "intel"
            | "vulkan"
            | "direct3d"
            | "dxgi"
            | "d3d11"
            | "d3d12"
            | "opengl"
    )
}

fn vendor_free(source: &str) -> Result<(), String> {
    for token in tokens(source) {
        let word = token.text.trim_matches('"');
        if graphics_vendor(word) {
            return Err(format!(
                "request graphics capabilities through the IR: {word}"
            ));
        }
    }
    Ok(())
}

fn device_source(source: &str) -> Result<(), String> {
    host_api::validate(source)?;
    let parsed = tokens(source);
    for (index, token) in parsed.iter().enumerate() {
        let word = token.text.trim_matches('"');
        let previous = index.checked_sub(1).and_then(|i| parsed.get(i));
        if native_word(word) && previous.is_none_or(|value| value.text != ".") {
            return Err(format!(
                "native software identity in hardware: {word}"
            ));
        }
        if word == "software" {
            return Err(String::from("hardware may not depend on software"));
        }
    }
    Ok(())
}

fn external_members(source: &str) -> BTreeSet<String> {
    let parsed = tokens(source);
    let mut found = BTreeSet::new();
    for (index, token) in parsed.iter().enumerate() {
        if token.text != "games_platform" {
            continue;
        }
        let after = parsed.get(index.saturating_add(1)..).unwrap_or_default();
        let path = after
            .iter()
            .take(5)
            .map(|value| value.text.as_str())
            .collect::<Vec<_>>();
        let member = match path.as_slice() {
            [":", ":", "ir", ..] | [":", ":", "r", "#", "ir", ..] => continue,
            [":", ":", member, ..] => *member,
            ["as" | ";" | "," | "}", ..] => "crate-root",
            _ => continue,
        };
        let _inserted = found.insert(member.to_owned());
    }
    found
}

fn public_root(source: &str) -> Result<(), String> {
    let parsed = tokens(source);
    let mut ir = false;
    let mut depth = 0usize;
    for (index, token) in parsed.iter().enumerate() {
        match token.text.as_str() {
            "{" => depth = depth.saturating_add(1),
            "}" => depth = depth.saturating_sub(1),
            "pub" if depth == 0 => {
                let after =
                    parsed.get(index.saturating_add(1)..).unwrap_or_default();
                let declaration = after
                    .iter()
                    .take(3)
                    .map(|value| value.text.as_str())
                    .collect::<Vec<_>>();
                if declaration.as_slice() == ["mod", "ir", ";"] && !ir {
                    ir = true;
                } else {
                    return Err(String::from("platform root exports only ir"));
                }
            },
            _ => {},
        }
    }
    if ir {
        Ok(())
    } else {
        Err(String::from("missing public IR root"))
    }
}

fn capability_source(path: &str, source: &str) -> Result<(), String> {
    let Some(capability) = path
        .strip_prefix("crates/platform/src/software/")
        .and_then(|relative| relative.split('/').nth(1))
    else {
        return Ok(());
    };
    let forbidden: &[&str] = match capability {
        "input" => &[
            "SceneSession",
            "SceneTextures",
            "NativeWindow",
            "scene_renderer",
            "NativeAudio",
        ],
        "audio" => &[
            "SceneSession",
            "SceneTextures",
            "NativeWindow",
            "scene_renderer",
            "NativeInput",
        ],
        "graphics" => &["NativeAudio"],
        _ => return Ok(()),
    };
    for token in tokens(source) {
        if forbidden.contains(&token.text.as_str()) {
            return Err(format!(
                "concrete capability coupling: {}",
                token.text
            ));
        }
    }
    Ok(())
}

fn import_boundary(path: &str, source: &str) -> Result<(), String> {
    if path.starts_with("crates/importer/")
        || path.starts_with("crates/platform/src/ir/importer/")
    {
        return Ok(());
    }
    for token in tokens(source) {
        if matches!(
            token.text.as_str(),
            "games_importer"
                | "AssetSelection"
                | "ConversionLimits"
                | "CartridgeGraphics"
                | "convert_audio"
                | "convert_graphics"
                | "convert_flat_zip"
                | "convert_snapshot"
        ) {
            return Err(format!(
                "production conversion belongs in importer IR: {}",
                token.text,
            ));
        }
    }
    Ok(())
}

fn consumer_source(source: &str) -> Result<(), String> {
    let found = external_members(source);
    if let Some(name) = found.first() {
        return Err(format!("consumer must import platform IR only: {name}"));
    }
    Ok(())
}

/// Guard new canonical hardware modules and explicit backend imports.
///
/// Existing legacy public platform modules are transitional and remain
/// covered by the migration task and existing portability inventory.
///
/// # Errors
/// Reports forbidden host operations or concrete backend imports.
pub(crate) fn validate(
    root: &OsStr,
    authored: &[OsString],
) -> Result<(), String> {
    for path in authored {
        if files::authored_extension(path).is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path.to_string_lossy().replace('\\', "/");
        if name.starts_with("tools/validator/") {
            continue;
        }
        let source = files::read_text(root, path)?;
        let result = if name.starts_with("crates/platform/src/hardware/") {
            device_source(&source)
        } else if name == "crates/platform/src/lib.rs" {
            public_root(&source)
        } else if name.starts_with("crates/platform/") {
            capability_source(&name, &source)
        } else if name.starts_with("crates/games/")
            || name.starts_with("crates/apps/")
            || name.starts_with("crates/engine/")
        {
            consumer_source(&source).and_then(|()| vendor_free(&source))
        } else {
            consumer_source(&source)
        };
        result
            .and_then(|()| import_boundary(&name, &source))
            .map_err(|error| format!("{name}: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        capability_source, consumer_source, device_source, external_members,
        import_boundary, public_root, vendor_free,
    };

    #[test]
    fn production_conversion_requires_import_ir() {
        for source in [
            "use games_importer::conversion::ConversionLimits as Limits;",
            "use games_importer::mame::Machine;",
            "let result = machine.convert_graphics(source, limits, stage);",
            "use games_importer::{neo::CartridgeGraphics as Profile};",
        ] {
            assert!(
                import_boundary("tools/build/src/main.rs", source).is_err()
            );
            assert!(
                import_boundary(
                    "crates/platform/src/ir/importer/import.rs",
                    source,
                )
                .is_ok()
            );
        }
        assert!(
            import_boundary(
                "tools/build/src/main.rs",
                "use games_platform::ir::importer::{ImportProfile, import};",
            )
            .is_ok()
        );
        assert!(
            import_boundary(
                "tools/build/src/reference_audio.rs",
                "use games_platform::ir::importer::reference::Machine;",
            )
            .is_ok()
        );
    }

    #[test]
    fn titles_request_rendering_not_vendor_specific_implementations() {
        for source in [
            "use nvapi::Device;",
            "use cuda::Context;",
            "let driver = \"NVIDIA\";",
            "if adapter == Vendor::Intel {}",
            "use vulkan::Instance;",
            "use dxgi::Factory;",
        ] {
            assert!(vendor_free(source).is_err(), "{source}");
        }
        for source in [
            "use games_platform::ir::graphics::Renderer;",
            "let preference = AdapterPreference::HighPerformance;",
            "// NVIDIA driver detection belongs in platform.",
            "let title = \"Metal Slug\";",
        ] {
            assert!(vendor_free(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn physical_protocols_are_host_free() {
        assert!(
            device_source(
                "use crate::ir::hardware::Report; fn decode(b: &[u8]) { \
             let _ = b.windows(2); }"
            )
            .is_ok()
        );
        for source in [
            "use std::fs::File;",
            "use std::{net::TcpStream as Stream};",
            "use crate::software::open;",
            "use super::super::{software as host};",
            "#[cfg(target_arch = \"x86_64\")] fn f() {}",
            "use xinput::Controller;",
            "use sdl2::controller::GameController;",
            "let platform = \"linux\";",
        ] {
            assert!(device_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn only_neutral_consumer_paths_are_admitted() {
        assert!(
            consumer_source("use games_platform::ir::hardware::InputReport;")
                .is_ok()
        );
        for source in [
            "use games_platform::software::linux::File;",
            "use games_platform::hardware::playstation::Four;",
            "use games_platform::{ir, software};",
            "use games_platform::*;",
            "use games_platform as platform;",
            "extern crate games_platform as platform;",
            "use games_platform;",
            "use {games_platform as platform};",
            "use games_platform::r#software::linux;",
        ] {
            assert!(consumer_source(source).is_err(), "{source}");
        }
        assert!(
            external_members("use games_platform::lan::Connection;")
                .contains("lan")
        );
    }

    #[test]
    fn public_surface_is_exactly_the_ir() {
        assert!(public_root("mod hardware; pub mod ir; mod software;").is_ok());
        for source in [
            "mod ir; mod software;",
            "pub mod ir; pub mod software;",
            "pub mod ir; pub use software::linux;",
            "pub mod ir; pub fn open() {}",
            "pub mod ir; pub(crate) mod software;",
        ] {
            assert!(public_root(source).is_err(), "{source}");
        }
    }

    #[test]
    fn separated_capabilities_do_not_depend_on_concrete_siblings() {
        for system in ["linux", "windows", "macos", "android"] {
            let root = format!("crates/platform/src/software/{system}");
            let input = format!("{root}/input/mod.rs");
            assert!(
                capability_source(
                    &input,
                    "use crate::ir::hardware::HostEvent;"
                )
                .is_ok()
            );
            assert!(
                capability_source(
                    &input,
                    "use super::graphics::scene_renderer::SceneSession as S;"
                )
                .is_err()
            );
            let audio = format!("{root}/audio/mod.rs");
            assert!(
                capability_source(&audio, "use super::input::NativeInput;")
                    .is_err()
            );
            let graphics = format!("{root}/graphics/mod.rs");
            assert!(
                capability_source(&graphics, "use super::audio::NativeAudio;")
                    .is_err()
            );
        }
    }

    #[test]
    fn comments_do_not_create_backend_dependencies() {
        assert!(device_source("// use std::fs;\nfn decode() {}").is_ok());
        assert!(
            consumer_source(
                "/* games_platform::software::linux */ fn start() {}"
            )
            .is_ok()
        );
    }
}
