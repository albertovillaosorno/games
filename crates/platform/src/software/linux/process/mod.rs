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
//   - Native process argument observation and status output.
// - Must-Not:
//   - Parse title or contributor options or select simulation policy.
// - Allows:
//   - Inputs: Explicit observation and formatted status requests.
//   - Outputs: Owned arguments and typed output failures.
//   - Side effects: Process argument reads and standard output writes.
// - Split-When:
//   - Native process capabilities need independent responsibility.
// - Merge-When:
//   - Another adapter owns the same native process observations.
// - Summary:
//   - Linux process-facing adapter for shared runtime and tools.
// - Description:
//   - Retains native argument encoding and writes locked status lines.
// - Usage:
//   - Select the adapter through the portable process contract.
// - Defaults:
//   - Output failures propagate without panic.

//! Native process argument and status adapter.

use std::env;
use std::ffi::OsString;
use std::fmt::Arguments;
use std::io::{Result as IoResult, Write as _, stdout};

use crate::ir::process::ProcessPort;

impl ProcessPort for () {
    fn arguments() -> IoResult<Vec<OsString>> {
        Ok(env::args_os().skip(1).collect())
    }

    fn write_status(message: Arguments<'_>) -> IoResult<()> {
        writeln!(stdout().lock(), "{message}")
    }
}
