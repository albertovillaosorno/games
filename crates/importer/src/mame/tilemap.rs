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
//   - Decode observed tile RAM, character banks and standard XY scroll.
// - Must-Not:
//   - Execute game code or claim screen composition.
// - Allows:
//   - Inputs: Exact tile RAM, bank/control registers and mixer state.
//   - Outputs: Tile indices, palette banks, flip and scroll attributes.
//   - Side effects: None.
// - Split-When:
//   - Another tile chip uses a different RAM organization.
// - Merge-When:
//   - A shared source decoder supports the same observed addressing.
// - Summary:
//   - Resolve individual K052109 tile cells in layer coordinates.
// - Description:
//   - Borrow bounded RAM and apply the selected machine's address wiring.
// - Usage:
//   - Bind a private observation before requesting any cells.
// - Defaults:
//   - Reject malformed extents and addresses outside three layers.

//! Observed Konami tile-cell address decoding.

use super::priority::VideoPriority;
use crate::ImportError;

const CELLS: usize = 2048;

pub(super) type Offset = (usize, usize);
type ScrollOffsets = [Offset; 3];

/// Resolved local tile attributes, before scrolling and composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileCell {
    /// Vertical reversal for this individual cell.
    pub flip_y: bool,
    /// Canonical tile index after source address wrapping.
    pub index: u16,
    /// Palette base in groups of sixteen pens.
    pub palette_bank: u8,
}

/// Borrowed three-layer tile RAM and observed address registers.
#[derive(Clone, Copy, Debug)]
pub struct TilemapSnapshot<'source> {
    address_map: u8,
    banks: [u8; 4],
    code_mask: u16,
    flip: u8,
    mixer: VideoPriority,
    ram: &'source [[u8; CELLS]],
}

impl<'source> TilemapSnapshot<'source> {
    /// Resolve a row-major cell (64 columns by 32 rows) in one of three layers.
    #[must_use]
    pub fn cell(self, layer: usize, offset: usize) -> Option<TileCell> {
        let input = self.mixer.layers.get(layer)?;
        let attr = *self.ram.get(layer)?.get(offset)?;
        let low = *self.ram.get(layer.checked_add(4)?)?.get(offset)?;
        let high = *self.ram.get(layer.checked_add(8)?)?.get(offset)?;
        let bank = *self.banks.get(usize::from((attr >> 2u32) & 3))?;
        let color = if self.address_map & 0x40 == 0 {
            (attr & !0x0c) | ((bank & 3) << 2u32)
        } else {
            attr
        };
        let code = u16::from_le_bytes([low, high])
            | (u16::from(color & 0x3f) << 8u32)
            | (u16::from(bank >> 2u32) << 14u32);
        Some(TileCell {
            flip_y: self.flip & 4 != 0 && color & 2 != 0,
            index: code & self.code_mask,
            palette_bank: input.palette_bank.checked_add(color >> 6)?,
        })
    }

    /// Whether the original tilemap reverses both axes globally.
    #[must_use]
    pub const fn global_flip(self) -> bool {
        self.flip & 1 != 0
    }

    pub(super) fn new(
        source: &'source [u8],
        controls: &[u8],
        mixer: VideoPriority,
        code_mask: u16,
    ) -> Result<Self, ImportError> {
        let (ram, remainder) = source.as_chunks::<CELLS>();
        let &[bank0, bank1, bank2, bank3, address_map, flip] =
            <&[u8; 6]>::try_from(controls)
                .map_err(|_error| ImportError::UnsupportedMember)?;
        let selectors = [bank0, bank1, bank2, bank3];
        if !remainder.is_empty()
            || ram.len() != 12
            || selectors.iter().any(|bank| *bank > 15)
            || flip > 7
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(Self {
            address_map,
            banks: selectors,
            code_mask,
            flip,
            mixer,
            ram,
        })
    }

    pub(super) const fn priority(self) -> VideoPriority {
        self.mixer
    }

    fn scroll_page(self, page: usize) -> Result<Offset, ImportError> {
        let ram = self.ram.get(page).ok_or(ImportError::UnsupportedMember)?;
        let horizontal =
            ram.get(512..514).ok_or(ImportError::UnsupportedMember)?;
        let scroll_x = u16::from_le_bytes(
            horizontal
                .try_into()
                .map_err(|_error| ImportError::UnsupportedMember)?,
        );
        let scroll_y = *ram.get(12).ok_or(ImportError::UnsupportedMember)?;
        Ok((usize::from(scroll_x), usize::from(scroll_y)))
    }

    /// Read the fixed layer and two global XY scroll offsets.
    /// # Errors
    /// Rejects row/column scroll, unknown modes and global reversal.
    pub(super) fn standard_scroll(
        self,
        control: u8,
    ) -> Result<ScrollOffsets, ImportError> {
        if control != 0 || self.global_flip() {
            return Err(ImportError::UnsupportedMember);
        }
        Ok([(0, 0), self.scroll_page(3)?, self.scroll_page(7)?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mame::Machine;

    #[test]
    fn resolves_bank_palette_wrap_and_flip() -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut ram = vec![0u8; CELLS * 12];
        for (index, value) in [(0, 0x86), (8192, 0x34), (16384, 0x80)] {
            *ram.get_mut(index).ok_or(ImportError::UnsupportedMember)? = value;
        }
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let snapshot =
            machine.tilemap_snapshot(&ram, &[0, 7, 0, 0, 0, 5], mixer)?;
        let expected = TileCell {
            flip_y: true,
            index: 0x4e34,
            palette_bank: 2,
        };
        if snapshot.cell(0, 0) != Some(expected)
            || !snapshot.global_flip()
            || snapshot.cell(3, 0).is_some()
            || snapshot.cell(0, CELLS).is_some()
            || machine.tilemap_snapshot(&ram, &[0; 5], mixer).is_ok()
            || machine.tilemap_snapshot(&[], &[0; 6], mixer).is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }
}
