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
//   - Shared compact bitmap row lookup for title-native UI symbols.
// - Must-Not:
//   - Own control mapping, gameplay state or GPU resources.
// - Allows:
//   - Inputs: Bitmap rows, width and logical pixel coordinates.
//   - Outputs: Whether the requested pixel is inked.
//   - Side effects: None.
// - Split-When:
//   - Independent font resources or text layout require separate policy.
// - Merge-When:
//   - Another title-local helper duplicates bit-mask symbol lookup.
// - Summary:
//   - Reuse bounded bitmap access for simple native UI glyphs.
// - Description:
//   - Reads most-significant in-width bit first; out-of-bounds is blank.
// - Usage:
//   - Supply one byte per row and a width of at most eight bits.
// - Defaults:
//   - Empty or invalid coordinates produce no ink.

//! Compact UI bitmap lookup.

pub(super) fn ink(rows: &[u8], width: u16, column: u16, row: u16) -> bool {
    let Some(shift) = width
        .checked_sub(1)
        .and_then(|last| last.checked_sub(column))
    else {
        return false;
    };
    let Some(mask) = 1u8.checked_shl(u32::from(shift)) else {
        return false;
    };
    rows.get(usize::from(row))
        .is_some_and(|bits| bits & mask != 0)
}
