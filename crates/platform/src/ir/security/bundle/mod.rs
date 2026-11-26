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
//   - Portable canonical metadata, payload and overlay admission.
// - Must-Not:
//   - Open host paths or expose native storage handles.
// - Allows:
//   - Inputs: Trusted digest, limits and caller-owned asset bytes.
//   - Outputs: Verified manifest identity and admitted asset inventories.
//   - Side effects: Bounded in-memory metadata parsing and inventory checks.
// - Split-When:
//   - Another storage capability has independent admission requirements.
// - Merge-When:
//   - Shared asset contracts already enforce identical invariants.
// - Summary:
//   - Verify canonical bundle content independently of storage.
// - Description:
//   - Bind exact wire digest to metadata; admit overlay descriptors.
// - Usage:
//   - Verify manifest bytes, payload inventories and replacement policy.
// - Defaults:
//   - Reject invalid integrity, excess sizes and unapproved media.

//! Shared canonical bundle and media-overlay admission.

use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};

use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};

use super::file::FileReadError;

/// A canonical bundle could not be safely read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleError {
    /// Canonical metadata or payload integrity failed.
    Asset(AssetError),
    /// The host storage rejected an operation.
    Io(ErrorKind),
    /// The requested logical key is absent from the pinned manifest.
    MissingKey,
    /// The backing storage failed its security checks.
    UnsafeStorage,
}

impl From<AssetError> for BundleError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

impl From<FileReadError> for BundleError {
    fn from(error: FileReadError) -> Self {
        match error {
            FileReadError::Io(kind) => Self::Io(kind),
            FileReadError::TooLarge => Self::Asset(AssetError::TooLarge),
            FileReadError::UnsafeFile => Self::UnsafeStorage,
        }
    }
}

impl From<IoError> for BundleError {
    fn from(error: IoError) -> Self {
        Self::Io(error.kind())
    }
}

/// Exact original wire identity bound to validated manifest metadata.
#[derive(Debug)]
pub(crate) struct VerifiedManifest {
    digest: ContentHash,
    manifest: AssetManifest,
}

impl VerifiedManifest {
    fn admit_inventory(
        &self,
        payloads: &BTreeMap<ContentHash, Vec<u8>>,
        exact: bool,
    ) -> Result<(), BundleError> {
        let expected = self.manifest.payload_sizes();
        if exact && payloads.len() != expected.len() {
            return Err(AssetError::PayloadMismatch.into());
        }
        for (identity, size) in expected {
            let payload =
                payloads.get(&identity).ok_or(BundleError::MissingKey)?;
            if u64::try_from(payload.len()).ok() != Some(size)
                || ContentHash::calculate(payload) != identity
            {
                return Err(AssetError::PayloadMismatch.into());
            }
        }
        Ok(())
    }

    pub(crate) fn admit_payloads(
        &self,
        payloads: &BTreeMap<ContentHash, Vec<u8>>,
    ) -> Result<(), BundleError> {
        self.admit_inventory(payloads, true)
    }

    pub(crate) fn admit_referenced_payloads(
        &self,
        payloads: &BTreeMap<ContentHash, Vec<u8>>,
    ) -> Result<(), BundleError> {
        self.admit_inventory(payloads, false)
    }

    pub(crate) const fn digest(&self) -> ContentHash {
        self.digest
    }

    pub(crate) const fn metadata(&self) -> &AssetManifest {
        &self.manifest
    }

    pub(crate) fn new(
        bytes: &[u8],
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<Self, BundleError> {
        if bytes.len() > limits.manifest_bytes.get() {
            return Err(AssetError::TooLarge.into());
        }
        if ContentHash::calculate(bytes) != digest {
            return Err(AssetError::PayloadMismatch.into());
        }
        let manifest = AssetManifest::parse_canonical(bytes, limits)?;
        Ok(Self { digest, manifest })
    }
}

/// Admit a replacement using a trusted title's passive-media allowlist.
pub(crate) fn admit_overlay(
    base: &AssetManifest,
    replacement: &AssetManifest,
    allowed_keys: &[&str],
) -> Result<(), BundleError> {
    if base.source().title != replacement.source().title {
        return Err(AssetError::InvalidIdentity.into());
    }
    for record in replacement.records() {
        base.validate_media_replacement(allowed_keys, record)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU64, NonZeroUsize};

    use games_assets::{AssetFormat, AssetRecord, SourceIdentity};

    use super::*;

    fn limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(15),
            records: NonZeroUsize::MIN.saturating_add(3),
            total_bytes: NonZeroU64::MIN.saturating_add(63),
        }
    }

    #[test]
    fn verified_manifest_admits_one_payload_for_shared_content()
    -> Result<(), BundleError> {
        let content = b"tile";
        let digest = ContentHash::calculate(content);
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            ["tiles/one", "tiles/two"]
                .into_iter()
                .map(|key| AssetRecord {
                    digest,
                    format: AssetFormat::Data,
                    key: String::from(key),
                    size: 4,
                })
                .collect(),
            limits(),
        )?;
        let encoded = manifest.encode(limits())?;
        let identity = ContentHash::calculate(&encoded);
        let verified = VerifiedManifest::new(&encoded, identity, limits())?;
        let other = ContentHash::calculate(b"fake");
        let valid = BTreeMap::from([(digest, content.to_vec())]);
        let wrong_identity = BTreeMap::from([(other, b"fake".to_vec())]);
        let wrong_size = BTreeMap::from([(digest, b"til".to_vec())]);
        let wrong_bytes = BTreeMap::from([(digest, b"fake".to_vec())]);
        let mut extra = valid.clone();
        let _previous = extra.insert(other, b"fake".to_vec());
        if verified.digest() != identity
            || verified.metadata() != &manifest
            || verified.admit_payloads(&valid).is_err()
            || verified.admit_payloads(&wrong_identity)
                != Err(BundleError::MissingKey)
        {
            return Err(BundleError::UnsafeStorage);
        }
        for rejected in [&wrong_size, &wrong_bytes, &extra] {
            if verified.admit_payloads(rejected)
                != Err(AssetError::PayloadMismatch.into())
            {
                return Err(BundleError::UnsafeStorage);
            }
        }
        Ok(())
    }
}
