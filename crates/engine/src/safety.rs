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
//   - Generic bounds, archive-entry policy and save integrity checks.
// - Must-Not:
//   - Extract archives, execute binaries, traverse host paths or load mods.
// - Allows:
//   - Inputs: Untrusted entry metadata and memory-resident save payloads.
//   - Outputs: Explicit rejection reasons and verified payload slices.
//   - Side effects: None outside bounded in-memory buffers.
// - Split-When:
//   - Platform extraction or filesystem access requires an adapter.
// - Merge-When:
//   - One shared hostile-input gate replaces these generic invariants.
// - Summary:
//   - Shared input safety primitives for importers and save adapters.
// - Description:
//   - Rejects unsafe archive members and corrupted save envelopes.
// - Usage:
//   - Validate entries before extraction and saves before decoding.
// - Defaults:
//   - Deny links and executables; enforce caller-supplied byte limits.
//
//! Pure safety checks for untrusted archive metadata and save data.

use std::num::NonZeroU64;

use sha2::{Digest as _, Sha256};

const SAVE_HEADER: usize = 40;
const SAVE_MAGIC: &[u8] = b"GMS1";

/// Untrusted archive member classification supplied by an archive adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    /// An ordinary directory with no payload.
    Directory,
    /// An ordinary data file that cannot be executed by the importer.
    File,
    /// An archive hardlink, forbidden in extracted assets.
    Hardlink,
    /// Any unsupported filesystem member type.
    Other,
    /// An archive symbolic link, forbidden in extracted assets.
    Symlink,
}

/// Rejections that must occur before filesystem access or save decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafetyError {
    /// An archive entry attempts to install a runnable program or script.
    Executable,
    /// Directory metadata or the streamed payload length is inconsistent.
    InvalidEntry,
    /// Save checksum, declared length or format header is invalid.
    InvalidSave,
    /// The archive entry is a hardlink, symlink or unsupported kind.
    LinkOrSpecial,
    /// Resource size or save buffer exceeds a configured bound.
    TooLarge,
    /// The archive entry path is empty, absolute or traverses directories.
    UnsafePath,
}

/// Constant-memory admission limits for a single untrusted archive.
///
/// Reserve each entry before decoding its contents. Abort the import on any
/// admission, streaming or filesystem error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveBudget {
    max_entry_bytes: NonZeroU64,
    remaining_bytes: u64,
    remaining_entries: u64,
}

impl ArchiveBudget {
    /// Admit a member and reserve its entire declared uncompressed size.
    ///
    /// Rejected entries do not consume the budget.
    ///
    /// # Errors
    ///
    /// Rejects invalid entries and exhaustion of either aggregate limit.
    pub fn admit(
        &mut self,
        path: &str,
        kind: EntryKind,
        size: u64,
    ) -> Result<EntryBudget, SafetyError> {
        validate_entry(path, kind, size, self.max_entry_bytes)?;
        let bytes = self
            .remaining_bytes
            .checked_sub(size)
            .ok_or(SafetyError::TooLarge)?;
        let entries = self
            .remaining_entries
            .checked_sub(1)
            .ok_or(SafetyError::TooLarge)?;
        self.remaining_bytes = bytes;
        self.remaining_entries = entries;
        Ok(EntryBudget { remaining: Some(size) })
    }

    /// Configure individual, aggregate and entry-count limits explicitly.
    #[must_use]
    pub const fn new(
        max_entry_bytes: NonZeroU64,
        max_total_bytes: NonZeroU64,
        max_entries: NonZeroU64,
    ) -> Self {
        Self {
            max_entry_bytes,
            remaining_bytes: max_total_bytes.get(),
            remaining_entries: max_entries.get(),
        }
    }
}

/// Tracks actual decoded bytes against one admitted archive entry.
///
/// Check each decoded chunk before writing it to the destination. An overrun
/// permanently invalidates this entry, even if later chunks are smaller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryBudget {
    remaining: Option<u64>,
}

impl EntryBudget {
    /// Verify the stream produced exactly the declared number of bytes.
    ///
    /// # Errors
    ///
    /// Rejects truncated streams and previously detected overruns.
    pub fn finish(self) -> Result<(), SafetyError> {
        if self.remaining == Some(0) {
            Ok(())
        } else {
            Err(SafetyError::InvalidEntry)
        }
    }

    /// Account for a decoded chunk before allowing it to be written.
    ///
    /// # Errors
    ///
    /// Rejects any chunk exceeding the remaining declared length.
    pub fn record(&mut self, bytes: usize) -> Result<(), SafetyError> {
        self.remaining = self.remaining.and_then(|remaining| {
            remaining.checked_sub(u64::try_from(bytes).ok()?)
        });
        self.remaining.ok_or(SafetyError::TooLarge).map(|_value| ())
    }
}

/// Validate archive metadata before allocating or extracting an entry.
///
/// Use a symlink-safe, non-overwriting filesystem API after this lexical check.
/// Archive-wide and streamed payload limits are enforced by [`ArchiveBudget`].
///
/// # Errors
///
/// Rejects links, unsafe paths, executable extensions and invalid sizes.
pub fn validate_entry(
    path: &str,
    kind: EntryKind,
    size: u64,
    max_size: NonZeroU64,
) -> Result<(), SafetyError> {
    if size > max_size.get() {
        return Err(SafetyError::TooLarge);
    }
    if !matches!(kind, EntryKind::File | EntryKind::Directory) {
        return Err(SafetyError::LinkOrSpecial);
    }
    if kind == EntryKind::Directory && size != 0 {
        return Err(SafetyError::InvalidEntry);
    }
    if path.len() > 4096 || path.split('/').any(unsafe_component) {
        return Err(SafetyError::UnsafePath);
    }
    if kind == EntryKind::File && executable_extension(path) {
        return Err(SafetyError::Executable);
    }
    Ok(())
}

fn executable_extension(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_stem, suffix)| {
        [
            "exe", "dll", "so", "dylib", "sh", "bat", "cmd", "ps1", "jar",
            "wasm", "com", "scr",
        ]
        .iter()
        .any(|extension| suffix.eq_ignore_ascii_case(extension))
    })
}

fn unsafe_component(part: &str) -> bool {
    if part.is_empty()
        || part.len() > 255
        || part.ends_with(['.', ' '])
        || part.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || part.chars().any(char::is_control)
    {
        return true;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    reserved_device(stem)
}

fn reserved_device(stem: &str) -> bool {
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    let Some((prefix, suffix)) = stem.split_at_checked(3) else {
        return false;
    };
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(
            suffix,
            "1" | "2"
                | "3"
                | "4"
                | "5"
                | "6"
                | "7"
                | "8"
                | "9"
                | "\u{b9}"
                | "\u{b2}"
                | "\u{b3}"
        )
}

fn checked_save(
    payload: &[u8],
    max_size: NonZeroU64,
) -> Result<u32, SafetyError> {
    if u64::try_from(payload.len()).unwrap_or(u64::MAX) > max_size.get() {
        return Err(SafetyError::TooLarge);
    }
    u32::try_from(payload.len()).map_err(|_error| SafetyError::TooLarge)
}

/// Encode a portable, integrity-checked save envelope with a size budget.
///
/// # Errors
///
/// Rejects payloads exceeding the configured maximum or format capacity.
pub fn encode_save(
    payload: &[u8],
    max_size: NonZeroU64,
) -> Result<Vec<u8>, SafetyError> {
    let length = checked_save(payload, max_size)?;
    let capacity = payload
        .len()
        .checked_add(SAVE_HEADER)
        .ok_or(SafetyError::TooLarge)?;
    let mut data = Vec::with_capacity(capacity);
    data.extend_from_slice(SAVE_MAGIC);
    data.extend_from_slice(&length.to_le_bytes());
    data.extend_from_slice(&Sha256::digest(payload));
    data.extend_from_slice(payload);
    Ok(data)
}

/// Validate and decode a portable save without trusting its declared size.
///
/// # Errors
///
/// Rejects truncated headers, wrong versions, excessive sizes and tampering.
pub fn decode_save(
    data: &[u8],
    max_size: NonZeroU64,
) -> Result<&[u8], SafetyError> {
    let magic = data.get(..4).ok_or(SafetyError::InvalidSave)?;
    if magic != SAVE_MAGIC {
        return Err(SafetyError::InvalidSave);
    }
    let declared = data.get(4..8).ok_or(SafetyError::InvalidSave)?;
    let length: [u8; 4] = declared
        .try_into()
        .map_err(|_error| SafetyError::InvalidSave)?;
    let checksum = data.get(8..SAVE_HEADER).ok_or(SafetyError::InvalidSave)?;
    let payload = data.get(SAVE_HEADER..).ok_or(SafetyError::InvalidSave)?;
    let actual = checked_save(payload, max_size)?;
    if u32::from_le_bytes(length) != actual
        || checksum != Sha256::digest(payload).as_slice()
    {
        return Err(SafetyError::InvalidSave);
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{
        ArchiveBudget, EntryKind, SafetyError, decode_save, encode_save,
        validate_entry,
    };

    fn limit(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)
    }

    #[test]
    fn archive_entries_reject_traversal_drive_paths_and_control_codes() {
        for name in [
            "../escape",
            "a/../../escape",
            "/etc/passwd",
            "C:/Windows",
            "\\\\server\\path",
            "a\\..\\evil",
            "a//b",
            "a/./b",
            "a/",
            "",
            "a\0b",
            "disk:stream",
        ] {
            assert_eq!(
                validate_entry(name, EntryKind::File, 1, limit(100)),
                Err(SafetyError::UnsafePath),
                "{name:?}",
            );
        }
        assert_eq!(
            validate_entry(
                "textures/menu.png",
                EntryKind::File,
                10,
                limit(100)
            ),
            Ok(()),
        );
    }

    #[test]
    fn links_special_files_and_executables_are_rejected() {
        for kind in [EntryKind::Symlink, EntryKind::Hardlink, EntryKind::Other]
        {
            assert_eq!(
                validate_entry("data/icon.png", kind, 2, limit(100)),
                Err(SafetyError::LinkOrSpecial),
            );
        }
        for name in ["run.EXE", "scripts/start.sh", "mod.wasm", "plugin.DLL"] {
            assert_eq!(
                validate_entry(name, EntryKind::File, 2, limit(100)),
                Err(SafetyError::Executable),
            );
        }
    }

    #[test]
    fn oversized_files_are_rejected_before_allocation() {
        assert_eq!(
            validate_entry(
                "textures/huge.png",
                EntryKind::File,
                101,
                limit(100)
            ),
            Err(SafetyError::TooLarge),
        );
        assert_eq!(
            encode_save(b"oversize", limit(2)),
            Err(SafetyError::TooLarge),
        );
    }

    #[test]
    fn valid_save_roundtrips_and_corruption_is_rejected() {
        let original = b"checkpoint;health=100;score=4";
        let encoded = encode_save(original, limit(1024));
        assert!(encoded.is_ok(), "payload fits the configured budget");
        let Ok(save) = encoded else {
            return;
        };
        assert_eq!(decode_save(&save, limit(1024)), Ok(original.as_slice()));
        assert_eq!(decode_save(&save, limit(2)), Err(SafetyError::TooLarge));
        for index in [0, 4, 8, 39, 40] {
            let mut modified = save.clone();
            if let Some(byte) = modified.get_mut(index) {
                *byte ^= 1;
            }
            assert_eq!(
                decode_save(&modified, limit(1024)),
                Err(SafetyError::InvalidSave),
            );
        }
        assert_eq!(
            decode_save(save.get(..3).unwrap_or(&[]), limit(1024)),
            Err(SafetyError::InvalidSave)
        );
    }

    #[test]
    fn every_truncated_save_prefix_is_rejected_without_panicking() {
        let encoded = encode_save(b"durable state", limit(1024));
        assert!(encoded.is_ok(), "save fixture must be encodable");
        let Ok(save) = encoded else {
            return;
        };
        for prefix_length in 0..save.len() {
            if let Some(prefix) = save.get(..prefix_length) {
                assert_eq!(
                    decode_save(prefix, limit(1024)),
                    Err(SafetyError::InvalidSave),
                );
            }
        }
        assert_eq!(
            decode_save(&save, limit(1024)),
            Ok(b"durable state".as_slice())
        );
    }

    #[test]
    fn an_empty_save_is_valid_only_with_an_intact_envelope() {
        let encoded = encode_save(b"", limit(1));
        assert!(encoded.is_ok(), "empty save is valid data");
        if let Ok(mut save) = encoded {
            assert_eq!(decode_save(&save, limit(1)), Ok(b"".as_slice()));
            if let Some(magic) = save.get_mut(3) {
                *magic = b'0';
            }
            assert_eq!(
                decode_save(&save, limit(1)),
                Err(SafetyError::InvalidSave)
            );
        }
    }

    #[test]
    fn portable_paths_reject_aliases_and_overlong_components() {
        for path in [
            "textures/CON.png",
            "AUX/folder.png",
            "Lpt9.dat",
            "com\u{b2}.bin",
            "nul .png",
            "folder./file.dat",
            "file.dat ",
            "run.exe.",
            "folder/.. /file",
            "a\nb",
            "a\u{7f}b",
            "a?b",
            "a<b",
            "a|b",
        ] {
            assert_eq!(
                validate_entry(path, EntryKind::File, 0, limit(1)),
                Err(SafetyError::UnsafePath),
                "{path:?}",
            );
        }
        let long_component = "a".repeat(256);
        let long_path = "a/".repeat(2049);
        for path in [&long_component, &long_path] {
            assert_eq!(
                validate_entry(path, EntryKind::File, 0, limit(1)),
                Err(SafetyError::UnsafePath),
            );
        }
        for path in ["com10.dat", "auxiliary.png", "level 1/tiles.bin"] {
            assert_eq!(
                validate_entry(path, EntryKind::File, 0, limit(1)),
                Ok(()),
            );
        }
        assert_eq!(
            validate_entry("sprites", EntryKind::Directory, 1, limit(1)),
            Err(SafetyError::InvalidEntry),
        );
    }

    #[test]
    fn aggregate_archive_limits_are_atomic_and_include_empty_members() {
        let results: Result<_, SafetyError> = (|| {
            let mut budget = ArchiveBudget::new(limit(8), limit(10), limit(3));
            let mut first = budget.admit("first", EntryKind::File, 7)?;
            first.record(7)?;
            first.finish()?;
            let excessive = budget.admit("too-big", EntryKind::File, 4);
            let invalid = budget.admit("../escape", EntryKind::File, 1);
            let mut second = budget.admit("second", EntryKind::File, 3)?;
            second.record(1)?;
            second.record(2)?;
            second.finish()?;
            budget
                .admit("directory", EntryKind::Directory, 0)?
                .finish()?;
            let empty = budget.admit("empty", EntryKind::File, 0);
            Ok([excessive, invalid, empty])
        })();
        assert_eq!(
            results,
            Ok([
                Err(SafetyError::TooLarge),
                Err(SafetyError::UnsafePath),
                Err(SafetyError::TooLarge),
            ]),
        );
    }

    #[test]
    fn declared_payload_length_is_enforced_during_streaming() {
        let results: Result<_, SafetyError> = (|| {
            let mut budget = ArchiveBudget::new(limit(8), limit(24), limit(3));
            let mut overrun = budget.admit("overrun", EntryKind::File, 4)?;
            overrun.record(2)?;
            let overflow = overrun.record(3);
            let retry = overrun.record(2);
            let invalid = overrun.finish();
            let mut truncated =
                budget.admit("truncated", EntryKind::File, 4)?;
            truncated.record(3)?;
            let short = truncated.finish();
            let mut exact = budget.admit("exact", EntryKind::File, 8)?;
            exact.record(0)?;
            exact.record(3)?;
            exact.record(5)?;
            Ok([overflow, retry, invalid, short, exact.finish()])
        })();
        assert_eq!(
            results,
            Ok([
                Err(SafetyError::TooLarge),
                Err(SafetyError::TooLarge),
                Err(SafetyError::InvalidEntry),
                Err(SafetyError::InvalidEntry),
                Ok(()),
            ]),
        );
    }
}
