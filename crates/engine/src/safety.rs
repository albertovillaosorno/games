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
#[path = "../../../tests/engine/safety.rs"]
mod tests;
