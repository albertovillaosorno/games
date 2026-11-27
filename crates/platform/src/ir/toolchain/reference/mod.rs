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
//   - Portable reference download dispatch and published path observations.
// - Must-Not:
//   - Contact upstreams, access storage or execute retrieved material.
// - Allows:
//   - Inputs: Admitted reference selections.
//   - Outputs: Published reference paths or typed failures.
//   - Side effects: Explicit dispatch only.
// - Split-When:
//   - Reference lifecycle needs independent service selection.
// - Merge-When:
//   - An existing toolchain contract owns the same request and outcome.
// - Summary:
//   - Download known references through the selected contributor backend.
// - Description:
//   - Require shared catalogue admission before reference acquisition.
// - Usage:
//   - Select a reference tool, then explicitly request its download.
// - Defaults:
//   - No implicit requests or unsupported-host success.

//! Portable reference download requests and receipts.

/// Bounded research archive retention.
pub mod retention;

use std::ffi::OsString;

use super::{ToolchainError, ToolchainFactory};
pub use crate::ir::security::toolchain::reference::ReferenceTool;

/// Acquire the selected reference and return its local published path.
///
/// # Errors
/// Reports unsupported capability, invalid revision or native failures.
pub fn download(tool: ReferenceTool) -> Result<OsString, ToolchainError> {
    <() as ToolchainFactory>::download_reference(tool)
}
