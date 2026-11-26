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
//   - Portable save-envelope size, encoding and integrity admission.
// - Must-Not:
//   - Open host paths, expose descriptors or interpret title payloads.
// - Allows:
//   - Inputs: Explicit byte budget, payload and independently sized stream.
//   - Outputs: Prepared envelopes, verified payloads and typed failures.
//   - Side effects: Caller-owned stream reads and bounded allocation.
// - Split-When:
//   - Another storage service requires independent envelope admission.
// - Merge-When:
//   - Another portable policy enforces identical envelope admission.
// - Summary:
//   - Admit complete save envelopes without native storage access.
// - Description:
//   - Bind admitted headers to payloads and verify exact read results.
// - Usage:
//   - Prepare a payload or verify a stream before storage dispatch.
// - Defaults:
//   - Reject malformed envelopes, excess sizes and integrity failures.

//! Shared portable save-envelope admission.

use std::io::{Error, ErrorKind, Read};
use std::num::NonZeroU64;

use games::safety::{
    SAVE_HEADER, SafetyError, admit_save_size, decode_save, encode_save_header,
    save_envelope_limit, validate_save_header,
};

use super::file::{FileReadError, admit_size, read_exact_snapshot};

/// Save admission or pre-publication failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveError {
    /// The save envelope is invalid or exceeds its payload limit.
    Envelope(SafetyError),
    /// A host storage operation failed before publication.
    Io(ErrorKind),
    /// Bounded file admission rejected the requested slot.
    Read(FileReadError),
    /// The selected storage location or destination is unsafe.
    UnsafeStorage,
}

impl From<Error> for SaveError {
    fn from(error: Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<SafetyError> for SaveError {
    fn from(error: SafetyError) -> Self {
        Self::Envelope(error)
    }
}

impl From<FileReadError> for SaveError {
    fn from(error: FileReadError) -> Self {
        Self::Read(error)
    }
}

/// One encoded, admitted save with a borrowed, unchanged payload.
///
/// The separate header avoids allocating a second copy of the payload.
#[derive(Debug)]
pub struct PreparedSave<'payload> {
    header: [u8; SAVE_HEADER],
    payload: &'payload [u8],
    size: u64,
}

impl<'payload> PreparedSave<'payload> {
    /// Exact encoded byte count, including the fixed header.
    #[must_use]
    pub const fn encoded_size(&self) -> u64 {
        self.size
    }

    /// Canonical fixed-width encoded header.
    #[must_use]
    pub const fn header(&self) -> &[u8; SAVE_HEADER] {
        &self.header
    }

    /// Prepare the complete envelope before dispatching a slot write.
    ///
    /// # Errors
    /// Rejects payloads exceeding the requested or format limit.
    pub fn new(
        payload: &'payload [u8],
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        let header = encode_save_header(payload, maximum)?;
        let size = header
            .len()
            .checked_add(payload.len())
            .and_then(|length| u64::try_from(length).ok())
            .ok_or(SafetyError::TooLarge)?;
        Ok(Self { header, payload, size })
    }

    /// Exact borrowed bytes following the encoded header.
    #[must_use]
    pub const fn payload(&self) -> &'payload [u8] {
        self.payload
    }
}

/// A complete save payload whose header and checksum have been verified.
#[derive(Debug)]
pub struct VerifiedSave(Vec<u8>);

impl VerifiedSave {
    /// Verify an owned memory-resident envelope and reuse its allocation.
    ///
    /// # Errors
    /// Rejects malformed, corrupt or oversized envelopes.
    pub fn from_encoded(
        mut bytes: Vec<u8>,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        let length = decode_save(&bytes, maximum)?.len();
        bytes.copy_within(SAVE_HEADER.., 0);
        bytes.truncate(length);
        Ok(Self(bytes))
    }

    /// Transfer the verified payload without copying it.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> {
        self.0
    }

    /// Verify one encoded save from an independently sized stream.
    ///
    /// # Errors
    /// Rejects oversized, truncated, extended or corrupt envelopes.
    pub fn read(
        input: &mut dyn Read,
        size: u64,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        admit_size(size, save_envelope_limit(maximum))?;
        let header_size = u64::try_from(SAVE_HEADER).unwrap_or(u64::MAX);
        let length = size
            .checked_sub(header_size)
            .ok_or(SafetyError::InvalidSave)?;
        let mut bytes = [0; SAVE_HEADER];
        input.read_exact(&mut bytes).map_err(FileReadError::from)?;
        let header = validate_save_header(&bytes, length, maximum)?;
        let mut payload = Vec::new();
        read_exact_snapshot(input, length, &mut payload)?;
        header.verify(&payload)?;
        Ok(Self(payload))
    }
}

/// Admit a verified payload against the requesting store's byte limit.
pub(crate) fn admit_payload(
    payload: &[u8],
    maximum: NonZeroU64,
) -> Result<(), SaveError> {
    let size =
        u64::try_from(payload.len()).map_err(|_error| SafetyError::TooLarge)?;
    let _length = admit_save_size(size, maximum)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use games::safety::encode_save;

    use super::*;

    #[test]
    fn prepared_save_preserves_wire_format_and_payload_limit()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN.saturating_add(3);
        for payload in [b"".as_slice(), b"four"] {
            let prepared = PreparedSave::new(payload, maximum)?;
            let encoded = encode_save(payload, maximum)?;
            let mut stream =
                prepared.header().as_slice().chain(prepared.payload());
            let verified = VerifiedSave::read(
                &mut stream,
                prepared.encoded_size(),
                maximum,
            )?;
            if encoded.get(..SAVE_HEADER) != Some(prepared.header().as_slice())
                || verified.into_payload() != payload
            {
                return Err(SaveError::UnsafeStorage);
            }
        }
        if PreparedSave::new(b"large", maximum).err()
            != Some(SafetyError::TooLarge.into())
            || admit_payload(b"large", maximum)
                != Err(SafetyError::TooLarge.into())
        {
            return Err(SaveError::UnsafeStorage);
        }
        Ok(())
    }

    #[test]
    fn portable_stream_admission_rejects_size_before_reading()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN;
        let limit = save_envelope_limit(maximum);
        let mut input = b"unchanged".as_slice();
        let result =
            VerifiedSave::read(&mut input, limit.saturating_add(1), maximum);
        if result.err() != Some(FileReadError::TooLarge.into())
            || input != b"unchanged"
        {
            return Err(SaveError::UnsafeStorage);
        }

        let header = encode_save_header(b"", maximum)?;
        let mut stream = header.as_slice();
        let malformed = VerifiedSave::read(&mut stream, limit, maximum);
        (malformed.err() == Some(SafetyError::InvalidSave.into()))
            .then_some(())
            .ok_or(SaveError::UnsafeStorage)
    }

    #[test]
    fn portable_stream_admission_rejects_truncation_and_trailing_bytes()
    -> Result<(), SaveError> {
        let maximum = NonZeroU64::MIN;
        let bytes = encode_save(b"x", maximum)?;
        let size = save_envelope_limit(maximum);
        let mut trailing = bytes.clone();
        trailing.push(0);
        let truncated = bytes
            .get(..bytes.len().saturating_sub(1))
            .ok_or(SafetyError::InvalidSave)?;
        let cases = [
            (truncated, FileReadError::Io(ErrorKind::UnexpectedEof)),
            (trailing.as_slice(), FileReadError::TooLarge),
        ];
        for (mut input, error) in cases {
            let observed = VerifiedSave::read(&mut input, size, maximum);
            if observed.err() != Some(error.into()) {
                return Err(SaveError::UnsafeStorage);
            }
        }
        Ok(())
    }
}
