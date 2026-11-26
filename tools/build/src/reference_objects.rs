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
//   - Read-only comparison of private CPU and device object snapshots.
// - Must-Not:
//   - Interpret source registers, simulate transfers or publish assets.
// - Allows:
//   - Inputs: Machine profile and bounded local object RAM files.
//   - Outputs: Exact ordered transfer counts.
//   - Side effects: Bounded reads and summary output.
// - Split-When:
//   - Another observation format requires independent comparison.
// - Merge-When:
//   - Shared reference admission supports the same observation format.
// - Summary:
//   - Measure whether observed device objects match CPU admission.
// - Description:
//   - Report mismatches without assuming a shared transfer instant.
// - Usage:
//   - Supply both object buffers captured at the same original frame.
// - Defaults:
//   - No tolerance, no private output and exact input extents.

//! Host-side original object buffer comparison.

use std::ffi::OsStr;
use std::num::NonZeroU64;

use games_platform::ir::importer::reference::{Machine, RAM_BYTES};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain;

/// Compare observed CPU and device object buffers.
/// # Errors
/// Rejects unsafe, incomplete or oversized local files.
pub fn check(
    machine: Machine,
    cpu: &OsStr,
    device: &OsStr,
) -> Result<(), String> {
    let limit = u64::try_from(RAM_BYTES)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| String::from("invalid object budget"))?;
    let cpu_ram = snapshot(cpu, limit)
        .map_err(|error| format!("CPU object RAM: {error:?}"))?;
    let device_ram = snapshot(device, limit)
        .map_err(|error| format!("device object RAM: {error:?}"))?;
    let report = machine
        .compare_object_transfer(&cpu_ram, &device_ram)
        .map_err(|error| format!("object transfer: {error:?}"))?;
    toolchain::write_line(&format!(
        "Selected {}, active {}, exact records {}, uncleared headers {}",
        report.selected, report.active, report.matched, report.uncleared,
    ))
    .map_err(|error| error.to_string())
}
