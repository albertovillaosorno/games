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
//   - Structural admission of uncompressed NEO version-one regions.
// - Must-Not:
//   - Execute program bytes, infer an edition or publish source data.
// - Allows:
//   - Inputs: Immutable bounded source bytes and archive resource budgets.
//   - Outputs: Borrowed regions in declared cartridge order.
//   - Side effects: Calls a caller-owned conversion callback.
// - Split-When:
//   - Another cartridge format requires independent admission.
// - Merge-When:
//   - A shared container parser provides the same admission contract.
// - Summary:
//   - Admit bounded Neo Geo cartridge regions without copying them.
// - Description:
//   - Validate the header and all extents before visiting any region.
// - Usage:
//   - Use the bounded SourceSnapshot cartridge visitor.
// - Defaults:
//   - Reject unknown versions, incomplete extents and trailing bytes.

use games_assets::archive::{ArchiveBudget, EntryKind};

use crate::{ImportError, SourceSnapshot};

const HEADER_BYTES: usize = 4096;
const REGION_NAMES: [&str; 6] = [
    "program",
    "fixed",
    "audio-program",
    "adpcm-a",
    "adpcm-b",
    "sprites",
];

impl SourceSnapshot<'_> {
    /// Visit all bounded NEO version-one cartridge regions in order.
    ///
    /// The caller selects the edition; the container header supplies only
    /// region sizes. Program regions remain opaque, unexecuted bytes.
    /// All regions must pass admission before any callback runs.
    ///
    /// # Errors
    /// Rejects malformed extents, unknown versions, oversized regions and
    /// callback errors.
    pub fn visit_neo<Visit>(
        &self,
        budget: ArchiveBudget,
        visit: Visit,
    ) -> Result<(), ImportError>
    where
        Visit: FnMut(&str, &[u8]) -> Result<(), ImportError>,
    {
        visit_regions(self.bytes, budget, visit)
    }
}

fn visit_regions<Visit>(
    bytes: &[u8],
    mut budget: ArchiveBudget,
    mut visit: Visit,
) -> Result<(), ImportError>
where
    Visit: FnMut(&str, &[u8]) -> Result<(), ImportError>,
{
    if !bytes.starts_with(b"NEO\x01") {
        return Err(ImportError::InvalidNeo);
    }
    let sizes = bytes.get(4..28).ok_or(ImportError::InvalidNeo)?;
    let mut remaining =
        bytes.get(HEADER_BYTES..).ok_or(ImportError::InvalidNeo)?;
    let mut regions = [None; REGION_NAMES.len()];
    for ((name, field), region) in REGION_NAMES
        .into_iter()
        .zip(sizes.as_chunks::<4>().0)
        .zip(&mut regions)
    {
        let size = u32::from_le_bytes(*field);
        if size == 0 {
            continue;
        }
        let mut reservation =
            budget.admit(name, EntryKind::File, u64::from(size))?;
        let length =
            usize::try_from(size).map_err(|_error| ImportError::TooLarge)?;
        let (payload, tail) = remaining
            .split_at_checked(length)
            .ok_or(ImportError::InvalidNeo)?;
        reservation.record(payload.len())?;
        reservation.finish()?;
        *region = Some(payload);
        remaining = tail;
    }
    if !remaining.is_empty()
        || regions.first().is_none_or(Option::is_none)
        || regions.last().is_none_or(Option::is_none)
    {
        return Err(ImportError::InvalidNeo);
    }
    for (name, region) in REGION_NAMES.into_iter().zip(regions) {
        if let Some(payload) = region {
            visit(name, payload)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use games_assets::archive::ArchiveError;

    use super::*;

    fn require(condition: bool) -> Result<(), ImportError> {
        condition.then_some(()).ok_or(ImportError::InvalidNeo)
    }

    fn fixture() -> Result<Vec<u8>, ImportError> {
        let mut bytes = vec![0; HEADER_BYTES];
        bytes
            .get_mut(..4)
            .ok_or(ImportError::InvalidNeo)?
            .copy_from_slice(b"NEO\x01");
        let sizes = bytes.get_mut(4..28).ok_or(ImportError::InvalidNeo)?;
        for (field, size) in sizes
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip([2u32, 1, 0, 2, 0, 3])
        {
            field.copy_from_slice(&size.to_le_bytes());
        }
        bytes.extend_from_slice(b"abcdefgh");
        Ok(bytes)
    }

    fn budget(entry: u64, total: u64, count: u64) -> ArchiveBudget {
        let bound = |value| NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN);
        ArchiveBudget::new(bound(entry), bound(total), bound(count))
    }

    #[test]
    fn cartridge_regions_are_borrowed_in_declared_order()
    -> Result<(), ImportError> {
        let bytes = fixture()?;
        let mut seen = Vec::new();
        visit_regions(&bytes, budget(3, 8, 4), |name, payload| {
            seen.push((name.to_owned(), payload.to_vec()));
            Ok(())
        })?;
        require(
            seen == [
                (String::from("program"), b"ab".to_vec()),
                (String::from("fixed"), b"c".to_vec()),
                (String::from("adpcm-a"), b"de".to_vec()),
                (String::from("sprites"), b"fgh".to_vec()),
            ],
        )
    }

    #[test]
    fn region_admission_rejects_every_budget_before_visiting()
    -> Result<(), ImportError> {
        let bytes = fixture()?;
        for limit in [budget(2, 8, 4), budget(3, 7, 4), budget(3, 8, 3)] {
            let mut visited = false;
            let result = visit_regions(&bytes, limit, |_name, _payload| {
                visited = true;
                Ok(())
            });
            require(
                result == Err(ImportError::Archive(ArchiveError::TooLarge))
                    && !visited,
            )?;
        }
        Ok(())
    }

    #[test]
    fn malformed_extents_fail_before_visiting() -> Result<(), ImportError> {
        let valid = fixture()?;
        let mut trailing = valid.clone();
        trailing.push(0);
        let mut unknown = valid.clone();
        *unknown.get_mut(3).ok_or(ImportError::InvalidNeo)? = 2;
        let mut missing = valid.clone();
        missing
            .get_mut(4..8)
            .ok_or(ImportError::InvalidNeo)?
            .fill(0);
        missing.truncate(valid.len().saturating_sub(2));
        let mut overflow = valid.clone();
        overflow
            .get_mut(24..28)
            .ok_or(ImportError::InvalidNeo)?
            .fill(u8::MAX);
        let cases = [
            valid.get(..3).ok_or(ImportError::InvalidNeo)?,
            valid.get(..HEADER_BYTES).ok_or(ImportError::InvalidNeo)?,
            valid
                .get(..valid.len().saturating_sub(1))
                .ok_or(ImportError::InvalidNeo)?,
            &trailing,
            &unknown,
            &missing,
        ];
        for bytes in cases {
            let mut visited = false;
            let result =
                visit_regions(bytes, budget(3, 8, 4), |_name, _payload| {
                    visited = true;
                    Ok(())
                });
            require(result == Err(ImportError::InvalidNeo) && !visited)?;
        }
        let result =
            visit_regions(&overflow, budget(3, 8, 4), |_name, _payload| {
                Err(ImportError::StageFailed)
            });
        require(result == Err(ImportError::Archive(ArchiveError::TooLarge)))
    }

    #[test]
    fn callback_error_stops_visiting() -> Result<(), ImportError> {
        let bytes = fixture()?;
        let mut calls = 0usize;
        let result =
            visit_regions(&bytes, budget(3, 8, 4), |_name, _payload| {
                calls = calls.saturating_add(1);
                Err(ImportError::StageFailed)
            });
        require(result == Err(ImportError::StageFailed) && calls == 1)
    }
}
