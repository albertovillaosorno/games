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
//   - Fixed test-only contributor process dispatch.
// - Must-Not:
//   - Expose a generic command runner or mutate a production repository.
// - Allows:
//   - Inputs: Private fixture roots and admitted Git or hook requests.
//   - Outputs: Explicit process completion or typed launch failures.
//   - Side effects: Native subprocesses inside the isolated test root.
// - Split-When:
//   - A distinct test process requires independent ownership.
// - Merge-When:
//   - Another test port covers the same operation.
// - Summary:
//   - Verify real Git and authored hook integration without host handles.
// - Description:
//   - Bind each operation to an isolated fixture or a fixed authored hook.
// - Usage:
//   - Select a request and retain the private fixture root.
// - Defaults:
//   - Fail closed on unavailable native test capabilities.

use std::io::Error;

use crate::ir::security::toolchain::validation::Arguments;
pub use crate::ir::security::toolchain::validation::{
    FixtureDelegate, FixtureGit, FixtureHook,
};
use crate::ir::storage::fixtures::TestRoot;
use crate::ir::storage::{PathKind, join_path, path_kind};
use crate::ir::toolchain::ToolchainError;

pub(crate) trait FixtureBackend {
    fn git(root: &TestRoot, args: &Arguments) -> Result<(), ToolchainError>;
    fn hook_syntax(hook: FixtureHook) -> Result<bool, ToolchainError>;
    fn run_hook(
        root: &TestRoot,
        hook: FixtureHook,
        failure: FixtureDelegate,
    ) -> Result<bool, ToolchainError>;
}

/// Execute a fixed Git fixture operation inside its owned temporary root.
/// # Errors
/// Rejects unsafe arguments and failed process completion.
pub fn git(
    root: &TestRoot,
    request: FixtureGit<'_>,
) -> Result<(), ToolchainError> {
    let args = request
        .admit()
        .map_err(|_error| ToolchainError::InvalidValidationRequest)?;
    let git_path = join_path(root.location(), ".git").map_err(fixture_io)?;
    let expected = match request {
        FixtureGit::Initialize => PathKind::Missing,
        FixtureGit::Stage { .. } => PathKind::Directory,
    };
    if path_kind(&git_path).map_err(fixture_io)? != expected {
        return Err(ToolchainError::InvalidValidationRequest);
    }
    <() as FixtureBackend>::git(root, &args)
}

/// Verify shell syntax of one fixed authored hook.
/// # Errors
/// Reports unavailable capabilities and process launch failures.
pub fn hook_syntax(hook: FixtureHook) -> Result<bool, ToolchainError> {
    <() as FixtureBackend>::hook_syntax(hook)
}

/// Exercise one authored hook with private fixture delegates.
/// # Errors
/// Reports unavailable capabilities and process launch failures.
pub fn run_hook(
    root: &TestRoot,
    hook: FixtureHook,
    failure: FixtureDelegate,
) -> Result<bool, ToolchainError> {
    <() as FixtureBackend>::run_hook(root, hook, failure)
}

const fn fixture_io(source: Error) -> ToolchainError {
    ToolchainError::Io {
        operation: "observe Git fixture",
        source,
    }
}
