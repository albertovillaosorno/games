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
//   - Checked layout and palette expansion of selected indexed atlas tiles.
// - Must-Not:
//   - Infer original palettes, allocate textures or interpret title gameplay.
// - Allows:
//   - Inputs: Verified indexed tiles, explicit palettes and layout limits.
//   - Outputs: A row-major RGBA atlas and logical tile rectangles.
//   - Side effects: Writes the complete atlas into a caller-owned buffer.
// - Split-When:
//   - Variable-sized assets require a separate packing algorithm.
// - Merge-When:
//   - Indexed media consumers duplicate tile-to-atlas row addressing.
// - Summary:
//   - Allocation-free preparation of selected tiles for texture upload.
// - Description:
//   - Packs uniform tiles into a bounded grid in caller-defined order.
// - Usage:
//   - Construct a layout, assign tile indices and palettes, then pack.
// - Defaults:
//   - Reject invalid selections and fill unused atlas space transparently.

//! Bounded palette-expanded texture atlases for indexed graphics.

use std::num::{NonZeroU32, NonZeroUsize};

use super::IndexedTiles;
use crate::AssetError;

/// One selected tile and its explicit straight-alpha RGBA palette.
#[derive(Clone, Copy, Debug)]
pub struct AtlasTile<'palette> {
    /// Index in the verified tile sheet.
    pub index: u32,
    /// Palette entries addressed by the tile's pixel indices.
    pub palette: &'palette [[u8; 4]],
}

/// The pixel rectangle of one selected tile within its atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasTileRect {
    /// Tile height in pixels.
    pub height: NonZeroU32,
    /// Tile width in pixels.
    pub width: NonZeroU32,
    /// Left edge in atlas pixels.
    pub x: u32,
    /// Top edge in atlas pixels.
    pub y: u32,
}

#[derive(Clone, Copy)]
struct TileColors<'source> {
    indices: &'source [u8],
    palette: &'source [[u8; 4]],
}

/// A checked grid for selected tiles with an explicit RGBA byte budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileAtlasLayout {
    bytes: usize,
    columns: NonZeroU32,
    height: NonZeroU32,
    slots: NonZeroU32,
    tile_height: NonZeroU32,
    tile_width: NonZeroU32,
    width: NonZeroU32,
}

impl TileAtlasLayout {
    /// Number of bytes in the complete tightly packed RGBA atlas.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Height in pixels, including the entire final row.
    #[must_use]
    pub const fn height(self) -> NonZeroU32 {
        self.height
    }

    /// Construct a bounded grid for a specific tile extent and selection count.
    ///
    /// The caller may choose fewer columns than slots for multiple rows.
    /// Empty cells at the end of the final row are transparent.
    /// # Errors
    /// Rejects excess columns, arithmetic overflow or excessive RGBA bytes.
    pub fn new(
        tiles: &IndexedTiles<'_>,
        slots: NonZeroU32,
        columns: NonZeroU32,
        max_bytes: NonZeroUsize,
    ) -> Result<Self, AssetError> {
        if columns > slots {
            return Err(AssetError::InvalidFormat);
        }
        let rows = NonZeroU32::new(slots.get().div_ceil(columns.get()))
            .ok_or(AssetError::InvalidFormat)?;
        let width = tiles
            .width()
            .checked_mul(columns)
            .ok_or(AssetError::TooLarge)?;
        let height = tiles
            .height()
            .checked_mul(rows)
            .ok_or(AssetError::TooLarge)?;
        let bytes = usize::try_from(width.get())
            .ok()
            .and_then(|pixels| {
                pixels.checked_mul(usize::try_from(height.get()).ok()?)
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AssetError::TooLarge)?;
        if bytes > max_bytes.get() {
            return Err(AssetError::TooLarge);
        }
        Ok(Self {
            bytes,
            columns,
            height,
            slots,
            tile_height: tiles.height(),
            tile_width: tiles.width(),
            width,
        })
    }

    /// Expand each selected tile into its row-major atlas cell.
    ///
    /// Multiple selections may reuse the same tile with different palettes.
    /// The entire output is validated before any pixel is changed.
    /// # Errors
    /// Rejects mismatched tile extents, selections, palettes or output length.
    pub fn pack(
        self,
        tiles: &IndexedTiles<'_>,
        selections: &[AtlasTile<'_>],
        output: &mut [[u8; 4]],
    ) -> Result<(), AssetError> {
        self.validate(tiles, selections, output.len())?;
        output.fill([0; 4]);
        for (slot, selection) in selections.iter().enumerate() {
            let rect = self
                .rect(
                    u32::try_from(slot)
                        .map_err(|_error| AssetError::TooLarge)?,
                )
                .ok_or(AssetError::InvalidFormat)?;
            let indices = tiles
                .tile(selection.index)
                .ok_or(AssetError::InvalidFormat)?;
            self.write_tile(
                TileColors {
                    indices,
                    palette: selection.palette,
                },
                rect,
                output,
            )?;
        }
        Ok(())
    }

    /// Number of RGBA pixels in the atlas.
    #[must_use]
    pub const fn pixels(self) -> usize {
        self.bytes.div_euclid(4)
    }

    /// Return the rectangle for a selected slot, not an unused grid cell.
    #[must_use]
    pub fn rect(self, slot: u32) -> Option<AtlasTileRect> {
        if slot >= self.slots.get() {
            return None;
        }
        let column = slot.checked_rem(self.columns.get())?;
        let row = slot.checked_div(self.columns.get())?;
        Some(AtlasTileRect {
            height: self.tile_height,
            width: self.tile_width,
            x: column.checked_mul(self.tile_width.get())?,
            y: row.checked_mul(self.tile_height.get())?,
        })
    }

    fn validate(
        self,
        tiles: &IndexedTiles<'_>,
        selections: &[AtlasTile<'_>],
        output_pixels: usize,
    ) -> Result<(), AssetError> {
        if tiles.width() != self.tile_width
            || tiles.height() != self.tile_height
            || u32::try_from(selections.len()).ok() != Some(self.slots.get())
            || output_pixels != self.pixels()
        {
            return Err(AssetError::InvalidFormat);
        }
        for selection in selections {
            let indices = tiles
                .tile(selection.index)
                .ok_or(AssetError::InvalidFormat)?;
            super::validate_palette(indices, selection.palette)?;
        }
        Ok(())
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(self) -> NonZeroU32 {
        self.width
    }

    fn write_tile(
        self,
        colors: TileColors<'_>,
        rect: AtlasTileRect,
        output: &mut [[u8; 4]],
    ) -> Result<(), AssetError> {
        let TileColors { indices, palette } = colors;
        let tile_width = usize::try_from(rect.width.get())
            .map_err(|_error| AssetError::TooLarge)?;
        let atlas_width = usize::try_from(self.width.get())
            .map_err(|_error| AssetError::TooLarge)?;
        let top =
            usize::try_from(rect.y).map_err(|_error| AssetError::TooLarge)?;
        let left =
            usize::try_from(rect.x).map_err(|_error| AssetError::TooLarge)?;
        let right = left.checked_add(tile_width).ok_or(AssetError::TooLarge)?;
        for (source, row) in indices
            .chunks_exact(tile_width)
            .zip(output.chunks_exact_mut(atlas_width).skip(top))
        {
            let target =
                row.get_mut(left..right).ok_or(AssetError::InvalidFormat)?;
            super::write_colors(source, palette, target)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{AtlasTile, AtlasTileRect, TileAtlasLayout};
    use crate::indexed::IndexedTiles;
    use crate::{AssetError, AssetFormat, AssetRecord, ContentHash};

    const PIXELS: [u8; 8] = [0, 1, 2, 3, 3, 2, 1, 0];
    const CLEAR: [u8; 4] = [0; 4];
    const BLUE: [u8; 4] = [0, 0, 255, 128];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const WHITE: [u8; 4] = [255; 4];
    const PALETTE: [[u8; 4]; 4] = [CLEAR, RED, GREEN, BLUE];
    const RECOLOR: [[u8; 4]; 4] = [WHITE, BLUE, RED, GREEN];

    fn budget(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
    }

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    fn sheet() -> Result<IndexedTiles<'static>, AssetError> {
        let record = AssetRecord {
            digest: ContentHash::calculate(&PIXELS),
            format: AssetFormat::IndexedTiles8 {
                height: nz(2),
                tiles: nz(2),
                width: nz(2),
            },
            key: String::from("graphics/test"),
            size: 8,
        };
        IndexedTiles::new(&record, &PIXELS)
    }

    fn selections() -> [AtlasTile<'static>; 3] {
        [
            AtlasTile {
                index: 0,
                palette: &PALETTE,
            },
            AtlasTile {
                index: 1,
                palette: &PALETTE,
            },
            AtlasTile {
                index: 0,
                palette: &RECOLOR,
            },
        ]
    }

    #[test]
    fn atlas_coordinates_and_rgba_rows_are_exact() {
        let fixture = sheet();
        assert!(fixture.is_ok(), "valid fixture");
        let Ok(tiles) = fixture else {
            return;
        };
        let result = TileAtlasLayout::new(&tiles, nz(3), nz(2), budget(64));
        assert!(result.is_ok(), "valid layout");
        let Ok(layout) = result else {
            return;
        };
        assert_eq!(layout.width(), nz(4));
        assert_eq!(layout.height(), nz(4));
        assert_eq!(layout.bytes(), 64);
        assert_eq!(
            layout.rect(2),
            Some(AtlasTileRect {
                height: nz(2),
                width: nz(2),
                x: 0,
                y: 2,
            })
        );
        assert_eq!(layout.rect(3), None);
        let mut output = [[42; 4]; 16];
        assert_eq!(layout.pack(&tiles, &selections(), &mut output), Ok(()));
        assert_eq!(output, [
            CLEAR, RED, BLUE, GREEN, GREEN, BLUE, RED, CLEAR, WHITE, BLUE,
            CLEAR, CLEAR, RED, GREEN, CLEAR, CLEAR,
        ]);
    }

    #[test]
    fn rejects_invalid_layout() {
        let result = sheet();
        assert!(result.is_ok(), "valid fixture");
        let Ok(tiles) = result else {
            return;
        };
        assert_eq!(
            TileAtlasLayout::new(&tiles, nz(3), nz(2), budget(63)),
            Err(AssetError::TooLarge)
        );
        assert_eq!(
            TileAtlasLayout::new(&tiles, nz(3), nz(4), budget(64)),
            Err(AssetError::InvalidFormat)
        );
        assert_eq!(
            TileAtlasLayout::new(
                &tiles,
                NonZeroU32::MAX,
                NonZeroU32::MAX,
                NonZeroUsize::MAX,
            ),
            Err(AssetError::TooLarge)
        );
    }

    #[test]
    fn rejects_invalid_selections_without_writing() {
        let fixture = sheet();
        assert!(fixture.is_ok(), "valid fixture");
        let Ok(tiles) = fixture else {
            return;
        };
        let result = TileAtlasLayout::new(&tiles, nz(3), nz(2), budget(64));
        assert!(result.is_ok(), "valid layout");
        let Ok(layout) = result else {
            return;
        };
        let [first, second, third] = selections();
        let invalid = AtlasTile { palette: &[], ..third };
        let mut output = [[42; 4]; 16];
        assert_eq!(
            layout.pack(&tiles, &[first, second, invalid], &mut output),
            Err(AssetError::InvalidFormat)
        );
        assert_eq!(output, [[42; 4]; 16]);
        let invalid_index = AtlasTile { index: 2, ..third };
        assert_eq!(
            layout.pack(&tiles, &[first, second, invalid_index], &mut output),
            Err(AssetError::InvalidFormat)
        );
        assert_eq!(output, [[42; 4]; 16]);
        assert_eq!(
            layout.pack(&tiles, &[first], &mut output),
            Err(AssetError::InvalidFormat)
        );
        assert_eq!(output, [[42; 4]; 16]);
        let mut short = [[42; 4]; 15];
        assert_eq!(
            layout.pack(&tiles, &selections(), &mut short),
            Err(AssetError::InvalidFormat)
        );
        assert_eq!(short, [[42; 4]; 15]);
    }
}
