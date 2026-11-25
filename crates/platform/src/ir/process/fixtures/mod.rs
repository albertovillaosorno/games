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
//   - Portable integration test opt-in and scoped concurrent execution.
// - Must-Not:
//   - Schedule gameplay or expose native thread or environment handles.
// - Allows:
//   - Inputs: Explicit smoke kind and scoped fallible test operations.
//   - Outputs: Opt-in status and joined operation results.
//   - Side effects: Delegated test-only host operations.
// - Split-When:
//   - Additional test facilities need independent lifecycle rules.
// - Merge-When:
//   - Another fixture contract owns equivalent scoped execution.
// - Summary:
//   - Host integration test coordination without native dependencies.
// - Description:
//   - Join workers on failure and preserve test operation errors.
// - Usage:
//   - Explicitly opt in to native smoke and exercise concurrent endpoints.
// - Defaults:
//   - No test-only capability in ordinary runtime builds.

use std::io::Result as IoResult;

pub use crate::ir::security::toolchain::environment::NativeSmoke;

pub(crate) trait FixturePort {
    fn smoke_requested(check: NativeSmoke) -> IoResult<bool>;
    fn with_worker<Output, Worker, Foreground>(
        worker: Worker,
        foreground: Foreground,
    ) -> IoResult<Output>
    where
        Output: Send,
        Worker: FnOnce() -> IoResult<Output> + Send,
        Foreground: FnOnce() -> IoResult<()>;
    fn yield_execution() -> IoResult<()>;
}

/// Observe the presence of the requested native smoke opt-in.
///
/// An empty value still counts as present; no raw values are returned.
///
/// # Errors
/// Reports unavailable native smoke observation.
pub fn smoke_requested(check: NativeSmoke) -> IoResult<bool> {
    <() as FixturePort>::smoke_requested(check)
}

/// Run a worker alongside a foreground test operation, then join it.
///
/// Both operations must have finite termination conditions. A foreground
/// error takes precedence but the worker is still joined before returning.
///
/// # Errors
/// Reports unavailable execution, spawn, join or operation failures.
pub fn with_worker<Output, Worker, Foreground>(
    worker: Worker,
    foreground: Foreground,
) -> IoResult<Output>
where
    Output: Send,
    Worker: FnOnce() -> IoResult<Output> + Send,
    Foreground: FnOnce() -> IoResult<()>,
{
    <() as FixturePort>::with_worker(worker, foreground)
}

/// Cooperatively yield execution while polling an integration fixture.
///
/// # Errors
/// Reports unavailable native scheduling.
pub fn yield_execution() -> IoResult<()> {
    <() as FixturePort>::yield_execution()
}
