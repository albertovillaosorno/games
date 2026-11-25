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
//   - Bounded local palette-reference comparison and summary output.
// - Must-Not:
//   - Execute originals, interpret color words or publish private observations.
// - Allows:
//   - Inputs: Selected machine, raw palette and observed ARGB32 files.
//   - Outputs: Exact comparison result or first mismatching pen index.
//   - Side effects: Read bounded regular files and write summary to stdout.
// - Split-When:
//   - Additional reference observation formats need independent readers.
// - Merge-When:
//   - A shared reference reader can express the same comparison.
// - Summary:
//   - Independently check decoded colors against local MAME observations.
// - Description:
//   - Delegates raw word decoding to the selected importer profile.
// - Usage:
//   - Pass a raw RAM dump and matching little-endian ARGB32 pen dump.
// - Defaults:
//   - Require exact lengths and do not write source media.

//! Host-side palette reference comparison without asset publication.

use std::num::NonZeroU64;
use std::path::Path;

use games_platform::ir::importer::reference::Machine;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain;

pub(super) type Colors = Vec<[u8; 4]>;

/// Compare a bounded raw palette snapshot with an observed pen dump.
/// # Errors
/// Rejects unsafe files, incorrect extents or mismatching palette entries.
pub fn check(
    machine: Machine,
    raw_path: &Path,
    reference_path: &Path,
) -> Result<(), String> {
    let raw = snapshot(raw_path.as_os_str(), byte_limit(machine, 2)?)
        .map_err(|error| format!("palette RAM: {error:?}"))?;
    let reference =
        snapshot(reference_path.as_os_str(), byte_limit(machine, 4)?)
            .map_err(|error| format!("palette reference: {error:?}"))?;
    let entries = compare(machine, &raw, &reference)?;
    toolchain::write_line(&format!("Matched {entries} palette entries"))
        .map_err(|error| error.to_string())
}

pub(super) fn byte_limit(
    machine: Machine,
    bytes_per_entry: u64,
) -> Result<NonZeroU64, String> {
    u64::try_from(machine.palette_entries().get())
        .ok()
        .and_then(|entries| entries.checked_mul(bytes_per_entry))
        .and_then(NonZeroU64::new)
        .ok_or_else(|| String::from("palette byte budget overflow"))
}

fn compare(
    machine: Machine,
    raw: &[u8],
    reference: &[u8],
) -> Result<usize, String> {
    let entries = machine.palette_entries().get();
    let (words, remainder) = reference.as_chunks::<4>();
    if !remainder.is_empty() || words.len() != entries {
        return Err(String::from("incorrect palette reference length"));
    }
    let colors = decode(machine, raw)?;
    for (pen, (color, [blue, green, red, alpha])) in
        colors.into_iter().zip(words.iter().copied()).enumerate()
    {
        if color != [red, green, blue, alpha] {
            return Err(format!("palette differs at pen {pen}"));
        }
    }
    Ok(entries)
}

pub(super) fn decode(machine: Machine, raw: &[u8]) -> Result<Colors, String> {
    let entries = machine.palette_entries().get();
    let mut colors = Vec::new();
    colors
        .try_reserve_exact(entries)
        .map_err(|_error| String::from("palette allocation failed"))?;
    colors.resize(entries, [0; 4]);
    machine
        .decode_palette(raw, &mut colors)
        .map_err(|error| format!("palette decode: {error:?}"))?;
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::{Machine, byte_limit, compare};

    #[test]
    fn compares_exact_colors_and_rejects_partial_observations()
    -> Result<(), String> {
        let machine = Machine::select("simpsons", "world-4p-set-1")
            .map_err(|error| format!("{error:?}"))?;
        let raw_size = usize::try_from(byte_limit(machine, 2)?.get())
            .map_err(|error| error.to_string())?;
        let entries = machine.palette_entries().get();
        let mut raw = vec![0u8; raw_size];
        let mut reference = vec![[0, 0, 0, 255u8]; entries];
        let first = raw
            .get_mut(..2)
            .ok_or_else(|| String::from("missing synthetic word"))?;
        first.copy_from_slice(&[0, 0x1f]);
        let first_color = reference
            .first_mut()
            .ok_or_else(|| String::from("missing synthetic color"))?;
        *first_color = [0, 0, 255, 255];
        let matched = compare(machine, &raw, reference.as_flattened())?;
        if matched != entries
            || compare(machine, &raw, &[]).is_ok()
            || compare(machine, &[], reference.as_flattened()).is_ok()
        {
            return Err(String::from("palette reference length not checked"));
        }
        let last = reference
            .last_mut()
            .ok_or_else(|| String::from("missing synthetic last color"))?;
        *last = [1, 0, 0, 255];
        let mismatch = compare(machine, &raw, reference.as_flattened());
        let expected_pen = entries
            .checked_sub(1)
            .ok_or_else(|| String::from("invalid profile entries"))?;
        (mismatch == Err(format!("palette differs at pen {expected_pen}")))
            .then_some(())
            .ok_or_else(|| {
                String::from("last mismatching pen was not detected")
            })
    }
}
