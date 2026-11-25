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
//   - Portable canonical asset verification and media overlays.
// - Must-Not:
//   - Open host paths or expose native storage handles.
// - Allows:
//   - Inputs: Trusted digest, limits and bounded backend observations.
//   - Outputs: Pinned canonical metadata and verified asset bytes.
//   - Side effects: Explicitly requested backend reads only.
// - Split-When:
//   - Another storage capability has independent admission requirements.
// - Merge-When:
//   - Shared asset contracts already enforce identical invariants.
// - Summary:
//   - Backend-independent verification of canonical local bundles.
// - Description:
//   - Verifies every requested payload against admitted metadata.
// - Usage:
//   - Pin a trusted manifest and load assets by logical key.
// - Defaults:
//   - Reject unknown keys, invalid integrity and unsupported storage.

//! Portable canonical bundle capability.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{Error as IoError, ErrorKind};

use games_assets::{
    AssetError, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};

use crate::ir::security::file::FileReadError;

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

/// Private selected backend for the verified bundle opening operation.
pub(crate) trait BundleFactory {
    fn open(
        parent: &OsStr,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<CanonicalBundle, BundleError>;
}

/// A selected backend reads only the descriptor supplied by the IR.
pub(crate) trait PayloadSource: Debug {
    fn read(&self, record: &AssetRecord) -> Result<Vec<u8>, BundleError>;
}

/// Read-only handle to a pinned canonical asset manifest.
///
/// A trusted manifest digest must come from outside untrusted metadata.
/// Each payload is verified independently on every read.
#[derive(Debug)]
pub struct CanonicalBundle {
    digest: ContentHash,
    manifest: AssetManifest,
    source: Box<dyn PayloadSource>,
}

impl CanonicalBundle {
    pub(crate) fn admit_manifest(
        bytes: &[u8],
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<AssetManifest, BundleError> {
        if ContentHash::calculate(bytes) != digest {
            return Err(AssetError::PayloadMismatch.into());
        }
        Ok(AssetManifest::parse_canonical(bytes, limits)?)
    }

    /// The exact pinned manifest address, including for legacy bundles.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.digest
    }

    /// Admit an exact canonical bundle supplied entirely in memory.
    ///
    /// The pinned manifest and every distinct payload must agree before
    /// admission. This does not access any host storage.
    /// # Errors
    /// Rejects invalid manifests, missing, unexpected or corrupted payloads.
    pub fn from_memory(
        bytes: &[u8],
        digest: ContentHash,
        limits: ManifestLimits,
        payloads: BTreeMap<ContentHash, Vec<u8>>,
    ) -> Result<Self, BundleError> {
        let manifest = Self::admit_manifest(bytes, digest, limits)?;
        let expected = manifest.payload_sizes();
        if payloads.len() != expected.len() {
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
        Ok(Self::with_source(digest, manifest, MemorySource(payloads)))
    }

    /// Access admitted metadata for title-specific interpretation.
    #[must_use]
    pub const fn manifest(&self) -> &AssetManifest {
        &self.manifest
    }

    /// Open a manifest-addressed bundle beneath a trusted parent.
    ///
    /// # Errors
    /// Rejects unsupported storage, unsafe paths, noncanonical metadata,
    /// exceeded limits and a manifest whose bytes do not match the digest.
    pub fn open<Parent>(
        parent: &Parent,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<Self, BundleError>
    where
        Parent: AsRef<OsStr> + ?Sized,
    {
        <() as BundleFactory>::open(parent.as_ref(), digest, limits)
    }

    /// Read the declared payload and verify its bytes after backend dispatch.
    ///
    /// # Errors
    /// Rejects unknown keys, invalid storage, short or oversized files and
    /// any mismatch against the pinned descriptor.
    pub fn read_payload(&self, key: &str) -> Result<Vec<u8>, BundleError> {
        let record =
            self.manifest.record(key).ok_or(BundleError::MissingKey)?;
        let bytes = self.source.read(record)?;
        record.verify(&bytes)?;
        Ok(bytes)
    }

    pub(crate) fn with_source(
        digest: ContentHash,
        manifest: AssetManifest,
        source: impl PayloadSource + 'static,
    ) -> Self {
        Self {
            digest,
            manifest,
            source: Box::new(source),
        }
    }
}

/// Verified payload bytes owned by a portable in-memory bundle.
#[derive(Debug)]
struct MemorySource(BTreeMap<ContentHash, Vec<u8>>);

impl PayloadSource for MemorySource {
    fn read(&self, record: &AssetRecord) -> Result<Vec<u8>, BundleError> {
        self.0
            .get(&record.digest)
            .cloned()
            .ok_or(BundleError::MissingKey)
    }
}

/// An explicitly selected, read-only passive-media replacement bundle.
///
/// The caller must pin the exact base and replacement manifests. Each
/// replacement must address an existing title-approved RGBA or PCM slot
/// with the same format and dimensions. Unknown or unapproved records
/// reject the entire overlay; corrupted payloads never silently fall back.
#[derive(Debug)]
pub struct MediaOverlay<'bundle> {
    base: &'bundle CanonicalBundle,
    replacement: &'bundle CanonicalBundle,
}

impl<'bundle> MediaOverlay<'bundle> {
    fn bundle_for(&self, key: &str) -> &CanonicalBundle {
        if self.replacement.manifest().record(key).is_some() {
            self.replacement
        } else {
            self.base
        }
    }

    /// Select a replacement after checking every declared descriptor.
    ///
    /// The allowlist must come from the trusted title, not either manifest.
    /// Payload integrity is verified on each subsequent read.
    ///
    /// # Errors
    /// Rejects another title, an unknown or unapproved slot, incompatible
    /// media, and attempts to replace declarative game data.
    pub fn new(
        base: &'bundle CanonicalBundle,
        replacement: &'bundle CanonicalBundle,
        allowed_keys: &[&str],
    ) -> Result<Self, BundleError> {
        if base.manifest().source().title
            != replacement.manifest().source().title
        {
            return Err(AssetError::InvalidIdentity.into());
        }
        for record in replacement.manifest().records() {
            base.manifest()
                .validate_media_replacement(allowed_keys, record)?;
        }
        Ok(Self { base, replacement })
    }

    /// Read the selected payload with its original bundle's integrity checks.
    ///
    /// # Errors
    /// Rejects absent keys, invalid storage and corrupted replacement or base
    /// content. A failed replacement read is never retried against the base.
    pub fn read_payload(&self, key: &str) -> Result<Vec<u8>, BundleError> {
        self.bundle_for(key).read_payload(key)
    }

    /// Return the effective descriptor for the selected payload, if present.
    #[must_use]
    pub fn record(&self, key: &str) -> Option<&AssetRecord> {
        self.bundle_for(key).manifest().record(key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::{NonZeroU64, NonZeroUsize};

    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
        SourceIdentity,
    };

    use super::{BundleError, CanonicalBundle};

    fn limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::new(4096)
                .unwrap_or(NonZeroUsize::MIN),
            payload_bytes: NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            records: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            total_bytes: NonZeroU64::new(64).unwrap_or(NonZeroU64::MIN),
        }
    }

    #[test]
    fn in_memory_bundle_verifies_exact_content() -> Result<(), BundleError> {
        let data = b"tile";
        let payload = ContentHash::calculate(data);
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            vec![AssetRecord {
                digest: payload,
                format: AssetFormat::Data,
                key: String::from("tiles/hero"),
                size: 4,
            }],
            limits(),
        )?;
        let encoded = manifest.encode(limits())?;
        let digest = ContentHash::calculate(&encoded);
        let bundle = CanonicalBundle::from_memory(
            &encoded,
            digest,
            limits(),
            BTreeMap::from([(payload, data.to_vec())]),
        )?;
        if bundle.read_payload("tiles/hero")? != data
            || CanonicalBundle::from_memory(
                &encoded,
                digest,
                limits(),
                BTreeMap::from([(payload, b"fake".to_vec())]),
            )
            .is_ok()
            || CanonicalBundle::from_memory(
                &encoded,
                digest,
                limits(),
                BTreeMap::new(),
            )
            .is_ok()
        {
            return Err(BundleError::MissingKey);
        }
        Ok(())
    }
}
