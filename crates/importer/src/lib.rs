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
//   - Bounded source snapshots and archive member decoding.
// - Must-Not:
//   - Execute original code, publish files or depend on a runtime.
// - Allows:
//   - Inputs: Immutable source bytes, selected layouts and resource limits.
//   - Outputs: Validated source members for explicit conversion.
//   - Side effects: Calls a caller-owned conversion callback.
// - Split-When:
//   - A new source format requires a separate decoder.
// - Merge-When:
//   - A shared source decoder already provides the same admission contract.
// - Summary:
//   - Admit bounded source media for safe structural decoding.
// - Description:
//   - Decode one bounded member at a time without filesystem extraction.
// - Usage:
//   - Admit the input snapshot, then visit its permitted source entries.
// - Defaults:
//   - Reject unknown source identities, ambiguous names and unsupported files.

//! Source identification and read-only archive conversion inputs.
/// Transactional canonical conversion of verified source members.
pub mod conversion;

/// Bounded source graphics decoding.
pub mod graphics;
/// MAME machine profiles and shared arcade ROM-set conversion.
pub mod mame;
/// NEO cartridge admission and selected graphics profiles.
pub mod neo;
/// Shared asset-family selection and conversion dispatch.
pub mod selection;
/// Strict PCM16 WAVE reference capture admission.
pub mod wave;

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::num::NonZeroU64;
use std::str;

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryBudget, EntryKind,
};
use games_assets::{AssetError, SourceIdentity, reserve_bounded};
use zip::CompressionMethod;
use zip::read::{ArchiveOffset, Config, ZipArchive, ZipFileEntry};
use zip::unstable::read::ZipIterable;

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
    /// The selected source layout is not supported by the converter.
    IncorrectSource,
    /// Cartridge header version or region extents are invalid.
    InvalidNeo,
    /// The title or revision identifier is not canonical.
    InvalidSourceIdentity,
    /// Reference WAVE headers or PCM frame extents are invalid.
    InvalidWave,
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

impl From<graphics::GraphicsError> for ImportError {
    fn from(error: graphics::GraphicsError) -> Self {
        match error {
            graphics::GraphicsError::Allocation
            | graphics::GraphicsError::Overflow => Self::TooLarge,
            graphics::GraphicsError::InvalidLayout
            | graphics::GraphicsError::SizeMismatch => Self::UnsupportedMember,
        }
    }
}

impl From<ArchiveError> for ImportError {
    fn from(error: ArchiveError) -> Self {
        Self::Archive(error)
    }
}

/// Immutable, size-bounded source snapshot for a selected source layout.
///
/// The bytes are borrowed throughout decoding. Source selection is caller
/// supplied; it is not evidence of the authenticity of the original media.
#[derive(Clone, Copy, Debug)]
pub struct SourceSnapshot<'source> {
    bytes: &'source [u8],
    identity: &'source SourceIdentity,
}

impl<'source> SourceSnapshot<'source> {
    /// Return the selected title and supported layout.
    #[must_use]
    pub const fn identity(&self) -> &SourceIdentity {
        self.identity
    }

    /// Admit an immutable bounded source snapshot before parsing it.
    ///
    /// # Errors
    /// Rejects invalid layout identifiers and oversized snapshots.
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
        Ok(Self { bytes, identity })
    }

    /// Decode a flat ZIP source such as an arcade ROM set.
    ///
    /// Only ordinary ASCII-named files using stored or deflated compression
    /// are accepted. All metadata and aggregate budgets are checked before
    /// decoding any payload. One complete member is held at a time; the
    /// callback receives it only after its size and CRC have been checked.
    /// Any callback or later member failure invalidates the entire import;
    /// the caller must stage outputs and publish only on success.
    ///
    /// # Errors
    /// Rejects malformed, unsafe, ambiguous, unsupported or oversized entries,
    /// and propagates conversion errors.
    pub fn visit_flat_zip<Visit>(
        &self,
        budget: ArchiveBudget,
        mut visit: Visit,
    ) -> Result<(), ImportError>
    where
        Visit: FnMut(&str, &[u8]) -> Result<(), ImportError>,
    {
        let config = Config {
            archive_offset: ArchiveOffset::Known(0),
        };
        let admitted = admit_members(self.bytes, config, budget)?;
        let mut archive =
            ZipArchive::with_config(config, Cursor::new(self.bytes))
                .map_err(|_error| ImportError::InvalidZip)?;
        if archive.has_duplicate_names() {
            return Err(ImportError::Collision);
        }
        if archive.central_directory_start() != admitted.directory_start
            || archive.len() != admitted.reservations.len()
        {
            return Err(ImportError::InvalidZip);
        }
        archive
            .validate_header_consistency()
            .map_err(|_error| ImportError::InvalidZip)?;
        let mut payload = Vec::new();
        for (index, reservation) in
            admitted.reservations.into_iter().enumerate()
        {
            let mut entry = archive
                .by_index(index)
                .map_err(|_error| ImportError::InvalidZip)?;
            decode_member(&mut entry, reservation, &mut payload)?;
            let name = str::from_utf8(entry.name_raw())
                .map_err(|_error| ImportError::UnsupportedMember)?;
            visit(name, &payload)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct AdmittedMembers {
    directory_start: u64,
    reservations: Vec<EntryBudget>,
}

// Admit metadata incrementally before constructing the indexed archive.
fn admit_members(
    bytes: &[u8],
    config: Config,
    mut budget: ArchiveBudget,
) -> Result<AdmittedMembers, ImportError> {
    let mut archive = ZipIterable::try_new(Cursor::new(bytes), config)
        .map_err(|_error| ImportError::InvalidZip)?
        .enforce_header_consistency(true);
    let count = archive.len();
    if count == 0 {
        return Err(ImportError::InvalidZip);
    }
    if u64::try_from(count).unwrap_or(u64::MAX) > budget.remaining_entries() {
        return Err(ArchiveError::TooLarge.into());
    }
    let mut directory_start = None;
    let mut names = BTreeSet::new();
    let mut reservations = Vec::new();
    for result in archive.files().map_err(|_error| ImportError::InvalidZip)? {
        let entry = result.map_err(|_error| ImportError::InvalidZip)?;
        if directory_start.is_none() {
            directory_start = Some(entry.central_header_start());
        }
        let name = admit_member(&entry)?;
        let reservation = budget.admit(name, EntryKind::File, entry.size())?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(ImportError::Collision);
        }
        reserve_bounded(&mut reservations, 1, count)
            .ok_or(ImportError::TooLarge)?;
        reservations.push(reservation);
    }
    if reservations.len() != count {
        return Err(ImportError::InvalidZip);
    }
    Ok(AdmittedMembers {
        directory_start: directory_start.ok_or(ImportError::InvalidZip)?,
        reservations,
    })
}

fn admit_member<'entry>(
    entry: &'entry ZipFileEntry<'_>,
) -> Result<&'entry str, ImportError> {
    if !entry.is_file()
        || entry.encrypted()
        || !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        )
        || entry.unix_mode().is_some_and(|mode| {
            !matches!(mode & 0o170_000, 0 | 0o100_000) || mode & 0o111 != 0
        })
    {
        return Err(ImportError::UnsupportedMember);
    }
    let name = str::from_utf8(entry.name_raw())
        .map_err(|_error| ImportError::UnsupportedMember)?;
    if !name.is_ascii()
        || name.contains('/')
        || entry.name().map_err(|_error| ImportError::InvalidZip)? != name
    {
        return Err(ImportError::UnsupportedMember);
    }
    Ok(name)
}

fn decode_member<R>(
    reader: &mut R,
    mut reservation: EntryBudget,
    payload: &mut Vec<u8>,
) -> Result<(), ImportError>
where
    R: Read,
{
    let remaining = reservation
        .remaining_bytes()
        .ok_or(ArchiveError::InvalidEntry)?;
    let maximum =
        usize::try_from(remaining).map_err(|_error| ImportError::TooLarge)?;
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
        reserve_bounded(payload, count, maximum)
            .ok_or(ImportError::TooLarge)?;
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

    fn identity() -> SourceIdentity {
        SourceIdentity {
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
        let expected = identity();
        let source = SourceSnapshot::new(bytes, &expected, bound(65_536))?;
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
    fn bounded_snapshot_admits_without_original_fingerprint()
    -> Result<(), ImportError> {
        let bytes =
            make_zip(&[("tile.bin", b"tile", CompressionMethod::Stored)])?;
        let expected = identity();
        let mut invalid = expected.clone();
        invalid.title = String::from("Invalid title");
        require(matches!(
            SourceSnapshot::new(&bytes, &invalid, bound(65_536)),
            Err(ImportError::InvalidSourceIdentity),
        ))?;
        require(
            SourceSnapshot::new(&bytes, &expected, bound(65_536))
                .is_ok_and(|source| source.identity() == &expected),
        )?;
        let different =
            make_zip(&[("tile.bin", b"new!", CompressionMethod::Stored)])?;
        require(
            inspect(&different, budget())?
                == vec![(String::from("tile.bin"), b"new!".to_vec())],
        )?;
        require(matches!(
            SourceSnapshot::new(&bytes, &expected, bound(1)),
            Err(ImportError::TooLarge),
        ))
    }

    #[test]
    fn mame_layout_rejects_missing_and_incorrectly_sized_lanes()
    -> Result<(), ImportError> {
        let machine = mame::Machine::select("simpsons", "world-4p-set-1")?;
        let expected = machine.identity();
        for name in ["unrelated.bin", "072-b07.18h"] {
            let bytes =
                make_zip(&[(name, b"short", CompressionMethod::Stored)])?;
            let source = SourceSnapshot::new(&bytes, &expected, bound(65_536))?;
            require(
                machine
                    .visit_layout(&source, budget(), |_name, _payload| Ok(()))
                    == Err(ImportError::UnsupportedMember),
            )?;
        }
        Ok(())
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
        let expected = identity();
        let source = SourceSnapshot::new(&bytes, &expected, bound(65_536))?;
        for limits in [
            ArchiveBudget::new(bound(8), bound(16), bound(1)),
            ArchiveBudget::new(bound(3), bound(16), bound(8)),
            ArchiveBudget::new(bound(8), bound(7), bound(8)),
        ] {
            let mut called = false;
            let result = source.visit_flat_zip(limits, |_name, _bytes| {
                called = true;
                Ok(())
            });
            require(result == Err(ArchiveError::TooLarge.into()) && !called)?;
        }
        Ok(())
    }

    #[test]
    fn excessive_directory_count_precedes_malformed_metadata()
    -> Result<(), ImportError> {
        let mut bytes = make_zip(&[
            ("first.bin", b"first", CompressionMethod::Stored),
            ("second.bin", b"second", CompressionMethod::Stored),
        ])?;
        let position = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x01\x02")
            .ok_or(ImportError::InvalidZip)?;
        let suffix =
            bytes.get_mut(position..).ok_or(ImportError::InvalidZip)?;
        replace_first(suffix, b"PK\x01\x02", b"PK\x00\x02")?;
        let one_entry = ArchiveBudget::new(bound(1024), bound(4096), bound(1));
        require(
            inspect(&bytes, one_entry) == Err(ArchiveError::TooLarge.into()),
        )?;
        require(inspect(&bytes, budget()) == Err(ImportError::InvalidZip))
    }

    #[test]
    fn raw_directory_count_includes_duplicate_names() -> Result<(), ImportError>
    {
        let mut bytes = make_zip(&[
            ("One.bin", b"one", CompressionMethod::Stored),
            ("one.bin", b"two", CompressionMethod::Stored),
        ])?;
        replace_first(&mut bytes, b"One.bin", b"one.bin")?;
        replace_first(&mut bytes, b"One.bin", b"one.bin")?;
        let one_entry = ArchiveBudget::new(bound(1024), bound(4096), bound(1));
        require(
            inspect(&bytes, one_entry) == Err(ArchiveError::TooLarge.into()),
        )?;
        require(inspect(&bytes, budget()) == Err(ImportError::Collision))
    }

    #[test]
    fn admitted_directory_cannot_hide_an_unlisted_local_header()
    -> Result<(), ImportError> {
        let bytes = make_zip(&[
            ("first.bin", b"first", CompressionMethod::Stored),
            ("second.bin", b"second", CompressionMethod::Stored),
        ])?;
        let archive = ZipArchive::new(Cursor::new(&bytes))
            .map_err(|_error| ImportError::InvalidZip)?;
        let directory = archive.central_directory_start();
        let hidden = archive
            .by_index_data(1)
            .map_err(|_error| ImportError::InvalidZip)?
            .central_header_start();
        let size = hidden
            .checked_sub(directory)
            .ok_or(ImportError::InvalidZip)?;
        let offset = usize::try_from(hidden)
            .map_err(|_error| ImportError::InvalidZip)?;
        let mut damaged =
            bytes.get(..offset).ok_or(ImportError::InvalidZip)?.to_vec();
        // Keep both local members but only the first central directory entry.
        damaged.extend_from_slice(b"PK\x05\x06");
        damaged.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        damaged.extend_from_slice(
            &u32::try_from(size)
                .map_err(|_error| ImportError::InvalidZip)?
                .to_le_bytes(),
        );
        damaged.extend_from_slice(
            &u32::try_from(directory)
                .map_err(|_error| ImportError::InvalidZip)?
                .to_le_bytes(),
        );
        damaged.extend_from_slice(&[0, 0]);
        let one_entry = ArchiveBudget::new(bound(1024), bound(4096), bound(1));
        require(inspect(&damaged, one_entry) == Err(ImportError::InvalidZip))
    }

    #[test]
    fn late_metadata_failure_precedes_payload_decoding()
    -> Result<(), ImportError> {
        for (late_name, rejection) in [
            ("loader.exe", ArchiveError::Executable.into()),
            ("FIRST.BIN", ImportError::Collision),
        ] {
            let mut bytes = make_zip(&[
                ("first.bin", b"unique-payload", CompressionMethod::Stored),
                (late_name, b"later", CompressionMethod::Stored),
            ])?;
            replace_first(&mut bytes, b"unique-payload", b"unique-payloae")?;
            let expected = identity();
            let source = SourceSnapshot::new(&bytes, &expected, bound(65_536))?;
            let mut called = false;
            let result = source.visit_flat_zip(budget(), |_name, _payload| {
                called = true;
                Ok(())
            });
            require(result == Err(rejection) && !called)?;
        }
        Ok(())
    }

    #[test]
    fn decoded_buffer_growth_is_bounded_by_declared_member()
    -> Result<(), ImportError> {
        // A short second chunk would ordinarily double an 8192-byte buffer.
        let bytes = vec![0x5a; 9001];
        let mut budget = ArchiveBudget::new(bound(9001), bound(9005), bound(2));
        let mut payload = Vec::new();
        let exact = budget.admit("exact.bin", EntryKind::File, 9001)?;
        decode_member(&mut Cursor::new(&bytes), exact, &mut payload)?;
        require(payload == bytes)?;
        let capacity = payload.capacity();
        require(capacity == 9001)?;
        let smaller = budget.admit("small.bin", EntryKind::File, 4)?;
        decode_member(&mut Cursor::new(b"tiny"), smaller, &mut payload)?;
        require(payload == b"tiny")?;
        require(payload.capacity() == capacity)
    }

    #[test]
    fn corrupt_payload_never_reaches_converter() -> Result<(), ImportError> {
        let mut bytes = make_zip(&[(
            "sample.bin",
            b"unique-payload",
            CompressionMethod::Stored,
        )])?;
        replace_first(&mut bytes, b"unique-payload", b"unique-payloae")?;
        let expected = identity();
        let source = SourceSnapshot::new(&bytes, &expected, bound(65_536))?;
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
        let expected = identity();
        let source = SourceSnapshot::new(&bytes, &expected, bound(65_536))?;
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
        let damaged_identity = identity();
        let damaged =
            SourceSnapshot::new(&bytes, &damaged_identity, bound(65_536))?;
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
