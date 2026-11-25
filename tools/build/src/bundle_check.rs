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
//   - Read-only verification of a canonical asset bundle.
// - Must-Not:
//   - Publish, modify or interpret original media.
// - Allows:
//   - Inputs: Selected profile, bundle parent and pinned digest.
//   - Outputs: Verified metadata or a diagnostic.
//   - Side effects: Bounded reads through the platform IR.
// - Split-When:
//   - Verification requires an independent admission contract.
// - Merge-When:
//   - Another portable verifier owns the same operation.
// - Summary:
//   - Independent bundle verification without publication capabilities.
// - Description:
//   - Reads and verifies all admitted canonical payloads.
// - Usage:
//   - Supply an explicit selection and pinned manifest digest.
// - Defaults:
//   - Reject missing or invalid bundles.

use std::path::Path;

use games_assets::ContentHash;
use games_importer::selection::AssetSelection;
use games_platform::ir::storage::bundle::CanonicalBundle;

/// Independently verify a published asset bundle.
///
/// # Errors
/// Rejects unexpected identities, invalid assets or output failures.
pub fn check(
    selection: AssetSelection,
    parent: &Path,
    digest: ContentHash,
) -> Result<(), String> {
    let limits = selection.limits();
    let bundle = CanonicalBundle::open(parent, digest, limits)
        .map_err(|error| format!("bundle: {error:?}"))?;
    let manifest = bundle.manifest();
    selection
        .validate_manifest(manifest)
        .map_err(|error| format!("profile manifest: {error:?}"))?;
    for record in manifest.records() {
        drop(
            bundle
                .read_payload(&record.key)
                .map_err(|error| format!("{}: {error:?}", record.key))?,
        );
    }
    super::report_manifest(manifest, bundle.digest())
}
