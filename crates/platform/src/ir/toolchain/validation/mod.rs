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
//   - Portable contributor validation dispatch and process observations.
// - Must-Not:
//   - Launch native processes or choose arbitrary executables.
// - Allows:
//   - Inputs: Explicit workspace and fixed admitted validation requests.
//   - Outputs: Complete captured streams and process completion outcomes.
//   - Side effects: Explicit admitted backend dispatch only.
// - Split-When:
//   - An independent repository command needs different lifetime rules.
// - Merge-When:
//   - Another shared port owns the same command and observation semantics.
// - Summary:
//   - Execute only the shared contributor validation catalogue.
// - Description:
//   - Keep native process handles and argument construction out of tools.
// - Usage:
//   - Admit a fixed request and interpret the returned diagnostic streams.
// - Defaults:
//   - Reject invalid inputs and unsupported host capabilities.

/// Explicitly enabled contributor subprocess fixtures.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixtures;
/// Exclusive validation cache and log state.
pub mod state;

use std::ffi::OsStr;

use super::ToolchainError;
use crate::ir::security::toolchain::validation::{
    Arguments, Completion, admit_head, reference_exists,
};
pub use crate::ir::security::toolchain::validation::{
    GitRequest, Report, Request,
};

pub(crate) trait ValidationBackend {
    fn execute_validation(
        root: &OsStr,
        args: &Arguments,
    ) -> Result<Completion, ToolchainError>;
}

/// A committed HEAD identifier, or an unborn symbolic branch.
pub type RepositoryHead = Option<Vec<u8>>;

/// Observe the repository HEAD, or an unborn symbolic branch.
///
/// # Errors
/// Reports failed Git observations rather than silently accepting them.
pub fn repository_head(root: &OsStr) -> Result<RepositoryHead, ToolchainError> {
    repository_head_with::<()>(root)
}

fn repository_head_with<Backend>(
    root: &OsStr,
) -> Result<RepositoryHead, ToolchainError>
where
    Backend: ValidationBackend,
{
    let head = run_with::<Backend>(root, Request::Git(GitRequest::Head))?;
    if head.success {
        admit_head(&head.stdout).map_err(|_error| {
            head_error("invalid committed HEAD observation".into())
        })?;
        return Ok(Some(head.stdout));
    }
    if head.exit_code == Some(1i32) && head.stdout.is_empty() {
        return unborn_head::<Backend>(root);
    }
    Err(head_error(head.diagnostic))
}

fn unborn_head<Backend>(root: &OsStr) -> Result<RepositoryHead, ToolchainError>
where
    Backend: ValidationBackend,
{
    let symbolic =
        run_with::<Backend>(root, Request::Git(GitRequest::SymbolicHead))?;
    if !symbolic.success {
        return Err(head_error(symbolic.diagnostic));
    }
    let lookup = reference_exists(&symbolic.stdout).map_err(|_error| {
        head_error("invalid symbolic HEAD observation".into())
    })?;
    let existence = run_admitted::<Backend>(root, &lookup)?;
    // Git reports 2 only when the exact reference does not exist.
    if existence.exit_code == Some(2i32) && existence.stdout.is_empty() {
        return Ok(None);
    }
    Err(head_error(
        "unresolved HEAD with an existing or invalid reference".into(),
    ))
}

const fn head_error(diagnostic: String) -> ToolchainError {
    ToolchainError::Failed {
        operation: "resolve repository HEAD",
        diagnostic,
    }
}

/// Execute one admitted workspace check and capture both output streams.
///
/// Unsuccessful process completion is returned in the report; invalid
/// arguments or inability to start a process return a typed failure.
/// # Errors
/// Rejects invalid requests, unavailable capabilities and launch errors.
pub fn run(
    root: &OsStr,
    request: Request<'_>,
) -> Result<Report, ToolchainError> {
    run_with::<()>(root, request)
}

fn run_with<Backend>(
    root: &OsStr,
    request: Request<'_>,
) -> Result<Report, ToolchainError>
where
    Backend: ValidationBackend,
{
    let args = request
        .admit()
        .map_err(|_error| ToolchainError::InvalidValidationRequest)?;
    run_admitted::<Backend>(root, &args)
}

fn run_admitted<Backend>(
    root: &OsStr,
    args: &Arguments,
) -> Result<Report, ToolchainError>
where
    Backend: ValidationBackend,
{
    let completion = Backend::execute_validation(root, args)?;
    Ok(Report::from_completion(args, completion))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD_ID: &[u8] = b"0123456789abcdef0123456789abcdef01234567\n";

    struct HeadRecording;

    impl ValidationBackend for HeadRecording {
        fn execute_validation(
            root: &OsStr,
            args: &Arguments,
        ) -> Result<Completion, ToolchainError> {
            if args.program() != "git" {
                return Err(ToolchainError::InvalidValidationRequest);
            }
            let request = if args.args()
                == ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]
            {
                "commit"
            } else if args.args()
                == ["symbolic-ref", "--quiet", "--no-recurse", "HEAD"]
            {
                "symbolic"
            } else if args.args() == ["show-ref", "--exists", "refs/heads/main"]
            {
                "exists"
            } else {
                return Err(ToolchainError::InvalidValidationRequest);
            };
            let (exit_code, bytes) = match (request, root.to_str()) {
                ("commit", Some("committed")) => (Some(0i32), HEAD_ID),
                ("commit", Some("malformed-commit")) => {
                    (Some(0i32), b"id".as_slice())
                },
                ("commit", Some("signaled")) => (None, b"".as_slice()),
                (
                    "commit",
                    Some(
                        "unborn" | "dangling" | "lookup-error"
                        | "malformed-symbolic" | "broken",
                    ),
                )
                | ("exists", Some("lookup-error")) => {
                    (Some(1i32), b"".as_slice())
                },
                ("symbolic", Some("unborn" | "dangling" | "lookup-error")) => {
                    (Some(0i32), b"refs/heads/main\n".as_slice())
                },
                ("symbolic", Some("malformed-symbolic")) => {
                    (Some(0i32), b"--option\n".as_slice())
                },
                ("exists", Some("unborn")) => (Some(2i32), b"".as_slice()),
                ("exists", Some("dangling")) => (Some(0i32), b"".as_slice()),
                _ => (Some(128i32), b"".as_slice()),
            };
            let stdout = bytes.to_vec();
            Ok(Completion {
                diagnostic: String::from("fixture"),
                exit_code,
                stderr: Vec::new(),
                stdout,
            })
        }
    }

    struct Recording;

    impl ValidationBackend for Recording {
        fn execute_validation(
            root: &OsStr,
            args: &Arguments,
        ) -> Result<Completion, ToolchainError> {
            if root != OsStr::new("workspace")
                || args.program() != "cargo"
                || args.args()
                    != [
                        "metadata",
                        "--no-deps",
                        "--locked",
                        "--format-version",
                        "1",
                    ]
            {
                return Err(ToolchainError::MissingArtifact);
            }
            Ok(Completion {
                diagnostic: String::from("fixture failed"),
                exit_code: Some(1i32),
                stderr: b"diagnostic".to_vec(),
                stdout: b"ordinary".to_vec(),
            })
        }
    }

    #[test]
    fn head_observation_distinguishes_commit_unborn_and_failure()
    -> Result<(), String> {
        let observe =
            |root| repository_head_with::<HeadRecording>(OsStr::new(root));
        if observe("committed").map_err(|error| error.to_string())?
            != Some(HEAD_ID.to_vec())
            || observe("unborn")
                .map_err(|error| error.to_string())?
                .is_some()
        {
            return Err("repository HEAD outcome changed".into());
        }
        for invalid in [
            "broken",
            "signaled",
            "dangling",
            "lookup-error",
            "malformed-commit",
            "malformed-symbolic",
        ] {
            if observe(invalid).is_ok() {
                return Err(format!("invalid HEAD admitted: {invalid}"));
            }
        }
        Ok(())
    }

    #[test]
    fn existing_unresolved_branch_is_not_an_unborn_repository()
    -> Result<(), String> {
        use fixtures::{FixtureGit, git};

        use crate::ir::storage::fixtures::{AVAILABLE, TestRoot};

        if !AVAILABLE {
            return Err("native fixture capability unavailable".into());
        }
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        git(&root, FixtureGit::Initialize)
            .map_err(|error| error.to_string())?;
        root.write_file(".git/HEAD", b"ref: refs/heads/fixture\n")
            .map_err(|error| error.to_string())?;
        if repository_head(root.location())
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("new repository was not unborn".into());
        }
        root.write_file(".git/refs/heads/fixture", HEAD_ID)
            .map_err(|error| error.to_string())?;
        if repository_head(root.location()).is_ok() {
            return Err(
                "unresolved existing branch was treated as unborn".into()
            );
        }
        Ok(())
    }

    #[test]
    fn rejection_precedes_dispatch_and_completion_is_preserved()
    -> Result<(), String> {
        let root = OsStr::new("workspace");
        if !matches!(
            run_with::<Recording>(root, Request::ClippyPackage("--option")),
            Err(ToolchainError::InvalidValidationRequest)
        ) {
            return Err("invalid request reached backend".into());
        }
        let result = run_with::<Recording>(root, Request::WorkspaceMetadata)
            .map_err(|error| error.to_string())?;
        if result.success
            || result.stdout != b"ordinary"
            || result.stderr != b"diagnostic"
            || result.diagnostic != "fixture failed"
            || result.command
                != "cargo metadata --no-deps --locked --format-version 1"
            || result.exit_code != Some(1i32)
        {
            return Err("completion changed".into());
        }
        Ok(())
    }
}
