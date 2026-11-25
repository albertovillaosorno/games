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
    /// Save checksum, declared length or format header is invalid.
    InvalidSave,
    /// The archive entry is a hardlink, symlink or unsupported kind.
    LinkOrSpecial,
    /// Resource size or save buffer exceeds a configured bound.
    TooLarge,
    /// The archive entry path is empty, absolute or traverses directories.
    UnsafePath,
}

/// Validate archive metadata before allocating or extracting an entry.
///
/// The importer must enforce the same size budget while streaming bytes,
/// and must use a symlink-safe filesystem API after this lexical check.
///
/// # Errors
///
/// Rejects links, traversal, unsafe file types or excessive entry sizes.
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
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':', '\0'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(SafetyError::UnsafePath);
    }
    let lower = path.to_ascii_lowercase();
    if kind == EntryKind::File
        && [
            ".exe", ".dll", ".so", ".dylib", ".sh", ".bat", ".cmd", ".ps1",
            ".jar", ".wasm", ".com", ".scr",
        ]
        .iter()
        .any(|extension| lower.ends_with(extension))
    {
        return Err(SafetyError::Executable);
    }
    Ok(())
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
        EntryKind, SafetyError, decode_save, encode_save, validate_entry,
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
}
