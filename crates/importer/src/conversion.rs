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
//   - Conversion admission, content deduplication and manifest assembly.
// - Must-Not:
//   - Decode title-specific game logic, write paths or publish assets.
// - Allows:
//   - Inputs: Verified source members, canonical payloads and budgets.
//   - Outputs: A validated manifest and unique payload staging requests.
//   - Side effects: Calls a caller-owned private staging sink.
// - Split-When:
//   - A distinct source format requires another conversion adapter.
// - Merge-When:
//   - Shared metadata assembly provides equivalent transactional behavior.
// - Summary:
//   - Produce a deterministic manifest only after a complete import.
// - Description:
//   - Validate each canonical payload and stage content once by SHA-256.
// - Usage:
//   - Convert verified members through a private, abortable output sink.
// - Defaults:
//   - No publication after conversion, source or staging failure.

//! Canonical conversion with a caller-owned private staging transaction.

use std::collections::{BTreeMap, BTreeSet};

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetError, AssetFormat, AssetManifest, AssetRecord, ContentHash,
    ManifestLimits, SourceIdentity,
};

use super::{ImportError, SourceSnapshot};

/// Explicit, independent admission budgets for source and output.
#[derive(Clone, Copy, Debug)]
pub struct ConversionLimits {
    /// Decoded source-member limits.
    pub archive: ArchiveBudget,
    /// Canonical manifest and payload limits.
    pub manifest: ManifestLimits,
}

#[derive(Clone, Copy, Debug)]
struct Payload {
    digest: ContentHash,
    format: AssetFormat,
    size: u64,
}

/// Canonical payload admission during one bounded source conversion.
///
/// The sink must keep data private until the returned manifest is accepted.
/// Logical keys never become destination paths; only content hashes are
/// supplied to the sink. A failed admission permanently aborts this conversion.
#[derive(Debug)]
pub struct Conversion<'source, Stage> {
    digests: BTreeSet<ContentHash>,
    failed: bool,
    limits: ManifestLimits,
    records: BTreeMap<String, Payload>,
    remaining_bytes: u64,
    source: &'source SourceIdentity,
    stage: Stage,
}

impl<'source, Stage> Conversion<'source, Stage>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
{
    fn admit(
        &mut self,
        key: &str,
        format: AssetFormat,
        bytes: &[u8],
    ) -> Result<(), ImportError> {
        if self.records.contains_key(key) {
            return Err(AssetError::InvalidOrder.into());
        }
        let size = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        if self.records.len() >= self.limits.records.get()
            || size > self.limits.payload_bytes.get()
        {
            return Err(AssetError::TooLarge.into());
        }
        let remaining = self
            .remaining_bytes
            .checked_sub(size)
            .ok_or(AssetError::TooLarge)?;
        AssetRecord::validate_descriptor(key, format, size)?;
        let digest = ContentHash::calculate(bytes);
        if self.digests.insert(digest) {
            (self.stage)(digest, bytes)?;
        }
        let _old = self.records.insert(key.to_owned(), Payload {
            digest,
            format,
            size,
        });
        self.remaining_bytes = remaining;
        Ok(())
    }

    fn finish(self) -> Result<AssetManifest, ImportError> {
        if self.failed {
            return Err(ImportError::Aborted);
        }
        if self.records.is_empty() {
            return Err(ImportError::EmptyConversion);
        }
        let records = self
            .records
            .into_iter()
            .map(|(key, payload)| AssetRecord {
                digest: payload.digest,
                format: payload.format,
                key,
                size: payload.size,
            })
            .collect();
        Ok(AssetManifest::new(
            self.source.clone(),
            records,
            self.limits,
        )?)
    }

    const fn new(
        source: &'source SourceIdentity,
        limits: ManifestLimits,
        stage: Stage,
    ) -> Self {
        Self {
            digests: BTreeSet::new(),
            failed: false,
            limits,
            records: BTreeMap::new(),
            remaining_bytes: limits.total_bytes.get(),
            source,
            stage,
        }
    }

    /// Stage a validated canonical payload under its content identity.
    ///
    /// Distinct keys may share the same bytes, but every logical record
    /// consumes the declared total and count budgets.
    ///
    /// # Errors
    /// Rejects malformed descriptors, duplicate keys, exhausted budgets,
    /// staging failures and any attempt to continue an aborted conversion.
    pub fn write(
        &mut self,
        key: &str,
        format: AssetFormat,
        bytes: &[u8],
    ) -> Result<(), ImportError> {
        if self.failed {
            return Err(ImportError::Aborted);
        }
        let result = self.admit(key, format, bytes);
        self.failed = result.is_err();
        result
    }
}

/// Convert a complete verified flat archive to canonical assets.
///
/// The caller provides a title-specific, non-executing member converter and
/// a private staging sink. A manifest is returned only after every source
/// member, canonical payload and output budget passes validation.
/// The caller must discard all staged bytes on error and publish only after
/// success; a successful manifest does not itself publish files.
///
/// # Errors
/// Propagates source, conversion, budget, descriptor or sink failures.
pub fn convert_flat_zip<Stage, Convert>(
    source: &SourceSnapshot<'_>,
    limits: ConversionLimits,
    stage: Stage,
    mut convert: Convert,
) -> Result<AssetManifest, ImportError>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    Convert: FnMut(
        &str,
        &[u8],
        &mut Conversion<'_, Stage>,
    ) -> Result<(), ImportError>,
{
    convert_snapshot(source, limits.manifest, stage, |verified, output| {
        verified.visit_flat_zip(limits.archive, |name, payload| {
            convert(name, payload, output)
        })
    })
}

/// Admit known logical payload sizes before decoding or staging.
/// The manifest byte limit is checked against its actual encoding.
pub(crate) fn preflight_payloads(
    limits: ManifestLimits,
    sizes: impl Iterator<Item = Result<u64, ImportError>>,
) -> Result<(), ImportError> {
    let mut remaining = limits.total_bytes.get();
    for (index, result) in sizes.enumerate() {
        let size = result?;
        if index >= limits.records.get() || size > limits.payload_bytes.get() {
            return Err(AssetError::TooLarge.into());
        }
        remaining = remaining.checked_sub(size).ok_or(AssetError::TooLarge)?;
    }
    Ok(())
}

/// Assemble assets from a bounded source, including multi-member decoders.
/// The private sink must be discarded if conversion or completion fails.
/// # Errors
/// Propagates conversion, staging and manifest validation failures.
pub(crate) fn convert_snapshot<Stage, Convert>(
    source: &SourceSnapshot<'_>,
    limits: ManifestLimits,
    stage: Stage,
    convert: Convert,
) -> Result<AssetManifest, ImportError>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    Convert: FnOnce(
        &SourceSnapshot<'_>,
        &mut Conversion<'_, Stage>,
    ) -> Result<(), ImportError>,
{
    let mut output = Conversion::new(source.identity(), limits, stage);
    convert(source, &mut output)?;
    output.finish()
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

    use super::*;

    fn require(condition: bool) -> Result<(), ImportError> {
        condition.then_some(()).ok_or(ImportError::StageFailed)
    }

    fn source() -> SourceIdentity {
        SourceIdentity {
            revision: String::from("fixture-1"),
            title: String::from("synthetic"),
        }
    }

    fn limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::new(4096)
                .unwrap_or(NonZeroUsize::MIN),
            payload_bytes: NonZeroU64::new(8).unwrap_or(NonZeroU64::MIN),
            records: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            total_bytes: NonZeroU64::new(12).unwrap_or(NonZeroU64::MIN),
        }
    }

    #[test]
    fn preflight_checks_each_independent_payload_budget() {
        type Case = (&'static [u64], bool);
        let cases: [Case; 5] = [
            (&[8, 4], true),
            (&[9], false),
            (&[8, 5], false),
            (&[1, 1, 1, 1, 1], false),
            (&[4, 4, 4], true),
        ];
        for (sizes, admitted) in cases {
            assert_eq!(
                preflight_payloads(limits(), sizes.iter().copied().map(Ok))
                    .is_ok(),
                admitted,
            );
        }
    }

    #[test]
    fn payloads_are_staged_once_and_manifest_order_is_stable()
    -> Result<(), ImportError> {
        let identity = source();
        let mut staged = Vec::new();
        let mut conversion =
            Conversion::new(&identity, limits(), |digest, payload| {
                staged.push((digest, payload.to_vec()));
                Ok(())
            });
        conversion.write("image/b", AssetFormat::Data, b"same")?;
        conversion.write("image/a", AssetFormat::Data, b"same")?;
        let manifest = conversion.finish()?;
        require(staged.len() == 1)?;
        require(
            manifest
                .records()
                .iter()
                .map(|record| record.key.as_str())
                .collect::<Vec<_>>()
                == ["image/a", "image/b"],
        )?;
        for record in manifest.records() {
            record.verify(b"same")?;
        }
        Ok(())
    }

    #[test]
    fn invalid_descriptor_aborts_before_staging() {
        let identity = source();
        let mut conversion =
            Conversion::new(&identity, limits(), |_digest, _bytes| {
                Err(ImportError::StageFailed)
            });
        let format = AssetFormat::Rgba8 {
            width: NonZeroU32::MIN,
            height: NonZeroU32::MIN,
        };
        assert_eq!(
            conversion.write("image", format, b"short"),
            Err(AssetError::InvalidFormat.into()),
        );
        assert_eq!(
            conversion.write("valid", AssetFormat::Data, b"x"),
            Err(ImportError::Aborted),
        );
        assert_eq!(conversion.finish(), Err(ImportError::Aborted));
    }

    #[test]
    fn oversized_key_aborts_without_staging() {
        let identity = source();
        let key = "a".repeat(65_536);
        let mut conversion =
            Conversion::new(&identity, limits(), |_digest, _bytes| {
                Err(ImportError::StageFailed)
            });
        assert_eq!(
            conversion.write(&key, AssetFormat::Data, b"x"),
            Err(AssetError::InvalidIdentity.into()),
        );
        assert_eq!(
            conversion.write("valid", AssetFormat::Data, b"x"),
            Err(ImportError::Aborted),
        );
        assert_eq!(conversion.finish(), Err(ImportError::Aborted));
    }

    #[test]
    fn duplicate_logical_records_and_totals_abort_conversion()
    -> Result<(), ImportError> {
        let identity = source();
        let mut duplicate =
            Conversion::new(&identity, limits(), |_digest, _bytes| Ok(()));
        duplicate.write("image", AssetFormat::Data, b"same")?;
        require(
            duplicate.write("image", AssetFormat::Data, b"other")
                == Err(AssetError::InvalidOrder.into()),
        )?;
        require(duplicate.finish() == Err(ImportError::Aborted))?;
        let mut total =
            Conversion::new(&identity, limits(), |_digest, _bytes| Ok(()));
        total.write("image/a", AssetFormat::Data, b"12345678")?;
        require(
            total.write("image/b", AssetFormat::Data, b"12345678")
                == Err(AssetError::TooLarge.into()),
        )?;
        require(total.finish() == Err(ImportError::Aborted))?;
        Ok(())
    }

    #[test]
    fn staging_failure_and_empty_conversion_never_emit_manifest() {
        let identity = source();
        let mut failed =
            Conversion::new(&identity, limits(), |_digest, _bytes| {
                Err(ImportError::StageFailed)
            });
        assert_eq!(
            failed.write("data", AssetFormat::Data, b"data"),
            Err(ImportError::StageFailed),
        );
        assert_eq!(failed.finish(), Err(ImportError::Aborted));
        let empty =
            Conversion::new(&identity, limits(), |_digest, _bytes| Ok(()));
        assert_eq!(empty.finish(), Err(ImportError::EmptyConversion));
    }
}
