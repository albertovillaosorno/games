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
//   - Checked tile views and palette expansion of canonical indexed media.
// - Must-Not:
//   - Infer source palettes, access files, allocate textures or draw frames.
// - Allows:
//   - Inputs: Verified descriptors, payloads, palettes and output buffers.
//   - Outputs: Borrowed tile indices and caller-owned RGBA pixels.
//   - Side effects: Writes a complete tile into the caller's output buffer.
// - Split-When:
//   - A renderer needs a separate GPU resource or composition contract.
// - Merge-When:
//   - Canonical media access otherwise duplicates tile addressing.
// - Summary:
//   - Allocation-free runtime access to indexed tile assets.
// - Description:
//   - Checks integrity once, then exposes tile-major pixels by tile index.
// - Usage:
//   - Bind a canonical asset; use its indices or expand a tile with a palette.
// - Defaults:
//   - Reject invalid descriptors, indices, palettes and output lengths.

//! Source-independent indexed graphics access.

use std::num::{NonZeroU32, NonZeroU64};

use crate::{AssetError, AssetFormat, AssetRecord};

/// Borrowed, integrity-checked tile-major pixel indices.
#[derive(Clone, Copy, Debug)]
pub struct IndexedTiles<'payload> {
    height: NonZeroU32,
    pixels: &'payload [u8],
    tile_pixels: usize,
    tiles: NonZeroU32,
    width: NonZeroU32,
}

impl<'payload> IndexedTiles<'payload> {
    /// Expand one tile into straight-alpha RGBA pixels in row-major order.
    ///
    /// The palette supplies alpha explicitly, including transparent entries.
    /// No output is changed if the tile, palette or output length is invalid.
    ///
    /// # Errors
    /// Rejects unknown tiles, missing palette entries or wrong output size.
    pub fn expand(
        &self,
        index: u32,
        palette: &[[u8; 4]],
        output: &mut [[u8; 4]],
    ) -> Result<(), AssetError> {
        let pixels = self.tile(index).ok_or(AssetError::InvalidFormat)?;
        if pixels.len() != output.len()
            || pixels
                .iter()
                .any(|pixel| usize::from(*pixel) >= palette.len())
        {
            return Err(AssetError::InvalidFormat);
        }
        for (pixel, color) in pixels.iter().zip(output) {
            *color = *palette
                .get(usize::from(*pixel))
                .ok_or(AssetError::InvalidFormat)?;
        }
        Ok(())
    }

    /// Height in pixels of every tile.
    #[must_use]
    pub const fn height(&self) -> NonZeroU32 {
        self.height
    }

    /// Bind a canonical indexed asset, verifying the exact payload once.
    ///
    /// # Errors
    /// Rejects non-indexed formats, malformed descriptors or wrong bytes.
    pub fn new(
        record: &AssetRecord,
        pixels: &'payload [u8],
    ) -> Result<Self, AssetError> {
        let AssetFormat::IndexedTiles8 { height, tiles, width } = record.format
        else {
            return Err(AssetError::InvalidFormat);
        };
        record.verify(pixels)?;
        let tile_pixels = usize::try_from(
            NonZeroU64::from(height)
                .checked_mul(NonZeroU64::from(width))
                .ok_or(AssetError::InvalidFormat)?
                .get(),
        )
        .map_err(|_error| AssetError::TooLarge)?;
        Ok(Self {
            height,
            pixels,
            tile_pixels,
            tiles,
            width,
        })
    }

    /// Return the complete tile-major byte array for indexed GPU uploads.
    #[must_use]
    pub const fn pixels(&self) -> &'payload [u8] {
        self.pixels
    }

    /// Borrow one row-major tile, without copying or hashing again.
    #[must_use]
    pub fn tile(&self, index: u32) -> Option<&'payload [u8]> {
        if index >= self.tiles.get() {
            return None;
        }
        let start =
            usize::try_from(index).ok()?.checked_mul(self.tile_pixels)?;
        let end = start.checked_add(self.tile_pixels)?;
        self.pixels.get(start..end)
    }

    /// Number of complete tiles.
    #[must_use]
    pub const fn tiles(&self) -> NonZeroU32 {
        self.tiles
    }

    /// Width in pixels of every tile.
    #[must_use]
    pub const fn width(&self) -> NonZeroU32 {
        self.width
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::IndexedTiles;
    use crate::{AssetError, AssetFormat, AssetRecord, ContentHash};

    const PIXELS: [u8; 12] = [0, 1, 2, 3, 2, 1, 3, 0, 1, 0, 2, 3];

    fn record() -> AssetRecord {
        AssetRecord {
            digest: ContentHash::calculate(&PIXELS),
            format: AssetFormat::IndexedTiles8 {
                height: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
                tiles: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
                width: NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            },
            key: String::from("graphics/tiles"),
            size: 12,
        }
    }

    #[test]
    fn exposes_exact_tile_boundaries() {
        let result = IndexedTiles::new(&record(), &PIXELS);
        assert!(result.is_ok(), "valid indexed asset");
        let Ok(sheet) = result else {
            return;
        };
        assert_eq!(sheet.width().get(), 3, "tile width");
        assert_eq!(sheet.height().get(), 2, "tile height");
        assert_eq!(sheet.tiles().get(), 2, "tile count");
        assert_eq!(sheet.pixels(), PIXELS, "complete indexed texture");
        assert_eq!(sheet.tile(0), PIXELS.get(..6), "first tile");
        assert_eq!(sheet.tile(1), PIXELS.get(6..), "last tile");
        assert_eq!(sheet.tile(2), None, "past end");
        assert_eq!(sheet.tile(u32::MAX), None, "extreme tile index");
    }

    #[test]
    fn expands_palette_without_partial_output() {
        let result = IndexedTiles::new(&record(), &PIXELS);
        assert!(result.is_ok(), "valid indexed asset");
        let Ok(sheet) = result else {
            return;
        };
        let palette = [[0, 0, 0, 0], [255, 0, 0, 255], [0, 255, 0, 255], [
            0, 0, 255, 128,
        ]];
        let mut output = [[42; 4]; 6];
        assert_eq!(
            sheet.expand(1, &palette, &mut output),
            Ok(()),
            "valid palette"
        );
        assert_eq!(
            output,
            [
                [0, 0, 255, 128],
                [0, 0, 0, 0],
                [255, 0, 0, 255],
                [0, 0, 0, 0],
                [0, 255, 0, 255],
                [0, 0, 255, 128],
            ],
            "row-major colors with explicit alpha"
        );
        let before = output;
        assert_eq!(
            sheet.expand(0, &[], &mut output),
            Err(AssetError::InvalidFormat),
            "missing palette"
        );
        assert_eq!(
            sheet.expand(2, &palette, &mut output),
            Err(AssetError::InvalidFormat),
            "missing tile"
        );
        assert_eq!(output, before, "rejections must be atomic");
        let mut short = [[42; 4]; 5];
        assert_eq!(
            sheet.expand(0, &palette, &mut short),
            Err(AssetError::InvalidFormat),
            "short buffer"
        );
        assert_eq!(short, [[42; 4]; 5], "short buffer stays unchanged");
    }

    #[test]
    fn rejects_noncanonical_payloads() {
        let mut descriptor = record();
        assert_eq!(
            IndexedTiles::new(&descriptor, &[0; 12]).err(),
            Some(AssetError::PayloadMismatch),
            "tampered bytes"
        );
        assert_eq!(
            IndexedTiles::new(&descriptor, &[]).err(),
            Some(AssetError::PayloadMismatch),
            "truncated bytes"
        );
        descriptor.format = AssetFormat::Data;
        assert_eq!(
            IndexedTiles::new(&descriptor, &PIXELS).err(),
            Some(AssetError::InvalidFormat),
            "wrong encoding"
        );
    }
}
