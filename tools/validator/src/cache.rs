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

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::SystemTime;

use sha2::{Digest as _, Sha256};

use crate::files::authored;

/// Function signature for a complete validation pass.
pub type Check = fn(&Path, &mut File) -> Result<(), String>;
type Fingerprint = fn(&Path) -> Result<Vec<u8>, String>;

fn hash_value(hash: &mut Sha256, value: &[u8]) {
    hash.update(value.len().to_le_bytes());
    hash.update(value);
}

fn tool_versions(root: &Path, hash: &mut Sha256) -> Result<(), String> {
    for (tool, args) in [
        ("rustc", &["-vV"][..]),
        ("cargo", &["-V"][..]),
        ("cargo", &["clippy", "-V"][..]),
        ("cargo", &["+nightly", "fmt", "--version"][..]),
        ("markdownlint-cli2", &["--version"][..]),
        ("rg", &["--version"][..]),
    ] {
        let output = Command::new(tool)
            .current_dir(root)
            .args(args)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!("cannot identify {tool}"));
        }
        hash_value(hash, &output.stdout);
        hash_value(hash, &output.stderr);
    }
    for key in [
        "PATH",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTUP_TOOLCHAIN",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "CARGO_TARGET_DIR",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_TARGET",
        "CARGO_PROFILE_DEV_DEBUG",
        "CLIPPY_CONF_DIR",
    ] {
        hash_value(hash, key.as_bytes());
        hash_value(
            hash,
            env::var_os(key).unwrap_or_default().as_encoded_bytes(),
        );
    }
    Ok(())
}

fn fingerprint(root: &Path) -> Result<Vec<u8>, String> {
    let mut hash = Sha256::new();
    tool_versions(root, &mut hash)?;
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
/// # Errors
///
/// Returns an error for failed checks, I/O or concurrent source changes.
pub fn run(root: &Path, fresh: bool, check: Check) -> Result<(), String> {
    run_with(root, fresh, check, fingerprint)
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
    let before = identify(root)?;
    let marker = root.join(".cache/validate/success");
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
#[path = "../../../tests/validator/cache.rs"]
mod tests;
