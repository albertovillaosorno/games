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
//   - Compare bounded local mixer observations with decoded priority state.
// - Must-Not:
//   - Interpret register layouts, execute originals or publish observations.
// - Allows:
//   - Inputs: Selected machine, raw mixer and observed seven LE32 values.
//   - Outputs: Exact comparison status.
//   - Side effects: Read bounded local files and write summary to stdout.
// - Split-When:
//   - Additional observations require independent comparison contracts.
// - Merge-When:
//   - A shared observation reader can express the same exact comparison.
// - Summary:
//   - Check decoded palette banks and ordered tile priorities.
// - Description:
//   - Compare layer palette banks, sorted priorities and sprite bank.
// - Usage:
//   - Supply raw mixer and matching independently observed values.
// - Defaults:
//   - Require exact extents and never write original state.

//! Host-side video priority comparison.

use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::path::Path;

use games_importer::mame::Machine;
use games_importer::mame::priority::VideoPriority;
use games_platform::ir::storage::snapshot;

/// Compare observed palette bases and sorted layer priorities.
/// # Errors
/// Rejects unsafe files, malformed registers or differing observations.
pub fn check(
    machine: Machine,
    raw_path: &Path,
    reference_path: &Path,
) -> Result<(), String> {
    let raw = snapshot(raw_path, NonZeroU64::MIN.saturating_add(15))
        .map_err(|error| format!("mixer registers: {error:?}"))?;
    let reference =
        snapshot(reference_path, NonZeroU64::MIN.saturating_add(27))
            .map_err(|error| format!("priority reference: {error:?}"))?;
    let decoded = machine
        .decode_video_priority(&raw)
        .map_err(|error| format!("priority decode: {error:?}"))?;
    compare(decoded, &reference)?;
    writeln!(stdout().lock(), "Matched seven video mixer observations")
        .map_err(|error| error.to_string())
}

fn compare(decoded: VideoPriority, reference: &[u8]) -> Result<(), String> {
    let order = decoded.ordered_layers();
    let mut actual = [0u32; 7];
    let [first, second, third, back, middle, front, sprite] = &mut actual;
    let [layer0, layer1, layer2] = decoded.layers;
    *first = u32::from(layer0.palette_bank);
    *second = u32::from(layer1.palette_bank);
    *third = u32::from(layer2.palette_bank);
    for (destination, index) in [back, middle, front].into_iter().zip(order) {
        let layer = decoded
            .layers
            .get(index)
            .ok_or_else(|| String::from("invalid decoded layer identity"))?;
        *destination = u32::from(layer.priority);
    }
    *sprite = u32::from(decoded.sprites.palette_bank);
    let (words, remainder) = reference.as_chunks::<4>();
    if !remainder.is_empty() || words.len() != actual.len() {
        return Err(String::from("incorrect priority observation extent"));
    }
    for (index, (expected, value)) in words.iter().zip(actual).enumerate() {
        if u32::from_le_bytes(*expected) != value {
            return Err(format!("video mixer differs at observation {index}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_palette_and_priority_in_observed_order() -> Result<(), String> {
        let machine = Machine::select("simpsons", "world-4p-set-1")
            .map_err(|error| format!("{error:?}"))?;
        let raw = [9, 8, 7, 2, 5, 0, 0, 0, 0, 45, 53, 0, 0, 0, 0, 0];
        let decoded = machine
            .decode_video_priority(&raw)
            .map_err(|error| format!("{error:?}"))?;
        let values = [64u32, 80, 96, 7, 5, 2, 96];
        let reference = values.map(u32::to_le_bytes);
        compare(decoded, reference.as_flattened())?;
        let wrong = [64u32, 80, 96, 7, 2, 5, 96].map(u32::to_le_bytes);
        if compare(decoded, wrong.as_flattened()).is_ok()
            || compare(decoded, &[]).is_ok()
        {
            return Err(String::from("incorrect observation accepted"));
        }
        Ok(())
    }
}
