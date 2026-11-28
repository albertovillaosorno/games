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
//   - Private Android ARM64 authored application packaging on Linux.
// - Must-Not:
//   - Package original assets, use signing credentials or modify upstream.
// - Allows:
//   - Inputs: Locked Cargo selection, local SDK and trusted staging parent.
//   - Outputs: Bounded, aligned unsigned development APK bytes.
//   - Side effects: Fixed build tools in a private temporary directory.
// - Split-When:
//   - Another application package requires independent build semantics.
// - Merge-When:
//   - A shared package backend provides equivalent Android integration.
// - Summary:
//   - Build the pinned SDL2 and independently authored Rust application.
// - Description:
//   - Adapt only the reviewed SDL Java entry; package explicit members.
// - Usage:
//   - Request Android packaging through the portable toolchain IR.
// - Defaults:
//   - Local development only, no signing or media acquisition.

mod archive;
mod sdk;
mod source;

use std::ffi::OsStr;
use std::io::Error;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, str};

use serde_json::Value;

use self::sdk::Sdk;
use crate::ir::security::toolchain::CargoArguments;
use crate::ir::storage::file::snapshot;
use crate::ir::storage::staging::AuthoredStaging;
use crate::ir::toolchain::{Target, ToolchainError, ToolchainFactory};

const MAX_ARTIFACT: NonZeroU64 = NonZeroU64::MIN.saturating_add(33_554_431);
const MAX_SOURCE: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const SDL_JAVA: &str = "android-project/app/src/main/java/org/libsdl/app";
const JAVA_FILES: [&str; 9] = [
    "HIDDevice.java",
    "HIDDeviceBLESteamController.java",
    "HIDDeviceManager.java",
    "HIDDeviceUSB.java",
    "SDL.java",
    "SDLActivity.java",
    "SDLAudioManager.java",
    "SDLControllerManager.java",
    "SDLSurface.java",
];
const APPLICATION_JAVA: &str = "java/org/games/runtime/GamesActivity.java";
const SDL_LIBRARY: &str = "lib/arm64-v8a/libSDL2.so";
const APPLICATION_LIBRARY: &str = "lib/arm64-v8a/libgames_application.so";

const SDL_BUILD: &str = "sdl-build";

type Outcome<T> = Result<T, ToolchainError>;

const fn failure(diagnostic: String) -> ToolchainError {
    ToolchainError::Failed {
        operation: "Android packaging",
        diagnostic,
    }
}

fn io_failure(operation: &'static str) -> impl FnOnce(Error) -> ToolchainError {
    move |source| ToolchainError::Io { operation, source }
}

pub(super) fn available() -> bool {
    Sdk::discover().is_ok()
}

fn run(command: &mut Command, operation: &'static str) -> Outcome<()> {
    let status = command.status().map_err(io_failure(operation))?;
    if !status.success() {
        return Err(ToolchainError::Failed {
            operation,
            diagnostic: status.to_string(),
        });
    }
    Ok(())
}

fn sdl_source(
    args: CargoArguments<'_>,
    root: &OsStr,
    manifest: &OsStr,
) -> Outcome<PathBuf> {
    let raw_metadata =
        <() as ToolchainFactory>::metadata(args, root, manifest)?;
    let metadata: Value = serde_json::from_slice(&raw_metadata)
        .map_err(|error| failure(format!("locked Cargo metadata: {error}")))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("missing Cargo packages".into()))?;
    let mut matches = packages.iter().filter(|package| {
        package.get("name").and_then(Value::as_str) == Some("sdl2-sys")
            && package.get("version").and_then(Value::as_str) == Some("0.39.0")
            && package.get("source").and_then(Value::as_str)
                == Some("registry+https://github.com/rust-lang/crates.io-index")
    });
    let selected = matches
        .next()
        .and_then(|package| package.get("manifest_path"))
        .and_then(Value::as_str)
        .ok_or_else(|| failure("missing reviewed SDL2 source".into()))?;
    if matches.next().is_some() {
        return Err(failure("ambiguous SDL2 source".into()));
    }
    let dependency_manifest = Path::new(selected);
    if !dependency_manifest.is_absolute() {
        return Err(failure("SDL2 manifest must be absolute".into()));
    }
    let source = dependency_manifest
        .parent()
        .ok_or_else(|| failure("invalid SDL2 manifest".into()))?
        .join("SDL");
    if !source.join("CMakeLists.txt").is_file() {
        return Err(failure("missing SDL2 native source".into()));
    }
    Ok(source)
}

fn stage_source(
    stage: &mut AuthoredStaging,
    sdl: &Path,
    title: &str,
) -> Outcome<()> {
    let manifest = source::manifest(title).ok_or_else(|| {
        failure("invalid Android application identifier".into())
    })?;
    stage
        .write_file("AndroidManifest.xml", manifest.as_bytes(), false)
        .map_err(|error| failure(format!("stage manifest: {error:?}")))?;
    for file in JAVA_FILES {
        let path = sdl.join(SDL_JAVA).join(file);
        let upstream = snapshot(path.as_os_str(), MAX_SOURCE)
            .map_err(|error| failure(format!("SDL Java source: {error:?}")))?;
        let bytes = if file == "SDLActivity.java" {
            let text = str::from_utf8(&upstream)
                .map_err(|error| failure(format!("SDL Activity: {error}")))?;
            source::adapt_activity(text)
                .ok_or_else(|| failure("unreviewed SDL Activity".into()))?
                .into_bytes()
        } else {
            upstream
        };
        stage
            .write_file(&format!("java/org/libsdl/app/{file}"), &bytes, false)
            .map_err(|error| {
                failure(format!("stage Java source: {error:?}"))
            })?;
    }
    stage
        .write_file(
            APPLICATION_JAVA,
            include_bytes!("../../../android/process/GamesActivity.java"),
            false,
        )
        .map_err(|error| failure(format!("stage launcher: {error:?}")))
}

fn native_sdl(sdk: &Sdk, source: &Path, directory: &Path) -> Outcome<PathBuf> {
    let build = directory.join(SDL_BUILD);
    run(
        Command::new("cmake")
            .current_dir(directory)
            .arg("-S")
            .arg(source)
            .arg("-B")
            .arg(&build)
            .arg(format!(
                "-DCMAKE_TOOLCHAIN_FILE={}",
                sdk.cmake_toolchain().display()
            ))
            .args([
                "-DANDROID_ABI=arm64-v8a",
                "-DANDROID_PLATFORM=android-24",
                "-DCMAKE_BUILD_TYPE=Release",
                "-DSDL_SHARED=ON",
                "-DSDL_STATIC=OFF",
                "-DSDL_TEST=OFF",
                "-DSDL_TESTS=OFF",
            ]),
        "configure Android SDL2",
    )?;
    run(
        Command::new("cmake")
            .current_dir(directory)
            .arg("--build")
            .arg(&build)
            .args(["--parallel", "4"]),
        "compile Android SDL2",
    )?;
    Ok(build)
}

fn title_library(
    args: CargoArguments<'_>,
    root: &OsStr,
    sdk: &Sdk,
    directory: &Path,
) -> Outcome<PathBuf> {
    let target_dir = directory.join("cargo");
    let native_dir = directory.join(SDL_BUILD);
    let sdl_path = native_dir
        .to_str()
        .ok_or_else(|| failure("SDL build path is not UTF-8".into()))?;
    run(
        Command::new("cargo")
            .current_dir(Path::new(root))
            .args(args.android_build())
            .arg("--target-dir")
            .arg(&target_dir)
            .env("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", sdk.clang())
            .env("CC_aarch64_linux_android", sdk.clang())
            .env("AR_aarch64_linux_android", sdk.archiver())
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                format!("-L\u{1f}native={sdl_path}"),
            ),
        "compile Android application library",
    )?;
    Ok(target_dir
        .join(Target::AndroidArm64.triple())
        .join("release")
        .join(format!("lib{}.so", args.package().replace('-', "_"))))
}

fn classes(sdk: &Sdk, directory: &Path) -> Outcome<()> {
    for name in ["classes", "dex"] {
        fs::create_dir_all(directory.join(name))
            .map_err(io_failure("create Android compiler output"))?;
    }
    let sources = JAVA_FILES.map(|file| format!("java/org/libsdl/app/{file}"));
    run(
        Command::new("javac")
            .current_dir(directory)
            .args(["--release", "8", "-classpath"])
            .arg(&sdk.android_jar)
            .args(["-d", "classes", APPLICATION_JAVA])
            .args(sources),
        "compile Android Java entry",
    )?;
    run(
        Command::new("jar").current_dir(directory).args([
            "--create",
            "--file",
            "classes.jar",
            "-C",
            "classes",
            ".",
        ]),
        "archive Android Java classes",
    )?;
    run(
        Command::new(sdk.build_tools.join("d8"))
            .current_dir(directory)
            .args(["--min-api", "24", "--lib"])
            .arg(&sdk.android_jar)
            .args(["--output", "dex", "classes.jar"]),
        "compile Android dex",
    )
}

fn stage_member(
    stage: &mut AuthoredStaging,
    name: &str,
    path: &Path,
) -> Outcome<()> {
    let bytes = snapshot(path.as_os_str(), MAX_ARTIFACT).map_err(|error| {
        failure(format!("Android member {name}: {error:?}"))
    })?;
    stage
        .write_file(&format!("apk/{name}"), &bytes, false)
        .map_err(|error| failure(format!("stage Android member: {error:?}")))
}

fn assemble(sdk: &Sdk, directory: &Path) -> Outcome<Vec<u8>> {
    run(
        Command::new(sdk.build_tools.join("aapt2"))
            .current_dir(directory)
            .args([
                "link",
                "-o",
                "base.apk",
                "--manifest",
                "AndroidManifest.xml",
                "-I",
            ])
            .arg(&sdk.android_jar),
        "link Android manifest",
    )?;
    let bytes = archive::assemble(directory)?;
    fs::write(directory.join("game.apk"), &bytes)
        .map_err(io_failure("write authored APK"))?;
    run(
        Command::new(sdk.build_tools.join("zipalign"))
            .current_dir(directory)
            .args(["-c", "-P", "16", "4", "game.apk"]),
        "verify Android application alignment",
    )?;
    Ok(bytes)
}

pub(super) fn package(
    args: CargoArguments<'_>,
    root: &OsStr,
    parent: &OsStr,
    manifest: &OsStr,
) -> Outcome<Vec<u8>> {
    let sdk = Sdk::discover()?;
    let upstream_sdl = sdl_source(args, root, manifest)?;
    let mut stage = AuthoredStaging::new(parent).map_err(|error| {
        failure(format!("private Android stage: {error:?}"))
    })?;
    stage_source(&mut stage, &upstream_sdl, args.binary())?;
    let directory = PathBuf::from(stage.path());
    let sdl = native_sdl(&sdk, &upstream_sdl, &directory)?;
    let title = title_library(args, root, &sdk, &directory)?;
    classes(&sdk, &directory)?;
    stage_member(
        &mut stage,
        "classes.dex",
        &directory.join("dex/classes.dex"),
    )?;
    stage_member(&mut stage, SDL_LIBRARY, &sdl.join("libSDL2.so"))?;
    stage_member(&mut stage, APPLICATION_LIBRARY, &title)?;
    assemble(&sdk, &directory)
}
