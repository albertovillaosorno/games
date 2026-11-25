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
//   - Bounded K053247 object RAM and observed transfer comparison.
// - Must-Not:
//   - Execute game code, simulate DMA timing or render sprite pixels.
// - Allows:
//   - Inputs: Exact CPU and device object RAM observations, transfer policy.
//   - Outputs: Record counts and exact ordered transfer comparison.
//   - Side effects: None.
// - Split-When:
//   - Another object chip requires a different record format.
// - Merge-When:
//   - A shared source decoder supports the same object organization.
// - Summary:
//   - Compare selected source objects with the captured device buffer.
// - Description:
//   - Ignore stale inactive payload words, but require cleared headers.
// - Usage:
//   - Supply matching private little-endian object snapshots.
// - Defaults:
//   - Reject malformed sizes and report rather than hide differences.

//! Bounded source-reference object RAM observation.

use crate::ImportError;

const OBJECTS: usize = 256;
const RECORD_BYTES: usize = 16;

/// Exact byte extent of a captured little-endian object RAM.
pub const RAM_BYTES: usize = OBJECTS.saturating_mul(RECORD_BYTES);

/// Result of comparing ordered selected CPU objects with device records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferReport {
    /// Device records whose active bit is set.
    pub active: usize,
    /// Selected CPU records matching the corresponding device record.
    pub matched: usize,
    /// CPU records selected by the configured transfer admission policy.
    pub selected: usize,
    /// Device headers that are nonzero beyond the selected record prefix.
    pub uncleared: usize,
}

impl TransferReport {
    /// Whether the observed buffer matches the selected CPU records.
    #[must_use]
    pub const fn exact(self) -> bool {
        self.active == self.selected
            && self.matched == self.selected
            && self.uncleared == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TransferPolicy {
    pub(super) nonzero_depth: bool,
}

impl TransferPolicy {
    const fn admits(self, entry: ObjectEntry<'_>) -> bool {
        entry.active() && (!self.nonzero_depth || entry.depth() != 0)
    }

    pub(super) fn compare(
        self,
        cpu: &[u8],
        device: &[u8],
    ) -> Result<TransferReport, ImportError> {
        let cpu_ram = ObjectRam::new(cpu)?;
        let device_ram = ObjectRam::new(device)?;
        let mut selected =
            cpu_ram.entries().filter(|entry| self.admits(*entry));
        let mut report = TransferReport {
            active: 0,
            matched: 0,
            selected: 0,
            uncleared: 0,
        };
        for observed in device_ram.entries() {
            if observed.active() {
                report.active = report.active.saturating_add(1);
            }
            if let Some(expected) = selected.next() {
                report.selected = report.selected.saturating_add(1);
                report.matched = report
                    .matched
                    .saturating_add(usize::from(expected == observed));
            } else if observed.header() != 0 {
                report.uncleared = report.uncleared.saturating_add(1);
            }
        }
        Ok(report)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ObjectRam<'source> {
    records: &'source [[u8; RECORD_BYTES]],
}

impl<'source> ObjectRam<'source> {
    pub(super) fn entries(self) -> impl Iterator<Item = ObjectEntry<'source>> {
        self.records.iter().map(ObjectEntry)
    }

    pub(super) const fn new(bytes: &'source [u8]) -> Result<Self, ImportError> {
        let (records, remainder) = bytes.as_chunks::<RECORD_BYTES>();
        if records.len() != OBJECTS || !remainder.is_empty() {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(Self { records })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectEntry<'source>(&'source [u8; RECORD_BYTES]);

impl ObjectEntry<'_> {
    pub(super) const fn active(self) -> bool {
        self.header() & 0x8000 != 0
    }

    pub(super) const fn depth(self) -> u8 {
        let &[depth, ..] = self.0;
        depth
    }

    pub(super) const fn header(self) -> u16 {
        let &[low, high, ..] = self.0;
        u16::from_le_bytes([low, high])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_compares_full_records_and_cleared_headers()
    -> Result<(), ImportError> {
        let mut cpu = [0u8; RAM_BYTES];
        let mut device = [0u8; RAM_BYTES];
        let first = [5, 0x80, 9, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4];
        let second = [6, 0x80, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];
        cpu.get_mut(16..32)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&first);
        cpu.get_mut(48..64)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&second);
        // Active zero-depth record must be omitted by this machine.
        *cpu.get_mut(33).ok_or(ImportError::UnsupportedMember)? = 0x80;
        device
            .get_mut(..16)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&first);
        device
            .get_mut(16..32)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&second);
        // Stale inactive payload is not a discrepancy.
        *device.last_mut().ok_or(ImportError::UnsupportedMember)? = 255;
        let policy = TransferPolicy { nonzero_depth: true };
        let expected = TransferReport {
            active: 2,
            matched: 2,
            selected: 2,
            uncleared: 0,
        };
        if policy.compare(&cpu, &device)? != expected || !expected.exact() {
            return Err(ImportError::UnsupportedMember);
        }
        *device.get_mut(2).ok_or(ImportError::UnsupportedMember)? = 10;
        *device.get_mut(32).ok_or(ImportError::UnsupportedMember)? = 1;
        let actual = policy.compare(&cpu, &device)?;
        if actual
            != (TransferReport {
                matched: 1,
                uncleared: 1,
                ..expected
            })
            || actual.exact()
            || policy.compare(&cpu, &[]).is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }
}
