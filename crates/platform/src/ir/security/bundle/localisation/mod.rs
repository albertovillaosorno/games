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
//   - Admission of bounded locale-to-canonical-media metadata.
// - Must-Not:
//   - Open storage, decode source formats or select title languages.
// - Allows:
//   - Inputs: Pinned metadata, verified payload bytes and explicit limits.
//   - Outputs: Validated locale bindings to records in that manifest.
//   - Side effects: Bounded in-memory locale metadata allocation.
// - Split-When:
//   - Another metadata format requires independent admission rules.
// - Merge-When:
//   - Shared catalogue admission subsumes canonical media references.
// - Summary:
//   - Validate resource variants against one pinned asset namespace.
// - Description:
//   - Admits only canonical tags, known keys and compatible encodings.
// - Usage:
//   - Check the metadata size before reading; admit its verified bytes.
// - Defaults:
//   - Reject malformed, ambiguous, unknown and oversized bindings.

use std::collections::BTreeMap;
use std::mem::discriminant;
use std::num::{NonZeroU64, NonZeroUsize};

use games::presentation::LocaleTag;
use games_assets::{AssetError, AssetFormat, AssetManifest, AssetRecord};

use super::BundleError;

const HEADER: &str = "media-locales-v1\n";

/// Independent admission budgets for canonical localised media metadata.
#[derive(Clone, Copy, Debug)]
pub struct MediaCatalogueLimits {
    /// Maximum number of resource/locale bindings.
    pub bindings: NonZeroUsize,
    /// Maximum verified metadata payload length.
    pub bytes: NonZeroU64,
}

type Fields<'row> = (&'row str, &'row str, &'row str);
type Variants<'manifest> = BTreeMap<LocaleTag, &'manifest AssetRecord>;

#[derive(Debug, Default)]
pub(crate) struct RecordBindings<'manifest> {
    entries: BTreeMap<&'manifest str, Variants<'manifest>>,
}

impl<'manifest> RecordBindings<'manifest> {
    pub(crate) fn admit(
        manifest: &'manifest AssetManifest,
        metadata: &AssetRecord,
        payload: &[u8],
        maximum: NonZeroUsize,
    ) -> Result<Self, BundleError> {
        let text = str::from_utf8(payload)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let body = text
            .strip_prefix(HEADER)
            .ok_or(AssetError::InvalidEncoding)?;
        let mut bindings = Self::default();
        if body.is_empty() {
            return Ok(bindings);
        }
        let rows =
            body.strip_suffix('\n').ok_or(AssetError::InvalidEncoding)?;
        let mut previous = None;
        for (index, row) in rows.split('\n').enumerate() {
            if index >= maximum.get() {
                return Err(AssetError::TooLarge.into());
            }
            let (base, tag, asset) = fields(row)?;
            if previous.is_some_and(|pair| pair >= (base, tag)) {
                return Err(AssetError::InvalidOrder.into());
            }
            bindings.insert(manifest, &metadata.key, (base, tag, asset))?;
            previous = Some((base, tag));
        }
        Ok(bindings)
    }

    fn insert(
        &mut self,
        manifest: &'manifest AssetManifest,
        metadata_key: &str,
        fields: Fields<'_>,
    ) -> Result<(), BundleError> {
        let (base, tag, asset) = fields;
        let locale = LocaleTag::parse(tag)
            .map_err(|_error| AssetError::InvalidIdentity)?;
        if locale.as_str() != tag {
            return Err(AssetError::InvalidEncoding.into());
        }
        if base == metadata_key || asset == metadata_key {
            return Err(AssetError::InvalidIdentity.into());
        }
        let original = manifest.record(base).ok_or(BundleError::MissingKey)?;
        let variant = manifest.record(asset).ok_or(BundleError::MissingKey)?;
        if discriminant(&original.format) != discriminant(&variant.format) {
            return Err(AssetError::InvalidFormat.into());
        }
        let _previous = self
            .entries
            .entry(&original.key)
            .or_default()
            .insert(locale, variant);
        Ok(())
    }

    pub(crate) fn variant(
        &self,
        resource: &str,
        selected: &LocaleTag,
        fallback: &LocaleTag,
    ) -> Option<&'manifest AssetRecord> {
        let variants = self.entries.get(resource)?;
        selected.resolve_with(fallback, |tag| variants.get(tag).copied())
    }
}

pub(crate) fn catalogue_record<'manifest>(
    manifest: &'manifest AssetManifest,
    key: &str,
    maximum: NonZeroU64,
) -> Result<&'manifest AssetRecord, BundleError> {
    let record = manifest.record(key).ok_or(BundleError::MissingKey)?;
    if record.format != AssetFormat::Data {
        return Err(AssetError::InvalidFormat.into());
    }
    if record.size > maximum.get() {
        return Err(AssetError::TooLarge.into());
    }
    Ok(record)
}

fn fields(row: &str) -> Result<Fields<'_>, AssetError> {
    let mut columns = row.split('\t');
    let (Some(base), Some(tag), Some(asset), None) = (
        columns.next(),
        columns.next(),
        columns.next(),
        columns.next(),
    ) else {
        return Err(AssetError::InvalidEncoding);
    };
    Ok((base, tag, asset))
}
