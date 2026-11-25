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

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::{fmt, io};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{
    AssetError, AssetFormat, AssetManifest, AssetRecord, ContentHash,
    ManifestDocument, ManifestLimits, SourceIdentity, VERSION, reserve_bounded,
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
enum Output<'input> {
    Collect(Vec<u8>),
    Validate {
        expected: Option<&'input [u8]>,
        matches: bool,
    },
}

#[derive(Debug)]
struct BoundedBytes<'input> {
    exceeded: bool,
    maximum: usize,
    output: Output<'input>,
    written: usize,
}

impl BoundedBytes<'_> {
    fn budget_error(&mut self) -> io::Error {
        self.exceeded = true;
        io::ErrorKind::OutOfMemory.into()
    }

    fn reserve(&mut self, additional: usize) -> io::Result<usize> {
        let end = self
            .written
            .checked_add(additional)
            .filter(|end| *end <= self.maximum)
            .ok_or_else(|| self.budget_error())?;
        if let Output::Collect(bytes) = &mut self.output
            && reserve_bounded(bytes, additional, self.maximum).is_none()
        {
            return Err(self.budget_error());
        }
        Ok(end)
    }

    fn serialize(
        &mut self,
        document: &impl Serialize,
    ) -> Result<(), AssetError> {
        serde_json::to_writer(&mut *self, document).map_err(|_error| {
            if self.exceeded {
                AssetError::TooLarge
            } else {
                AssetError::InvalidEncoding
            }
        })
    }
}

impl fmt::Write for BoundedBytes<'_> {
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

impl io::Write for BoundedBytes<'_> {
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
        let end = self.reserve(buf.len())?;
        match &mut self.output {
            Output::Collect(bytes) => bytes.extend_from_slice(buf),
            Output::Validate { expected, matches } => {
                if let Some(input) = expected {
                    *matches &= input.get(self.written..end) == Some(buf);
                }
            },
        }
        self.written = end;
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
        let _end = self.reserve(length)?;
        for buf in bufs {
            self.write_all(buf)?;
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
        manifest.validate(limits)?;
        validate_encoding(&manifest.0, limits.manifest_bytes, None)?;
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

    /// Return the distinct content identities and their exact payload sizes.
    ///
    /// Multiple logical records may share content only when their declared
    /// sizes agree. Each still consumes its own logical admission budget.
    #[must_use]
    pub fn payload_sizes(&self) -> BTreeMap<ContentHash, u64> {
        self.records()
            .iter()
            .map(|record| (record.digest, record.size))
            .collect()
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
        let mut admission = RecordAdmission::new(limits);
        for record in &self.0.records {
            admission.admit(record, previous)?;
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

struct RecordAdmission {
    limits: ManifestLimits,
    remaining: u64,
    sizes: BTreeMap<ContentHash, u64>,
}

impl RecordAdmission {
    fn admit(
        &mut self,
        record: &AssetRecord,
        previous: Option<&str>,
    ) -> Result<(), AssetError> {
        record.validate()?;
        if previous.is_some_and(|key| key >= record.key.as_str()) {
            return Err(AssetError::InvalidOrder);
        }
        if record.size > self.limits.payload_bytes.get() {
            return Err(AssetError::TooLarge);
        }
        self.remaining = self
            .remaining
            .checked_sub(record.size)
            .ok_or(AssetError::TooLarge)?;
        if self
            .sizes
            .insert(record.digest, record.size)
            .is_some_and(|size| size != record.size)
        {
            return Err(AssetError::PayloadMismatch);
        }
        Ok(())
    }

    const fn new(limits: ManifestLimits) -> Self {
        Self {
            limits,
            remaining: limits.total_bytes.get(),
            sizes: BTreeMap::new(),
        }
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
    if document.version == 1 {
        return decode_legacy(bytes, document.source, document.records, limits);
    }
    let source: SourceIdentity = serde_json::from_str(document.source.get())
        .map_err(|_error| AssetError::InvalidEncoding)?;
    source.validate()?;
    let manifest = ManifestDocument {
        records: decode_records(document.records, limits)?,
        source,
        version: VERSION,
    };
    if canonical {
        validate_encoding(&manifest, limits.manifest_bytes, Some(bytes))?;
    }
    Ok(manifest)
}

fn decode_legacy(
    bytes: &[u8],
    raw_source: &RawValue,
    raw_records: &RawValue,
    limits: ManifestLimits,
) -> Result<ManifestDocument, AssetError> {
    let legacy_source: LegacySource = serde_json::from_str(raw_source.get())
        .map_err(|_error| AssetError::InvalidEncoding)?;
    SourceIdentity::validate_fields(
        &legacy_source.title,
        &legacy_source.revision,
    )?;
    let records = decode_records(raw_records, limits)?;
    let legacy = LegacyDocument {
        records: &records,
        source: legacy_source,
        version: 1,
    };
    validate_encoding(&legacy, limits.manifest_bytes, Some(bytes))?;
    let source = SourceIdentity {
        revision: legacy.source.revision,
        title: legacy.source.title,
    };
    Ok(ManifestDocument {
        records,
        source,
        version: VERSION,
    })
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
    let mut admission = RecordAdmission::new(limits);
    while !remaining.is_empty() {
        reserve_bounded(&mut records, 1, limits.records.get())
            .ok_or(AssetError::TooLarge)?;
        let mut stream = serde_json::Deserializer::from_str(remaining)
            .into_iter::<AssetRecord>();
        let record = stream
            .next()
            .ok_or(AssetError::InvalidEncoding)?
            .map_err(|_error| AssetError::InvalidEncoding)?;
        admission.admit(
            &record,
            records.last().map(|previous| previous.key.as_str()),
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
        exceeded: false,
        maximum: maximum.get(),
        output: Output::Collect(Vec::new()),
        written: 0,
    };
    output.serialize(document)?;
    match output.output {
        Output::Collect(bytes) => Ok(bytes),
        Output::Validate { .. } => Err(AssetError::InvalidEncoding),
    }
}

/// Check the serialized size and optionally compare exact canonical bytes.
fn validate_encoding(
    document: &impl Serialize,
    maximum: NonZeroUsize,
    expected: Option<&[u8]>,
) -> Result<(), AssetError> {
    let mut output = BoundedBytes {
        exceeded: false,
        maximum: maximum.get(),
        output: Output::Validate { expected, matches: true },
        written: 0,
    };
    output.serialize(document)?;
    if let Output::Validate { matches: true, .. } = output.output
        && expected.is_none_or(|bytes| bytes.len() == output.written)
    {
        return Ok(());
    }
    Err(AssetError::InvalidEncoding)
}
