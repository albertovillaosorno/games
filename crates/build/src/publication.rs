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
//   - Composition of selected graphics conversion and native bundle access.
// - Must-Not:
//   - Execute originals, decode media layouts or bypass platform storage.
// - Allows:
//   - Inputs: Verified source, explicit budgets, trusted publication parent.
//   - Outputs: Published bundle receipt or verified payload summary.
//   - Side effects: Private staging, atomic publication and stdout summaries.
// - Split-When:
//   - Other title asset policies need separate host adapters.
// - Merge-When:
//   - A common host composition can express all title policies.
// - Summary:
//   - Publish and independently check selected canonical graphics.
// - Description:
//   - Connect the verified importer sink to the platform transaction.
// - Usage:
//   - Select an explicit output parent or expected manifest digest.
// - Defaults:
//   - Never overwrite and do not publish on conversion failure.

use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::path::Path;

use games_assets::archive::ArchiveBudget;
use games_assets::{ContentHash, ManifestLimits};
use games_importer::conversion::ConversionLimits;
use games_importer::{ImportError, VerifiedSource, simpsons};
use games_platform_storage::bundle::CanonicalBundle;
use games_platform_storage::staging::CanonicalStaging;

/// Independently verify a published graphics bundle.
///
/// # Errors
/// Rejects unexpected identities, invalid assets or output failures.
pub fn check(parent: &Path, digest: ContentHash) -> Result<(), String> {
    let limits = simpsons::graphics_limits();
    let bundle = CanonicalBundle::open(parent, digest, limits)
        .map_err(|error| format!("bundle: {error:?}"))?;
    let manifest = bundle.manifest();
    let identity = manifest.source();
    if identity.title != simpsons::TITLE
        || identity.revision != simpsons::REVISION
        || identity.digest
            != simpsons::SHA256.parse().map_err(|_error| {
                String::from("invalid selected source identity")
            })?
        || manifest.records().len() != limits.records.get()
    {
        return Err(String::from("unexpected graphics bundle identity"));
    }
    for record in manifest.records() {
        drop(
            bundle
                .read_payload(&record.key)
                .map_err(|error| format!("{}: {error:?}", record.key))?,
        );
    }
    super::report_manifest(manifest, limits)
}

/// Convert and publish a verified archive without replacing any output.
///
/// # Errors
/// Returns conversion, staging, publication or output failures.
pub fn publish(
    parent: &Path,
    source: &VerifiedSource<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let manifest = simpsons::graphics_limits();
    let mut stage = CanonicalStaging::new(parent, staging_budget(manifest)?)
        .map_err(|error| format!("stage: {error:?}"))?;
    let mut staging_error = None;
    let converted = simpsons::convert_graphics(
        source,
        ConversionLimits { archive, manifest },
        |digest, bytes| {
            stage.write_payload(digest, bytes).map_err(|error| {
                staging_error = Some(error);
                ImportError::StageFailed
            })
        },
    )
    .map_err(|error| {
        staging_error.map_or_else(
            || format!("graphics: {error:?}"),
            |failure| format!("stage: {failure:?}"),
        )
    })?;
    let staged = stage
        .finish(&converted, manifest)
        .map_err(|error| format!("complete: {error:?}"))?;
    let digest = staged.digest();
    let receipt = staged
        .publish()
        .map_err(|error| format!("publish: {error:?}"))?;
    if let Err(error) = receipt.durability {
        return Err(format!(
            "published {digest} at {}, but directory sync failed: {error:?}",
            receipt.path.display()
        ));
    }
    writeln!(
        stdout().lock(),
        "Published canonical bundle {digest} at {}",
        receipt.path.display()
    )
    .map_err(|error| error.to_string())?;
    super::report_manifest(&converted, manifest)
}

fn staging_budget(limits: ManifestLimits) -> Result<ArchiveBudget, String> {
    let overflow = || String::from("staging budget overflow");
    let metadata = u64::try_from(limits.manifest_bytes.get())
        .map_err(|_error| overflow())?;
    let max_entry = NonZeroU64::new(metadata.max(limits.payload_bytes.get()))
        .ok_or_else(overflow)?;
    let total = limits
        .total_bytes
        .get()
        .checked_add(metadata)
        .and_then(NonZeroU64::new)
        .ok_or_else(overflow)?;
    let count = u64::try_from(limits.records.get())
        .ok()
        .and_then(|value| value.checked_add(2))
        .and_then(NonZeroU64::new)
        .ok_or_else(overflow)?;
    Ok(ArchiveBudget::new(max_entry, total, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_budget_rejects_overflow() {
        let mut limits = simpsons::graphics_limits();
        limits.total_bytes = NonZeroU64::MAX;
        assert!(staging_budget(limits).is_err());
        assert!(staging_budget(simpsons::graphics_limits()).is_ok());
    }
}
