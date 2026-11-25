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
//   - Native execution of admitted contributor validation commands.
// - Must-Not:
//   - Construct new requests, invoke a shell or interpret game assets.
// - Allows:
//   - Inputs: Admitted program, arguments and explicit workspace.
//   - Outputs: Captured output and completion observations.
//   - Side effects: Explicit contributor tool subprocess execution.
// - Split-When:
//   - A different tool requires independent process lifecycle handling.
// - Merge-When:
//   - A common toolchain executor covers the same process contract.
// - Summary:
//   - Run admitted validation commands without shell interpretation.
// - Description:
//   - Keep native process types private and return complete observations.
// - Usage:
//   - Dispatch only via the portable validation capability.
// - Defaults:
//   - Propagate launch errors and preserve unsuccessful completion.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use crate::ir::security::toolchain::validation::Arguments;
use crate::ir::toolchain::ToolchainError;
use crate::ir::toolchain::validation::{Report, ValidationBackend};

impl ValidationBackend for () {
    fn execute_validation(
        root: &OsStr,
        args: &Arguments,
        command: String,
    ) -> Result<Report, ToolchainError> {
        let program = args.program();
        let output = command_for(root, args).output().map_err(|source| {
            ToolchainError::Io {
                operation: program,
                source,
            }
        })?;
        Ok(Report {
            command,
            diagnostic: output.status.to_string(),
            exit_code: output.status.code(),
            stderr: output.stderr,
            stdout: output.stdout,
            success: output.status.success(),
        })
    }
}

fn command_for(root: &OsStr, args: &Arguments) -> Command {
    let mut command = Command::new(args.program());
    let _configured = command.current_dir(Path::new(root)).args(args.args());
    command
}

#[cfg(test)]
mod tests {
    use super::command_for;
    use crate::ir::security::toolchain::validation::Request;
    use crate::ir::storage::fixtures::TestRoot;

    #[test]
    fn discovery_ignores_personal_ripgrep_config() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        root.write_file(".gitignore", b"local.conf\n")
            .map_err(|error| error.to_string())?;
        root.write_file("source.rs", b"// authored")
            .map_err(|error| error.to_string())?;
        root.write_file("local.conf", b"--glob\n!*.rs\n")
            .map_err(|error| error.to_string())?;
        let args = Request::AuthoredFiles
            .admit()
            .map_err(|error| format!("{error:?}"))?;
        let output = command_for(root.path().as_os_str(), &args)
            .env("RIPGREP_CONFIG_PATH", root.path().join("local.conf"))
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success()
            || !output
                .stdout
                .split(|byte| *byte == 0)
                .any(|path| path == b"source.rs")
        {
            return Err("personal ripgrep config hid authored source".into());
        }
        Ok(())
    }
}
