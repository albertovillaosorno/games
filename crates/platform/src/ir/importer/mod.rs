// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Public source-independent asset import capability.
// - Must-Not:
//   - Open host files, execute originals or duplicate format decoders.
// - Allows:
//   - Inputs: Explicit source format, profile, family and bounded bytes.
//   - Outputs: Canonical manifest, staged payloads or typed failure.
//   - Side effects: Only the caller-owned private staging callback.
// - Split-When:
//   - A new operation needs independent admission guarantees.
// - Merge-When:
//   - Another entry point implements the same import transaction.
// - Summary:
//   - A single import contract for all supported source formats.
// - Description:
//   - Dispatches to existing host-independent source format adapters.
// - Usage:
//   - Select a profile, admit a source and request import.
// - Defaults:
//   - Reject unsupported profiles, families and invalid source data.

//! Shared asset import capability.

mod import;
/// Read-only format observations for reference research.
pub mod reference;

pub use games_assets::{AssetManifest, ContentHash, ManifestLimits};
pub use games_importer::{ImportError, SourceSnapshot};
pub use import::{
    AssetFamily, ImportProfile, SourceFormat, Verification, VerifiedSource,
    import, verify,
};
