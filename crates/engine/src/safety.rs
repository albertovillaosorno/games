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
//   - Portable save envelopes and integrity checks.
// - Must-Not:
//   - Extract archives, execute binaries or access the host filesystem.
// - Allows:
//   - Inputs: Memory-resident save payloads.
//   - Outputs: Explicit rejection reasons and verified payload slices.
//   - Side effects: None outside bounded in-memory buffers.
// - Split-When:
//   - Filesystem access requires a platform adapter.
// - Merge-When:
//   - One shared hostile-input gate replaces these generic invariants.
// - Summary:
//   - Shared save integrity primitives.
// - Description:
//   - Rejects corrupted or oversized save envelopes.
// - Usage:
//   - Validate saves before decoding.
// - Defaults:
//   - Enforce caller-supplied byte limits.
//
//! Pure safety checks for untrusted save data.

use std::num::NonZeroU64;

use sha2::{Digest as _, Sha256};

/// Fixed byte length of a portable save envelope header.
pub const SAVE_HEADER: usize = 40;
const SAVE_MAGIC: &[u8] = b"GMS1";

/// Rejections before save decoding or allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafetyError {
    /// Save checksum, declared length or format header is invalid.
    InvalidSave,
    /// Resource size or save buffer exceeds a configured bound.
    TooLarge,
}

/// An admitted save header; the payload remains untrusted until verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveHeader {
    checksum: [u8; 32],
    length: u32,
}

impl SaveHeader {
    /// Verify exact payload length and checksum.
    ///
    /// # Errors
    /// Rejects any payload that does not match the admitted header.
    pub fn verify(self, payload: &[u8]) -> Result<(), SafetyError> {
        if payload_size(payload) != u64::from(self.length)
            || self.checksum.as_slice() != Sha256::digest(payload).as_slice()
        {
            return Err(SafetyError::InvalidSave);
        }
        Ok(())
    }
}

/// Admit an observed payload length against the budget and envelope format.
///
/// # Errors
/// Rejects lengths exceeding the configured or format maximum.
pub fn admit_save_size(
    size: u64,
    max_size: NonZeroU64,
) -> Result<u32, SafetyError> {
    if size > max_size.get() {
        return Err(SafetyError::TooLarge);
    }
    u32::try_from(size).map_err(|_error| SafetyError::TooLarge)
}

fn payload_size(payload: &[u8]) -> u64 {
    u64::try_from(payload.len()).unwrap_or(u64::MAX)
}

/// Maximum encoded bytes accepted for a payload limit.
#[must_use]
pub fn save_envelope_limit(max_size: NonZeroU64) -> u64 {
    max_size
        .get()
        .min(u64::from(u32::MAX))
        .saturating_add(u64::try_from(SAVE_HEADER).unwrap_or(u64::MAX))
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
    let header = encode_save_header(payload, max_size)?;
    let capacity = payload
        .len()
        .checked_add(SAVE_HEADER)
        .ok_or(SafetyError::TooLarge)?;
    let mut data = Vec::new();
    data.try_reserve_exact(capacity)
        .map_err(|_error| SafetyError::TooLarge)?;
    data.extend_from_slice(&header);
    data.extend_from_slice(payload);
    Ok(data)
}

/// Encode only the fixed header of a portable save envelope.
///
/// Follow these bytes with the exact borrowed payload to produce the same
/// envelope as [`encode_save`], without allocating a second payload buffer.
///
/// # Errors
/// Rejects payloads exceeding the configured maximum or format capacity.
pub fn encode_save_header(
    payload: &[u8],
    max_size: NonZeroU64,
) -> Result<[u8; SAVE_HEADER], SafetyError> {
    let length = admit_save_size(payload_size(payload), max_size)?;
    let mut header = [0u8; SAVE_HEADER];
    let (magic, remainder) = header.split_at_mut(4);
    let (declared, checksum) = remainder.split_at_mut(4);
    magic.copy_from_slice(SAVE_MAGIC);
    declared.copy_from_slice(&length.to_le_bytes());
    checksum.copy_from_slice(&Sha256::digest(payload));
    Ok(header)
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
    let (header, payload) = data
        .split_at_checked(SAVE_HEADER)
        .ok_or(SafetyError::InvalidSave)?;
    validate_save_header(header, payload_size(payload), max_size)?
        .verify(payload)?;
    Ok(payload)
}

/// Admit a fixed save header against an independently observed payload size.
///
/// This check can precede allocation; it does not authenticate the payload.
///
/// # Errors
/// Rejects malformed headers, mismatched lengths and excessive sizes.
pub fn validate_save_header(
    header: &[u8],
    size: u64,
    max_size: NonZeroU64,
) -> Result<SaveHeader, SafetyError> {
    if header.len() != SAVE_HEADER || header.get(..4) != Some(SAVE_MAGIC) {
        return Err(SafetyError::InvalidSave);
    }
    let declared = header.get(4..8).ok_or(SafetyError::InvalidSave)?;
    let length = u32::from_le_bytes(
        declared
            .try_into()
            .map_err(|_error| SafetyError::InvalidSave)?,
    );
    if length != admit_save_size(size, max_size)? {
        return Err(SafetyError::InvalidSave);
    }
    let checksum = header
        .get(8..SAVE_HEADER)
        .ok_or(SafetyError::InvalidSave)?
        .try_into()
        .map_err(|_error| SafetyError::InvalidSave)?;
    Ok(SaveHeader { checksum, length })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{
        SafetyError, decode_save, encode_save, encode_save_header,
        save_envelope_limit, validate_save_header,
    };

    fn limit(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)
    }

    #[test]
    fn encoded_read_limit_accounts_for_envelope_and_format_capacity() {
        let maximum = limit(4);
        let encoded = encode_save(b"four", maximum);
        assert_eq!(
            encoded
                .ok()
                .and_then(|bytes| u64::try_from(bytes.len()).ok()),
            Some(save_envelope_limit(maximum)),
        );
        assert_eq!(save_envelope_limit(limit(u64::MAX)), 4_294_967_335);
    }

    #[test]
    fn header_admission_checks_size_before_payload_verification() {
        let encoded = encode_save_header(b"four", limit(4));
        assert!(encoded.is_ok());
        let Ok(header) = encoded else {
            return;
        };
        let validated = validate_save_header(&header, 4, limit(4));
        assert!(validated.is_ok());
        let Ok(admitted) = validated else {
            return;
        };
        assert_eq!(admitted.verify(b"four"), Ok(()));
        assert_eq!(admitted.verify(b"five"), Err(SafetyError::InvalidSave));
        assert_eq!(admitted.verify(b""), Err(SafetyError::InvalidSave));
        assert_eq!(
            validate_save_header(&header, 3, limit(4)),
            Err(SafetyError::InvalidSave)
        );
        assert_eq!(
            validate_save_header(&header, 5, limit(4)),
            Err(SafetyError::TooLarge)
        );
    }

    #[test]
    fn oversized_saves_are_rejected_before_allocation() {
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
