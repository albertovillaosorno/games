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
//   - Incremental bounded transport framing over injected byte operations.
// - Must-Not:
//   - Open sockets, read clocks, authenticate peers or decode game actions.
// - Allows:
//   - Inputs: Admitted limits, payload bytes and explicit byte operations.
//   - Outputs: Complete frames, write progress and typed I/O failures.
//   - Side effects: Bounded in-memory frame storage.
// - Split-When:
//   - Another wire protocol has independent framing requirements.
// - Merge-When:
//   - A shared framing contract already enforces these wire boundaries.
// - Summary:
//   - Preserve exact frame boundaries independently of transport or host.
// - Description:
//   - Allocate for received bytes only and never interleave outgoing frames.
// - Usage:
//   - Advance one admitted frame using the owning adapter's byte operations.
// - Defaults:
//   - Invalid framing fails; interrupted and pending operations retain
//     progress.

//! Incremental transport-neutral framing for the shared LAN service.

use core::result::Result as AdmissionResult;
use std::io::{Error, ErrorKind, Result};

use super::{LanAdmissionError, LanLimits};

const RECEIVE_CHUNK: usize = 4096;

#[derive(Debug)]
struct PendingFrame {
    bytes: Vec<u8>,
    length: usize,
}

impl PendingFrame {
    fn advance<Read>(&mut self, read: Read) -> Result<bool>
    where
        Read: FnOnce(&mut [u8]) -> Result<usize>,
    {
        // A length header alone must not allocate the advertised payload.
        // Read only a bounded chunk, then reserve for actual received bytes.
        let remaining = self.length.saturating_sub(self.bytes.len());
        let mut scratch = [0u8; RECEIVE_CHUNK];
        let target = scratch
            .get_mut(..remaining.min(RECEIVE_CHUNK))
            .ok_or(ErrorKind::InvalidData)?;
        let count = read_frame_chunk(target, read)?;
        let received = target.get(..count).ok_or(ErrorKind::InvalidData)?;
        let required = self
            .bytes
            .len()
            .checked_add(count)
            .ok_or(ErrorKind::InvalidData)?;
        if required > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(required)
                .min(self.length);
            self.bytes
                .try_reserve_exact(capacity.saturating_sub(self.bytes.len()))
                .map_err(|_error| ErrorKind::OutOfMemory)?;
        }
        self.bytes.extend_from_slice(received);
        Ok(self.bytes.len() == self.length)
    }
}

/// Incremental decoder with an immutable admitted incoming frame budget.
#[derive(Debug)]
pub struct FrameReader {
    failed: bool,
    filled: usize,
    header: [u8; 4],
    limits: LanLimits,
    payload: Option<PendingFrame>,
}

impl FrameReader {
    /// Advance at most one header or bounded payload operation.
    ///
    /// Returns a complete frame, including an explicitly empty payload, or
    /// absence while more bytes are required. No following frame is consumed.
    ///
    /// # Errors
    /// Propagates the byte operation's failure. An invalid count, incomplete
    /// end of stream, excessive length or allocation failure rejects the
    /// current stream. Only pending or interrupted operations can be retried.
    pub fn advance<Read>(&mut self, read: Read) -> Result<Option<Vec<u8>>>
    where
        Read: FnOnce(&mut [u8]) -> Result<usize>,
    {
        if self.failed {
            return Err(ErrorKind::InvalidData.into());
        }
        let result = self.advance_inner(read);
        if result.as_ref().is_err_and(fatal_error) {
            self.failed = true;
            self.payload = None;
        }
        result
    }

    fn advance_inner<Read>(&mut self, read: Read) -> Result<Option<Vec<u8>>>
    where
        Read: FnOnce(&mut [u8]) -> Result<usize>,
    {
        if let Some(payload) = &mut self.payload {
            return if payload.advance(read)? {
                Ok(self.payload.take().map(|frame| frame.bytes))
            } else {
                Ok(None)
            };
        }
        let target = self
            .header
            .get_mut(self.filled..)
            .ok_or(ErrorKind::InvalidData)?;
        let count = read_frame_chunk(target, read)?;
        self.filled = self
            .filled
            .checked_add(count)
            .ok_or(ErrorKind::InvalidData)?;
        if self.filled < self.header.len() {
            return Ok(None);
        }
        self.filled = 0;
        let length = self
            .limits
            .inbound_length(u32::from_be_bytes(self.header))
            .map_err(|_error| ErrorKind::InvalidData)?;
        if length == 0 {
            return Ok(Some(Vec::new()));
        }
        self.payload = Some(PendingFrame {
            bytes: Vec::new(),
            length,
        });
        Ok(None)
    }

    #[cfg(test)]
    pub(crate) const fn header_bytes_received(&self) -> usize {
        self.filled
    }

    /// Begin decoding a new stream with its admitted limits.
    #[must_use]
    pub const fn new(limits: LanLimits) -> Self {
        Self {
            failed: false,
            filled: 0,
            header: [0; 4],
            limits,
            payload: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn payload_bytes_received(&self) -> Option<usize> {
        self.payload.as_ref().map(|frame| frame.bytes.len())
    }
}

/// Progress of the single outstanding outgoing frame on a channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendProgress {
    /// All plaintext and pending transport bytes have been submitted.
    Complete,
    /// No outgoing frame has been queued.
    Idle,
    /// The frame is incomplete or the peer is applying backpressure.
    Pending,
}

/// Portable admission of the single outstanding application frame.
///
/// A successful queue must retain one frame; only its completed submission
/// may release that capacity. This checks adapter receipts independently of
/// native transport framing and deadlines.
#[derive(Debug, Default)]
pub(crate) struct SendAdmission {
    pending: bool,
}

impl SendAdmission {
    pub(crate) fn check(&self, observed: bool) -> Result<()> {
        if self.pending == observed {
            Ok(())
        } else {
            Err(ErrorKind::InvalidData.into())
        }
    }

    pub(crate) const fn pending(&self) -> bool {
        self.pending
    }

    pub(crate) fn progressed(
        &mut self,
        progress: SendProgress,
        observed: bool,
    ) -> Result<()> {
        let expected = match (self.pending, progress) {
            (false, SendProgress::Idle) | (true, SendProgress::Complete) => {
                false
            },
            (true, SendProgress::Pending) => true,
            _ => return Err(ErrorKind::InvalidData.into()),
        };
        if observed != expected {
            return Err(ErrorKind::InvalidData.into());
        }
        self.pending = expected;
        Ok(())
    }

    pub(crate) fn queued(&mut self, observed: bool) -> Result<()> {
        if self.pending || !observed {
            return Err(ErrorKind::InvalidData.into());
        }
        self.pending = true;
        Ok(())
    }

    pub(crate) fn require_idle(&self) -> Result<()> {
        if self.pending {
            Err(ErrorKind::AlreadyExists.into())
        } else {
            Ok(())
        }
    }
}

/// Incremental encoder retaining one admitted owned or borrowed payload.
#[derive(Debug)]
pub struct FrameWriter<Payload> {
    failed: bool,
    header: [u8; 4],
    header_written: usize,
    payload: Payload,
    payload_written: usize,
}

impl<Payload> FrameWriter<Payload>
where
    Payload: AsRef<[u8]>,
{
    /// Advance at most one header or payload write operation.
    ///
    /// Completion means all plaintext bytes have been submitted, not that
    /// encrypted transport bytes have been flushed or the peer has received
    /// them. Callers must not mutate a payload while it is being submitted.
    ///
    /// # Errors
    /// Rejects inconsistent lengths, zero or impossible write counts, and
    /// propagates byte operation failures. A fatal error invalidates the
    /// enclosing stream; only pending or interrupted writes may be retried.
    pub fn advance<Write>(&mut self, write: Write) -> Result<bool>
    where
        Write: FnOnce(&[u8]) -> Result<usize>,
    {
        if self.failed {
            return Err(ErrorKind::InvalidData.into());
        }
        let result = self.advance_inner(write);
        if result.as_ref().is_err_and(fatal_error) {
            self.failed = true;
        }
        result
    }

    fn advance_inner<Write>(&mut self, write: Write) -> Result<bool>
    where
        Write: FnOnce(&[u8]) -> Result<usize>,
    {
        let payload = self.payload.as_ref();
        if u32::try_from(payload.len()) != Ok(u32::from_be_bytes(self.header)) {
            return Err(ErrorKind::InvalidData.into());
        }
        let (bytes, offset) = if self.header_written < self.header.len() {
            (self.header.as_slice(), &mut self.header_written)
        } else {
            (payload, &mut self.payload_written)
        };
        let chunk = bytes.get(*offset..).ok_or(ErrorKind::InvalidData)?;
        if !chunk.is_empty() {
            let count = write(chunk)?;
            if count == 0 {
                return Err(ErrorKind::WriteZero.into());
            }
            if count > chunk.len() {
                return Err(ErrorKind::InvalidData.into());
            }
            *offset =
                offset.checked_add(count).ok_or(ErrorKind::InvalidData)?;
        }
        Ok(self.header_written == self.header.len()
            && self.payload_written == payload.len())
    }

    /// Admit one payload and its wire length before any write operation.
    ///
    /// # Errors
    /// Rejects lengths outside the configured or representable wire budget.
    pub fn new(
        payload: Payload,
        limits: LanLimits,
    ) -> AdmissionResult<Self, LanAdmissionError> {
        let length = limits.outbound_length(payload.as_ref().len())?;
        Ok(Self {
            failed: false,
            header: length.to_be_bytes(),
            header_written: 0,
            payload,
            payload_written: 0,
        })
    }

    /// Submit the remaining plaintext using an injected blocking write port.
    ///
    /// # Errors
    /// Propagates write failures other than interrupted operations.
    pub fn write_all<Write>(&mut self, mut write: Write) -> Result<()>
    where
        Write: FnMut(&[u8]) -> Result<usize>,
    {
        loop {
            match self.advance(&mut write) {
                Ok(true) => return Ok(()),
                Ok(false) => {},
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }
    }
}

fn fatal_error(error: &Error) -> bool {
    !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

fn read_frame_chunk<Read>(target: &mut [u8], read: Read) -> Result<usize>
where
    Read: FnOnce(&mut [u8]) -> Result<usize>,
{
    let count = read(target)?;
    if count == 0 {
        return Err(ErrorKind::UnexpectedEof.into());
    }
    if count > target.len() {
        return Err(ErrorKind::InvalidData.into());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::network::SessionConfig;

    use super::*;

    fn config(maximum: usize) -> Result<LanLimits> {
        LanLimits::new(
            SessionConfig {
                allow_lan: true,
                max_message_bytes: NonZeroUsize::new(maximum)
                    .ok_or(ErrorKind::InvalidInput)?,
                max_players: NonZeroU32::MIN,
            },
            Duration::from_secs(2),
        )
        .map_err(|error| Error::other(format!("{error:?}")))
    }

    fn require(condition: bool, message: &str) -> Result<()> {
        condition
            .then_some(())
            .ok_or_else(|| Error::other(message.to_owned()))
    }

    fn error_kind<Value>(result: Result<Value>) -> Option<ErrorKind> {
        result.err().map(|error| error.kind())
    }
    #[test]
    fn frame_allocation_follows_received_payload() -> Result<()> {
        let mut reader = FrameReader::new(config(1_048_576)?);
        let header = 1_048_576u32.to_be_bytes();
        require(
            reader
                .advance(|target| {
                    target.copy_from_slice(&header);
                    Ok(header.len())
                })?
                .is_none(),
            "header must remain incomplete",
        )?;
        require(
            reader.header_bytes_received() == 0
                && reader.payload_bytes_received() == Some(0),
            "completed header starts a fresh payload",
        )?;
        let pending = reader.payload.as_ref().ok_or(ErrorKind::InvalidData)?;
        require(pending.bytes.capacity() == 0, "no header allocation")?;
        require(
            error_kind(
                reader.advance(|_target| Err(ErrorKind::WouldBlock.into())),
            ) == Some(ErrorKind::WouldBlock),
            "idle payload read",
        )?;
        require(
            reader
                .advance(|target| {
                    let first =
                        target.first_mut().ok_or(ErrorKind::InvalidData)?;
                    *first = 42;
                    Ok(1)
                })?
                .is_none(),
            "partial payload is not delivered",
        )?;
        let partial = reader.payload.as_ref().ok_or(ErrorKind::InvalidData)?;
        require(
            partial.bytes == [42]
                && reader.payload_bytes_received() == Some(1)
                && partial.bytes.capacity() < RECEIVE_CHUNK,
            "allocation follows actual payload instead of declared length",
        )
    }

    #[test]
    fn chunked_frames_preserve_exact_boundaries() -> Result<()> {
        let length = RECEIVE_CHUNK.saturating_mul(2).saturating_add(1);
        let mut wire = u32::try_from(length)
            .map_err(|_error| ErrorKind::InvalidInput)?
            .to_be_bytes()
            .to_vec();
        wire.resize(wire.len().saturating_add(length), 42);
        wire.extend_from_slice(&1u32.to_be_bytes());
        wire.push(7);
        let mut bytes = wire.as_slice();
        let mut reader = FrameReader::new(config(length)?);
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if let Some(frame) = reader.advance(|target| bytes.read(target))? {
                frames.push(frame);
            }
        }
        let [first, second] = frames.as_slice() else {
            return Err(ErrorKind::InvalidData.into());
        };
        require(
            first.len() == length
                && first.iter().all(|&byte| byte == 42)
                && second == &[7],
            "multiple chunks and the following frame must remain distinct",
        )
    }

    #[test]
    fn invalid_header_cannot_resynchronize_on_following_bytes() -> Result<()> {
        let mut reader = FrameReader::new(config(8)?);
        let wire = 9u32.to_be_bytes();
        require(
            error_kind(reader.advance(|target| {
                target.copy_from_slice(&wire);
                Ok(wire.len())
            })) == Some(ErrorKind::InvalidData),
            "oversized frame header",
        )?;
        let mut called = false;
        require(
            error_kind(reader.advance(|_target| {
                called = true;
                Ok(0)
            })) == Some(ErrorKind::InvalidData)
                && !called,
            "fatal framing errors cannot consume another frame",
        )
    }

    #[test]
    fn writer_keeps_one_payload_and_retries_only_transient_operations()
    -> Result<()> {
        let limits = config(3)?;
        require(
            matches!(
                FrameWriter::new(b"four", limits),
                Err(LanAdmissionError::OutboundLength)
            ),
            "oversized outgoing frame",
        )?;
        let mut writer = FrameWriter::new(b"abc", limits)
            .map_err(|error| Error::other(format!("{error:?}")))?;
        require(
            error_kind(
                writer.advance(|_chunk| Err(ErrorKind::WouldBlock.into())),
            ) == Some(ErrorKind::WouldBlock),
            "backpressure does not consume bytes",
        )?;
        let mut wire = Vec::new();
        let mut interrupted = false;
        writer.write_all(|chunk| {
            if !interrupted {
                interrupted = true;
                return Err(ErrorKind::Interrupted.into());
            }
            let part = chunk
                .get(..chunk.len().min(2))
                .ok_or(ErrorKind::InvalidData)?;
            wire.extend_from_slice(part);
            Ok(part.len())
        })?;
        require(
            wire == b"\0\0\0\x03abc",
            "short writes preserve header and payload order",
        )?;
        require(
            writer.advance(|_chunk| Err(ErrorKind::InvalidData.into()))?,
            "completed frame makes no additional write",
        )
    }

    #[test]
    fn fatal_write_cannot_resume_on_a_different_stream() -> Result<()> {
        let limits = config(3)?;
        let mut writer = FrameWriter::new(b"abc", limits)
            .map_err(|error| Error::other(format!("{error:?}")))?;
        require(
            error_kind(writer.advance(|_chunk| Ok(0)))
                == Some(ErrorKind::WriteZero),
            "zero byte write fails",
        )?;
        let mut called = false;
        require(
            error_kind(writer.advance(|chunk| {
                called = true;
                Ok(chunk.len())
            })) == Some(ErrorKind::InvalidData)
                && !called,
            "fatal write rejects subsequent attempts",
        )
    }
}
