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
//   - Native storage adapter composition.
// - Must-Not:
//   - Own game simulation or source media interpretation.
// - Allows:
//   - Inputs: Shared engine configuration and host-provided resources.
//   - Outputs: Native platform capabilities.
//   - Side effects: Explicit host operating-system access.
// - Split-When:
//   - A platform adapter needs an independent responsibility.
// - Merge-When:
//   - Multiple adapters implement the same platform boundary.
// - Summary:
//   - Platform integration outside the transport-neutral engine.
// - Description:
//   - Exposes native storage adapters behind target-specific module boundaries.
// - Usage:
//   - Compose adapters with the engine in a host application.
// - Defaults:
//   - No operating-system resources opened implicitly.

//! Native storage adapters for verified local assets.

/// Read-only native canonical asset bundle access.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod bundle;
/// Bounded native regular-file snapshots.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod file;
/// Atomic persistence for portable display and input preferences.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod preferences;

/// Integrity-checked local save slots.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod save;

/// Private, bounded native archive staging.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod staging;
