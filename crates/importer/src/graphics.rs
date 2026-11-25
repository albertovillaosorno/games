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
//   - Bounded conversion of interleaved words and planar indexed graphics.
// - Must-Not:
//   - Choose title ROMs, infer palettes, render or execute original code.
// - Allows:
//   - Inputs: Source slices, explicit bit layouts and output buffers.
//   - Outputs: Interleaved source regions or row-major palette indices.
//   - Side effects: Writes to supplied or exactly allocated output buffers.
// - Split-When:
//   - A distinct encoding requires independent source-format admission.
// - Merge-When:
//   - A shared decoder implements equivalent bounded conversion.
// - Summary:
//   - Decode source graphics without filesystem access.
// - Description:
//   - Keep title hardware layout parameters separate from decoding.
// - Usage:
//   - Interleave verified lanes, then decode with the selected layout.
// - Defaults:
//   - Reject inconsistent sizes and layouts before writing output.

//! Bounded source-region assembly and indexed tile decoding.

use std::num::{NonZeroU32, NonZeroUsize};

use games_assets::AssetFormat;

/// A source graphics layout or buffer is not representable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsError {
    /// Could not allocate the exact bounded output.
    Allocation,
    /// A dimension, plane count or bit offset is invalid.
    InvalidLayout,
    /// Arithmetic exceeded the supported buffer size.
    Overflow,
    /// Source or destination lengths do not exactly match the layout.
    SizeMismatch,
}

/// One tile's bit layout, with no palette or title-specific assumptions.
///
/// Bit zero is the most significant bit of the first source byte. Planes
/// are listed from most to least significant output index bit. Rows and
/// columns are emitted in their supplied order.
#[derive(Clone, Copy, Debug)]
pub struct TileLayout<'layout> {
    /// Bit offsets for columns, in visual left-to-right order.
    pub column_bits: &'layout [usize],
    /// Bit offsets for color planes, most significant first.
    pub plane_bits: &'layout [usize],
    /// Bit offsets for rows, in visual top-to-bottom order.
    pub row_bits: &'layout [usize],
    /// Bytes occupied by each complete source tile.
    pub tile_bytes: NonZeroUsize,
}

impl TileLayout<'_> {
    /// Decode complete tiles into tightly packed row-major color indices.
    ///
    /// # Errors
    /// Rejects invalid layouts, partial tiles and incorrect output lengths
    /// before modifying the destination.
    pub fn decode(
        &self,
        source: &[u8],
        destination: &mut [u8],
    ) -> Result<(), GraphicsError> {
        let expected = self.output_len(source.len())?;
        if destination.len() != expected {
            return Err(GraphicsError::SizeMismatch);
        }
        let pixels = self.pixels_per_tile()?;
        for (tile, decoded) in source
            .chunks_exact(self.tile_bytes.get())
            .zip(destination.chunks_exact_mut(pixels))
        {
            self.decode_tile(tile, decoded)?;
        }
        Ok(())
    }

    /// Allocate and decode a complete canonical index buffer.
    /// # Errors
    /// Rejects malformed layouts, overflow and allocation failure.
    pub fn decode_owned(
        &self,
        source: &[u8],
    ) -> Result<Vec<u8>, GraphicsError> {
        let mut decoded = allocated_bytes(self.output_len(source.len())?)?;
        self.decode(source, &mut decoded)?;
        Ok(decoded)
    }

    fn decode_pixel(
        &self,
        tile: &[u8],
        base: usize,
    ) -> Result<u8, GraphicsError> {
        let mut index = 0u8;
        for plane in self.plane_bits {
            let offset =
                base.checked_add(*plane).ok_or(GraphicsError::Overflow)?;
            let byte = tile
                .get(offset >> 3usize)
                .ok_or(GraphicsError::InvalidLayout)?;
            let set = byte & (0x80u8 >> (offset & 7usize)) != 0;
            index = index.wrapping_shl(1) | u8::from(set);
        }
        Ok(index)
    }

    fn decode_tile(
        &self,
        tile: &[u8],
        destination: &mut [u8],
    ) -> Result<(), GraphicsError> {
        for (row, output) in self
            .row_bits
            .iter()
            .zip(destination.chunks_exact_mut(self.column_bits.len()))
        {
            for (column, pixel) in self.column_bits.iter().zip(output) {
                let base =
                    row.checked_add(*column).ok_or(GraphicsError::Overflow)?;
                *pixel = self.decode_pixel(tile, base)?;
            }
        }
        Ok(())
    }

    /// Describe the canonical indexed tiles for a complete source region.
    /// # Errors
    /// Rejects invalid layouts, partial tiles and unrepresentable dimensions.
    pub fn indexed_format(
        &self,
        source_bytes: usize,
    ) -> Result<AssetFormat, GraphicsError> {
        let _size = self.output_len(source_bytes)?;
        let dimension = |value: usize| {
            u32::try_from(value)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or(GraphicsError::Overflow)
        };
        Ok(AssetFormat::IndexedTiles8 {
            height: dimension(self.row_bits.len())?,
            tiles: dimension(
                source_bytes
                    .checked_div(self.tile_bytes.get())
                    .ok_or(GraphicsError::InvalidLayout)?,
            )?,
            width: dimension(self.column_bits.len())?,
        })
    }

    /// Return the exact number of bytes required for the decoded indices.
    ///
    /// # Errors
    /// Rejects invalid layouts, partial tiles and size overflow.
    pub fn output_len(
        &self,
        source_bytes: usize,
    ) -> Result<usize, GraphicsError> {
        let pixels = self.pixels_per_tile()?;
        if source_bytes == 0
            || source_bytes.checked_rem(self.tile_bytes.get()) != Some(0)
        {
            return Err(GraphicsError::SizeMismatch);
        }
        let tiles = source_bytes
            .checked_div(self.tile_bytes.get())
            .ok_or(GraphicsError::InvalidLayout)?;
        tiles.checked_mul(pixels).ok_or(GraphicsError::Overflow)
    }

    fn pixels_per_tile(&self) -> Result<usize, GraphicsError> {
        if self.plane_bits.len() > 8 {
            return Err(GraphicsError::InvalidLayout);
        }
        let maximum = |offsets: &[usize]| {
            offsets
                .iter()
                .copied()
                .max()
                .ok_or(GraphicsError::InvalidLayout)
        };
        let plane = maximum(self.plane_bits)?;
        let row = maximum(self.row_bits)?;
        let column = maximum(self.column_bits)?;
        let last = plane
            .checked_add(row)
            .and_then(|offset| offset.checked_add(column))
            .ok_or(GraphicsError::Overflow)?;
        if last >> 3usize >= self.tile_bytes.get() {
            return Err(GraphicsError::InvalidLayout);
        }
        self.column_bits
            .len()
            .checked_mul(self.row_bits.len())
            .ok_or(GraphicsError::Overflow)
    }
}

pub(crate) fn allocated_bytes(size: usize) -> Result<Vec<u8>, GraphicsError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_error| GraphicsError::Allocation)?;
    bytes.resize(size, 0);
    Ok(bytes)
}

/// Interleave equal-length source lanes one complete word at a time.
///
/// Lanes retain their supplied order; bytes within a word are never swapped.
///
/// # Errors
/// Rejects empty or unequal lanes, incomplete words, size overflow and
/// incorrect destination length before writing the destination.
pub fn interleave_words(
    lanes: &[&[u8]],
    word_bytes: NonZeroUsize,
    destination: &mut [u8],
) -> Result<(), GraphicsError> {
    let lane_bytes = lanes.first().ok_or(GraphicsError::InvalidLayout)?.len();
    let word = word_bytes.get();
    if lane_bytes == 0
        || lane_bytes.checked_rem(word) != Some(0)
        || lanes.iter().any(|lane| lane.len() != lane_bytes)
    {
        return Err(GraphicsError::SizeMismatch);
    }
    let expected = lane_bytes
        .checked_mul(lanes.len())
        .ok_or(GraphicsError::Overflow)?;
    let stride = word
        .checked_mul(lanes.len())
        .ok_or(GraphicsError::Overflow)?;
    if destination.len() != expected {
        return Err(GraphicsError::SizeMismatch);
    }
    for (index, group) in destination.chunks_exact_mut(stride).enumerate() {
        let start = index.checked_mul(word).ok_or(GraphicsError::Overflow)?;
        let end = start.checked_add(word).ok_or(GraphicsError::Overflow)?;
        for (lane, output) in lanes.iter().zip(group.chunks_exact_mut(word)) {
            let source =
                lane.get(start..end).ok_or(GraphicsError::SizeMismatch)?;
            output.copy_from_slice(source);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_plane() -> TileLayout<'static> {
        TileLayout {
            column_bits: &[0, 1, 2],
            plane_bits: &[8, 0],
            row_bits: &[0],
            tile_bytes: NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
        }
    }

    #[test]
    fn interleaves_words_without_reversing_bytes() {
        let lanes: [&[u8]; 4] =
            [&[0, 1, 8, 9], &[2, 3, 10, 11], &[4, 5, 12, 13], &[
                6, 7, 14, 15,
            ]];
        let mut output = [0u8; 16];
        let word = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
        assert_eq!(interleave_words(&lanes, word, &mut output), Ok(()));
        assert_eq!(output, [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
        ]);
    }

    #[test]
    fn rejects_interleave_sizes_without_writing() {
        type LanePair = [&'static [u8]; 2];
        let word = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
        let mut output = [0xaau8; 8];
        let cases: [LanePair; 3] = [
            [&[1, 2, 3], &[4, 5, 6]],
            [&[1, 2], &[3, 4, 5, 6]],
            [&[1, 2], &[3, 4]],
        ];
        for lanes in cases {
            assert_eq!(
                interleave_words(&lanes, word, &mut output),
                Err(GraphicsError::SizeMismatch)
            );
            assert_eq!(output, [0xaa; 8]);
        }
    }

    #[test]
    fn decodes_plane_significance_and_complete_tile_order() {
        let layout = two_plane();
        let source = [0xa0, 0x40, 0x80, 0x20];
        let mut output = [0u8; 6];
        assert_eq!(layout.output_len(source.len()), Ok(6));
        assert_eq!(layout.decode(&source, &mut output), Ok(()));
        assert_eq!(output, [1, 2, 1, 1, 0, 2]);
    }

    #[test]
    fn supports_explicit_row_and_column_permutations() {
        let layout = TileLayout {
            column_bits: &[4, 0],
            plane_bits: &[0, 1, 2, 3],
            row_bits: &[8, 0],
            tile_bytes: NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
        };
        let mut output = [0u8; 4];
        assert_eq!(layout.decode(&[0x12, 0x34], &mut output), Ok(()));
        assert_eq!(output, [4, 3, 2, 1]);
    }

    #[test]
    fn rejects_layout_and_size_errors_before_writing() {
        let valid = two_plane();
        let mut output = [0xaau8; 3];
        assert_eq!(
            valid.decode(&[0], &mut output),
            Err(GraphicsError::SizeMismatch)
        );
        let invalid = [
            TileLayout { plane_bits: &[], ..valid },
            TileLayout {
                plane_bits: &[0; 9],
                ..valid
            },
            TileLayout {
                column_bits: &[],
                ..valid
            },
            TileLayout { row_bits: &[16], ..valid },
            TileLayout {
                row_bits: &[usize::MAX],
                ..valid
            },
        ];
        for layout in invalid {
            assert!(layout.decode(&[0; 2], &mut output).is_err());
            assert_eq!(output, [0xaa; 3]);
        }
        assert_eq!(
            valid.decode(&[0; 4], &mut output),
            Err(GraphicsError::SizeMismatch)
        );
        assert_eq!(output, [0xaa; 3]);
    }
}
