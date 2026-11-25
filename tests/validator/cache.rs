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
//   - Validation cache regression tests.
// - Must-Not:
//   - Implement game behaviour or modify source.
// - Allows:
//   - Inputs: Repository files and local tool configuration.
//   - Outputs: Validation data or diagnostic.
//   - Side effects: Temporary fixture directory writes.
// - Split-When:
//   - An independent validation responsibility emerges.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Validation cache regression tests.
// - Description:
//   - Exercises the cache with independent test fixtures.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Reject failed or changed-source results.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::thread;

use super::run_with;
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
    fs::write(root.join("input"), b"changed").map_err(|error| error.to_string())
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
