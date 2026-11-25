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
//   - Unified contributor validation.
// - Must-Not:
//   - Approve untested behaviour or modify authored source.
// - Allows:
//   - Inputs: Command arguments and repository files.
//   - Outputs: Diagnostics and process exit status.
//   - Side effects: Normal tool caches and subprocess execution.
// - Split-When:
//   - A check requires an independent policy.
// - Merge-When:
//   - A shared runner supersedes the entry point.
// - Summary:
//   - One-command contributor validation.
// - Description:
//   - Delegates formatting, linting, tests and contract checks.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Fail if any required check fails.
//

//! Contributor validation entry point.

/// Source and dependency boundary checks.
pub mod boundaries;
/// Substantial source clone detection.
pub mod clones;

/// Cached validation and isolated run logs.
pub mod cache;
/// Cargo configuration discovery and include admission.
pub mod cargo_config;
/// Repository contract checks.
pub mod contracts;
/// Authored file discovery and language policy.
pub mod files;
/// Isolated test fixture helpers.
#[cfg(test)]
pub mod fixtures;
/// Typed task validation.
pub mod todo;

use std::env;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHECKS: &[&[&str]] = &[
    &[
        "run",
        "--locked",
        "--quiet",
        "-p",
        "games-validator",
        "--bin",
        "games-hooks",
        "--",
        "history",
    ],
    &[
        "+nightly",
        "fmt",
        "--all",
        "--",
        "--config-path",
        "rustfmt.toml",
        "--check",
    ],
    &[
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ],
    &[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ],
    &["test", "--workspace", "--locked"],
    &["test", "--workspace", "--all-features", "--locked"],
    &["check", "--workspace", "--all-targets", "--locked"],
    &[
        "check",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
    ],
    &[
        "package",
        "--package",
        "games",
        "--locked",
        "--offline",
        "--allow-dirty",
    ],
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn execute(command: &mut Command, log: &mut File) -> Result<(), String> {
    let output = command.output().map_err(|error| error.to_string())?;
    log.write_all(&output.stdout)
        .map_err(|error| error.to_string())?;
    log.write_all(&output.stderr)
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("validation subprocess failed: {}", output.status))
    }
}

fn run_validation(root: &Path, log: &mut File) -> Result<(), String> {
    writeln!(log, "Workspace and task contracts")
        .map_err(|error| error.to_string())?;
    contracts::validate(root)?;
    writeln!(log, "Authored language and width policy")
        .map_err(|error| error.to_string())?;
    files::validate(root)?;
    for args in CHECKS {
        writeln!(log, "cargo {}", args.join(" "))
            .map_err(|error| error.to_string())?;
        execute(Command::new("cargo").current_dir(root).args(*args), log)?;
    }
    let paths = files::authored(root)?;
    let markdown = paths.iter().filter(|path| {
        path.extension()
            .is_some_and(|ext| ext == "md" || ext == "mdc")
    });
    writeln!(log, "markdownlint-cli2").map_err(|error| error.to_string())?;
    execute(
        Command::new("markdownlint-cli2")
            .current_dir(root)
            .args(markdown),
        log,
    )
}

fn main() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => cache::run(&root(), false, run_validation),
        [mode] if mode == "--no-cache" => {
            cache::run(&root(), true, run_validation)
        },
        _ => Err(String::from("usage: cargo validate [--no-cache]")),
    }
}
