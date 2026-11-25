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
//   - Contributor environment observation and validation run identity.
// - Must-Not:
//   - Read native environment, clocks or process identifiers directly.
// - Allows:
//   - Inputs: Explicit contributor observation requests.
//   - Outputs: Admitted environment values or a validation log identifier.
//   - Side effects: Only dispatch to the selected private adapter.
// - Split-When:
//   - Another operation needs different observation admission.
// - Merge-When:
//   - A shared port owns equivalent contributor observations.
// - Summary:
//   - Observe only validation-specific host state.
// - Description:
//   - Keep environment and process details out of the validation runner.
// - Usage:
//   - Request a snapshot for fingerprinting or a local run log identifier.
// - Defaults:
//   - Explicit unsupported errors; never disclose raw values in diagnostics.

use super::ToolchainError;
pub use crate::ir::security::toolchain::environment::{
    Environment, FINGERPRINT_KEYS, FRESH_CHECK_FLAGS,
};

pub(crate) trait EnvironmentBackend {
    fn environment() -> Result<Environment, ToolchainError>;
    fn validation_log_name() -> Result<String, ToolchainError>;
}

/// Capture the admitted contributor validation environment.
///
/// # Errors
/// Reports unavailable contributor observation capability.
pub fn capture() -> Result<Environment, ToolchainError> {
    <() as EnvironmentBackend>::environment()
}

/// Observe the current validation run's local log filename.
///
/// # Errors
/// Reports unavailable capability or an invalid clock observation.
pub fn validation_log_name() -> Result<String, ToolchainError> {
    <() as EnvironmentBackend>::validation_log_name()
}
