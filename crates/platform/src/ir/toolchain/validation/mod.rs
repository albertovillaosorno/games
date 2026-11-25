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

use std::ffi::OsStr;

use super::ToolchainError;
use crate::ir::security::toolchain::validation::Arguments;
pub use crate::ir::security::toolchain::validation::{GitRequest, Request};

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

pub(crate) trait ValidationBackend {
    fn execute_validation(
        root: &OsStr,
        args: &Arguments,
        command: String,
    ) -> Result<Report, ToolchainError>;
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
    let command = format!(
        "{} {}",
        args.program(),
        args.args()
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    );
    Backend::execute_validation(root, &args, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recording;

    impl ValidationBackend for Recording {
        fn execute_validation(
            root: &OsStr,
            args: &Arguments,
            command: String,
        ) -> Result<Report, ToolchainError> {
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
            Ok(Report {
                command,
                diagnostic: String::from("fixture failed"),
                exit_code: Some(1),
                stderr: b"diagnostic".to_vec(),
                stdout: b"ordinary".to_vec(),
                success: false,
            })
        }
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
        {
            return Err("completion changed".into());
        }
        Ok(())
    }
}
