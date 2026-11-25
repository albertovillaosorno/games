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
//   - Deterministic planning of missing canonical payloads.
// - Must-Not:
//   - Read or write files or trust unverified existing bytes.
// - Allows:
//   - Inputs: Verified existing identities and requested manifest metadata.
//   - Outputs: Unique missing payloads and their total physical bytes.
//   - Side effects: None.
// - Split-When:
//   - Independent publication policy needs a separate contract.
// - Merge-When:
//   - Another shared contract plans the same content identities.
// - Summary:
//   - Cross-manifest payload reuse without duplicate publication.
// - Description:
//   - Every identity has one length; existing verified content is reused.
// - Usage:
//   - Plan missing content before opening a publication transaction.
// - Defaults:
//   - Reject conflicting lengths and caller-selected budget exhaustion.

use std::collections::BTreeMap;

use crate::{AssetError, AssetManifest, ContentHash};

/// Missing distinct payloads and their exact required byte count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationPlan {
    /// Only identities absent from the verified inventory.
    pub missing: BTreeMap<ContentHash, u64>,
    /// Total bytes of new content, not logical references.
    pub new_bytes: u64,
}

/// Plan publication across manifests against a verified title inventory.
///
/// The backend must verify existing content before supplying its inventory.
/// The caller chooses the maximum number of new bytes, including zero.
/// # Errors
/// Rejects conflicting lengths, overflow and budget exhaustion.
pub fn plan(
    existing: &BTreeMap<ContentHash, u64>,
    requested: &[AssetManifest],
    maximum_new_bytes: u64,
) -> Result<PublicationPlan, AssetError> {
    let mut missing = BTreeMap::new();
    let mut new_bytes = 0u64;
    for (digest, size) in
        requested.iter().flat_map(AssetManifest::payload_sizes)
    {
        match existing.get(&digest).or_else(|| missing.get(&digest)) {
            Some(&known) if size != known => {
                return Err(AssetError::PayloadMismatch);
            },
            Some(_) => {},
            None => {
                new_bytes = new_bytes
                    .checked_add(size)
                    .filter(|total| *total <= maximum_new_bytes)
                    .ok_or(AssetError::TooLarge)?;
                let _previous = missing.insert(digest, size);
            },
        }
    }
    Ok(PublicationPlan { missing, new_bytes })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::{NonZeroU64, NonZeroUsize};
    use std::slice;

    use super::plan;
    use crate::{
        AssetError, AssetFormat, AssetManifest, AssetRecord, ContentHash,
        ManifestLimits, SourceIdentity,
    };

    type FixtureRecord<'record> = (&'record str, ContentHash, u64);

    fn manifest(
        records: &[FixtureRecord<'_>],
    ) -> Result<AssetManifest, AssetError> {
        AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("example"),
            },
            records
                .iter()
                .map(|&(key, digest, size)| AssetRecord {
                    digest,
                    format: AssetFormat::Data,
                    key: key.to_owned(),
                    size,
                })
                .collect(),
            ManifestLimits {
                manifest_bytes: NonZeroUsize::MAX,
                payload_bytes: NonZeroU64::MAX,
                records: NonZeroUsize::MAX,
                total_bytes: NonZeroU64::MAX,
            },
        )
    }

    #[test]
    fn huge_assets_use_checked_metadata_only() -> Result<(), AssetError> {
        let first = ContentHash::calculate(b"first");
        let second = ContentHash::calculate(b"second");
        let huge = manifest(&[("first", first, u64::MAX)])?;
        let small = manifest(&[("second", second, 1)])?;
        if plan(&BTreeMap::new(), &[huge, small], u64::MAX)
            != Err(AssetError::TooLarge)
        {
            return Err(AssetError::PayloadMismatch);
        }
        Ok(())
    }

    #[test]
    fn repeated_manifests_require_no_repeated_payload_bytes()
    -> Result<(), AssetError> {
        let first = ContentHash::calculate(b"first");
        let second = ContentHash::calculate(b"next");
        let original = manifest(&[("first", first, 5)])?;
        let updated = manifest(&[("first", first, 5), ("next", second, 4)])?;
        let planned = plan(&BTreeMap::new(), &[original, updated.clone()], 9)?;
        if planned.new_bytes != 9 || planned.missing.len() != 2 {
            return Err(AssetError::PayloadMismatch);
        }
        let installed = plan(&planned.missing, &[updated], 0)?;
        if installed.new_bytes != 0 || !installed.missing.is_empty() {
            return Err(AssetError::PayloadMismatch);
        }
        Ok(())
    }

    #[test]
    fn conflicting_sizes_and_insufficient_budgets_fail()
    -> Result<(), AssetError> {
        let identity = ContentHash::calculate(b"tile");
        let first = manifest(&[("first", identity, 4)])?;
        let second = manifest(&[("second", identity, 5)])?;
        let existing = BTreeMap::from([(identity, 4)]);
        if plan(&existing, slice::from_ref(&second), 100)
            != Err(AssetError::PayloadMismatch)
            || plan(&BTreeMap::new(), &[first.clone(), second], 100)
                != Err(AssetError::PayloadMismatch)
            || plan(&BTreeMap::new(), &[first], 3) != Err(AssetError::TooLarge)
        {
            return Err(AssetError::InvalidFormat);
        }
        Ok(())
    }
}
