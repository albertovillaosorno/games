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
//   - Native process arguments, output streams and scoped retry scheduling.
// - Must-Not:
//   - Parse title or contributor options or select simulation policy.
// - Allows:
//   - Inputs: Explicit observation and formatted status requests.
//   - Outputs: Owned arguments and typed output failures.
//   - Side effects: Process argument reads and standard output and error
//     writes.
// - Split-When:
//   - Native process capabilities need independent responsibility.
// - Merge-When:
//   - Another adapter owns the same native process observations.
// - Summary:
//   - Linux process-facing adapter for shared runtime and tools.
// - Description:
//   - Retains native argument encoding and writes locked output lines.
// - Usage:
//   - Select the adapter through the portable process contract.
// - Defaults:
//   - Output failures propagate without panic.

//! Native process argument and status adapter.

use std::ffi::OsString;
use std::fmt::Arguments;
#[cfg(any(test, feature = "test-fixtures"))]
use std::io::Error;
use std::io::{Result as IoResult, Write as _, stderr, stdout};
use std::time::Duration;
use std::{env, thread};

use crate::ir::process::ProcessPort;
#[cfg(any(test, feature = "test-fixtures"))]
use crate::ir::process::fixtures::{FixturePort, NativeSmoke};

impl ProcessPort for () {
    fn arguments() -> IoResult<Vec<OsString>> {
        Ok(env::args_os().skip(1).collect())
    }

    fn wait_for_retry(delay: Duration) -> IoResult<()> {
        thread::sleep(delay);
        Ok(())
    }

    fn write_diagnostic(message: Arguments<'_>) -> IoResult<()> {
        writeln!(stderr().lock(), "{message}")
    }

    fn write_status(message: Arguments<'_>) -> IoResult<()> {
        writeln!(stdout().lock(), "{message}")
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl FixturePort for () {
    fn smoke_requested(check: NativeSmoke) -> IoResult<bool> {
        Ok(env::var_os(check.key()).is_some())
    }

    fn with_worker<Output, Worker, Foreground>(
        worker: Worker,
        foreground: Foreground,
    ) -> IoResult<Output>
    where
        Output: Send,
        Worker: FnOnce() -> IoResult<Output> + Send,
        Foreground: FnOnce() -> IoResult<()>,
    {
        thread::scope(|scope| {
            let handle = thread::Builder::new().spawn_scoped(scope, worker)?;
            let result = foreground();
            let joined = handle.join().map_err(|_panic| {
                Error::other("integration fixture worker panicked")
            });
            result?;
            joined?
        })
    }

    fn yield_execution() -> IoResult<()> {
        thread::yield_now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::ir::process::fixtures::{with_worker, yield_execution};

    #[test]
    fn fixture_joins_on_success_and_foreground_error() -> super::IoResult<()> {
        let joined = with_worker(|| Ok(7u8), yield_execution)?;
        if joined != 7 {
            return Err(ErrorKind::InvalidData.into());
        }
        let completed = AtomicBool::new(false);
        let rejected = with_worker(
            || {
                completed.store(true, Ordering::Release);
                Ok(())
            },
            || Err(ErrorKind::InvalidInput.into()),
        );
        if !completed.load(Ordering::Acquire)
            || !rejected
                .is_err_and(|error| error.kind() == ErrorKind::InvalidInput)
        {
            return Err(ErrorKind::InvalidData.into());
        }
        Ok(())
    }
}
