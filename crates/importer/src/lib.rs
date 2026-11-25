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
//   - Verified source media and bounded archive member decoding.
// - Must-Not:
//   - Execute original code, publish files or depend on a runtime.
// - Allows:
//   - Inputs: Immutable source bytes, pinned identities and resource limits.
//   - Outputs: Validated source members for explicit conversion.
//   - Side effects: Calls a caller-owned conversion callback.
// - Split-When:
//   - A new source format requires a separate decoder.
// - Merge-When:
//   - A shared source decoder already provides the same admission contract.
// - Summary:
//   - Verify original media before any source-format interpretation.
// - Description:
//   - Decode one bounded member at a time without filesystem extraction.
// - Usage:
//   - Verify the input snapshot, then visit its permitted source entries.
// - Defaults:
//   - Reject unknown source identities, ambiguous names and unsupported files.

//! Source identification and read-only archive conversion inputs.
/// Transactional canonical conversion of verified source members.
pub mod conversion;

/// Bounded source graphics decoding.
pub mod graphics;

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::num::NonZeroU64;
use std::str;

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryBudget, EntryKind,
};
use games_assets::{AssetError, ContentHash, SourceIdentity};
use zip::CompressionMethod;
use zip::read::{ArchiveOffset, Config, ZipArchive};

/// A source admission or conversion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportError {
    /// A conversion attempted to continue after an earlier failure.
    Aborted,
    /// Shared archive safety policy rejected the metadata or decoded size.
    Archive(ArchiveError),
    /// Canonical asset metadata or payload validation failed.
    Asset(AssetError),
    /// Two member names are identical or collide under ASCII case folding.
    Collision,
    /// The converter produced no canonical assets.
    EmptyConversion,
    /// Source bytes do not match the exact expected identity.
    IncorrectSource,
    /// The title or revision identifier is not canonical.
    InvalidSourceIdentity,
    /// Archive headers, compressed bytes or checksum are invalid.
    InvalidZip,
    /// A caller-owned private staging sink failed to accept an asset.
    StageFailed,
    /// A source, allocation or archive budget was exceeded.
    TooLarge,
    /// The source uses a member type, path or compression not yet supported.
    UnsupportedMember,
}

impl From<AssetError> for ImportError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

impl From<ArchiveError> for ImportError {
    fn from(error: ArchiveError) -> Self {
        Self::Archive(error)
    }
}

/// Immutable source snapshot verified against a selected original edition.
///
/// The bytes are borrowed so the decoded archive cannot change after hashing.
/// The caller owns source selection and must use a trusted expected identity.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedSource<'source> {
    bytes: &'source [u8],
    identity: &'source SourceIdentity,
}

impl<'source> VerifiedSource<'source> {
    /// Return the selected, verified source identity.
    #[must_use]
    pub const fn identity(&self) -> &SourceIdentity {
        self.identity
    }

    /// Verify an immutable bounded source snapshot before parsing it.
    ///
    /// # Errors
    /// Rejects invalid identities, oversized snapshots and mismatched SHA-256.
    pub fn new(
        bytes: &'source [u8],
        identity: &'source SourceIdentity,
        limit: NonZeroU64,
    ) -> Result<Self, ImportError> {
        identity
            .validate()
            .map_err(|_error| ImportError::InvalidSourceIdentity)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit.get() {
            return Err(ImportError::TooLarge);
        }
        if ContentHash::calculate(bytes) != identity.digest {
            return Err(ImportError::IncorrectSource);
        }
        Ok(Self { bytes, identity })
    }

    /// Decode a flat ZIP source such as an arcade ROM set.
    ///
    /// Only ordinary ASCII-named files using stored or deflated compression
    /// are accepted. One complete member is held at a time. The callback
    /// receives it only after its size and CRC have been checked.
    /// Any callback or later member failure invalidates the entire import;
    /// the caller must stage outputs and publish only on success.
    ///
    /// # Errors
    /// Rejects malformed, unsafe, ambiguous, unsupported or oversized entries,
    /// and propagates conversion errors.
    pub fn visit_flat_zip<Visit>(
        &self,
        mut budget: ArchiveBudget,
        mut visit: Visit,
    ) -> Result<(), ImportError>
    where
        Visit: FnMut(&str, &[u8]) -> Result<(), ImportError>,
    {
        let config = Config {
            archive_offset: ArchiveOffset::Known(0),
        };
        let mut archive =
            ZipArchive::with_config(config, Cursor::new(self.bytes))
                .map_err(|_error| ImportError::InvalidZip)?;
        if archive.is_empty() {
            return Err(ImportError::InvalidZip);
        }
        if u64::try_from(archive.len()).unwrap_or(u64::MAX)
            > budget.remaining_entries()
        {
            return Err(ArchiveError::TooLarge.into());
        }

        if archive.has_duplicate_names() {
            return Err(ImportError::Collision);
        }
        archive
            .validate_header_consistency()
            .map_err(|_error| ImportError::InvalidZip)?;
        let mut names = BTreeSet::new();
        let mut payload = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|_error| ImportError::InvalidZip)?;
            if !entry.is_file()
                || entry.encrypted()
                || !matches!(
                    entry.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                )
                || entry.unix_mode().is_some_and(|mode| {
                    !matches!(mode & 0o170_000, 0 | 0o100_000)
                        || mode & 0o111 != 0
                })
            {
                return Err(ImportError::UnsupportedMember);
            }
            let name = str::from_utf8(entry.name_raw())
                .map_err(|_error| ImportError::UnsupportedMember)?;
            if !name.is_ascii()
                || name.contains('/')
                || entry.name().map_err(|_error| ImportError::InvalidZip)?
                    != name
            {
                return Err(ImportError::UnsupportedMember);
            }
            let owned_name = name.to_owned();
            let reservation =
                budget.admit(&owned_name, EntryKind::File, entry.size())?;
            if !names.insert(owned_name.to_ascii_lowercase()) {
                return Err(ImportError::Collision);
            }
            decode_member(&mut entry, reservation, &mut payload)?;
            visit(&owned_name, &payload)?;
        }
        Ok(())
    }
}

fn decode_member<R>(
    reader: &mut R,
    mut reservation: EntryBudget,
    payload: &mut Vec<u8>,
) -> Result<(), ImportError>
where
    R: Read,
{
    payload.clear();
    let mut buffer = [0; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_error| ImportError::InvalidZip)?;
        if count == 0 {
            break;
        }
        reservation.record(count)?;
        let chunk = buffer.get(..count).ok_or(ImportError::InvalidZip)?;
        payload
            .try_reserve(count)
            .map_err(|_error| ImportError::TooLarge)?;
        payload.extend_from_slice(chunk);
    }
    reservation.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::num::NonZeroUsize;

    use games_assets::{AssetFormat, ManifestLimits};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::conversion::{ConversionLimits, convert_flat_zip};
    use super::*;
    type FixtureMember<'data> = (&'data str, &'data [u8], CompressionMethod);
    type DecodedMember = (String, Vec<u8>);

    fn bound(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)
    }

    fn budget() -> ArchiveBudget {
        ArchiveBudget::new(bound(1024), bound(4096), bound(8))
    }

    fn identity(bytes: &[u8]) -> SourceIdentity {
        SourceIdentity {
            digest: ContentHash::calculate(bytes),
            revision: String::from("fixture-1"),
            title: String::from("synthetic"),
        }
    }

    fn options(method: CompressionMethod) -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(method)
            .unix_permissions(0o644)
    }

    fn make_zip(members: &[FixtureMember<'_>]) -> Result<Vec<u8>, ImportError> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, payload, method) in members {
            writer
                .start_file(name, options(*method))
                .map_err(|_error| ImportError::InvalidZip)?;
            writer
                .write_all(payload)
                .map_err(|_error| ImportError::InvalidZip)?;
        }
        writer
            .finish()
            .map(Cursor::into_inner)
            .map_err(|_error| ImportError::InvalidZip)
    }

    fn inspect(
        bytes: &[u8],
        limits: ArchiveBudget,
    ) -> Result<Vec<DecodedMember>, ImportError> {
        let expected = identity(bytes);
        let source = VerifiedSource::new(bytes, &expected, bound(65_536))?;
        let mut members = Vec::new();
        source.visit_flat_zip(limits, |name, payload| {
            members.push((name.to_owned(), payload.to_vec()));
            Ok(())
        })?;
        Ok(members)
    }

    fn require(condition: bool) -> Result<(), ImportError> {
        condition.then_some(()).ok_or(ImportError::InvalidZip)
    }

    fn replace_first(
        bytes: &mut [u8],
        old: &[u8],
        new: &[u8],
    ) -> Result<(), ImportError> {
        let start = bytes
            .windows(old.len())
            .position(|window| window == old)
            .ok_or(ImportError::InvalidZip)?;
        let end = start
            .checked_add(old.len())
            .ok_or(ImportError::InvalidZip)?;
        let target =
            bytes.get_mut(start..end).ok_or(ImportError::InvalidZip)?;
        if target.len() != new.len() {
            return Err(ImportError::InvalidZip);
        }
        target.copy_from_slice(new);
        Ok(())
    }

    #[test]
    fn exact_source_identity_precedes_decoding() -> Result<(), ImportError> {
        let bytes =
            make_zip(&[("tile.bin", b"tile", CompressionMethod::Stored)])?;
        let expected = identity(&bytes);
        let mut invalid = expected.clone();
        invalid.title = String::from("Invalid title");
        require(matches!(
            VerifiedSource::new(&bytes, &invalid, bound(65_536)),
            Err(ImportError::InvalidSourceIdentity),
        ))?;
        require(
            VerifiedSource::new(&bytes, &expected, bound(65_536))
                .is_ok_and(|source| source.identity() == &expected),
        )?;
        require(matches!(
            VerifiedSource::new(&bytes, &identity(b"wrong"), bound(65_536)),
            Err(ImportError::IncorrectSource),
        ))?;
        require(matches!(
            VerifiedSource::new(&bytes, &expected, bound(1)),
            Err(ImportError::TooLarge),
        ))
    }

    #[test]
    fn stored_and_deflated_members_are_delivered_exactly()
    -> Result<(), ImportError> {
        let compressed = [42u8; 768];
        let bytes = make_zip(&[
            ("tile.bin", b"tile", CompressionMethod::Stored),
            ("sound.bin", &compressed, CompressionMethod::Deflated),
        ])?;
        require(
            inspect(&bytes, budget())?
                == vec![
                    (String::from("tile.bin"), b"tile".to_vec()),
                    (String::from("sound.bin"), compressed.to_vec()),
                ],
        )
    }

    #[test]
    fn duplicate_case_and_inconsistent_headers_fail() -> Result<(), ImportError>
    {
        let bytes = make_zip(&[
            ("One.bin", b"one", CompressionMethod::Stored),
            ("one.bin", b"two", CompressionMethod::Stored),
        ])?;
        require(inspect(&bytes, budget()) == Err(ImportError::Collision))?;
        let mut duplicate = bytes.clone();
        replace_first(&mut duplicate, b"One.bin", b"one.bin")?;
        replace_first(&mut duplicate, b"One.bin", b"one.bin")?;
        require(inspect(&duplicate, budget()) == Err(ImportError::Collision))?;
        let mut inconsistent = bytes;
        replace_first(&mut inconsistent, b"One.bin", b"Two.bin")?;
        require(
            inspect(&inconsistent, budget()) == Err(ImportError::InvalidZip),
        )
    }

    #[test]
    fn unsafe_names_links_and_executable_modes_fail() -> Result<(), ImportError>
    {
        for name in ["../escape.bin", "CON.bin", "loader.exe", "a/b.bin"] {
            let bytes = make_zip(&[(name, b"x", CompressionMethod::Stored)])?;
            require(inspect(&bytes, budget()).is_err())?;
        }

        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("link", "target", options(CompressionMethod::Stored))
            .map_err(|_error| ImportError::InvalidZip)?;
        let link = writer
            .finish()
            .map_err(|_error| ImportError::InvalidZip)?
            .into_inner();
        require(
            inspect(&link, budget()) == Err(ImportError::UnsupportedMember),
        )?;

        let mut executable = ZipWriter::new(Cursor::new(Vec::new()));
        executable
            .start_file(
                "program.bin",
                options(CompressionMethod::Stored).unix_permissions(0o755),
            )
            .map_err(|_error| ImportError::InvalidZip)?;
        let program = executable
            .finish()
            .map_err(|_error| ImportError::InvalidZip)?
            .into_inner();
        require(
            inspect(&program, budget()) == Err(ImportError::UnsupportedMember),
        )
    }

    #[test]
    fn aggregate_and_member_budgets_reject_before_delivery()
    -> Result<(), ImportError> {
        let bytes = make_zip(&[
            ("first.bin", b"1234", CompressionMethod::Stored),
            ("second.bin", b"5678", CompressionMethod::Stored),
        ])?;
        let expected = identity(&bytes);
        let source = VerifiedSource::new(&bytes, &expected, bound(65_536))?;
        for limits in [
            ArchiveBudget::new(bound(8), bound(16), bound(1)),
            ArchiveBudget::new(bound(3), bound(16), bound(8)),
        ] {
            let mut called = false;
            let result = source.visit_flat_zip(limits, |_name, _bytes| {
                called = true;
                Ok(())
            });
            require(result == Err(ArchiveError::TooLarge.into()) && !called)?;
        }
        let limits = ArchiveBudget::new(bound(8), bound(7), bound(8));
        require(inspect(&bytes, limits) == Err(ArchiveError::TooLarge.into()))
    }

    #[test]
    fn corrupt_payload_never_reaches_converter() -> Result<(), ImportError> {
        let mut bytes = make_zip(&[(
            "sample.bin",
            b"unique-payload",
            CompressionMethod::Stored,
        )])?;
        replace_first(&mut bytes, b"unique-payload", b"unique-payloae")?;
        let expected = identity(&bytes);
        let source = VerifiedSource::new(&bytes, &expected, bound(65_536))?;
        let mut called = false;
        let result = source.visit_flat_zip(budget(), |_name, _payload| {
            called = true;
            Ok(())
        });
        require(result == Err(ImportError::InvalidZip) && !called)
    }

    #[test]
    fn conversion_requires_every_source_entry() -> Result<(), ImportError> {
        let mut bytes = make_zip(&[
            ("first.bin", b"first", CompressionMethod::Stored),
            ("second.bin", b"second-payload", CompressionMethod::Stored),
        ])?;
        let limits = ConversionLimits {
            archive: budget(),
            manifest: ManifestLimits {
                manifest_bytes: NonZeroUsize::new(4096)
                    .unwrap_or(NonZeroUsize::MIN),
                payload_bytes: bound(1024),
                records: NonZeroUsize::new(8).unwrap_or(NonZeroUsize::MIN),
                total_bytes: bound(4096),
            },
        };
        let expected = identity(&bytes);
        let source = VerifiedSource::new(&bytes, &expected, bound(65_536))?;
        let mut staged = 0usize;
        let manifest = convert_flat_zip(
            &source,
            limits,
            |_digest, _data| {
                staged += 1;
                Ok(())
            },
            |name, data, output| {
                let key = name
                    .strip_suffix(".bin")
                    .ok_or(ImportError::UnsupportedMember)?;
                output.write(key, AssetFormat::Data, data)
            },
        )?;
        require(staged == 2 && manifest.records().len() == 2)?;
        replace_first(&mut bytes, b"second-payload", b"damagd-payload")?;
        let damaged_identity = identity(&bytes);
        let damaged =
            VerifiedSource::new(&bytes, &damaged_identity, bound(65_536))?;
        let result = convert_flat_zip(
            &damaged,
            limits,
            |_digest, _data| Ok(()),
            |name, data, output| {
                let key = name
                    .strip_suffix(".bin")
                    .ok_or(ImportError::UnsupportedMember)?;
                output.write(key, AssetFormat::Data, data)
            },
        );
        require(result == Err(ImportError::InvalidZip))
    }
}
