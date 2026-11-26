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
//   - Fixed contributor validation commands and their argument admission.
// - Must-Not:
//   - Access a filesystem, execute a process or interpret tool output.
// - Allows:
//   - Inputs: Authored package identifiers, targets and Markdown names.
//   - Outputs: Admitted executable and argument selections.
//   - Side effects: None.
// - Split-When:
//   - A different validation protocol needs independent argument policy.
// - Merge-When:
//   - A shared request contract provides the same admission semantics.
// - Summary:
//   - Admit only the fixed workspace validation operations.
// - Description:
//   - Prevent options and arbitrary executable selection in dynamic inputs.
// - Usage:
//   - Select a validation request before native process dispatch.
// - Defaults:
//   - Locked Cargo operations; reject unsafe or empty dynamic inputs.

use core::str::from_utf8;
use std::ffi::{OsStr, OsString};

use games_assets::archive::validate_path;

use super::identifier;
use crate::ir::toolchain::Target;

/// Fixed contributor check with explicitly admitted dynamic inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request<'input> {
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
            Self::BuildGraph
            | Self::ClippyPackage(_)
            | Self::ClippyWorkspace
            | Self::CompileWorkspace { .. }
            | Self::Format
            | Self::History
            | Self::PackageEngine
            | Self::Portable { .. }
            | Self::RuntimeGraph(_)
            | Self::TestWorkspace { .. }
            | Self::WorkspaceMetadata => ("cargo", self.cargo()?),
        };
        Ok(Arguments {
            args: args.into_iter().map(OsString::from).collect(),
            program,
        })
    }

    fn cargo(self) -> Result<Vec<&'input str>, InvalidRequest> {
        match self {
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
            Self::AuthoredFiles | Self::Git(_) | Self::Markdown(_) => {
                Err(InvalidRequest)
            },
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
            Self::WorkspaceMetadata => Ok(vec![
                "metadata",
                "--no-deps",
                "--locked",
                "--format-version",
                "1",
            ]),
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

/// Admit complete NUL-delimited relative paths from an inventory tool.
///
/// Empty inventories are valid; malformed framing and unsafe names are not.
/// # Errors
/// Rejects incomplete records, non-UTF-8 names or unsafe path components.
pub fn admit_paths(bytes: &[u8]) -> Result<Vec<&str>, InvalidRequest> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let complete = bytes.strip_suffix(&[0]).ok_or(InvalidRequest)?;
    complete
        .split(|byte| *byte == 0)
        .map(|part| {
            let name = from_utf8(part).map_err(|_error| InvalidRequest)?;
            validate_path(name).map_err(|_error| InvalidRequest)?;
            Ok(name)
        })
        .collect()
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
        GitRequest::Head => vec!["rev-parse", "--verify", "--quiet", "HEAD"],
        GitRequest::History => vec!["log", "-z", "--format=%H%n%B", "HEAD"],
        GitRequest::Inventory => vec![
            "ls-files",
            "--cached",
            "--others",
            "--exclude-from=.gitignore",
            "-z",
        ],
        GitRequest::SymbolicHead => vec!["symbolic-ref", "--quiet", "HEAD"],
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
    fn package_inputs_cannot_select_options_or_paths() {
        for invalid in ["", "--help", "a/b", "a b", "a,b"] {
            assert!(Request::ClippyPackage(invalid).admit().is_err());
            assert!(Request::RuntimeGraph(invalid).admit().is_err());
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
