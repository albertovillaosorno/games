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
/// Parsed host API policy for portable crates.
pub mod host_api;
/// Platform isolation and migration guard.
pub mod portability;
/// Typed task validation.
pub mod todo;
/// Unified IR and device protocol boundaries.
pub mod unified_ir;

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, str};

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

fn build_dependency_graph(graph: &str) -> Result<(), String> {
    for line in graph.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        if matches!(name, "rustls" | "ring" | "rcgen" | "rustls-webpki") {
            return Err(format!(
                "asset build must not enable authenticated LAN: {name}"
            ));
        }
    }
    Ok(())
}

fn validate_feature_graphs(
    root: &Path,
    packages: &[String],
    log: &mut File,
) -> Result<(), String> {
    let graph = Command::new("cargo")
        .current_dir(root)
        .args([
            "tree",
            "--locked",
            "-p",
            "games-build",
            "--all-features",
            "--edges",
            "normal,build,dev",
            "--prefix",
            "none",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !graph.status.success() {
        return Err(String::from("build dependency graph failed"));
    }
    build_dependency_graph(
        str::from_utf8(&graph.stdout).map_err(|error| error.to_string())?,
    )?;
    writeln!(log, "Independent all-feature deliverable graphs")
        .map_err(|error| error.to_string())?;
    for package in packages {
        let args = [
            "clippy",
            "--locked",
            "-p",
            package,
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ];
        writeln!(log, "cargo {}", args.join(" "))
            .map_err(|error| error.to_string())?;
        execute(Command::new("cargo").current_dir(root).args(args), log)?;
    }
    Ok(())
}

fn run_validation(root: &Path, log: &mut File) -> Result<(), String> {
    writeln!(log, "Workspace and task contracts")
        .map_err(|error| error.to_string())?;
    let packages = contracts::validate(root)?;
    writeln!(log, "Authored language and width policy")
        .map_err(|error| error.to_string())?;
    files::validate(root)?;
    for args in CHECKS {
        writeln!(log, "cargo {}", args.join(" "))
            .map_err(|error| error.to_string())?;
        execute(Command::new("cargo").current_dir(root).args(*args), log)?;
    }
    validate_feature_graphs(root, &packages, log)?;
    if env::var_os(cache::PORTABLE_CORE_FLAG).is_some() {
        for target in [
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            "aarch64-pc-windows-gnullvm",
            "aarch64-linux-android",
            "wasm32-unknown-unknown",
            "x86_64-pc-windows-gnu",
        ] {
            let args = [
                "check",
                "--locked",
                "--target",
                target,
                "--lib",
                "-p",
                "games",
                "-p",
                "games-assets",
                "-p",
                "games-importer",
                "-p",
                "games-simpsons-arcade",
                "-p",
                "games-platform",
            ];
            writeln!(log, "cargo {}", args.join(" "))
                .map_err(|error| error.to_string())?;
            execute(Command::new("cargo").current_dir(root).args(args), log)?;
        }
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

#[cfg(test)]
mod tests {
    use super::build_dependency_graph;

    #[test]
    fn asset_build_excludes_runtime_authentication_dependencies() {
        assert!(
            build_dependency_graph(
                "games-build v0.1.0\nzip v9.8.2\ngetrandom v0.4.3\n"
            )
            .is_ok()
        );
        for package in ["rustls", "ring", "rcgen", "rustls-webpki"] {
            let graph = format!("games-build v0.1.0\n{package} v1.0.0\n");
            assert!(build_dependency_graph(&graph).is_err(), "{package}");
        }
    }
}
