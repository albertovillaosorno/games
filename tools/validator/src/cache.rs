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
//   - Validation fingerprints, locking and run logs.
// - Must-Not:
//   - Implement game behaviour or modify source.
// - Allows:
//   - Inputs: Repository files and local tool configuration.
//   - Outputs: Validation data or diagnostic.
//   - Side effects: Local cache and log writes.
// - Split-When:
//   - An independent validation responsibility emerges.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Validation fingerprints, locking and run logs.
// - Description:
//   - Reuses only successful unchanged validation results.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Fail closed on invalid inputs.
//

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Write as _, stdout};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{env, process};

use games_platform::ir::toolchain::validation::{
    GitRequest, Request, run as observe,
};
use games_platform::ir::toolchain::{ValidationTool, probe_version};
use sha2::{Digest as _, Sha256};

use crate::files::{authored, ledger_location};
use crate::{cargo_config, portability};

const FRESH_CHECK_FLAGS: [&str; 2] =
    ["GAMES_NATIVE_AUDIO_SMOKE", "GAMES_NATIVE_RENDER_SMOKE"];

/// Function signature for a complete validation pass.
pub type Check = fn(&Path, &mut File) -> Result<(), String>;
type Fingerprint = fn(&Path) -> Result<Vec<u8>, String>;

fn hash_value(hash: &mut Sha256, value: &[u8]) {
    hash.update(value.len().to_le_bytes());
    hash.update(value);
}

fn hash_environment(hash: &mut Sha256, key: &str, value: Option<&OsStr>) {
    hash_value(hash, key.as_bytes());
    hash_value(hash, &[u8::from(value.is_some())]);
    if let Some(present) = value {
        hash_value(hash, present.as_encoded_bytes());
    }
}

// cc and pkg-config accept both HOST_/TARGET_ prefixes and target suffixes.
// Include the native inputs used by Ring and SDL without collecting unrelated
// shell variables or printing their potentially private values.
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

fn hash_build_overrides(
    hash: &mut Sha256,
    values: impl IntoIterator<Item = (OsString, OsString)>,
) {
    let overrides: BTreeMap<_, _> = values
        .into_iter()
        .filter(|(key, _value)| {
            let name = key.as_encoded_bytes();
            [
                b"CARGO_BUILD_".as_slice(),
                b"CARGO_TARGET_",
                b"CARGO_PROFILE_",
            ]
            .into_iter()
            .any(|prefix| name.starts_with(prefix))
                || native_override(name)
        })
        .collect();
    hash_value(hash, b"build-overrides");
    hash_value(hash, &overrides.len().to_le_bytes());
    for (key, value) in overrides {
        hash_value(hash, key.as_encoded_bytes());
        hash_value(hash, value.as_encoded_bytes());
    }
}

fn fresh_check_requested(enabled: impl FnMut(&str) -> bool) -> bool {
    FRESH_CHECK_FLAGS.into_iter().any(enabled)
}

fn cargo_home(root: &Path) -> Result<PathBuf, String> {
    if let Some(home) =
        env::var_os("CARGO_HOME").filter(|path| !path.is_empty())
    {
        return Ok(root.join(home));
    }
    env::home_dir()
        .map(|home| home.join(".cargo"))
        .ok_or_else(|| String::from("cannot resolve Cargo home"))
}

fn hash_cargo_config_files(
    root: &Path,
    home: &Path,
    hash: &mut Sha256,
) -> Result<(), String> {
    hash_value(hash, b"cargo-config-files");
    cargo_config::visit_active(root, home, &mut |path, document| {
        hash_value(hash, path.as_os_str().as_encoded_bytes());
        hash_value(hash, &[u8::from(document.is_some())]);
        if let Some((text, _value)) = document {
            hash_value(hash, text.as_bytes());
        }
        Ok(())
    })
}

fn tool_versions(root: &Path, hash: &mut Sha256) -> Result<(), String> {
    for tool in [
        ValidationTool::Rustc,
        ValidationTool::Cargo,
        ValidationTool::Clippy,
        ValidationTool::Rustfmt,
        ValidationTool::RustupTargets,
        ValidationTool::Markdown,
        ValidationTool::Ripgrep,
    ] {
        let output = probe_version(root.as_os_str(), tool)
            .map_err(|error| error.to_string())?;
        hash_value(hash, &output.stdout);
        hash_value(hash, &output.stderr);
    }
    for key in [
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
    ]
    .into_iter()
    .chain(FRESH_CHECK_FLAGS)
    {
        hash_environment(hash, key, env::var_os(key).as_deref());
    }
    hash_build_overrides(hash, env::vars_os());
    Ok(())
}

fn fingerprint(root: &Path) -> Result<Vec<u8>, String> {
    ledger_location(root)?;
    portability::preflight(root)?;
    let mut hash = Sha256::new();
    tool_versions(root, &mut hash)?;
    let canonical = root.canonicalize().map_err(|error| error.to_string())?;
    hash_cargo_config_files(&canonical, &cargo_home(&canonical)?, &mut hash)?;
    if root.join(".git").exists() {
        let head = observe(root.as_os_str(), Request::Git(GitRequest::Head))
            .map_err(|error| error.to_string())?;
        hash_value(&mut hash, b"git-history");
        hash_value(&mut hash, &head.stdout);
    }
    for path in authored(root)? {
        hash_value(&mut hash, path.as_os_str().as_encoded_bytes());
        let bytes =
            fs::read(root.join(path)).map_err(|error| error.to_string())?;
        hash_value(&mut hash, &bytes);
    }
    Ok(hash.finalize().to_vec())
}

fn lock(root: &Path) -> Result<File, String> {
    let directory = root.join(".cache/validate");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("lock"))
        .map_err(|error| error.to_string())?;
    file.lock().map_err(|error| error.to_string())?;
    Ok(file)
}

fn log_path(root: &Path) -> Result<PathBuf, String> {
    let directory = root.join(".temp/validate");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(directory.join(format!("{stamp}-{}.log", process::id())))
}

fn announce(message: &str) -> Result<(), String> {
    writeln!(stdout().lock(), "{message}").map_err(|error| error.to_string())
}

/// Serialize validation and reuse only successful unchanged results.
///
/// Explicit native smoke requests run fresh.
/// The fingerprint also includes the installed Rust target inventory.
///
/// # Errors
///
/// Returns an error for failed checks, I/O or concurrent source changes.
pub fn run(root: &Path, fresh: bool, check: Check) -> Result<(), String> {
    let requested = fresh_check_requested(|key| env::var_os(key).is_some());
    run_with(root, fresh || requested, check, fingerprint)
}

fn run_with(
    root: &Path,
    fresh: bool,
    check: Check,
    identify: Fingerprint,
) -> Result<(), String> {
    let _guard = lock(root)?;
    let path = log_path(root)?;
    let mut log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    announce(&format!("Validation log: {}", path.display()))?;
    let marker = root.join(".cache/validate/success");
    let before = match identify(root) {
        Ok(value) => value,
        Err(error) => {
            if marker.exists() {
                fs::remove_file(&marker).map_err(|err| err.to_string())?;
            }
            writeln!(log, "FAILED: {error}").map_err(|err| err.to_string())?;
            return Err(error);
        },
    };
    if !fresh && fs::read(&marker).is_ok_and(|key| key == before) {
        writeln!(log, "Cache hit").map_err(|error| error.to_string())?;
        return announce("Validation passed (cache hit).");
    }
    if marker.exists() {
        fs::remove_file(&marker).map_err(|error| error.to_string())?;
    }
    if let Err(error) = check(root, &mut log) {
        writeln!(log, "FAILED: {error}").map_err(|err| err.to_string())?;
        return Err(error);
    }
    if identify(root)? != before {
        return Err(String::from("source changed during validation; rerun"));
    }
    writeln!(log, "PASS").map_err(|error| error.to_string())?;
    let pending = root.join(".cache/validate/pending");
    fs::write(&pending, before).map_err(|error| error.to_string())?;
    fs::rename(pending, marker).map_err(|error| error.to_string())?;
    announce("Validation passed (fresh).")
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File};
    use std::io::Write as _;
    use std::path::Path;
    use std::thread;

    use sha2::{Digest as _, Sha256};

    use super::{
        FRESH_CHECK_FLAGS, fingerprint, fresh_check_requested,
        hash_build_overrides, hash_cargo_config_files, hash_environment,
        run_with,
    };
    use crate::fixtures::{directory, require};

    fn key(root: &Path) -> Result<Vec<u8>, String> {
        fs::read(root.join("input")).map_err(|error| error.to_string())
    }

    fn pass(_root: &Path, log: &mut File) -> Result<(), String> {
        writeln!(log, "executed").map_err(|error| error.to_string())
    }

    fn fail(root: &Path, _log: &mut File) -> Result<(), String> {
        fs::read(root.join("missing"))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn mutate(root: &Path, _log: &mut File) -> Result<(), String> {
        fs::write(root.join("input"), b"changed")
            .map_err(|error| error.to_string())
    }

    #[test]
    fn caches_only_unchanged_success() -> Result<(), String> {
        let root = directory("cache")?;
        let input = root.join("input");
        fs::write(&input, b"original").map_err(|error| error.to_string())?;
        let marker = root.join(".cache/validate/success");

        run_with(&root, false, pass, key)?;
        require(marker.exists(), "successful result must be cached")?;
        run_with(&root, false, fail, key)?;
        require(
            run_with(&root, true, fail, key).is_err(),
            "fresh validation must bypass the cache",
        )?;
        require(!marker.exists(), "failure must invalidate the cache")?;

        run_with(&root, false, pass, key)?;
        fs::write(&input, b"updated").map_err(|error| error.to_string())?;
        require(
            run_with(&root, false, fail, key).is_err(),
            "changed inputs must trigger checks",
        )?;
        require(
            !marker.exists(),
            "changed and failed input must not be cached",
        )?;
        require(
            run_with(&root, true, mutate, key).is_err(),
            "changing inputs during checks must invalidate the pass",
        )?;
        require(
            !marker.exists(),
            "concurrent modification must not be cached",
        )?;

        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn ignored_disposable_ledger_invalidates_cached_success()
    -> Result<(), String> {
        let root = directory("cache-ledger")?;
        fs::write(root.join("input"), b"previously admitted")
            .map_err(|error| error.to_string())?;
        run_with(&root, false, pass, key)?;
        let marker = root.join(".cache/validate/success");
        require(marker.exists(), "fixture success marker")?;
        fs::create_dir_all(root.join(".temp/ledger"))
            .map_err(|error| error.to_string())?;
        let result = run_with(&root, false, pass, fingerprint);
        require(
            result.is_err_and(|error| error.contains(".reference/ledger")),
            "durable ledger must fail before cache lookup",
        )?;
        require(!marker.exists(), "stale marker must be removed")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn forbidden_source_invalidates_cached_success() -> Result<(), String> {
        let root = directory("cache-portability")?;
        let source = root.join("crates/games/example/src");
        fs::create_dir_all(&source).map_err(|error| error.to_string())?;
        fs::write(root.join(".gitignore"), "/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        fs::write(root.join("Cargo.toml"), "[workspace.dependencies]\n")
            .map_err(|error| error.to_string())?;
        fs::write(root.join("input"), b"previously admitted")
            .map_err(|error| error.to_string())?;
        run_with(&root, false, pass, key)?;
        let marker = root.join(".cache/validate/success");
        require(marker.exists(), "fixture must have a success marker")?;
        fs::write(source.join("main.rs"), "#[cfg(unix)] fn run() {}")
            .map_err(|error| error.to_string())?;
        let result = run_with(&root, false, pass, fingerprint);
        let diagnostic = result.err().unwrap_or_default();
        require(
            diagnostic.contains("os:unix"),
            "portability must fail first",
        )?;
        require(!marker.exists(), "stale marker must be invalidated")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn fresh_requests_cannot_reuse_ordinary_cache() -> Result<(), String> {
        let root = directory("fresh-cache")?;
        fs::write(root.join("input"), b"unchanged")
            .map_err(|error| error.to_string())?;
        require(
            !fresh_check_requested(|_flag| false),
            "ordinary validation may reuse cache",
        )?;
        for flag in FRESH_CHECK_FLAGS {
            run_with(&root, false, pass, key)?;
            run_with(&root, false, fail, key)?;
            let requested =
                fresh_check_requested(|candidate| candidate == flag);
            require(
                run_with(&root, requested, fail, key).is_err(),
                "explicit check must execute instead of reusing success",
            )?;
        }
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn environment_presence_and_value_change_the_fingerprint() {
        let fingerprint = |value: Option<&OsStr>| {
            let mut hash = Sha256::new();
            hash_environment(&mut hash, "GAMES_NATIVE_RENDER_SMOKE", value);
            hash.finalize()
        };
        let absent = fingerprint(None);
        let empty = fingerprint(Some(OsStr::new("")));
        let enabled = fingerprint(Some(OsStr::new("1")));
        assert_ne!(absent, empty);
        assert_ne!(empty, enabled);
        assert_ne!(absent, enabled);
    }

    fn override_fingerprint(values: &[(&str, &str)]) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash_build_overrides(
            &mut hash,
            values.iter().map(|&(key, value)| {
                (OsString::from(key), OsString::from(value))
            }),
        );
        hash.finalize().to_vec()
    }

    #[test]
    fn cargo_overrides_are_order_independent_and_sensitive() {
        let flags = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS";
        let profile = "CARGO_PROFILE_TEST_DEBUG";
        let build = "CARGO_BUILD_RUSTDOCFLAGS";
        let baseline =
            override_fingerprint(&[(flags, "-D warnings"), (profile, "0")]);
        assert_eq!(
            baseline,
            override_fingerprint(&[(profile, "0"), (flags, "-D warnings")])
        );
        assert_ne!(
            baseline,
            override_fingerprint(&[(flags, ""), (profile, "0")])
        );
        assert_ne!(baseline, override_fingerprint(&[(flags, "-D warnings")]));
        assert_ne!(
            override_fingerprint(&[]),
            override_fingerprint(&[(build, "")])
        );
        assert_eq!(
            override_fingerprint(&[]),
            override_fingerprint(&[("CARGO_MAKEFLAGS", "jobserver")])
        );
    }

    #[test]
    fn native_overrides_are_order_independent_and_sensitive() {
        let flags = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS";
        for native in [
            "CC",
            "CC_x86_64-unknown-linux-gnu",
            "CFLAGS_aarch64_linux_android",
            "HOST_AR",
            "TARGET_PKG_CONFIG_PATH",
            "PKG_CONFIG",
            "PKG_CONFIG_LIBDIR_x86_64_unknown_linux_gnu",
            "SDL2_NO_PKG_CONFIG",
            "CC_FORCE_DISABLE",
            "RING_PREGENERATE_ASM",
        ] {
            let empty = override_fingerprint(&[(native, "")]);
            let set = override_fingerprint(&[(native, "value")]);
            assert_ne!(override_fingerprint(&[]), empty);
            assert_ne!(empty, set);
            assert_eq!(
                override_fingerprint(&[
                    (native, "value"),
                    (flags, "-D warnings")
                ]),
                override_fingerprint(&[
                    (flags, "-D warnings"),
                    (native, "value")
                ])
            );
        }
        assert_eq!(
            override_fingerprint(&[]),
            override_fingerprint(&[("CCACHE_DIR", "x")])
        );
        assert_eq!(
            override_fingerprint(&[]),
            override_fingerprint(&[("UNRELATED", "x")])
        );
    }

    #[test]
    fn cargo_config_fingerprint_tracks_hierarchy_and_includes()
    -> Result<(), String> {
        let fixture = directory("cargo-config-cache")?;
        let root = fixture.join("repo");
        let parent = fixture.join(".cargo");
        let home = fixture.join("cargo-home");
        for path in [&root, &parent, &home] {
            fs::create_dir_all(path).map_err(|error| error.to_string())?;
        }
        let fingerprint = || -> Result<_, String> {
            let mut hash = Sha256::new();
            hash_cargo_config_files(&root, &home, &mut hash)?;
            Ok(hash.finalize())
        };
        let original = fingerprint()?;
        let config = parent.join("config.toml");
        let include = parent.join("child.toml");
        fs::write(&config, "include = [{path=\"child.toml\", optional=true}]")
            .map_err(|error| error.to_string())?;
        let missing = fingerprint()?;
        require(original != missing, "ancestor config added")?;
        fs::write(&include, "[build]\njobs = 1")
            .map_err(|error| error.to_string())?;
        let present = fingerprint()?;
        require(missing != present, "optional include added")?;
        fs::write(&include, "[build]\njobs = 2")
            .map_err(|error| error.to_string())?;
        require(present != fingerprint()?, "include content changed")?;
        fs::remove_file(include).map_err(|error| error.to_string())?;
        require(missing == fingerprint()?, "optional include removed")?;
        fs::write(parent.join("config"), "[build]\njobs = 3")
            .map_err(|error| error.to_string())?;
        let legacy = fingerprint()?;
        require(missing != legacy, "extensionless config added")?;
        fs::write(config, "[build]\njobs = 4")
            .map_err(|error| error.to_string())?;
        require(legacy == fingerprint()?, "shadow config ignored")?;
        fs::write(home.join("config.toml"), "[build]\njobs = 5")
            .map_err(|error| error.to_string())?;
        require(legacy != fingerprint()?, "Cargo home config added")?;
        fs::remove_dir_all(fixture).map_err(|error| error.to_string())
    }

    #[test]
    fn concurrent_runs_have_distinct_logs() -> Result<(), String> {
        let root = directory("concurrent")?;
        fs::write(root.join("input"), b"original")
            .map_err(|error| error.to_string())?;
        thread::scope(|scope| {
            let first = scope.spawn(|| run_with(&root, true, pass, key));
            let second = scope.spawn(|| run_with(&root, true, pass, key));
            first.join().map_err(|_panic| "first worker panicked")??;
            second.join().map_err(|_panic| "second worker panicked")?
        })?;
        let logs = fs::read_dir(root.join(".temp/validate"))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        require(logs.len() == 2, "one unique log per validation")?;
        for entry in logs {
            let log = fs::read_to_string(entry.path())
                .map_err(|error| error.to_string())?;
            require(log == "executed\nPASS\n", "complete independent log")?;
        }
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }
}
