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
//   - Native environment and validation run identity observations.
// - Must-Not:
//   - Return arbitrary environment variables or choose cache policy.
// - Allows:
//   - Inputs: Fixed contributor observation requests.
//   - Outputs: Filtered environment snapshot and log filename.
//   - Side effects: Reads process environment, clock and identity.
// - Split-When:
//   - A different capability needs separate native observation policy.
// - Merge-When:
//   - A common adapter provides equivalent contributor observations.
// - Summary:
//   - Collect admitted validation inputs in the selected native backend.
// - Description:
//   - Filter observations through the shared portable admission contract.
// - Usage:
//   - Dispatch through the contributor environment IR.
// - Defaults:
//   - Retain exact native values, report clock errors explicitly.

use std::path::PathBuf;
use std::time::SystemTime;
use std::{env, process};

use crate::ir::toolchain::ToolchainError;
use crate::ir::toolchain::environment::{Environment, EnvironmentBackend};

impl EnvironmentBackend for () {
    fn environment() -> Result<Environment, ToolchainError> {
        Ok(Environment::new(
            env::vars_os(),
            env::home_dir().map(PathBuf::into_os_string),
        ))
    }

    fn validation_log_name() -> Result<String, ToolchainError> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|error| ToolchainError::Failed {
                operation: "validation clock",
                diagnostic: error.to_string(),
            })?
            .as_nanos();
        Ok(format!("{stamp}-{}.log", process::id()))
    }
}
