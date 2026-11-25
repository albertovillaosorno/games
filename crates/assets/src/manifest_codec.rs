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
//   - Canonical manifest construction, validation and bounded JSON coding.
// - Must-Not:
//   - Interpret source media, execute assets or access storage.
// - Allows:
//   - Inputs: Manifest bytes or metadata and explicit byte/record limits.
//   - Outputs: Decoded metadata or deterministic JSON within those limits.
//   - Side effects: Bounded in-memory allocation.
// - Split-When:
//   - A different manifest format needs independent admission rules.
// - Merge-When:
//   - Another codec implements the same JSON resource limits.
// - Summary:
//   - Apply manifest resource limits while encoding and decoding.
// - Description:
//   - Reject excess records before decoding and excess bytes before writing.
// - Usage:
//   - Construct, validate or decode the canonical asset manifest.
// - Defaults:
//   - No unbounded record or output buffer growth.

use std::num::NonZeroUsize;
use std::{fmt, io};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{
    AssetError, AssetFormat, AssetManifest, AssetRecord, ContentHash,
    ManifestDocument, ManifestLimits, SourceIdentity, VERSION,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedDocument<'input> {
    #[serde(borrow)]
    records: &'input RawValue,
    #[serde(borrow)]
    source: &'input RawValue,
    version: u16,
}

// Read-only compatibility with previously published, pinned v1 bundles.
// The obsolete fingerprint is never retained or emitted in a new manifest.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacySource {
    digest: ContentHash,
    revision: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct LegacyDocument<'input> {
    records: &'input [AssetRecord],
    source: LegacySource,
    version: u16,
}

#[derive(Debug)]
struct BoundedBytes {
    bytes: Vec<u8>,
    exceeded: bool,
    maximum: usize,
}

impl BoundedBytes {
    fn budget_error(&mut self) -> io::Error {
        self.exceeded = true;
        io::ErrorKind::OutOfMemory.into()
    }

    fn reserve(&mut self, additional: usize) -> io::Result<()> {
        reserve_bounded(&mut self.bytes, additional, self.maximum)
            .ok_or_else(|| self.budget_error())
    }
}

impl fmt::Write for BoundedBytes {
    fn write_char(&mut self, c: char) -> fmt::Result {
        self.write_str(c.encode_utf8(&mut [0; 4]))
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        fmt::write(self, args)
    }

    fn write_str(&mut self, s: &str) -> fmt::Result {
        io::Write::write_all(self, s.as_bytes()).map_err(|_error| fmt::Error)
    }
}

impl io::Write for BoundedBytes {
    fn by_ref(&mut self) -> &mut Self {
        self
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.reserve(buf.len())?;
        self.bytes.extend_from_slice(buf);
        Ok(())
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        fmt::write(self, args).map_err(|_error| {
            if self.exceeded {
                io::ErrorKind::OutOfMemory.into()
            } else {
                io::Error::other("manifest formatting")
            }
        })
    }

    fn write_vectored(
        &mut self,
        bufs: &[io::IoSlice<'_>],
    ) -> io::Result<usize> {
        let length = bufs
            .iter()
            .try_fold(0usize, |total, buf| total.checked_add(buf.len()))
            .ok_or_else(|| self.budget_error())?;
        self.reserve(length)?;
        for buf in bufs {
            self.bytes.extend_from_slice(buf);
        }
        Ok(length)
    }
}

impl AssetManifest {
    /// Encode validated manifest metadata deterministically.
    ///
    /// # Errors
    /// Rejects serialization failure or an exceeded metadata budget.
    pub fn encode(
        &self,
        limits: ManifestLimits,
    ) -> Result<Vec<u8>, AssetError> {
        self.validate(limits)?;
        encode(&self.0, limits.manifest_bytes)
    }

    /// Build a version-two manifest, sorting by canonical logical key.
    ///
    /// # Errors
    /// Rejects invalid identities, duplicates and exceeded budgets.
    pub fn new(
        source: SourceIdentity,
        mut records: Vec<AssetRecord>,
        limits: ManifestLimits,
    ) -> Result<Self, AssetError> {
        if records.len() > limits.records.get() {
            return Err(AssetError::TooLarge);
        }
        records.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        let manifest = Self(ManifestDocument {
            records,
            source,
            version: VERSION,
        });
        let _encoded = manifest.encode(limits)?;
        Ok(manifest)
    }

    /// Decode bounded metadata and verify version and all descriptors.
    /// Legacy canonical v1 metadata is normalized to v2 on encoding.
    ///
    /// # Errors
    /// Rejects malformed, unsupported, unordered or oversized metadata.
    pub fn parse(
        bytes: &[u8],
        limits: ManifestLimits,
    ) -> Result<Self, AssetError> {
        decode(bytes, limits, false).map(Self)
    }

    /// Decode only exact canonical wire bytes, including legacy v1 bundles.
    ///
    /// The returned metadata contains only the selected title and layout.
    /// A caller pinning a legacy bundle must hash its original wire bytes,
    /// not the normalized version-two encoding.
    ///
    /// # Errors
    /// Rejects noncanonical bytes, unsupported versions and invalid metadata.
    pub fn parse_canonical(
        bytes: &[u8],
        limits: ManifestLimits,
    ) -> Result<Self, AssetError> {
        decode(bytes, limits, true).map(Self)
    }

    /// Return an exact logical-key match in the validated sorted manifest.
    #[must_use]
    pub fn record(&self, key: &str) -> Option<&AssetRecord> {
        let index = self
            .records()
            .binary_search_by(|record| record.key.as_str().cmp(key))
            .ok()?;
        self.records().get(index)
    }

    /// Return records in deterministic logical-key order.
    #[must_use]
    pub fn records(&self) -> &[AssetRecord] {
        &self.0.records
    }

    /// Return the selected source layout, not proof of media authenticity.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.0.source
    }

    fn validate(&self, limits: ManifestLimits) -> Result<(), AssetError> {
        if self.0.version != VERSION {
            return Err(AssetError::UnsupportedVersion);
        }
        self.0.source.validate()?;
        if self.0.records.len() > limits.records.get() {
            return Err(AssetError::TooLarge);
        }
        let mut previous = None;
        let mut remaining = limits.total_bytes.get();
        for record in &self.0.records {
            admit_record(record, previous, &mut remaining, limits)?;
            previous = Some(record.key.as_str());
        }
        Ok(())
    }

    /// Validate a replacement descriptor for a title-approved media slot.
    ///
    /// The trusted runtime supplies the allowlist for this base manifest;
    /// it must not derive approval from the untrusted replacement record.
    /// Only existing RGBA8 and PCM slots with identical formats and byte
    /// lengths can be replaced. Declarative game data requires separate
    /// title-specific validation. No files are opened or installed here.
    ///
    /// # Errors
    /// Rejects unknown or unapproved keys and incompatible media.
    /// Payload bytes still require independent verification before use.
    pub fn validate_media_replacement(
        &self,
        allowed_keys: &[&str],
        replacement: &AssetRecord,
    ) -> Result<(), AssetError> {
        let key = replacement.key.as_str();
        if !allowed_keys.contains(&key) {
            return Err(AssetError::InvalidIdentity);
        }
        let original = self.record(key).ok_or(AssetError::InvalidIdentity)?;
        if !matches!(
            original.format,
            AssetFormat::Rgba8 { .. } | AssetFormat::PcmS16Le { .. }
        ) || replacement.format != original.format
            || replacement.size != original.size
        {
            return Err(AssetError::InvalidFormat);
        }
        replacement.validate()
    }

    /// Verify a non-executing replacement and its exact payload bytes.
    ///
    /// # Errors
    /// Rejects unapproved or incompatible media and mismatched payloads.
    pub fn verify_media_replacement(
        &self,
        allowed_keys: &[&str],
        replacement: &AssetRecord,
        payload: &[u8],
    ) -> Result<(), AssetError> {
        self.validate_media_replacement(allowed_keys, replacement)?;
        replacement.verify(payload)
    }
}

/// Decode metadata without constructing records beyond the caller's limit.
///
/// The JSON parser first validates syntax and borrows the raw array. Each
/// element is then decoded individually using the same JSON parser.
fn decode(
    bytes: &[u8],
    limits: ManifestLimits,
    canonical: bool,
) -> Result<ManifestDocument, AssetError> {
    if bytes.len() > limits.manifest_bytes.get() {
        return Err(AssetError::TooLarge);
    }
    let document: BorrowedDocument<'_> = serde_json::from_slice(bytes)
        .map_err(|_error| AssetError::InvalidEncoding)?;
    if !matches!(document.version, 1 | VERSION) {
        return Err(AssetError::UnsupportedVersion);
    }
    let records = decode_records(document.records, limits)?;
    if document.version == 1 {
        return decode_legacy(bytes, document.source, records, limits);
    }
    let manifest = ManifestDocument {
        records,
        source: serde_json::from_str(document.source.get())
            .map_err(|_error| AssetError::InvalidEncoding)?,
        version: VERSION,
    };
    manifest.source.validate()?;
    if canonical && encode(&manifest, limits.manifest_bytes)? != bytes {
        return Err(AssetError::InvalidEncoding);
    }
    Ok(manifest)
}

fn decode_legacy(
    bytes: &[u8],
    raw_source: &RawValue,
    records: Vec<AssetRecord>,
    limits: ManifestLimits,
) -> Result<ManifestDocument, AssetError> {
    let legacy = LegacyDocument {
        records: &records,
        source: serde_json::from_str(raw_source.get())
            .map_err(|_error| AssetError::InvalidEncoding)?,
        version: 1,
    };
    if encode(&legacy, limits.manifest_bytes)? != bytes {
        return Err(AssetError::InvalidEncoding);
    }
    let source = SourceIdentity {
        revision: legacy.source.revision,
        title: legacy.source.title,
    };
    source.validate()?;
    Ok(ManifestDocument {
        records,
        source,
        version: VERSION,
    })
}

fn admit_record(
    record: &AssetRecord,
    previous: Option<&str>,
    remaining: &mut u64,
    limits: ManifestLimits,
) -> Result<(), AssetError> {
    record.validate()?;
    if previous.is_some_and(|key| key >= record.key.as_str()) {
        return Err(AssetError::InvalidOrder);
    }
    if record.size > limits.payload_bytes.get() {
        return Err(AssetError::TooLarge);
    }
    *remaining = remaining
        .checked_sub(record.size)
        .ok_or(AssetError::TooLarge)?;
    Ok(())
}

fn decode_records(
    raw: &RawValue,
    limits: ManifestLimits,
) -> Result<Vec<AssetRecord>, AssetError> {
    let mut remaining = raw
        .get()
        .strip_prefix('[')
        .and_then(|body| body.strip_suffix(']'))
        .ok_or(AssetError::InvalidEncoding)?
        .trim_start();
    let mut records: Vec<AssetRecord> = Vec::new();
    let mut payload_remaining = limits.total_bytes.get();
    while !remaining.is_empty() {
        reserve_bounded(&mut records, 1, limits.records.get())
            .ok_or(AssetError::TooLarge)?;
        let mut stream = serde_json::Deserializer::from_str(remaining)
            .into_iter::<AssetRecord>();
        let record = stream
            .next()
            .ok_or(AssetError::InvalidEncoding)?
            .map_err(|_error| AssetError::InvalidEncoding)?;
        admit_record(
            &record,
            records.last().map(|previous| previous.key.as_str()),
            &mut payload_remaining,
            limits,
        )?;
        records.push(record);
        let rest = remaining
            .get(stream.byte_offset()..)
            .ok_or(AssetError::InvalidEncoding)?
            .trim_start();
        if rest.is_empty() {
            break;
        }
        remaining = rest
            .strip_prefix(',')
            .ok_or(AssetError::InvalidEncoding)?
            .trim_start();
    }
    Ok(records)
}

/// Encode using the existing JSON serializer into a bounded output buffer.
fn encode(
    document: &impl Serialize,
    maximum: NonZeroUsize,
) -> Result<Vec<u8>, AssetError> {
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        exceeded: false,
        maximum: maximum.get(),
    };
    serde_json::to_writer(&mut output, document).map_err(|_error| {
        if output.exceeded {
            AssetError::TooLarge
        } else {
            AssetError::InvalidEncoding
        }
    })?;
    Ok(output.bytes)
}

fn reserve_bounded<T>(
    items: &mut Vec<T>,
    additional: usize,
    maximum: usize,
) -> Option<()> {
    let required = items.len().checked_add(additional)?;
    if required > maximum {
        return None;
    }
    if required > items.capacity() {
        let target = items
            .capacity()
            .saturating_mul(2)
            .max(required)
            .min(maximum);
        items
            .try_reserve_exact(target.checked_sub(items.len())?)
            .ok()?;
    }
    Some(())
}
