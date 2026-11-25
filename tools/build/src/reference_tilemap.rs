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
//   - Bounded read-only admission of private tilemap observations.
// - Must-Not:
//   - Interpret hardware registers or claim rendered pixel parity.
// - Allows:
//   - Inputs: Selected machine, tile RAM and packed control/mixer bytes.
//   - Outputs: Count of locally resolved tile cells.
//   - Side effects: Bounded file reads and summary output.
// - Split-When:
//   - Rendered layer comparison needs independent pixel references.
// - Merge-When:
//   - Shared reference admission supports the same snapshot format.
// - Summary:
//   - Exercise every observed cell before attempting scene reconstruction.
// - Description:
//   - Delegate address decoding to the selected machine profile.
// - Usage:
//   - Supply matching RAM and six control plus sixteen mixer bytes.
// - Defaults:
//   - Exact lengths; no screen composition or original asset publication.

//! Host-side tile RAM admission diagnostic.

use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::path::Path;

use games_importer::mame::Machine;
use games_platform::ir::storage::file::snapshot;

/// Resolve all observed cells from a bounded local snapshot.
/// # Errors
/// Rejects unsafe files, malformed observations or unresolved cells.
pub fn check(
    machine: Machine,
    ram_path: &Path,
    registers_path: &Path,
) -> Result<(), String> {
    let ram = snapshot(ram_path, NonZeroU64::MIN.saturating_add(24_575))
        .map_err(|error| format!("tile RAM: {error:?}"))?;
    let registers =
        snapshot(registers_path, NonZeroU64::MIN.saturating_add(21))
            .map_err(|error| format!("tile registers: {error:?}"))?;
    let count = resolve(machine, &ram, &registers)?;
    writeln!(stdout().lock(), "Resolved {count} local tile cells")
        .map_err(|error| error.to_string())
}

fn resolve(
    machine: Machine,
    ram: &[u8],
    registers: &[u8],
) -> Result<usize, String> {
    let (controls, mixer_raw) = registers
        .split_first_chunk::<6>()
        .ok_or_else(|| String::from("missing tile control registers"))?;
    let mixer = machine
        .decode_video_priority(mixer_raw)
        .map_err(|error| format!("mixer decode: {error:?}"))?;
    let tiles = machine
        .tilemap_snapshot(ram, controls, mixer)
        .map_err(|error| format!("tile RAM decode: {error:?}"))?;
    for layer in 0..3 {
        for offset in 0..2048 {
            let cell = tiles
                .cell(layer, offset)
                .ok_or_else(|| String::from("unresolved tile cell"))?;
            let last_pen = (usize::from(cell.palette_bank) << 4u32) | 15;
            if last_pen >= machine.palette_entries().get() {
                return Err(String::from("palette bank out of range"));
            }
        }
    }
    Ok(6144)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_snapshot_and_rejects_truncation() -> Result<(), String> {
        let machine = Machine::select("simpsons", "world-4p-set-1")
            .map_err(|error| format!("{error:?}"))?;
        let ram = vec![0; 24_576];
        if resolve(machine, &ram, &[0; 22])? != 6144
            || resolve(machine, &ram, &[0; 21]).is_ok()
            || resolve(machine, &[], &[0; 22]).is_ok()
        {
            return Err(String::from("invalid tile snapshot acceptance"));
        }
        Ok(())
    }
}
