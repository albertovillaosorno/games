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
use std::io::Write;

use games_platform::ir::storage::join_path;
use games_platform::ir::toolchain::environment::{
    self, Environment, FINGERPRINT_KEYS, FRESH_CHECK_FLAGS,
};
use games_platform::ir::toolchain::validation::repository_head;
use games_platform::ir::toolchain::validation::state::{
    ValidationKey, ValidationState,
};
use games_platform::ir::toolchain::{
    ValidationTool, probe_version, write_line,
};
use sha2::{Digest as _, Sha256};

use crate::files::{
    authored, canonical, ledger_location, read_authored, repository_present,
};
use crate::{cargo_config, portability};

/// Function signature for a complete validation pass.
pub type Check = fn(&OsStr, &mut dyn Write) -> Result<(), String>;
type Fingerprint = fn(&OsStr) -> Result<ValidationKey, String>;

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

fn hash_build_overrides(
    hash: &mut Sha256,
    overrides: &BTreeMap<OsString, OsString>,
) {
    hash_value(hash, b"build-overrides");
    hash_value(hash, &overrides.len().to_le_bytes());
    for (key, value) in overrides {
        hash_value(hash, key.as_encoded_bytes());
        hash_value(hash, value.as_encoded_bytes());
    }
}

fn cargo_home(
    root: &OsStr,
    observed: &Environment,
) -> Result<OsString, String> {
    if let Some(home) =
        observed.value("CARGO_HOME").filter(|path| !path.is_empty())
    {
        return join_path(root, home).map_err(|error| error.to_string());
    }
    observed
        .home()
        .ok_or_else(|| String::from("cannot resolve Cargo home"))
        .and_then(|home| {
            join_path(home, ".cargo").map_err(|error| error.to_string())
        })
}

fn hash_cargo_config_files(
    root: &OsStr,
    home: &OsStr,
    hash: &mut Sha256,
) -> Result<(), String> {
    hash_value(hash, b"cargo-config-files");
    cargo_config::visit_active(root, home, &mut |path, document| {
        hash_value(hash, path.as_encoded_bytes());
        hash_value(hash, &[u8::from(document.is_some())]);
        if let Some((text, _value)) = document {
            hash_value(hash, text.as_bytes());
        }
        Ok(())
    })
}

fn tool_versions(
    root: &OsStr,
    hash: &mut Sha256,
    observed: &Environment,
) -> Result<(), String> {
    for tool in [
        ValidationTool::Rustc,
        ValidationTool::Cargo,
        ValidationTool::Clippy,
        ValidationTool::Rustfmt,
        ValidationTool::RustupTargets,
        ValidationTool::Markdown,
        ValidationTool::Node,
        ValidationTool::Ripgrep,
        ValidationTool::Git,
    ] {
        let output =
            probe_version(root, tool).map_err(|error| error.to_string())?;
        hash_value(hash, &output.stdout);
        hash_value(hash, &output.stderr);
    }
    for key in FINGERPRINT_KEYS.iter().copied().chain(FRESH_CHECK_FLAGS) {
        hash_environment(hash, key, observed.value(key));
    }
    hash_build_overrides(hash, observed.build_overrides());
    Ok(())
}

fn fingerprint(root: &OsStr) -> Result<ValidationKey, String> {
    ledger_location(root)?;
    portability::preflight(root)?;
    let mut hash = Sha256::new();
    let observed = environment::capture().map_err(|error| error.to_string())?;
    tool_versions(root, &mut hash, &observed)?;
    let canonical = canonical(root)?;
    hash_cargo_config_files(
        &canonical,
        &cargo_home(&canonical, &observed)?,
        &mut hash,
    )?;
    if repository_present(root)? {
        let head = repository_head(root).map_err(|error| error.to_string())?;
        hash_value(&mut hash, b"git-history");
        hash_value(&mut hash, &[u8::from(head.is_some())]);
        if let Some(identity) = head {
            hash_value(&mut hash, &identity);
        }
    }
    for path in authored(root)? {
        hash_value(&mut hash, path.as_encoded_bytes());
        let bytes = read_authored(root, &path)?;
        hash_value(&mut hash, &bytes);
    }
    Ok(hash.finalize().into())
}

fn announce(message: &str) -> Result<(), String> {
    write_line(message).map_err(|error| error.to_string())
}

/// Serialize validation and reuse only successful unchanged results.
///
/// Explicit native smoke requests run fresh.
/// The fingerprint also includes the installed Rust target inventory.
///
/// # Errors
///
/// Returns an error for failed checks, I/O or concurrent source changes.
pub fn run(root: &OsStr, fresh: bool, check: Check) -> Result<(), String> {
    let requested = environment::capture()
        .map_err(|error| error.to_string())?
        .fresh_requested();
    run_with(root, fresh || requested, check, fingerprint)
}

fn run_with<Operation>(
    root: &OsStr,
    fresh: bool,
    check: Operation,
    identify: Fingerprint,
) -> Result<(), String>
where
    Operation: FnOnce(&OsStr, &mut dyn Write) -> Result<(), String>,
{
    let mut state =
        ValidationState::open(root).map_err(|error| error.to_string())?;
    announce(&format!(
        "Validation log: {}",
        state.log_path().to_string_lossy()
    ))?;
    let before = match identify(root) {
        Ok(value) => value,
        Err(error) => {
            state.invalidate().map_err(|err| err.to_string())?;
            writeln!(state.log_writer(), "FAILED: {error}")
                .map_err(|err| err.to_string())?;
            return Err(error);
        },
    };
    if !fresh && state.matches(&before).map_err(|error| error.to_string())? {
        writeln!(state.log_writer(), "Cache hit")
            .map_err(|error| error.to_string())?;
        return announce("Validation passed (cache hit).");
    }
    state.invalidate().map_err(|error| error.to_string())?;
    if let Err(error) = check(root, state.log_writer()) {
        writeln!(state.log_writer(), "FAILED: {error}")
            .map_err(|err| err.to_string())?;
        return Err(error);
    }
    if identify(root)? != before {
        return Err(String::from("source changed during validation; rerun"));
    }
    writeln!(state.log_writer(), "PASS").map_err(|error| error.to_string())?;
    state.publish(&before).map_err(|error| error.to_string())?;
    announce("Validation passed (fresh).")
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::io::{Error, Write};
    use std::num::NonZeroUsize;

    use games_platform::ir::process::fixtures::with_worker;
    use games_platform::ir::storage::{directory_entries, join_path};
    use sha2::{Digest as _, Sha256};

    use super::{
        Environment, FRESH_CHECK_FLAGS, ValidationKey, fingerprint,
        hash_build_overrides, hash_cargo_config_files, hash_environment,
        run_with,
    };
    use crate::fixtures::{directory, exists, read, require};

    fn key(root: &OsStr) -> Result<ValidationKey, String> {
        let bytes = read(
            &join_path(root, "input").map_err(|error| error.to_string())?,
        )?;
        Ok(Sha256::digest(bytes).into())
    }

    fn pass(_root: &OsStr, log: &mut dyn Write) -> Result<(), String> {
        writeln!(log, "executed").map_err(|error| error.to_string())
    }

    fn fail(root: &OsStr, _log: &mut dyn Write) -> Result<(), String> {
        read(&join_path(root, "missing").map_err(|error| error.to_string())?)
            .map(|_| ())
    }

    #[test]
    fn caches_only_unchanged_success() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        root_owner
            .write_file("input", b"original")
            .map_err(|error| error.to_string())?;
        let marker = join_path(root, ".cache/validate/success")
            .map_err(|error| error.to_string())?;

        run_with(root, false, pass, key)?;
        require(exists(&marker)?, "successful result must be cached")?;
        run_with(root, false, fail, key)?;
        require(
            run_with(root, true, fail, key).is_err(),
            "fresh validation must bypass the cache",
        )?;
        require(!exists(&marker)?, "failure must invalidate the cache")?;

        run_with(root, false, pass, key)?;
        root_owner
            .write_file("input", b"updated")
            .map_err(|error| error.to_string())?;
        require(
            run_with(root, false, fail, key).is_err(),
            "changed inputs must trigger checks",
        )?;
        require(
            !exists(&marker)?,
            "changed and failed input must not be cached",
        )?;
        require(
            run_with(
                root,
                true,
                |_, _| {
                    root_owner
                        .write_file("input", b"changed")
                        .map_err(|error| error.to_string())
                },
                key,
            )
            .is_err(),
            "changing inputs during checks must invalidate the pass",
        )?;
        require(
            !exists(&marker)?,
            "concurrent modification must not be cached",
        )?;

        Ok(())
    }

    #[test]
    fn ignored_disposable_ledger_invalidates_cached_success()
    -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        root_owner
            .write_file("input", b"previously admitted")
            .map_err(|error| error.to_string())?;
        run_with(root, false, pass, key)?;
        let marker = join_path(root, ".cache/validate/success")
            .map_err(|error| error.to_string())?;
        require(exists(&marker)?, "fixture success marker")?;
        root_owner
            .create_directories(".temp/ledger")
            .map_err(|error| error.to_string())?;
        let result = run_with(root, false, pass, fingerprint);
        require(
            result.is_err_and(|error| error.contains(".reference/ledger")),
            "durable ledger must fail before cache lookup",
        )?;
        require(!exists(&marker)?, "stale marker must be removed")?;
        Ok(())
    }

    #[test]
    fn forbidden_source_invalidates_cached_success() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        root_owner
            .create_directories("crates/games/example/src")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file(".gitignore", b"/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("Cargo.toml", b"[workspace.dependencies]\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("input", b"previously admitted")
            .map_err(|error| error.to_string())?;
        run_with(root, false, pass, key)?;
        let marker = join_path(root, ".cache/validate/success")
            .map_err(|error| error.to_string())?;
        require(exists(&marker)?, "fixture must have a success marker")?;
        root_owner
            .write_file(
                "crates/games/example/src/main.rs",
                b"#[cfg(unix)] fn run() {}",
            )
            .map_err(|error| error.to_string())?;
        let result = run_with(root, false, pass, fingerprint);
        let diagnostic = result.err().unwrap_or_default();
        require(
            diagnostic.contains("os:unix"),
            "portability must fail first",
        )?;
        require(!exists(&marker)?, "stale marker must be invalidated")?;
        Ok(())
    }

    #[test]
    fn fresh_requests_cannot_reuse_ordinary_cache() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        root_owner
            .write_file("input", b"unchanged")
            .map_err(|error| error.to_string())?;
        require(
            !Environment::new([], None).fresh_requested(),
            "ordinary validation may reuse cache",
        )?;
        for flag in FRESH_CHECK_FLAGS {
            run_with(root, false, pass, key)?;
            run_with(root, false, fail, key)?;
            let requested =
                Environment::new([(flag.into(), OsString::new())], None)
                    .fresh_requested();
            require(
                run_with(root, requested, fail, key).is_err(),
                "explicit check must execute instead of reusing success",
            )?;
        }
        Ok(())
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
        let environment = Environment::new(
            values.iter().map(|&(key, value)| {
                (OsString::from(key), OsString::from(value))
            }),
            None,
        );
        hash_build_overrides(&mut hash, environment.build_overrides());
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
        let fixture_owner = directory()?;
        let fixture = fixture_owner.location();
        let root =
            join_path(fixture, "repo").map_err(|error| error.to_string())?;
        let home = join_path(fixture, "cargo-home")
            .map_err(|error| error.to_string())?;
        for name in ["repo", ".cargo", "cargo-home"] {
            fixture_owner
                .create_directories(name)
                .map_err(|error| error.to_string())?;
        }
        let fingerprint = || -> Result<_, String> {
            let mut hash = Sha256::new();
            hash_cargo_config_files(&root, &home, &mut hash)?;
            Ok(hash.finalize())
        };
        let original = fingerprint()?;
        fixture_owner
            .write_file(
                ".cargo/config.toml",
                b"include = [{path=\"child.toml\", optional=true}]",
            )
            .map_err(|error| error.to_string())?;
        let missing = fingerprint()?;
        require(original != missing, "ancestor config added")?;
        fixture_owner
            .write_file(".cargo/child.toml", b"[build]\njobs = 1")
            .map_err(|error| error.to_string())?;
        let present = fingerprint()?;
        require(missing != present, "optional include added")?;
        fixture_owner
            .write_file(".cargo/child.toml", b"[build]\njobs = 2")
            .map_err(|error| error.to_string())?;
        require(present != fingerprint()?, "include content changed")?;
        fixture_owner
            .remove_file(".cargo/child.toml")
            .map_err(|error| error.to_string())?;
        require(missing == fingerprint()?, "optional include removed")?;
        fixture_owner
            .write_file(".cargo/config", b"[build]\njobs = 3")
            .map_err(|error| error.to_string())?;
        let legacy = fingerprint()?;
        require(missing != legacy, "extensionless config added")?;
        fixture_owner
            .write_file(".cargo/config.toml", b"[build]\njobs = 4")
            .map_err(|error| error.to_string())?;
        require(legacy == fingerprint()?, "shadow config ignored")?;
        fixture_owner
            .write_file("cargo-home/config.toml", b"[build]\njobs = 5")
            .map_err(|error| error.to_string())?;
        require(legacy != fingerprint()?, "Cargo home config added")?;
        Ok(())
    }

    #[test]
    fn concurrent_runs_have_distinct_logs() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        root_owner
            .write_file("input", b"original")
            .map_err(|error| error.to_string())?;
        with_worker(
            || run_with(root, true, pass, key).map_err(Error::other),
            || run_with(root, true, pass, key).map_err(Error::other),
        )
        .map_err(|error| error.to_string())?;
        let log_dir = join_path(root, ".temp/validate")
            .map_err(|error| error.to_string())?;
        let logs = directory_entries(
            log_dir.as_os_str(),
            NonZeroUsize::MIN.saturating_add(2),
        )
        .map_err(|error| error.to_string())?;
        require(logs.len() == 2, "one unique log per validation")?;
        for entry in logs {
            let log = read(
                &join_path(&log_dir, &entry.name)
                    .map_err(|error| error.to_string())?,
            )?;
            require(log == b"executed\nPASS\n", "complete independent log")?;
        }
        Ok(())
    }
}
