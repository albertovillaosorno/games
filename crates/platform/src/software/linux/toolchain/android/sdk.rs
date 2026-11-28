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
//   - Discovery of the reviewed local Android contributor toolchain.
// - Must-Not:
//   - Download or install SDKs or read credentials.
// - Allows:
//   - Inputs: Explicit SDK environment and reviewed local version paths.
//   - Outputs: Absolute SDK tool and platform paths or rejection.
//   - Side effects: Read-only path resolution.
// - Split-When:
//   - Another SDK release requires independent tooling semantics.
// - Merge-When:
//   - A shared host SDK discovery owner provides the same admission.
// - Summary:
//   - Require the pinned SDK, NDK and build-tools before packaging.
// - Description:
//   - Never silently search arbitrary SDK locations or install tools.
// - Usage:
//   - Set ANDROID_HOME to the reviewed installed SDK root.
// - Defaults:
//   - Missing or conflicting roots are unavailable.

use std::env;
use std::path::{Path, PathBuf};

use super::{Outcome, failure, io_failure};

const NDK: &str = "ndk/28.2.13676358";
const NDK_TOOLS: &str = "toolchains/llvm/prebuilt/linux-x86_64/bin";

#[derive(Debug)]
pub(super) struct Sdk {
    pub(super) android_jar: PathBuf,
    pub(super) build_tools: PathBuf,
    pub(super) ndk: PathBuf,
}

impl Sdk {
    pub(super) fn archiver(&self) -> PathBuf {
        self.ndk.join(NDK_TOOLS).join("llvm-ar")
    }

    pub(super) fn clang(&self) -> PathBuf {
        self.ndk
            .join(NDK_TOOLS)
            .join("aarch64-linux-android24-clang")
    }

    pub(super) fn cmake_toolchain(&self) -> PathBuf {
        self.ndk.join("build/cmake/android.toolchain.cmake")
    }

    pub(super) fn discover() -> Outcome<Self> {
        let primary = env::var_os("ANDROID_HOME");
        let alternate = env::var_os("ANDROID_SDK_ROOT");
        let selected = primary
            .as_deref()
            .or(alternate.as_deref())
            .ok_or_else(|| failure("set ANDROID_HOME".into()))?;
        let candidate = Path::new(selected);
        if !candidate.is_absolute() {
            return Err(failure("Android SDK root must be absolute".into()));
        }
        let root = candidate
            .canonicalize()
            .map_err(io_failure("resolve Android SDK"))?;
        if let Some(other) = alternate {
            let other_path = Path::new(&other);
            if !other_path.is_absolute()
                || other_path.canonicalize().ok().as_ref() != Some(&root)
            {
                return Err(failure("conflicting Android SDK roots".into()));
            }
        }
        let sdk = Self {
            android_jar: root.join("platforms/android-35/android.jar"),
            build_tools: root.join("build-tools/35.0.0"),
            ndk: root.join(NDK),
        };
        sdk.require_files()?;
        Ok(sdk)
    }

    fn require_files(&self) -> Outcome<()> {
        for path in [
            self.android_jar.clone(),
            self.build_tools.join("aapt2"),
            self.build_tools.join("d8"),
            self.build_tools.join("zipalign"),
            self.clang(),
            self.archiver(),
            self.cmake_toolchain(),
        ] {
            if !path.is_file() {
                return Err(failure(format!(
                    "missing reviewed Android tool or input: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}
