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

/// Explicit selected canonical directory admission.
pub mod directory;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::num::{NonZeroU64, NonZeroUsize};
use std::rc::Rc;

use games_assets::{
    AssetError, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};

pub use crate::ir::security::bundle::BundleError;
use crate::ir::security::bundle::{VerifiedManifest, admit_overlay};

pub(crate) const MANIFEST_NAME: &str = "manifest.json";
pub(crate) const PAYLOADS_DIRECTORY: &str = "payloads";
pub(crate) const OBJECTS_DIRECTORY: &str = "objects";
pub(crate) const OBJECT_PAYLOAD: &str = "payload";

pub(crate) type OpenBundle = Result<Box<dyn BundlePort>, BundleError>;
pub(crate) type OpenPayloads = Result<Box<dyn PayloadSource>, BundleError>;
type MemoryObjects = Rc<BTreeMap<ContentHash, Vec<u8>>>;

/// Selected backend pins the manifest namespace without admitting metadata.
pub(crate) trait BundleFactory {
    fn open(parent: &OsStr, digest: ContentHash) -> OpenBundle;
}

/// Read bounded bytes before opening any payload namespace.
pub(crate) trait BundlePort: Debug {
    fn open_payloads(self: Box<Self>, has_payloads: bool) -> OpenPayloads;

    fn read_manifest(
        &self,
        maximum: NonZeroUsize,
    ) -> Result<Vec<u8>, BundleError>;
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
    manifest: VerifiedManifest,
    source: Box<dyn PayloadSource>,
}

impl CanonicalBundle {
    /// The exact pinned manifest address, including for legacy bundles.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.manifest.digest()
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
        let manifest = VerifiedManifest::new(bytes, digest, limits)?;
        manifest.admit_payloads(&payloads)?;
        Ok(Self {
            manifest,
            source: Box::new(MemorySource(Rc::new(payloads))),
        })
    }

    fn from_port(
        port: Box<dyn BundlePort>,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<Self, BundleError> {
        let bytes = port.read_manifest(limits.manifest_bytes)?;
        let manifest = VerifiedManifest::new(&bytes, digest, limits)?;
        let source =
            port.open_payloads(!manifest.metadata().records().is_empty())?;
        Ok(Self { manifest, source })
    }

    /// Access admitted metadata for title-specific interpretation.
    #[must_use]
    pub const fn manifest(&self) -> &AssetManifest {
        self.manifest.metadata()
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
        Self::from_port(
            <() as BundleFactory>::open(parent.as_ref(), digest)?,
            digest,
            limits,
        )
    }

    /// Read the declared payload and verify its bytes after backend dispatch.
    ///
    /// # Errors
    /// Rejects unknown keys, invalid storage, short or oversized files and
    /// any mismatch against the pinned descriptor.
    pub fn read_payload(&self, key: &str) -> Result<Vec<u8>, BundleError> {
        let record =
            self.manifest().record(key).ok_or(BundleError::MissingKey)?;
        let bytes = self.source.read(record)?;
        record.verify(&bytes)?;
        Ok(bytes)
    }
}

/// Aggregate admission for one caller-owned canonical memory store.
#[derive(Clone, Copy, Debug)]
pub struct MemoryBundleLimits {
    /// Maximum combined byte length of manifests and distinct objects.
    pub bytes: NonZeroU64,
    /// Maximum number of distinct manifest addresses.
    pub manifests: NonZeroUsize,
    /// Maximum number of distinct payload addresses.
    pub objects: NonZeroUsize,
}

/// A private, immutable snapshot of manifest and shared object bytes.
///
/// The caller supplies the selected manifest digest separately at open time.
/// No stored manifest is trusted merely because it occupies an address.
#[derive(Debug)]
pub struct MemoryBundleStore {
    manifests: BTreeMap<ContentHash, Vec<u8>>,
    objects: MemoryObjects,
}

impl MemoryBundleStore {
    /// Admit the aggregate inventory without cloning any content bytes.
    ///
    /// # Errors
    /// Rejects excessive inventory counts or total stored byte length.
    pub fn new(
        manifests: BTreeMap<ContentHash, Vec<u8>>,
        objects: BTreeMap<ContentHash, Vec<u8>>,
        limits: MemoryBundleLimits,
    ) -> Result<Self, BundleError> {
        if manifests.len() > limits.manifests.get()
            || objects.len() > limits.objects.get()
        {
            return Err(AssetError::TooLarge.into());
        }
        let mut total = 0u64;
        for bytes in manifests.values().chain(objects.values()) {
            let length = u64::try_from(bytes.len())
                .map_err(|_error| AssetError::TooLarge)?;
            total = total.checked_add(length).ok_or(AssetError::TooLarge)?;
            if total > limits.bytes.get() {
                return Err(AssetError::TooLarge.into());
            }
        }
        Ok(Self {
            manifests,
            objects: Rc::new(objects),
        })
    }

    /// Verify a pinned manifest and all its declared objects.
    ///
    /// Distinct bundles share the immutable object allocation. Each logical
    /// payload read still verifies the selected record.
    ///
    /// # Errors
    /// Rejects absent or invalid content and exceeded per-manifest limits.
    pub fn open(
        &self,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<CanonicalBundle, BundleError> {
        let bytes =
            self.manifests.get(&digest).ok_or(BundleError::MissingKey)?;
        let manifest = VerifiedManifest::new(bytes, digest, limits)?;
        manifest.admit_referenced_payloads(&self.objects)?;
        Ok(CanonicalBundle {
            manifest,
            source: Box::new(MemorySource(Rc::clone(&self.objects))),
        })
    }
}

/// Select a canonical asset store independently of a title's media rules.
#[derive(Clone, Copy, Debug)]
pub enum BundleSource<'store> {
    /// Trusted local manifest parent; storage admission remains backend-owned.
    Local(&'store OsStr),
    /// Caller-admitted in-memory canonical snapshot.
    Memory(&'store MemoryBundleStore),
}

impl BundleSource<'_> {
    /// Open the selected manifest with identical per-bundle admission.
    ///
    /// # Errors
    /// Rejects unsupported storage, absent content or invalid metadata.
    pub fn open(
        self,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<CanonicalBundle, BundleError> {
        match self {
            Self::Local(parent) => {
                CanonicalBundle::open(parent, digest, limits)
            },
            Self::Memory(store) => store.open(digest, limits),
        }
    }
}

/// Shared immutable payload bytes owned by an in-memory source.
#[derive(Debug)]
struct MemorySource(MemoryObjects);

impl PayloadSource for MemorySource {
    fn read(&self, record: &AssetRecord) -> Result<Vec<u8>, BundleError> {
        let payload =
            self.0.get(&record.digest).ok_or(BundleError::MissingKey)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(payload.len())
            .map_err(|_error| AssetError::TooLarge)?;
        bytes.extend_from_slice(payload);
        Ok(bytes)
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
        admit_overlay(base.manifest(), replacement.manifest(), allowed_keys)?;
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

pub(crate) fn bundle_name(digest: ContentHash) -> String {
    format!("bundle-{digest}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

    use games_assets::{
        AssetError, AssetFormat, AssetManifest, AssetRecord, ContentHash,
        ManifestLimits, SourceIdentity,
    };

    use super::{
        BundleError, BundlePort, BundleSource, CanonicalBundle, MediaOverlay,
        MemoryBundleLimits, MemoryBundleStore, OpenPayloads, PayloadSource,
    };

    #[derive(Debug)]
    struct IncorrectPayload;

    impl PayloadSource for IncorrectPayload {
        fn read(&self, _record: &AssetRecord) -> Result<Vec<u8>, BundleError> {
            Ok(b"oops".to_vec())
        }
    }

    #[derive(Debug)]
    struct FixturePort {
        bytes: Vec<u8>,
        expected_payloads: bool,
        open_error: Option<BundleError>,
    }

    impl BundlePort for FixturePort {
        fn open_payloads(self: Box<Self>, has_payloads: bool) -> OpenPayloads {
            if has_payloads != self.expected_payloads {
                return Err(BundleError::UnsafeStorage);
            }
            if let Some(error) = self.open_error {
                return Err(error);
            }
            Ok(Box::new(IncorrectPayload))
        }

        fn read_manifest(
            &self,
            maximum: NonZeroUsize,
        ) -> Result<Vec<u8>, BundleError> {
            if maximum != limits().manifest_bytes {
                return Err(BundleError::UnsafeStorage);
            }
            // A fake backend deliberately ignores the read bound.
            Ok(self.bytes.clone())
        }
    }

    type FixtureRecord<'record> = (&'record str, AssetFormat, &'record [u8; 4]);

    fn port(
        bytes: &[u8],
        expected_payloads: bool,
        open_error: Option<BundleError>,
    ) -> Box<dyn BundlePort> {
        Box::new(FixturePort {
            bytes: bytes.to_vec(),
            expected_payloads,
            open_error,
        })
    }

    #[test]
    fn portable_open_rejects_metadata_before_opening_payloads()
    -> Result<(), BundleError> {
        let wrong_digest = ContentHash::calculate(b"wrong");
        let too_large =
            vec![0; limits().manifest_bytes.get().saturating_add(1)];
        let invalid = b"not metadata";
        let open_error = Some(BundleError::MissingKey);

        let cases = [
            (too_large.as_slice(), wrong_digest, AssetError::TooLarge),
            (
                invalid.as_slice(),
                wrong_digest,
                AssetError::PayloadMismatch,
            ),
            (
                invalid.as_slice(),
                ContentHash::calculate(invalid),
                AssetError::InvalidEncoding,
            ),
        ];
        for (bytes, digest, expected) in cases {
            let result = CanonicalBundle::from_port(
                port(bytes, false, open_error),
                digest,
                limits(),
            );
            if result.err() != Some(BundleError::Asset(expected)) {
                return Err(BundleError::UnsafeStorage);
            }
        }
        Ok(())
    }

    #[test]
    fn portable_open_selects_payload_namespace_after_admission()
    -> Result<(), BundleError> {
        for populated in [false, true] {
            let records: &[FixtureRecord<'_>] = if populated {
                &[("asset", AssetFormat::Data, b"data")]
            } else {
                &[]
            };
            let fixture = fixture_bundle("test", records)?;
            let bytes = fixture.manifest().encode(limits())?;
            let digest = fixture.digest();
            let opened = CanonicalBundle::from_port(
                port(&bytes, populated, None),
                digest,
                limits(),
            )?;
            if opened.digest() != digest
                || opened.manifest().records() != fixture.manifest().records()
            {
                return Err(BundleError::UnsafeStorage);
            }
            let result = CanonicalBundle::from_port(
                port(&bytes, populated, Some(BundleError::MissingKey)),
                digest,
                limits(),
            );
            if !matches!(result, Err(BundleError::MissingKey)) {
                return Err(BundleError::UnsafeStorage);
            }
        }
        Ok(())
    }

    fn fixture_bundle(
        title: &str,
        records: &[FixtureRecord<'_>],
    ) -> Result<CanonicalBundle, BundleError> {
        let entries = records.iter().map(|(key, format, data)| {
            let digest = ContentHash::calculate(*data);
            (
                AssetRecord {
                    digest,
                    format: *format,
                    key: String::from(*key),
                    size: 4,
                },
                (digest, data.to_vec()),
            )
        });
        let (descriptors, payloads): (Vec<_>, BTreeMap<_, _>) = entries.unzip();
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from(title),
            },
            descriptors,
            limits(),
        )?;
        let encoded = manifest.encode(limits())?;
        CanonicalBundle::from_memory(
            &encoded,
            ContentHash::calculate(&encoded),
            limits(),
            payloads,
        )
    }

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

    #[test]
    fn shared_memory_store_opens_distinct_manifests_with_one_object()
    -> Result<(), BundleError> {
        let first =
            fixture_bundle("test", &[("one", AssetFormat::Data, b"tile")])?;
        let second =
            fixture_bundle("test", &[("two", AssetFormat::Data, b"tile")])?;
        let first_bytes = first.manifest().encode(limits())?;
        let second_bytes = second.manifest().encode(limits())?;
        let object = ContentHash::calculate(b"tile");
        let other = ContentHash::calculate(b"more");
        let total = u64::try_from(
            first_bytes
                .len()
                .saturating_add(second_bytes.len())
                .saturating_add(8),
        )
        .map_err(|_error| AssetError::TooLarge)?;
        let limits_for_store = MemoryBundleLimits {
            bytes: NonZeroU64::new(total).ok_or(AssetError::TooLarge)?,
            manifests: NonZeroUsize::MIN.saturating_add(1),
            objects: NonZeroUsize::MIN.saturating_add(1),
        };
        let manifests = BTreeMap::from([
            (first.digest(), first_bytes),
            (second.digest(), second_bytes),
        ]);
        let objects = BTreeMap::from([
            (object, b"tile".to_vec()),
            (other, b"more".to_vec()),
        ]);
        let store =
            MemoryBundleStore::new(manifests, objects, limits_for_store)?;
        let selected = BundleSource::Memory(&store);
        let one = selected.open(first.digest(), limits())?;
        let two = selected.open(second.digest(), limits())?;
        drop(store);
        if one.read_payload("one")? != b"tile"
            || two.read_payload("two")? != b"tile"
            || one.read_payload("two") != Err(BundleError::MissingKey)
            || two.read_payload("one") != Err(BundleError::MissingKey)
        {
            return Err(BundleError::UnsafeStorage);
        }
        Ok(())
    }

    #[test]
    fn shared_store_enforces_aggregate_limits() -> Result<(), BundleError> {
        let first = ContentHash::calculate(b"one");
        let second = ContentHash::calculate(b"two");
        let manifests = BTreeMap::from([
            (first, b"one".to_vec()),
            (second, b"two".to_vec()),
        ]);
        let objects = manifests.clone();
        let budget = MemoryBundleLimits {
            bytes: NonZeroU64::MIN.saturating_add(11),
            manifests: NonZeroUsize::MIN.saturating_add(1),
            objects: NonZeroUsize::MIN.saturating_add(1),
        };
        for insufficient in [
            MemoryBundleLimits {
                manifests: NonZeroUsize::MIN,
                ..budget
            },
            MemoryBundleLimits {
                objects: NonZeroUsize::MIN,
                ..budget
            },
            MemoryBundleLimits {
                bytes: NonZeroU64::MIN.saturating_add(10),
                ..budget
            },
        ] {
            if MemoryBundleStore::new(
                manifests.clone(),
                objects.clone(),
                insufficient,
            )
            .err()
                != Some(AssetError::TooLarge.into())
            {
                return Err(BundleError::UnsafeStorage);
            }
        }
        drop(MemoryBundleStore::new(manifests, objects, budget)?);
        Ok(())
    }

    #[test]
    fn shared_memory_store_rejects_invalid_selected_inventory()
    -> Result<(), BundleError> {
        let fixture =
            fixture_bundle("test", &[("one", AssetFormat::Data, b"tile")])?;
        let bytes = fixture.manifest().encode(limits())?;
        let digest = fixture.digest();
        let object = ContentHash::calculate(b"tile");
        let budget = MemoryBundleLimits {
            bytes: NonZeroU64::MIN.saturating_add(4095),
            manifests: NonZeroUsize::MIN,
            objects: NonZeroUsize::MIN,
        };
        for (objects, expected) in [
            (BTreeMap::new(), BundleError::MissingKey),
            (
                BTreeMap::from([(object, b"fake".to_vec())]),
                AssetError::PayloadMismatch.into(),
            ),
            (
                BTreeMap::from([(object, b"tiles".to_vec())]),
                AssetError::PayloadMismatch.into(),
            ),
        ] {
            let store = MemoryBundleStore::new(
                BTreeMap::from([(digest, bytes.clone())]),
                objects,
                budget,
            )?;
            if store.open(digest, limits()).err() != Some(expected) {
                return Err(BundleError::UnsafeStorage);
            }
        }
        let wrong = ContentHash::calculate(b"wrong");
        let wrong_store = MemoryBundleStore::new(
            BTreeMap::from([(wrong, bytes.clone())]),
            BTreeMap::from([(object, b"tile".to_vec())]),
            budget,
        )?;
        if wrong_store.open(digest, limits()).err()
            != Some(BundleError::MissingKey)
            || wrong_store.open(wrong, limits()).err()
                != Some(AssetError::PayloadMismatch.into())
        {
            return Err(BundleError::UnsafeStorage);
        }
        let store = MemoryBundleStore::new(
            BTreeMap::from([(digest, bytes)]),
            BTreeMap::from([(object, b"tile".to_vec())]),
            budget,
        )?;
        (store
            .open(digest, ManifestLimits {
                manifest_bytes: NonZeroUsize::MIN,
                ..limits()
            })
            .err()
            == Some(AssetError::TooLarge.into()))
        .then_some(())
        .ok_or(BundleError::UnsafeStorage)
    }

    #[test]
    fn passive_overlay_admission_is_portable() -> Result<(), BundleError> {
        let rgba = AssetFormat::Rgba8 {
            height: NonZeroU32::MIN,
            width: NonZeroU32::MIN,
        };
        let base = fixture_bundle("test", &[
            ("media/hero", rgba, b"base"),
            ("stage/rules", AssetFormat::Data, b"rule"),
        ])?;
        let replacement =
            fixture_bundle("test", &[("media/hero", rgba, b"swap")])?;
        let foreign =
            fixture_bundle("other", &[("media/hero", rgba, b"swap")])?;
        let rules = fixture_bundle("test", &[(
            "media/hero",
            AssetFormat::Data,
            b"rule",
        )])?;
        let allowed = ["media/hero"];
        let overlay = MediaOverlay::new(&base, &replacement, &allowed)?;
        if overlay.read_payload("media/hero")? != b"swap"
            || base.read_payload("media/hero")? != b"base"
            || overlay.read_payload("stage/rules")? != b"rule"
            || overlay.record("media/hero")
                != replacement.manifest().record("media/hero")
            || overlay.record("stage/rules")
                != base.manifest().record("stage/rules")
            || overlay.read_payload("media/missing")
                != Err(BundleError::MissingKey)
            || !matches!(
                MediaOverlay::new(&base, &foreign, &allowed),
                Err(BundleError::Asset(AssetError::InvalidIdentity))
            )
            || !matches!(
                MediaOverlay::new(&base, &replacement, &[]),
                Err(BundleError::Asset(AssetError::InvalidIdentity))
            )
            || !matches!(
                MediaOverlay::new(&rules, &rules, &allowed),
                Err(BundleError::Asset(AssetError::InvalidFormat))
            )
        {
            return Err(BundleError::UnsafeStorage);
        }
        let encoded = replacement.manifest().encode(limits())?;
        let rejected = CanonicalBundle::from_port(
            port(&encoded, true, None),
            replacement.digest(),
            limits(),
        )?;
        let rejected_overlay = MediaOverlay::new(&base, &rejected, &allowed)?;
        if rejected_overlay.read_payload("media/hero")
            != Err(AssetError::PayloadMismatch.into())
            || rejected_overlay.read_payload("stage/rules")? != b"rule"
        {
            return Err(BundleError::UnsafeStorage);
        }
        Ok(())
    }
}
