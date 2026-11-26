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
//   - Native execution of isolated Git and authored hook fixtures.
// - Must-Not:
//   - Select arbitrary programs or modify the authored repository.
// - Allows:
//   - Inputs: Private fixture root and admitted fixed test requests.
//   - Outputs: Process completion or typed launch failures.
//   - Side effects: Explicit isolated fixture processes.
// - Split-When:
//   - Another test process needs a separate lifetime contract.
// - Merge-When:
//   - Another adapter executes equivalent isolated fixtures.
// - Summary:
//   - Exercise real native integration without leaking host process handles.
// - Description:
//   - Exclude user Git configuration and use private hook delegates.
// - Usage:
//   - Invoke only through the test-only validation IR.
// - Defaults:
//   - No inherited Git repository or hook delegate state.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ir::security::toolchain::validation::Arguments;
use crate::ir::storage::fixtures::TestRoot;
use crate::ir::toolchain::validation::fixtures::{
    FixtureBackend, FixtureDelegate, FixtureHook,
};
use crate::ir::toolchain::{ToolchainError, workspace_root};

impl FixtureBackend for () {
    fn git(root: &TestRoot, args: &Arguments) -> Result<(), ToolchainError> {
        let location = Path::new(root.location());
        let output = Command::new(args.program())
            .current_dir(location)
            .env_clear()
            .env("GIT_DIR", location.join(".git"))
            .env("GIT_WORK_TREE", location)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args.args())
            .output()
            .map_err(|source| ToolchainError::Io {
                operation: "Git fixture",
                source,
            })?;
        if !output.status.success() {
            return Err(ToolchainError::Failed {
                operation: "Git fixture",
                diagnostic: String::from_utf8_lossy(&output.stderr)
                    .into_owned(),
            });
        }
        Ok(())
    }

    fn hook_syntax(hook: FixtureHook) -> Result<bool, ToolchainError> {
        Command::new("/bin/sh")
            .arg("-n")
            .arg(hook_path(hook)?)
            .status()
            .map(|status| status.success())
            .map_err(|source| ToolchainError::Io {
                operation: "hook syntax fixture",
                source,
            })
    }

    fn run_hook(
        root: &TestRoot,
        hook: FixtureHook,
        failure: FixtureDelegate,
    ) -> Result<bool, ToolchainError> {
        let location = Path::new(root.location());
        Command::new("/bin/sh")
            .arg(hook_path(hook)?)
            .arg(location.join("COMMIT_MSG"))
            .current_dir(location)
            .env_clear()
            .env("PATH", location)
            .env("HOOK_FIXTURE", location)
            .env("HOOK_FAIL", failure.token())
            .status()
            .map(|status| status.success())
            .map_err(|source| ToolchainError::Io {
                operation: "hook delegate fixture",
                source,
            })
    }
}

fn hook_path(hook: FixtureHook) -> Result<PathBuf, ToolchainError> {
    Ok(PathBuf::from(workspace_root()?)
        .join(".githooks")
        .join(hook.filename()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::toolchain::validation::fixtures::{FixtureGit, git};

    #[test]
    fn git_fixture_rejects_linked_repository() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let other = TestRoot::new().map_err(|error| error.to_string())?;
        root.link(other.location(), ".git")
            .map_err(|error| error.to_string())?;
        for request in [FixtureGit::Initialize, FixtureGit::Stage {
            name: "ordinary",
            force: false,
        }] {
            if !matches!(
                git(&root, request),
                Err(ToolchainError::InvalidValidationRequest)
            ) {
                return Err("linked repository admitted".into());
            }
        }
        if other.entry_count().map_err(|error| error.to_string())? != 0 {
            return Err("fixture modified linked directory".into());
        }
        Ok(())
    }
}
