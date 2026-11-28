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
//   - Fixed contributor validation requests and completion admission.
// - Must-Not:
//   - Access a filesystem, execute a process or parse diagnostic bytes.
// - Allows:
//   - Inputs: Authored identifiers, selected targets and exit observations.
//   - Outputs: Admitted arguments and consistent captured reports.
//   - Side effects: None.
// - Split-When:
//   - A different validation protocol needs independent argument policy.
// - Merge-When:
//   - A shared request contract provides the same admission semantics.
// - Summary:
//   - Admit fixed workspace operations and observed outcomes.
// - Description:
//   - Bind report labels and success to admitted arguments and exit code.
// - Usage:
//   - Admit a request before dispatch and construct its report afterward.
// - Defaults:
//   - Locked Cargo operations; reject unsafe or empty dynamic inputs.

use core::str::from_utf8;
use std::ffi::{OsStr, OsString};

use games_assets::archive::validate_path;

use super::identifier;
use crate::ir::toolchain::Target;

const WORKSPACE_METADATA: [&str; 5] =
    ["metadata", "--no-deps", "--locked", "--format-version", "1"];

/// Captured result of one admitted validation subprocess.
#[derive(Debug)]
pub struct Report {
    /// Human-readable command for the local validation log.
    pub command: String,
    /// Exit status or other completion diagnostic.
    pub diagnostic: String,
    /// Numeric process exit code, absent if no code is available.
    pub exit_code: Option<i32>,
    /// Diagnostic output, in original bytes.
    pub stderr: Vec<u8>,
    /// Ordinary output, in original bytes.
    pub stdout: Vec<u8>,
    /// Whether the process exited successfully.
    pub success: bool,
}

/// Native exit and byte-stream observations, without a claimed success flag.
#[derive(Debug)]
pub(crate) struct Completion {
    pub(crate) diagnostic: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout: Vec<u8>,
}

impl Report {
    pub(crate) fn from_completion(
        args: &Arguments,
        completion: Completion,
    ) -> Self {
        let command = format!(
            "{} {}",
            args.program(),
            args.args()
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        Self {
            command,
            diagnostic: completion.diagnostic,
            exit_code: completion.exit_code,
            stderr: completion.stderr,
            stdout: completion.stdout,
            success: completion.exit_code == Some(0),
        }
    }
}

/// Framed path bytes that have not passed name admission.
pub type PathRecords<'input> = Vec<&'input [u8]>;

/// Fixed contributor check with explicitly admitted dynamic inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request<'input> {
    /// Lint the ARM64 Android interactive platform adapter.
    AndroidInteractive,
    /// Discover authored files using only the repository ignore policy.
    AuthoredFiles,
    /// Inspect the asset build graph across all targets.
    BuildGraph,
    /// Lint one independent all-feature package.
    ClippyPackage(&'input str),
    /// Lint the default workspace targets.
    ClippyWorkspace,
    /// Compile workspace targets with the selected feature coverage.
    CompileWorkspace {
        /// Include all workspace features.
        all_features: bool,
    },
    /// Check formatting with the pinned formatter and configuration.
    Format,
    /// Observe repository state through a read-only Git request.
    Git(GitRequest),
    /// Validate existing Git history through the authored hook runner.
    History,
    /// Check authored Markdown paths, with no caller-selected options.
    Markdown(&'input [&'input OsStr]),
    /// Verify the independently packaged engine offline.
    PackageEngine,
    /// Compile selected portable libraries with no default features.
    Portable {
        /// Nonempty set of workspace library package names.
        packages: &'input [&'input str],
        /// One of the required portable targets.
        target: Target,
    },
    /// Inspect one runtime's normal/build dependency graph on all targets.
    RuntimeGraph(&'input str),
    /// Run workspace tests with the selected feature coverage.
    TestWorkspace {
        /// Include all workspace features.
        all_features: bool,
    },
    /// Link a portable executable without its native preview features.
    WasmSmokeBuild {
        /// Workspace package containing the executable.
        package: &'input str,
        /// Authored smoke executable name.
        binary: &'input str,
        /// Optional capability feature; default features remain disabled.
        feature: Option<&'input str>,
    },
    /// Execute a linked, import-free WebAssembly smoke artifact.
    WasmSmokeRun(&'input str),
    /// Obtain the locked workspace member metadata, without dependencies.
    WorkspaceMetadata,
}

/// Fixed read-only observations of repository state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRequest {
    /// List staged additions, copies, modifications, renames and type changes.
    ChangedStaged,
    /// List staged deletions.
    DeletedStaged,
    /// Resolve HEAD, preserving an unborn branch observation.
    Head,
    /// Read NUL-framed commit identities and messages.
    History,
    /// List tracked and nonignored untracked paths.
    Inventory,
    /// Resolve the symbolic HEAD reference.
    SymbolicHead,
    /// List all unstaged changes.
    Unstaged,
}

/// Fixed Git operations for an isolated temporary repository.
#[cfg(any(test, feature = "test-fixtures"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureGit<'name> {
    /// Initialize an empty repository without user templates.
    Initialize,
    /// Stage one admitted relative file, optionally overriding ignore rules.
    Stage {
        /// Relative fixture name interpreted literally.
        name: &'name str,
        /// Whether the fixture deliberately stages an ignored file.
        force: bool,
    },
}

/// An authored repository hook, never a caller-selected script path.
#[cfg(any(test, feature = "test-fixtures"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureHook {
    /// Commit message validation.
    CommitMessage,
    /// Staged file and full validation gate.
    PreCommit,
}

/// Deliberately fail a fixed hook delegate in a private test repository.
#[cfg(any(test, feature = "test-fixtures"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureDelegate {
    /// Reject a commit message.
    CommitMessage,
    /// All delegates succeed.
    None,
    /// Reject staged files.
    Staged,
    /// Reject validation.
    Validate,
}

/// A dynamic validation argument did not meet the shared policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidRequest;

/// One admitted executable with no arbitrary command or argument constructor.
#[derive(Debug)]
pub(crate) struct Arguments {
    args: Vec<OsString>,
    program: &'static str,
}

impl Arguments {
    pub(crate) fn args(&self) -> &[OsString] {
        &self.args
    }

    pub(crate) const fn program(&self) -> &'static str {
        self.program
    }
}

impl<'input> Request<'input> {
    pub(crate) fn admit(self) -> Result<Arguments, InvalidRequest> {
        let (program, args) = match self {
            Self::AuthoredFiles => ("rg", vec![
                "--no-config",
                "--files",
                "--hidden",
                "--no-ignore",
                "--ignore-file",
                ".gitignore",
                "-g",
                "!.git/**",
                "-0",
            ]),
            Self::Markdown(paths) => {
                if paths.is_empty() || !paths.iter().all(|path| markdown(path))
                {
                    return Err(InvalidRequest);
                }
                return Ok(Arguments {
                    args: paths
                        .iter()
                        .map(|path| {
                            let mut literal = OsString::from(":");
                            literal.push(path);
                            literal
                        })
                        .collect(),
                    program: "markdownlint-cli2",
                });
            },
            Self::Git(request) => ("git", git(request)),
            Self::WasmSmokeRun(binary) => return wasm_smoke_runner(binary),
            Self::AndroidInteractive
            | Self::BuildGraph
            | Self::ClippyPackage(_)
            | Self::ClippyWorkspace
            | Self::CompileWorkspace { .. }
            | Self::Format
            | Self::History
            | Self::PackageEngine
            | Self::Portable { .. }
            | Self::RuntimeGraph(_)
            | Self::TestWorkspace { .. }
            | Self::WasmSmokeBuild { .. }
            | Self::WorkspaceMetadata => ("cargo", self.cargo()?),
        };
        Ok(Arguments {
            args: args.into_iter().map(OsString::from).collect(),
            program,
        })
    }

    fn cargo(self) -> Result<Vec<&'input str>, InvalidRequest> {
        match self {
            Self::AndroidInteractive => Ok(interactive_clippy()),
            Self::BuildGraph => graph("games-build", "normal,build,dev"),
            Self::ClippyPackage(package) => clippy(package),
            Self::ClippyWorkspace => Ok(vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ]),
            Self::CompileWorkspace { all_features } => {
                Ok(workspace("check", all_features, true))
            },
            Self::Format => Ok(vec![
                "+nightly",
                "fmt",
                "--all",
                "--",
                "--config-path",
                "rustfmt.toml",
                "--check",
            ]),
            Self::AuthoredFiles
            | Self::Git(_)
            | Self::Markdown(_)
            | Self::WasmSmokeRun(_) => Err(InvalidRequest),
            Self::History => Ok(vec![
                "run",
                "--locked",
                "--quiet",
                "-p",
                "games-validator",
                "--bin",
                "games-hooks",
                "--",
                "history",
            ]),
            Self::PackageEngine => Ok(vec![
                "package",
                "--package",
                "games",
                "--locked",
                "--offline",
                "--allow-dirty",
            ]),
            Self::Portable { packages, target } => portable(packages, target),
            Self::RuntimeGraph(package) => graph(package, "normal,build"),
            Self::TestWorkspace { all_features } => {
                Ok(workspace("test", all_features, false))
            },
            Self::WasmSmokeBuild { package, binary, feature } => {
                wasm_smoke_build(package, binary, feature)
            },
            Self::WorkspaceMetadata => Ok(WORKSPACE_METADATA.to_vec()),
        }
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl FixtureGit<'_> {
    pub(crate) fn admit(self) -> Result<Arguments, InvalidRequest> {
        let args = match self {
            Self::Initialize => vec!["init", "--quiet", "--template="],
            Self::Stage { name, force } => {
                validate_path(name).map_err(|_error| InvalidRequest)?;
                if name
                    .split('/')
                    .any(|part| part.eq_ignore_ascii_case(".git"))
                {
                    return Err(InvalidRequest);
                }
                let mut args = vec!["--literal-pathspecs", "add"];
                if force {
                    args.push("--force");
                }
                args.extend(["--", name]);
                args
            },
        };
        Ok(Arguments {
            args: args.into_iter().map(OsString::from).collect(),
            program: "git",
        })
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl FixtureHook {
    /// Fixed authored hook filename.
    #[must_use]
    pub const fn filename(self) -> &'static str {
        match self {
            Self::PreCommit => "pre-commit",
            Self::CommitMessage => "commit-msg",
        }
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl FixtureDelegate {
    /// The corresponding authored fixture delegate token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Staged => "staged",
            Self::Validate => "validate",
            Self::CommitMessage => "commit-message",
        }
    }
}

/// Admit a successful Git object identity without changing its wire bytes.
///
/// # Errors
/// Rejects incomplete, noncanonical or unsupported object identifiers.
pub(crate) fn admit_head(bytes: &[u8]) -> Result<(), InvalidRequest> {
    let id = bytes.strip_suffix(b"\n").ok_or(InvalidRequest)?;
    if !matches!(id.len(), 40 | 64)
        || !id
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || id.iter().all(|byte| *byte == b'0')
    {
        return Err(InvalidRequest);
    }
    Ok(())
}

/// Admit a symbolic branch observation for an exact reference lookup.
///
/// # Errors
/// Rejects non-branch, malformed or ambiguous reference names.
pub(crate) fn reference_exists(
    bytes: &[u8],
) -> Result<Arguments, InvalidRequest> {
    let line = bytes.strip_suffix(b"\n").ok_or(InvalidRequest)?;
    let name = from_utf8(line).map_err(|_error| InvalidRequest)?;
    let branch = name.strip_prefix("refs/heads/").ok_or(InvalidRequest)?;
    if branch.contains("..")
        || branch.contains("@{")
        || branch.ends_with('.')
        || branch.split('/').any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || part.as_bytes().ends_with(b".lock")
        })
        || branch.chars().any(|character| {
            character.is_ascii_control() || " ~^:?*[\\".contains(character)
        })
    {
        return Err(InvalidRequest);
    }
    Ok(Arguments {
        args: ["show-ref", "--exists", name]
            .into_iter()
            .map(OsString::from)
            .collect(),
        program: "git",
    })
}

/// Admit complete NUL-delimited relative paths from an inventory tool.
///
/// Empty inventories are valid; malformed framing and unsafe names are not.
/// # Errors
/// Rejects incomplete records, non-UTF-8 names or unsafe path components.
pub fn admit_paths(bytes: &[u8]) -> Result<Vec<&str>, InvalidRequest> {
    path_records(bytes)?
        .into_iter()
        .map(|part| {
            let name = from_utf8(part).map_err(|_error| InvalidRequest)?;
            admit_path(name)
        })
        .collect()
}

/// Decode NUL-delimited path records without interpreting their names.
///
/// These bytes are not admitted paths. Use them only for identity comparison
/// or further admission; legacy deletions may contain nonportable names.
/// # Errors
/// Rejects incomplete framing and empty records.
pub fn path_records(bytes: &[u8]) -> Result<PathRecords<'_>, InvalidRequest> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let complete = bytes.strip_suffix(&[0]).ok_or(InvalidRequest)?;
    let records: Vec<_> = complete.split(|byte| *byte == 0).collect();
    if records.iter().any(|record| record.is_empty()) {
        return Err(InvalidRequest);
    }
    Ok(records)
}

/// Admit a legacy relative name only for staged deletion checks.
///
/// The portable name policy may reject older tracked names. Permit these
/// to be removed, but never admit traversal or an absolute name.
/// # Errors
/// Rejects non-UTF-8, ambiguous separators and unsafe components.
pub fn admit_deleted_path(bytes: &[u8]) -> Result<&str, InvalidRequest> {
    let name = from_utf8(bytes).map_err(|_error| InvalidRequest)?;
    if name.contains(['\\', ':'])
        || name.chars().any(char::is_control)
        || name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(InvalidRequest);
    }
    Ok(name)
}

/// Admit a canonical portable authored relative name.
/// # Errors
/// Rejects unsafe or nonportable names.
pub fn admit_path(name: &str) -> Result<&str, InvalidRequest> {
    validate_path(name).map_err(|_error| InvalidRequest)?;
    Ok(name)
}

fn markdown(path: &OsStr) -> bool {
    path.to_str().is_some_and(|name| {
        validate_path(name).is_ok()
            && !name.starts_with(['-', '!'])
            && !name.contains(['*', '?', '[', ']', '{', '}', '!'])
            && name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| matches!(ext, "md" | "mdc"))
    })
}

fn interactive_clippy() -> Vec<&'static str> {
    vec![
        "clippy",
        "--locked",
        "--target",
        Target::AndroidArm64.triple(),
        "--lib",
        "--no-default-features",
        "-p",
        "games-platform",
        "--features",
        "interactive,application-entry",
        "--",
        "-D",
        "warnings",
    ]
}

fn clippy(package: &str) -> Result<Vec<&str>, InvalidRequest> {
    if !identifier(package) {
        return Err(InvalidRequest);
    }
    Ok(vec![
        "clippy",
        "--locked",
        "-p",
        package,
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])
}

fn git(request: GitRequest) -> Vec<&'static str> {
    match request {
        GitRequest::ChangedStaged => {
            git_diff(&["--cached", "--diff-filter=ACMRT"])
        },
        GitRequest::DeletedStaged => git_diff(&["--cached", "--diff-filter=D"]),
        GitRequest::Head => {
            vec!["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]
        },
        GitRequest::History => vec!["log", "-z", "--format=%H%n%B", "HEAD"],
        GitRequest::Inventory => vec![
            "ls-files",
            "--cached",
            "--others",
            "--exclude-from=.gitignore",
            "-z",
        ],
        GitRequest::SymbolicHead => {
            vec!["symbolic-ref", "--quiet", "--no-recurse", "HEAD"]
        },
        GitRequest::Unstaged => git_diff(&[]),
    }
}

fn git_diff(options: &[&'static str]) -> Vec<&'static str> {
    let mut args = vec![
        "diff",
        "--name-only",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "-z",
    ];
    args.extend_from_slice(options);
    args.push("--");
    args
}

fn graph<'input>(
    package: &'input str,
    edges: &'static str,
) -> Result<Vec<&'input str>, InvalidRequest> {
    if !identifier(package) {
        return Err(InvalidRequest);
    }
    Ok(vec![
        "tree",
        "--locked",
        "-p",
        package,
        "--all-features",
        "--target",
        "all",
        "--edges",
        edges,
        "--prefix",
        "none",
    ])
}

fn wasm_smoke_build<'input>(
    package: &'input str,
    binary: &'input str,
    feature: Option<&'input str>,
) -> Result<Vec<&'input str>, InvalidRequest> {
    if !identifier(package)
        || !identifier(binary)
        || feature.is_some_and(|value| !identifier(value))
    {
        return Err(InvalidRequest);
    }
    let mut args = vec![
        "build",
        "--locked",
        "--release",
        "--target",
        "wasm32-unknown-unknown",
        "--target-dir",
        "target",
        "--no-default-features",
        "-p",
        package,
        "--bin",
        binary,
    ];
    if let Some(selected_feature) = feature {
        args.extend(["--features", selected_feature]);
    }
    Ok(args)
}

fn wasm_smoke_runner(binary: &str) -> Result<Arguments, InvalidRequest> {
    const SCRIPT: &str = concat!(
        "const fs = require('node:fs'); ",
        "const m = new WebAssembly.Module(fs.readFileSync(process.argv[1])); ",
        "const instance = new WebAssembly.Instance(m, {}); ",
        "if (instance.exports.main(0, 0) !== 0) ",
        "throw new Error('WebAssembly core smoke failed'); ",
        "console.log('WebAssembly core smoke passed');",
    );
    if !identifier(binary) {
        return Err(InvalidRequest);
    }
    Ok(Arguments {
        args: vec![
            "--no-addons".into(),
            "--eval".into(),
            SCRIPT.into(),
            "--".into(),
            format!("target/wasm32-unknown-unknown/release/{binary}.wasm")
                .into(),
        ],
        program: "node",
    })
}

fn portable<'input>(
    packages: &[&'input str],
    target: Target,
) -> Result<Vec<&'input str>, InvalidRequest> {
    if packages.is_empty() || !packages.iter().copied().all(identifier) {
        return Err(InvalidRequest);
    }
    let mut args = vec![
        "check",
        "--locked",
        "--target",
        target.triple(),
        "--lib",
        "--no-default-features",
    ];
    for &package in packages {
        args.extend(["-p", package]);
    }
    Ok(args)
}

fn workspace(
    operation: &'static str,
    all_features: bool,
    all_targets: bool,
) -> Vec<&'static str> {
    let mut args = vec![operation, "--workspace"];
    if all_targets {
        args.push("--all-targets");
    }
    if all_features {
        args.push("--all-features");
    }
    args.push("--locked");
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_head_requires_one_canonical_object_identity() {
        for valid in [
            b"0123456789abcdef0123456789abcdef01234567\n".as_slice(),
            concat!(
                "0123456789abcdef0123456789abcdef",
                "0123456789abcdef0123456789abcdef\n"
            )
            .as_bytes(),
        ] {
            assert_eq!(admit_head(valid), Ok(()));
        }
        for invalid in [
            b"".as_slice(),
            b"0123456789abcdef0123456789abcdef01234567",
            b"0123456789ABCDEF0123456789abcdef01234567\n",
            b"0123456789abcdef0123456789abcdef0123456g\n",
            b"0000000000000000000000000000000000000000\n",
            b"0123456789abcdef0123456789abcdef01234567\n\n",
        ] {
            assert_eq!(admit_head(invalid), Err(InvalidRequest));
        }
    }

    #[test]
    fn symbolic_head_lookup_requires_one_exact_branch()
    -> Result<(), InvalidRequest> {
        let args = reference_exists(b"refs/heads/team/main\n")?;
        if args.program() != "git"
            || args.args() != ["show-ref", "--exists", "refs/heads/team/main"]
        {
            return Err(InvalidRequest);
        }
        for invalid in [
            b"--option\n".as_slice(),
            b"refs/tags/main\n",
            b"refs/heads/\n",
            b"refs/heads/main",
            b"refs/heads/main\nother\n",
            b"refs/heads/.private\n",
            b"refs/heads/a//b\n",
            b"refs/heads/a..b\n",
            b"refs/heads/a.lock/b\n",
            b"refs/heads/a@{b\n",
            b"refs/heads/a b\n",
            b"refs/heads/a\\b\n",
            b"refs/heads/a\xff\n",
        ] {
            if reference_exists(invalid).is_ok() {
                return Err(InvalidRequest);
            }
        }
        Ok(())
    }

    #[test]
    fn completion_derives_success_from_exit_code() -> Result<(), InvalidRequest>
    {
        let args = Request::WorkspaceMetadata.admit()?;
        for (exit_code, success) in [
            (Some(0i32), true),
            (Some(1i32), false),
            (None, false),
            (Some(-1i32), false),
        ] {
            let report = Report::from_completion(&args, Completion {
                diagnostic: String::from("diagnostic"),
                exit_code,
                stderr: vec![0xff],
                stdout: b"output".to_vec(),
            });
            if report.command
                != "cargo metadata --no-deps --locked --format-version 1"
                || report.exit_code != exit_code
                || report.success != success
                || report.diagnostic != "diagnostic"
                || report.stderr != [0xff]
                || report.stdout != b"output"
            {
                return Err(InvalidRequest);
            }
        }
        Ok(())
    }

    #[test]
    fn package_inputs_cannot_select_options_or_paths() {
        for invalid in ["", "--help", "a/b", "a b", "a,b"] {
            assert!(Request::ClippyPackage(invalid).admit().is_err());
            assert!(Request::RuntimeGraph(invalid).admit().is_err());
            assert!(Request::WasmSmokeRun(invalid).admit().is_err());
            assert!(
                Request::WasmSmokeBuild {
                    package: "title",
                    binary: "smoke",
                    feature: Some(invalid),
                }
                .admit()
                .is_err()
            );
            assert!(
                Request::WasmSmokeBuild {
                    package: invalid,
                    binary: "smoke",
                    feature: None,
                }
                .admit()
                .is_err()
            );
            assert!(
                Request::WasmSmokeBuild {
                    package: "title",
                    binary: invalid,
                    feature: None,
                }
                .admit()
                .is_err()
            );
            assert!(
                Request::Portable {
                    packages: &[invalid],
                    target: Target::WebAssembly,
                }
                .admit()
                .is_err()
            );
        }
        assert!(
            Request::Portable {
                packages: &[],
                target: Target::WebAssembly,
            }
            .admit()
            .is_err()
        );
    }

    #[test]
    fn interactive_adapter_lint_is_admitted() -> Result<(), InvalidRequest> {
        let request = Request::AndroidInteractive.admit()?;
        if request.program() != "cargo"
            || request.args()
                != [
                    "clippy",
                    "--locked",
                    "--target",
                    "aarch64-linux-android",
                    "--lib",
                    "--no-default-features",
                    "-p",
                    "games-platform",
                    "--features",
                    "interactive,application-entry",
                    "--",
                    "-D",
                    "warnings",
                ]
        {
            return Err(InvalidRequest);
        }
        Ok(())
    }

    #[test]
    fn portable_and_runtime_commands_preserve_admitted_coverage()
    -> Result<(), InvalidRequest> {
        let request = Request::Portable {
            packages: &["games", "game-example"],
            target: Target::AndroidArm64,
        }
        .admit()?;
        if request.program() != "cargo"
            || request.args()
                != [
                    "check",
                    "--locked",
                    "--target",
                    "aarch64-linux-android",
                    "--lib",
                    "--no-default-features",
                    "-p",
                    "games",
                    "-p",
                    "game-example",
                ]
        {
            return Err(InvalidRequest);
        }
        let graph = Request::RuntimeGraph("game-example").admit()?;
        if graph.args()
            != [
                "tree",
                "--locked",
                "-p",
                "game-example",
                "--all-features",
                "--target",
                "all",
                "--edges",
                "normal,build",
                "--prefix",
                "none",
            ]
        {
            return Err(InvalidRequest);
        }
        Ok(())
    }

    #[test]
    fn wasm_smoke_requests_link_and_execute_the_same_fixed_artifact()
    -> Result<(), InvalidRequest> {
        let build = Request::WasmSmokeBuild {
            package: "game-example",
            binary: "core-smoke",
            feature: None,
        }
        .admit()?;
        if build.program() != "cargo"
            || build.args()
                != [
                    "build",
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--target-dir",
                    "target",
                    "--no-default-features",
                    "-p",
                    "game-example",
                    "--bin",
                    "core-smoke",
                ]
        {
            return Err(InvalidRequest);
        }
        let browser = Request::WasmSmokeBuild {
            package: "game-example",
            binary: "storage-smoke",
            feature: Some("browser-storage-smoke"),
        }
        .admit()?;
        if !browser.args().ends_with(&[
            OsString::from("--features"),
            OsString::from("browser-storage-smoke"),
        ]) {
            return Err(InvalidRequest);
        }
        let run = Request::WasmSmokeRun("core-smoke").admit()?;
        if run.program() != "node"
            || run.args().last()
                != Some(&OsString::from(
                    "target/wasm32-unknown-unknown/release/core-smoke.wasm",
                ))
        {
            return Err(InvalidRequest);
        }
        Ok(())
    }

    #[test]
    fn inventory_requires_complete_safe_relative_records() {
        for invalid in [
            b"missing-terminator".as_slice(),
            b"\0",
            b"src\0\0",
            b"../outside\0",
            b"/absolute\0",
            b"docs/../outside\0",
            b"bad\xff\0",
        ] {
            assert!(admit_paths(invalid).is_err());
        }
        assert_eq!(admit_paths(b""), Ok(Vec::new()));
        assert_eq!(
            admit_paths(b".gitignore\0src/lib.rs\0"),
            Ok(vec![".gitignore", "src/lib.rs"])
        );
    }

    #[test]
    fn deletion_and_dirty_identity_do_not_admit_new_authored_names() {
        let legacy = b"docs/NUL.txt";
        assert_eq!(admit_deleted_path(legacy), Ok("docs/NUL.txt"));
        assert!(admit_path("docs/NUL.txt").is_err());
        assert_eq!(
            path_records(b"docs/NUL.txt\0bad\xff\0"),
            Ok(vec![legacy.as_slice(), b"bad\xff".as_slice()])
        );
        for name in [
            b"".as_slice(),
            b"../outside",
            b"/outside",
            b"dir//name",
            b"dir/./name",
            b"C:/name",
            b"bad\\name",
            b"bad\xff",
        ] {
            assert!(admit_deleted_path(name).is_err());
        }
    }

    #[test]
    fn markdown_inputs_are_literal_authored_documents() {
        for invalid in [
            "--config.md",
            "../README.md",
            "!docs/a.md",
            "docs/*.md",
            "/README.md",
            "file.zip",
            "docs/[a].md",
        ] {
            assert!(Request::Markdown(&[OsStr::new(invalid)]).admit().is_err());
        }
        assert!(Request::Markdown(&[]).admit().is_err());
        let admitted = Request::Markdown(&[
            OsStr::new("#review.md"),
            OsStr::new("docs/todo/open/a.mdc"),
        ])
        .admit();
        assert!(admitted.is_ok_and(
            |args| args.args() == [":#review.md", ":docs/todo/open/a.mdc"]
        ));
    }

    #[test]
    fn fixture_git_admits_only_literal_local_names()
    -> Result<(), InvalidRequest> {
        for name in ["../escape", "/root", ".git/config", "a/.GIT/b", ""] {
            if (FixtureGit::Stage { name, force: false }).admit().is_ok() {
                return Err(InvalidRequest);
            }
        }
        let args = FixtureGit::Stage {
            name: "-file",
            force: true,
        }
        .admit()?;
        if args.args()
            == ["--literal-pathspecs", "add", "--force", "--", "-file"]
        {
            Ok(())
        } else {
            Err(InvalidRequest)
        }
    }
}
