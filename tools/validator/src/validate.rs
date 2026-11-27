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

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::str;

use games_platform::ir::toolchain::validation::{self, Request};
use games_platform::ir::toolchain::{self, Target};

use crate::contracts::Package;

const CHECKS: [Request<'static>; 8] = [
    Request::History,
    Request::Format,
    Request::ClippyWorkspace,
    Request::TestWorkspace { all_features: false },
    Request::TestWorkspace { all_features: true },
    Request::CompileWorkspace { all_features: false },
    Request::CompileWorkspace { all_features: true },
    Request::PackageEngine,
];

fn execute(
    root: &OsStr,
    request: Request<'_>,
    log: &mut dyn Write,
) -> Result<(), String> {
    let output =
        validation::run(root, request).map_err(|error| error.to_string())?;
    writeln!(log, "{}", output.command).map_err(|error| error.to_string())?;
    log.write_all(&output.stdout)
        .map_err(|error| error.to_string())?;
    log.write_all(&output.stderr)
        .map_err(|error| error.to_string())?;
    if output.success {
        Ok(())
    } else {
        Err(format!(
            "validation subprocess failed: {}",
            output.diagnostic
        ))
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

fn runtime_dependency_graph(graph: &str) -> Result<(), String> {
    for line in graph.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        if name == "games-importer" {
            return Err(String::from(
                "runtime graph must exclude source decoders",
            ));
        }
    }
    Ok(())
}

fn validate_runtime_graph(root: &OsStr, package: &str) -> Result<(), String> {
    let graph = validation::run(root, Request::RuntimeGraph(package))
        .map_err(|error| error.to_string())?;
    if !graph.success {
        return Err(format!("runtime dependency graph failed: {package}"));
    }
    runtime_dependency_graph(
        str::from_utf8(&graph.stdout).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("{package}: {error}"))
}

fn portable_packages(packages: &[Package]) -> Vec<&str> {
    packages
        .iter()
        .filter(|package| package.portable)
        .map(|package| package.name.as_str())
        .collect()
}

fn validate_portable(
    root: &OsStr,
    packages: &[Package],
    log: &mut dyn Write,
) -> Result<(), String> {
    let selected = portable_packages(packages);
    for target in Target::portable_checks() {
        execute(
            root,
            Request::Portable {
                packages: &selected,
                target,
            },
            log,
        )?;
    }
    Ok(())
}

fn validate_feature_graphs(
    root: &OsStr,
    packages: &[Package],
    log: &mut dyn Write,
) -> Result<(), String> {
    let graph = validation::run(root, Request::BuildGraph)
        .map_err(|error| error.to_string())?;
    if !graph.success {
        return Err(String::from("build dependency graph failed"));
    }
    build_dependency_graph(
        str::from_utf8(&graph.stdout).map_err(|error| error.to_string())?,
    )?;
    writeln!(log, "Independent all-feature deliverable graphs")
        .map_err(|error| error.to_string())?;
    for package in packages {
        if package.runtime {
            validate_runtime_graph(root, &package.name)?;
        }
        execute(root, Request::ClippyPackage(&package.name), log)?;
    }
    Ok(())
}

fn validate_browser_smokes(
    root: &OsStr,
    log: &mut dyn Write,
) -> Result<(), String> {
    for (binary, feature) in [
        (
            "simpsons-arcade-browser-storage-smoke",
            "browser-storage-smoke",
        ),
        ("simpsons-arcade-browser-input-smoke", "browser-input-smoke"),
        ("simpsons-arcade-browser-scene-smoke", "browser-scene-smoke"),
        ("simpsons-arcade-browser-webgl-smoke", "browser-webgl-smoke"),
        (
            "simpsons-arcade-browser-window-smoke",
            "browser-window-smoke",
        ),
        ("simpsons-arcade-browser-frame-smoke", "browser-frame-smoke"),
        ("simpsons-arcade-browser-title-smoke", "browser-title-smoke"),
        (
            "simpsons-arcade-browser-scenery-smoke",
            "browser-title-smoke",
        ),
        (
            "simpsons-arcade-browser-soundtrack-smoke",
            "browser-title-smoke",
        ),
        ("simpsons-arcade-browser-preview", "browser-preview"),
        ("simpsons-arcade-browser-audio-smoke", "browser-audio-smoke"),
    ] {
        execute(
            root,
            Request::WasmSmokeBuild {
                package: "games-simpsons-arcade",
                binary,
                feature: Some(feature),
            },
            log,
        )?;
    }
    Ok(())
}

fn run_validation(root: &OsStr, log: &mut dyn Write) -> Result<(), String> {
    writeln!(log, "Workspace and task contracts")
        .map_err(|error| error.to_string())?;
    let packages = contracts::validate(root)?;
    writeln!(log, "Authored language and width policy")
        .map_err(|error| error.to_string())?;
    files::validate(root)?;
    for request in CHECKS {
        execute(root, request, log)?;
    }
    validate_feature_graphs(root, &packages, log)?;
    validate_portable(root, &packages, log)?;
    execute(
        root,
        Request::WasmSmokeBuild {
            package: "games-simpsons-arcade",
            binary: "simpsons-arcade-core-smoke",
            feature: None,
        },
        log,
    )?;
    execute(
        root,
        Request::WasmSmokeRun("simpsons-arcade-core-smoke"),
        log,
    )?;
    validate_browser_smokes(root, log)?;
    let paths = files::authored(root)?;
    let markdown: Vec<_> = paths
        .iter()
        .filter(|path| {
            files::authored_extension(path)
                .is_some_and(|ext| ext == "md" || ext == "mdc")
        })
        .map(OsString::as_os_str)
        .collect();
    execute(root, Request::Markdown(&markdown), log)
}

fn main() -> Result<(), String> {
    let args = toolchain::arguments().map_err(|error| error.to_string())?;
    let root =
        toolchain::workspace_root().map_err(|error| error.to_string())?;
    match args.as_slice() {
        [] => cache::run(&root, false, run_validation),
        [mode] if mode == "--no-cache" => {
            cache::run(&root, true, run_validation)
        },
        _ => Err(String::from("usage: cargo validate [--no-cache]")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Package, build_dependency_graph, portable_packages,
        runtime_dependency_graph,
    };

    #[test]
    fn runtime_graph_rejects_transitive_importer_activation() {
        assert!(
            runtime_dependency_graph(
                "new-title v0.1.0\ngames-platform v0.1.0\n"
            )
            .is_ok()
        );
        let graph = concat!(
            "new-title v0.1.0\ngames-platform v0.1.0\n",
            "games-importer v0.1.0\n",
        );
        assert!(runtime_dependency_graph(graph).is_err());
    }

    #[test]
    fn portable_matrix_includes_every_library_not_host_tools() {
        let packages = [
            Package {
                name: String::from("new-title"),
                portable: true,
                runtime: true,
            },
            Package {
                name: String::from("new-shared"),
                portable: true,
                runtime: false,
            },
            Package {
                name: String::from("host-tool"),
                portable: false,
                runtime: false,
            },
        ];
        assert_eq!(portable_packages(&packages), ["new-title", "new-shared"]);
    }

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
