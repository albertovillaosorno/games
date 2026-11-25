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
//   - Compose observed standard-scrolling indexed tile layers.
// - Must-Not:
//   - Execute game code, draw sprites or implement gameplay.
// - Allows:
//   - Inputs: Profile raster, observed tile state and canonical tiles.
//   - Outputs: Visible row-major palette indices and layer coverage.
//   - Side effects: Writes to caller-owned raster buffer.
// - Split-When:
//   - A different tilemap layout needs an independent decoder.
// - Merge-When:
//   - Shared source composition supports the same raster contract.
// - Summary:
//   - Reconstruct background pixels from verified source observations.
// - Description:
//   - Wrap tile coordinates and compose transparent pens in mixer order.
// - Usage:
//   - Supply exact observed RAM, global scroll and indexed tile data.
// - Defaults:
//   - Reject unsupported modes and invalid buffer or tile extents.

//! Source-reference background raster, not a gameplay renderer.

use games_assets::indexed::IndexedTiles;

use super::tilemap::{Offset, TilemapSnapshot};
use crate::ImportError;

/// Canonical tiles and raw tilemap state for one observation.
#[derive(Clone, Copy, Debug)]
pub struct Background<'state, 'tiles> {
    /// Original scroll-mode register.
    pub scroll_control: u8,
    /// Decoded tilemap and priority state.
    pub state: TilemapSnapshot<'state>,
    /// Canonical, integrity-checked tile indices.
    pub tiles: &'tiles IndexedTiles<'tiles>,
}

/// A reconstructed palette index with tile coverage and sprite effects.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pixel {
    pub(super) pen: u16,
    pub(super) priority: u8,
    pub(super) shadow: bool,
}

impl Pixel {
    /// Construct an unmodified palette index without coverage.
    #[must_use]
    pub const fn from_pen(pen: u16) -> Self {
        Self {
            pen,
            priority: 0,
            shadow: false,
        }
    }

    /// Unmodified palette index, before any shadow.
    #[must_use]
    pub const fn pen(self) -> u16 {
        self.pen
    }

    /// Resolve ordinary and indexed-palette shadow pixels.
    /// Returns no value for a missing palette entry.
    #[must_use]
    pub fn rgba(self, palette: &[[u8; 4]]) -> Option<[u8; 4]> {
        let color = *palette.get(usize::from(self.pen))?;
        if !self.shadow {
            return Some(color);
        }
        let [red, green, blue, alpha] = color;
        let channel = |value: u8| {
            u8::try_from(u16::from(value).saturating_mul(3).checked_div(5)?)
                .ok()
        };
        Some([channel(red)?, channel(green)?, channel(blue)?, alpha])
    }
}

/// Fixed visible raster geometry selected by machine profile.
#[derive(Clone, Copy, Debug)]
pub struct Raster {
    pub(super) height: usize,
    pub(super) layer_x: [usize; 3],
    pub(super) origin_y: usize,
    pub(super) width: usize,
}

impl Raster {
    /// Compose only the three tile layers and their backdrop.
    ///
    /// Sprite pixels, shadows, line scroll and global reversal are excluded.
    /// # Errors
    /// Rejects invalid extents, missing tiles, pens or unsupported scroll.
    /// A decoding failure may leave a partially filled output.
    pub fn compose(
        self,
        background: Background<'_, '_>,
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        if output.len() != self.pixels()
            || background.tiles.width().get() != 8
            || background.tiles.height().get() != 8
        {
            return Err(ImportError::UnsupportedMember);
        }
        let scroll = background
            .state
            .standard_scroll(background.scroll_control)?;
        let mixer = background.state.priority();
        output.fill(Pixel {
            pen: u16::from(mixer.backdrop.palette_bank) << 4u32,
            ..Pixel::default()
        });
        for (layer, coverage) in
            mixer.ordered_layers().into_iter().zip([1, 2, 4])
        {
            let offset =
                *scroll.get(layer).ok_or(ImportError::UnsupportedMember)?;
            self.compose_layer(
                Layer {
                    background,
                    coverage,
                    index: layer,
                },
                offset,
                output,
            )?;
        }
        Ok(())
    }

    fn compose_layer(
        self,
        layer: Layer<'_, '_>,
        offset: Offset,
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        let (scroll_x, scroll_y) = offset;
        let alignment = *self
            .layer_x
            .get(layer.index)
            .ok_or(ImportError::UnsupportedMember)?;
        let start_x = scroll_x.wrapping_add(alignment);
        let start_y = scroll_y.wrapping_add(self.origin_y);
        for (row, pixels) in output.chunks_exact_mut(self.width).enumerate() {
            layer.compose_row((start_x, start_y.wrapping_add(row)), pixels)?;
        }
        Ok(())
    }

    /// Width and height of the visible raster in logical pixels.
    #[must_use]
    pub const fn extent(self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Number of visible pixels in the machine's bounded raster.
    #[must_use]
    pub const fn pixels(self) -> usize {
        self.width.saturating_mul(self.height)
    }
}

#[derive(Clone, Copy, Debug)]
struct Layer<'state, 'tiles> {
    background: Background<'state, 'tiles>,
    coverage: u8,
    index: usize,
}

impl Layer<'_, '_> {
    fn compose_row(
        self,
        origin: Offset,
        mut pixels: &mut [Pixel],
    ) -> Result<(), ImportError> {
        let mut column = origin.0 & 511;
        let row = origin.1 & 255;
        while !pixels.is_empty() {
            let count = pixels.len().min(8usize.saturating_sub(column & 7));
            let (span, remaining) = pixels
                .split_at_mut_checked(count)
                .ok_or(ImportError::UnsupportedMember)?;
            self.compose_span((column, row), span)?;
            column = column.wrapping_add(count) & 511;
            pixels = remaining;
        }
        Ok(())
    }

    fn compose_span(
        self,
        point: Offset,
        pixels: &mut [Pixel],
    ) -> Result<(), ImportError> {
        let (column, row) = point;
        let cell_index = ((row >> 3u32) << 6u32) | (column >> 3u32);
        let cell = self
            .background
            .state
            .cell(self.index, cell_index)
            .ok_or(ImportError::UnsupportedMember)?;
        let tile_row = if cell.flip_y {
            7 ^ (row & 7)
        } else {
            row & 7
        };
        let start = (tile_row << 3u32) | (column & 7);
        let end = start.saturating_add(pixels.len());
        let pens = self
            .background
            .tiles
            .tile(u32::from(cell.index))
            .and_then(|tile| tile.get(start..end))
            .ok_or(ImportError::UnsupportedMember)?;
        let palette = u16::from(cell.palette_bank) << 4u32;
        for (pixel, &pen) in pixels.iter_mut().zip(pens) {
            if pen > 15 {
                return Err(ImportError::UnsupportedMember);
            }
            if pen != 0 {
                pixel.pen = palette | u16::from(pen);
                pixel.priority |= self.coverage;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use games_assets::{AssetFormat, AssetRecord, ContentHash};

    use super::*;
    use crate::mame::Machine;

    fn sheet(pixels: &[u8]) -> Result<IndexedTiles<'_>, ImportError> {
        let side = NonZeroU32::new(8).ok_or(ImportError::UnsupportedMember)?;
        let byte_count = u64::try_from(pixels.len())
            .map_err(|_error| ImportError::TooLarge)?;
        let tiles = u32::try_from(pixels.len() >> 6u32)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(ImportError::UnsupportedMember)?;
        let record = AssetRecord {
            digest: ContentHash::calculate(pixels),
            format: AssetFormat::IndexedTiles8 {
                height: side,
                tiles,
                width: side,
            },
            key: String::from("graphics/tiles"),
            size: byte_count,
        };
        Ok(IndexedTiles::new(&record, pixels)?)
    }

    #[test]
    fn spans_preserve_transparency_and_wrap_across_tile_boundaries()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pixels = [0u8; 128];
        pixels
            .get_mut(..8)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[1, 0, 3, 4, 5, 6, 7, 8]);
        pixels
            .get_mut(64..72)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[8, 7, 6, 5, 4, 3, 2, 1]);
        let sheet = sheet(&pixels)?;
        let mut ram = vec![0u8; 24_576];
        *ram.get_mut(8192 + 63)
            .ok_or(ImportError::UnsupportedMember)? = 1;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let state = machine.tilemap_snapshot(&ram, &[0; 6], mixer)?;
        let layer = Layer {
            background: Background {
                scroll_control: 0,
                state,
                tiles: &sheet,
            },
            coverage: 4,
            index: 0,
        };
        let initial = Pixel {
            pen: 99,
            priority: 1,
            shadow: false,
        };
        let mut output = [initial; 12];
        layer.compose_row((509, 0), &mut output)?;
        let expected = [3, 2, 1, 1, 99, 3, 4, 5, 6, 7, 8, 1];
        if output.map(Pixel::pen) != expected
            || output.map(|pixel| pixel.priority)
                != [5, 5, 5, 5, 1, 5, 5, 5, 5, 5, 5, 5]
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }

    #[test]
    fn local_flip_and_unsupported_global_flip() -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pixels = vec![0u8; 32_832];
        *pixels
            .get_mut(32_768)
            .ok_or(ImportError::UnsupportedMember)? = 1;
        *pixels
            .get_mut(32_824)
            .ok_or(ImportError::UnsupportedMember)? = 5;
        let sheet = sheet(&pixels)?;
        let mut ram = vec![0u8; 24_576];
        *ram.first_mut().ok_or(ImportError::UnsupportedMember)? = 2;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let raster = Raster {
            height: 1,
            layer_x: [0; 3],
            origin_y: 0,
            width: 1,
        };
        let mut output = [Pixel::default()];
        for (flip, expected) in [(0, 1), (4, 5)] {
            let state = machine.tilemap_snapshot(
                &ram,
                &[0, 0, 0, 0, 0, flip],
                mixer,
            )?;
            raster.compose(
                Background {
                    scroll_control: 0,
                    state,
                    tiles: &sheet,
                },
                &mut output,
            )?;
            if output.map(Pixel::pen) != [expected] {
                return Err(ImportError::UnsupportedMember);
            }
        }
        let state =
            machine.tilemap_snapshot(&ram, &[0, 0, 0, 0, 0, 1], mixer)?;
        if raster
            .compose(
                Background {
                    scroll_control: 0,
                    state,
                    tiles: &sheet,
                },
                &mut output,
            )
            .is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }

    #[test]
    fn wraps_and_composes_transparent_pens_in_priority_order()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pixels = [0u8; 64];
        *pixels.first_mut().ok_or(ImportError::UnsupportedMember)? = 2;
        *pixels.last_mut().ok_or(ImportError::UnsupportedMember)? = 3;
        let sheet = sheet(&pixels)?;
        let mut ram = vec![0u8; 24_576];
        // Layer 1 scrolls right one pixel and wraps vertically.
        for (index, value) in [(0x1a00, 1), (0x180c, 255)] {
            *ram.get_mut(index).ok_or(ImportError::UnsupportedMember)? = value;
        }
        let mut registers = [0; 16];
        // Distinct palette banks make the winning layer observable.
        *registers.get_mut(9).ok_or(ImportError::UnsupportedMember)? = 32;
        *registers
            .get_mut(10)
            .ok_or(ImportError::UnsupportedMember)? = 17;
        let mixer = machine.decode_video_priority(&registers)?;
        let state = machine.tilemap_snapshot(&ram, &[0; 6], mixer)?;
        let raster = Raster {
            height: 2,
            layer_x: [511, 510, 511],
            origin_y: 0,
            width: 2,
        };
        let input = Background {
            scroll_control: 0,
            state,
            tiles: &sheet,
        };
        let mut output = [Pixel::default(); 4];
        raster.compose(input, &mut output)?;
        // Layer 1 contributes its final pen at row 255, column 511.
        // Layer 2 contributes its first pen at row 0, column 0.
        // At the next row, layer 1 contributes its first pen at column 0.
        if output.map(Pixel::pen) != [259, 514, 0, 258]
            || raster.compose(input, &mut [Pixel::default(); 3]).is_ok()
            || raster
                .compose(
                    Background {
                        scroll_control: 2,
                        ..input
                    },
                    &mut output,
                )
                .is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }
}
