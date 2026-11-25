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
//   - Shared platform security admission policies.
// - Must-Not:
//   - Open host resources or perform native security operations.
// - Allows:
//   - Inputs: Typed resource and service requests.
//   - Outputs: Admitted values or explicit policy failures.
//   - Side effects: None.
// - Split-When:
//   - An independent service requires different admission guarantees.
// - Merge-When:
//   - Another shared policy enforces the same admission contract.
// - Summary:
//   - Backend-independent security checks before host dispatch.
// - Description:
//   - Native backends retain additional platform-specific safety checks.
// - Usage:
//   - Validate untrusted requests before acquiring host resources.
// - Defaults:
//   - No implicit permissions or resource access.

//! Shared security admission policies.

/// Bounded audio playback admission.
pub mod audio;
/// Bounded file snapshot admission.
pub mod file;
/// Bounded frame layout and rendering errors.
pub mod frame;
/// Portable feedback capability and permission admission.
pub mod haptics;
/// Local-network transport admission.
pub mod lan;
/// Versioned portable preference admission and encoding.
pub mod preferences;
/// Bounded atlas and ordered scene admission.
pub mod scene;
/// Fixed authored toolchain argument admission.
pub mod toolchain;
