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
//   - Locale-aware selection and verified reading of canonical media.
// - Must-Not:
//   - Interpret original formats, duplicate assets or own language policy.
// - Allows:
//   - Inputs: Pinned bundle, metadata key, admitted locales and limits.
//   - Outputs: Verified canonical records and payload views.
//   - Side effects: Explicitly requested verified bundle reads.
// - Split-When:
//   - A resource type needs its own runtime interpretation.
// - Merge-When:
//   - Existing bundle selection subsumes localised resource lookup.
// - Summary:
//   - Bind localised resource metadata to one verified canonical bundle.
// - Description:
//   - All variants reuse the normal content-addressed asset namespace.
// - Usage:
//   - Open an edition's optional catalogue and select a resource by locale.
// - Defaults:
//   - Exact locale, parents, configured fallback, then original resource.

//! Optional localised media catalogue in a canonical asset bundle.

use games::presentation::LocaleTag;
use games_assets::AssetRecord;

use super::{BundleError, CanonicalBundle, PayloadView};
pub use crate::ir::security::bundle::localisation::MediaCatalogueLimits;
use crate::ir::security::bundle::localisation::{
    RecordBindings, catalogue_record,
};

/// Admitted locale variants bound to one verified manifest.
/// The selected and fallback languages are supplied by the title.
#[derive(Debug)]
pub struct LocalisedMediaCatalogue<'bundle> {
    bindings: RecordBindings<'bundle>,
    bundle: &'bundle CanonicalBundle,
    metadata: &'bundle AssetRecord,
}

impl<'bundle> LocalisedMediaCatalogue<'bundle> {
    /// Open a versioned media catalogue from a canonical Data asset.
    /// The metadata itself is size-checked before dispatch and digest-verified.
    /// # Errors
    /// Rejects missing, malformed, oversized or incompatible metadata.
    pub fn open(
        bundle: &'bundle CanonicalBundle,
        key: &str,
        limits: MediaCatalogueLimits,
    ) -> Result<Self, BundleError> {
        let metadata = catalogue_record(bundle.manifest(), key, limits.bytes)?;
        let bytes = bundle.view_payload(key)?;
        let bindings = RecordBindings::admit(
            bundle.manifest(),
            metadata,
            &bytes,
            limits.bindings,
        )?;
        Ok(Self {
            bindings,
            bundle,
            metadata,
        })
    }

    /// Select one canonical record without loading or duplicating its bytes.
    /// Falls back to the original base key if neither locale has a variant.
    /// Variant keys are never interpreted recursively.
    /// # Errors
    /// Rejects unknown resource keys or attempts to select the catalogue.
    pub fn record(
        &self,
        resource: &str,
        selected: &LocaleTag,
        fallback: &LocaleTag,
    ) -> Result<&'bundle AssetRecord, BundleError> {
        if resource == self.metadata.key {
            return Err(BundleError::MissingKey);
        }
        self.bindings
            .variant(resource, selected, fallback)
            .or_else(|| self.bundle.manifest().record(resource))
            .ok_or(BundleError::MissingKey)
    }

    /// Read and verify the selected canonical payload through the bundle.
    /// # Errors
    /// Propagates selection, storage or integrity failures without silently
    /// substituting another language for damaged content.
    pub fn view_payload(
        &self,
        resource: &str,
        selected: &LocaleTag,
        fallback: &LocaleTag,
    ) -> Result<PayloadView<'bundle>, BundleError> {
        let record = self.record(resource, selected, fallback)?;
        self.bundle.view_payload(&record.key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
    use std::ptr;

    use games_assets::{
        AssetError, AssetFormat, AssetManifest, ContentHash, ManifestLimits,
        SourceIdentity,
    };

    use super::super::PayloadSource;
    use super::*;

    const KEY: &str = "localisation/media";
    const BASE: &str = "text/title";
    const CATALOGUE: &[u8] = b"media-locales-v1\n\
text/title\ten\ttext/title-en\n\
text/title\tes\ttext/title-es\n\
text/title\tes-MX\ttext/title-es-mx\n\
text/title\tfr\ttext/title-fr\n";

    type Sample<'payload> = (&'payload str, AssetFormat, &'payload [u8]);

    #[derive(Debug)]
    struct CorruptSelection {
        inner: Box<dyn PayloadSource>,
    }

    impl PayloadSource for CorruptSelection {
        fn read(
            &self,
            record: &AssetRecord,
        ) -> Result<PayloadView<'_>, BundleError> {
            if record.key == "text/title-es" {
                return Ok(PayloadView::Borrowed(b"broken"));
            }
            self.inner.read(record)
        }
    }

    #[derive(Debug)]
    struct RejectRead;

    impl PayloadSource for RejectRead {
        fn read(
            &self,
            _record: &AssetRecord,
        ) -> Result<PayloadView<'_>, BundleError> {
            Err(BundleError::UnsafeStorage)
        }
    }

    fn admitted(tag: &str) -> Result<LocaleTag, BundleError> {
        LocaleTag::parse(tag)
            .map_err(|_error| AssetError::InvalidIdentity.into())
    }

    fn fixture(payload: &[u8]) -> Result<CanonicalBundle, BundleError> {
        let data = AssetFormat::Data;
        let samples: [Sample<'_>; 8] = [
            (KEY, data, payload),
            (BASE, data, b"original"),
            ("text/title-en", data, b"english"),
            ("text/title-es", data, b"shared"),
            ("text/title-es-mx", data, b"mexican"),
            ("text/title-fr", data, b"shared"),
            ("text/untranslated", data, b"unchanged"),
            (
                "art/notice",
                AssetFormat::Rgba8 {
                    width: NonZeroU32::MIN,
                    height: NonZeroU32::MIN,
                },
                b"abcd",
            ),
        ];
        let mut records = Vec::new();
        let mut objects = BTreeMap::new();
        for (key, format, bytes) in samples {
            let digest = ContentHash::calculate(bytes);
            let size = u64::try_from(bytes.len())
                .map_err(|_error| AssetError::TooLarge)?;
            records.push(AssetRecord {
                digest,
                format,
                key: key.into(),
                size,
            });
            let _previous = objects.insert(digest, bytes.to_vec());
        }
        let limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(1023),
            records: NonZeroUsize::MIN.saturating_add(7),
            total_bytes: NonZeroU64::MIN.saturating_add(2047),
        };
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: "fixture".into(),
                title: "test".into(),
            },
            records,
            limits,
        )?;
        let encoded = manifest.encode(limits)?;
        CanonicalBundle::from_memory(
            &encoded,
            ContentHash::calculate(&encoded),
            limits,
            objects,
        )
    }

    const fn limits() -> MediaCatalogueLimits {
        MediaCatalogueLimits {
            bindings: NonZeroUsize::MIN.saturating_add(3),
            bytes: NonZeroU64::MIN.saturating_add(1023),
        }
    }

    fn require(condition: bool) -> Result<(), BundleError> {
        condition.then_some(()).ok_or(BundleError::UnsafeStorage)
    }

    #[test]
    fn exact_parent_fallback_and_original_use_verified_records()
    -> Result<(), BundleError> {
        let bundle = fixture(CATALOGUE)?;
        let catalogue = LocalisedMediaCatalogue::open(&bundle, KEY, limits())?;
        let fallback = admitted("en-US")?;
        for (selected, expected) in [
            ("es-MX", b"mexican".as_slice()),
            ("es-MX-1996", b"mexican"),
            ("es-ES", b"shared"),
            ("fr-CA", b"shared"),
            ("ja", b"english"),
        ] {
            let requested = admitted(selected)?;
            require(
                catalogue
                    .view_payload(BASE, &requested, &fallback)?
                    .as_ref()
                    == expected,
            )?;
            require(
                catalogue
                    .view_payload("text/untranslated", &requested, &fallback)?
                    .as_ref()
                    == b"unchanged",
            )?;
        }
        let selected = admitted("ja")?;
        let no_variant = admitted("de")?;
        require(
            catalogue
                .view_payload(BASE, &selected, &no_variant)?
                .as_ref()
                == b"original",
        )?;
        require(
            catalogue.record(KEY, &selected, &fallback)
                == Err(BundleError::MissingKey),
        )?;
        require(
            catalogue.record("unknown", &selected, &fallback)
                == Err(BundleError::MissingKey),
        )
    }

    #[test]
    fn language_selection_reuses_identical_canonical_payload()
    -> Result<(), BundleError> {
        let bundle = fixture(CATALOGUE)?;
        let catalogue = LocalisedMediaCatalogue::open(&bundle, KEY, limits())?;
        let en = admitted("en")?;
        let es = admitted("es")?;
        let fr = admitted("fr")?;
        let first = catalogue.record(BASE, &es, &en)?;
        let second = catalogue.record(BASE, &fr, &en)?;
        require(first.key != second.key && first.digest == second.digest)?;
        let inventory = bundle.manifest().payload_sizes();
        require(inventory.len() < bundle.manifest().records().len())?;
        let first_bytes = catalogue.view_payload(BASE, &es, &en)?;
        let second_bytes = catalogue.view_payload(BASE, &fr, &en)?;
        require(first_bytes == second_bytes)?;
        require(matches!((first_bytes, second_bytes),
            (PayloadView::Borrowed(left), PayloadView::Borrowed(right))
                if ptr::eq(left, right)))
    }

    #[test]
    fn untrusted_catalogue_rejects_bad_versions_rows_and_identities()
    -> Result<(), BundleError> {
        for (payload, expected) in [
            (
                b"media-locales-v2\n".as_slice(),
                AssetError::InvalidEncoding,
            ),
            (b"media-locales-v1\n\xff", AssetError::InvalidEncoding),
            (
                b"media-locales-v1\ntext/title\tes\ttext/title-es",
                AssetError::InvalidEncoding,
            ),
            (
                b"media-locales-v1\ntext/title\tes\ttext/title-es\textra\n",
                AssetError::InvalidEncoding,
            ),
            (
                b"media-locales-v1\ntext/title\tES\ttext/title-es\n",
                AssetError::InvalidEncoding,
            ),
            (
                b"media-locales-v1\ntext/title\tx-private\ttext/title-es\n",
                AssetError::InvalidIdentity,
            ),
            (
                b"media-locales-v1\ntext/title\tes\tlocalisation/media\n",
                AssetError::InvalidIdentity,
            ),
            (
                b"media-locales-v1\ntext/title\tes\tart/notice\n",
                AssetError::InvalidFormat,
            ),
            (
                b"media-locales-v1\ntext/title\tes\ttext/title-es\n\
text/title\tes\ttext/title-fr\n",
                AssetError::InvalidOrder,
            ),
            (
                b"media-locales-v1\ntext/title\tfr\ttext/title-fr\n\
text/title\tes\ttext/title-es\n",
                AssetError::InvalidOrder,
            ),
        ] {
            let bundle = fixture(payload)?;
            let result = LocalisedMediaCatalogue::open(&bundle, KEY, limits());
            require(result.err() == Some(BundleError::Asset(expected)))?;
        }
        Ok(())
    }

    #[test]
    fn catalogue_rejects_missing_assets_and_independent_limits()
    -> Result<(), BundleError> {
        for payload in [
            b"media-locales-v1\nunknown\tes\ttext/title-es\n".as_slice(),
            b"media-locales-v1\ntext/title\tes\tmissing\n",
        ] {
            let bundle = fixture(payload)?;
            require(
                LocalisedMediaCatalogue::open(&bundle, KEY, limits()).err()
                    == Some(BundleError::MissingKey),
            )?;
        }
        let bundle = fixture(CATALOGUE)?;
        for restrictive in [
            MediaCatalogueLimits {
                bytes: NonZeroU64::MIN,
                ..limits()
            },
            MediaCatalogueLimits {
                bindings: NonZeroUsize::MIN,
                ..limits()
            },
        ] {
            require(
                LocalisedMediaCatalogue::open(&bundle, KEY, restrictive).err()
                    == Some(BundleError::Asset(AssetError::TooLarge)),
            )?;
        }
        require(
            LocalisedMediaCatalogue::open(&bundle, "art/notice", limits())
                .err()
                == Some(BundleError::Asset(AssetError::InvalidFormat)),
        )?;
        require(
            LocalisedMediaCatalogue::open(&bundle, "missing", limits()).err()
                == Some(BundleError::MissingKey),
        )
    }

    #[test]
    fn exact_admission_budget_and_direct_variant_selection()
    -> Result<(), BundleError> {
        let payload = b"media-locales-v1\n\
text/title\tes\ttext/title-es\n\
text/title-es\tes\ttext/title-fr\n";
        let bundle = fixture(payload)?;
        let exact = MediaCatalogueLimits {
            bindings: NonZeroUsize::MIN.saturating_add(1),
            bytes: NonZeroU64::new(
                u64::try_from(payload.len())
                    .map_err(|_error| AssetError::TooLarge)?,
            )
            .ok_or(AssetError::TooLarge)?,
        };
        let catalogue = LocalisedMediaCatalogue::open(&bundle, KEY, exact)?;
        let selected = admitted("es")?;
        let fallback = admitted("en")?;
        require(
            catalogue.record(BASE, &selected, &fallback)?.key
                == "text/title-es",
        )?;
        require(
            catalogue.record("text/title-es", &selected, &fallback)?.key
                == "text/title-fr",
        )?;
        let empty = fixture(b"media-locales-v1\n")?;
        let neutral = LocalisedMediaCatalogue::open(&empty, KEY, limits())?;
        require(neutral.record(BASE, &selected, &fallback)?.key == BASE)
    }

    #[test]
    fn corrupt_selected_payload_never_changes_language()
    -> Result<(), BundleError> {
        let mut bundle = fixture(CATALOGUE)?;
        bundle.source = Box::new(CorruptSelection { inner: bundle.source });
        let catalogue = LocalisedMediaCatalogue::open(&bundle, KEY, limits())?;
        let selected = admitted("es")?;
        let fallback = admitted("en")?;
        require(
            catalogue.record(BASE, &selected, &fallback)?.key
                == "text/title-es",
        )?;
        require(
            catalogue.view_payload(BASE, &selected, &fallback).err()
                == Some(BundleError::Asset(AssetError::PayloadMismatch)),
        )?;
        require(
            catalogue.view_payload(BASE, &fallback, &fallback)?.as_ref()
                == b"english",
        )
    }

    #[test]
    fn metadata_budget_is_checked_before_backend_read()
    -> Result<(), BundleError> {
        let mut bundle = fixture(CATALOGUE)?;
        bundle.source = Box::new(RejectRead);
        let restrictive = MediaCatalogueLimits {
            bytes: NonZeroU64::MIN,
            ..limits()
        };
        require(
            LocalisedMediaCatalogue::open(&bundle, KEY, restrictive).err()
                == Some(BundleError::Asset(AssetError::TooLarge)),
        )?;
        require(
            LocalisedMediaCatalogue::open(&bundle, KEY, limits()).err()
                == Some(BundleError::UnsafeStorage),
        )
    }
}
