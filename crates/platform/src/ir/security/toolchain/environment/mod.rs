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
//   - Admission of validation-relevant host environment observations.
// - Must-Not:
//   - Read the process environment or disclose raw values in diagnostics.
// - Allows:
//   - Inputs: Host-provided environment names and opaque values.
//   - Outputs: A filtered contributor validation observation.
//   - Side effects: None.
// - Split-When:
//   - Another contributor operation needs distinct environment admission.
// - Merge-When:
//   - A shared security contract owns equivalent observation filtering.
// - Summary:
//   - Retain only validation-relevant environment observations.
// - Description:
//   - Preserve absent, empty and non-Unicode values for fingerprints.
// - Usage:
//   - Filter host observations before exposing them to the validator.
// - Defaults:
//   - Exclude unrelated variables and redact all values in debug output.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;

/// Fixed environment inputs that can alter validation outcomes.
pub const FINGERPRINT_KEYS: &[&str] = &[
    "PATH",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "RUSTUP_TOOLCHAIN",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTC",
    "RUSTDOC",
    "RUSTC_BOOTSTRAP",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CLIPPY_CONF_DIR",
    "NODE_OPTIONS",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
];

/// Native smoke checks require fresh evidence even if sources are unchanged.
pub const FRESH_CHECK_FLAGS: [&str; 2] =
    [NativeSmoke::Audio.key(), NativeSmoke::Render.key()];

/// Explicit opt-in for a native integration smoke check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSmoke {
    /// Exercise native audio playback.
    Audio,
    /// Exercise native graphics and input presentation.
    Render,
}

impl NativeSmoke {
    /// Shared environment name used for opt-in and cache invalidation.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Audio => "GAMES_NATIVE_AUDIO_SMOKE",
            Self::Render => "GAMES_NATIVE_RENDER_SMOKE",
        }
    }
}

/// An admitted environment observation; debug output never reveals values.
pub struct Environment {
    build_overrides: BTreeMap<OsString, OsString>,
    fixed: BTreeMap<OsString, OsString>,
    home: Option<OsString>,
}

impl Environment {
    /// Build overrides in stable native-string order.
    #[must_use]
    pub const fn build_overrides(&self) -> &BTreeMap<OsString, OsString> {
        &self.build_overrides
    }

    /// Observe whether an explicitly requested native smoke check is present.
    #[must_use]
    pub fn fresh_requested(&self) -> bool {
        FRESH_CHECK_FLAGS
            .iter()
            .any(|flag| self.value(flag).is_some())
    }

    /// Observe the host home directory, when available.
    #[must_use]
    pub fn home(&self) -> Option<&OsStr> {
        self.home.as_deref()
    }

    /// Admit only fixed keys and known compiler/build overrides.
    #[must_use]
    pub fn new<Values>(values: Values, home: Option<OsString>) -> Self
    where
        Values: IntoIterator<Item = (OsString, OsString)>,
    {
        let mut fixed = BTreeMap::new();
        let mut build_overrides = BTreeMap::new();
        for (key, value) in values {
            let name = key.as_encoded_bytes();
            if key.to_str().is_some_and(|admitted| {
                FINGERPRINT_KEYS.contains(&admitted)
                    || FRESH_CHECK_FLAGS.contains(&admitted)
            }) {
                let _previous = fixed.insert(key, value);
            } else if build_override(name) {
                let _previous = build_overrides.insert(key, value);
            }
        }
        Self {
            build_overrides,
            fixed,
            home,
        }
    }

    /// Observe a fixed admitted variable, preserving absence and empty values.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&OsStr> {
        self.fixed.get(OsStr::new(name)).map(OsString::as_os_str)
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Environment")
            .field("build_override_count", &self.build_overrides.len())
            .field("fixed_count", &self.fixed.len())
            .field("home_available", &self.home.is_some())
            .finish_non_exhaustive()
    }
}

fn build_override(name: &[u8]) -> bool {
    [
        b"CARGO_BUILD_".as_slice(),
        b"CARGO_TARGET_",
        b"CARGO_PROFILE_",
    ]
    .into_iter()
    .any(|prefix| name.starts_with(prefix))
        || native_override(name)
}

fn native_override(raw: &[u8]) -> bool {
    let name = raw
        .strip_prefix(b"HOST_")
        .or_else(|| raw.strip_prefix(b"TARGET_"))
        .unwrap_or(raw);
    [
        b"CC".as_slice(),
        b"CFLAGS",
        b"CXX",
        b"CXXFLAGS",
        b"AR",
        b"ARFLAGS",
        b"RANLIB",
        b"RANLIBFLAGS",
        b"PKG_CONFIG",
        b"SDL2",
        b"CROSS_COMPILE",
        b"CRATE_CC_NO_DEFAULTS",
        b"RING_PREGENERATE_ASM",
    ]
    .into_iter()
    .any(|base| {
        name == base
            || name
                .strip_prefix(base)
                .is_some_and(|tail| tail.starts_with(b"_"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_preserves_only_selected_values_and_redacts_debug() {
        let values = [
            ("PATH", "path-secret"),
            ("CARGO_HOME", ""),
            ("GAMES_NATIVE_AUDIO_SMOKE", ""),
            ("CC", "compiler-secret"),
            ("UNRELATED", "unrelated-secret"),
        ];
        let environment = Environment::new(
            values.map(|(name, value)| (name.into(), value.into())),
            Some("home-secret".into()),
        );
        assert_eq!(environment.value("PATH"), Some(OsStr::new("path-secret")));
        assert_eq!(environment.value("CARGO_HOME"), Some(OsStr::new("")));
        assert_eq!(environment.value("UNRELATED"), None);
        assert_eq!(environment.build_overrides().len(), 1);
        assert!(environment.fresh_requested());
        assert!(!format!("{environment:?}").contains("secret"));
    }
}
