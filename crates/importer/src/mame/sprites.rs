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
//   - Reconstruct supported observed K053247 sprite pixels and priority.
// - Must-Not:
//   - Execute program ROMs, simulate device timing or implement gameplay.
// - Allows:
//   - Inputs: Bounded object records, control registers and canonical tiles.
//   - Outputs: Visible coverage or a bounded, independently registered group.
//   - Side effects: Writes only to caller-owned pixels.
// - Split-When:
//   - A different object chip requires a different geometry decoder.
// - Merge-When:
//   - Shared arcade reference composition supports the same object format.
// - Summary:
//   - Resolve observed sprites with per-axis fixed-point zoom and Y mirror.
// - Description:
//   - Reject unsupported control modes and ambiguous equal-depth objects.
// - Usage:
//   - Compose a background first, then overlay a matching device snapshot.
// - Defaults:
//   - Clip to the profile raster and preserve transparent pixels.

//! Source-reference sprite reconstruction, not a gameplay renderer.

use std::num::NonZeroUsize;
use std::ops::Range;

use games_assets::indexed::IndexedTiles;

use super::background::{Pixel, Raster};
use super::objects::{ObjectEntry, ObjectRam};
use super::priority::VideoPriority;
use crate::ImportError;

#[derive(Clone, Copy, Debug)]
pub(super) struct Wiring {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[derive(Clone, Copy, Debug)]
struct Display {
    offset_x: i32,
    offset_y: i32,
    wiring: Wiring,
}

/// Exact device snapshot with the supported original display controls.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot<'source> {
    display: Display,
    mixer: VideoPriority,
    ram: ObjectRam<'source>,
}

/// Complete isolated object-group pixels and their signed screen registration.
#[derive(Debug)]
pub struct ObjectRegion {
    /// Width and height of the original object coverage bounds.
    pub extent: (usize, usize),
    /// Top-left location relative to the original visible raster.
    pub origin: (i32, i32),
    /// Row-major pixels; uncovered pixels have pen `u16::MAX`.
    pub pixels: Vec<Pixel>,
}

impl<'source> Snapshot<'source> {
    /// Reconstruct the entire selected group, including offscreen objects.
    ///
    /// No original display coordinates are altered before wrapping or zoom.
    /// The caller chooses the maximum number of pixels that may be allocated.
    /// # Errors
    /// Rejects empty groups, unsupported objects, ambiguous depths or a
    /// group whose checked extent exceeds the allocation budget.
    pub fn isolate(
        self,
        raster: Raster,
        tiles: &IndexedTiles<'_>,
        max_pixels: NonZeroUsize,
    ) -> Result<ObjectRegion, ImportError> {
        let objects = self.objects(tiles)?;
        let mut bounds = None;
        for object in &objects {
            let (horizontal, vertical) =
                (object.x.bounds()?, object.y.bounds()?);
            bounds = Some(bounds.map_or(
                (
                    horizontal.start,
                    vertical.start,
                    horizontal.end,
                    vertical.end,
                ),
                |(left, top, right, bottom): (i32, i32, i32, i32)| {
                    (
                        left.min(horizontal.start),
                        top.min(vertical.start),
                        right.max(horizontal.end),
                        bottom.max(vertical.end),
                    )
                },
            ));
        }
        let (left, top, right, bottom) =
            bounds.ok_or(ImportError::UnsupportedMember)?;
        let width = usize::try_from(right.saturating_sub(left))
            .map_err(|_error| ImportError::TooLarge)?;
        let height = usize::try_from(bottom.saturating_sub(top))
            .map_err(|_error| ImportError::TooLarge)?;
        let count = width
            .checked_mul(height)
            .filter(|count| *count != 0 && *count <= max_pixels.get())
            .ok_or(ImportError::TooLarge)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_error| ImportError::TooLarge)?;
        pixels.resize(count, Pixel::from_pen(u16::MAX));
        let region = Raster {
            height,
            layer_x: [0; 3],
            origin_y: 0,
            width,
        };
        for mut object in objects {
            object.x.start = object.x.start.saturating_sub(left);
            object.y.start = object.y.start.saturating_sub(top);
            object.draw(region, tiles, &mut pixels)?;
        }
        let origin_y = i32::try_from(raster.origin_y)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        Ok(ObjectRegion {
            extent: (width, height),
            origin: (left, top.saturating_sub(origin_y)),
            pixels,
        })
    }

    pub(super) fn new(
        wiring: Wiring,
        ram: &'source [u8],
        controls: &[u8],
        mixer: VideoPriority,
    ) -> Result<Self, ImportError> {
        let (window, rest) = controls
            .split_first_chunk::<8>()
            .ok_or(ImportError::UnsupportedMember)?;
        let (opset, rejection) = rest
            .split_first_chunk::<32>()
            .ok_or(ImportError::UnsupportedMember)?;
        let &[x_high, x_low, y_high, y_low, _, flags, _, _] = window;
        // Only observed normal-width, non-reversed, 1024-wrap mode.
        if flags & 0x0b != 0
            || opset.get(12).is_none_or(|value| value & 0x50 != 0)
            || rejection != [255; 4]
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(Self {
            display: Display {
                offset_x: i32::from(i16::from_be_bytes([x_high, x_low])),
                offset_y: i32::from(i16::from_be_bytes([y_high, y_low])),
                wiring,
            },
            mixer,
            ram: ObjectRam::new(ram)?,
        })
    }

    fn objects(
        self,
        tiles: &IndexedTiles<'_>,
    ) -> Result<Vec<Object>, ImportError> {
        if tiles.width().get() != 16 || tiles.height().get() != 16 {
            return Err(ImportError::UnsupportedMember);
        }
        let mut entries: Vec<_> =
            self.ram.entries().filter(|entry| entry.active()).collect();
        entries.sort_unstable_by_key(|entry| entry.depth());
        if entries.windows(2).any(|pair| {
            matches!(pair, [first, second] if first.depth() == second.depth())
        }) {
            return Err(ImportError::UnsupportedMember);
        }
        entries
            .into_iter()
            .map(|entry| Object::new(entry, self.display, self.mixer))
            .collect()
    }

    /// Overlay supported sprite records on an already composed raster.
    /// # Errors
    /// Rejects unsupported modes, equal depths, and invalid sprite tiles.
    /// An invalid observation may leave a partly composed output.
    pub fn overlay(
        self,
        raster: Raster,
        tiles: &IndexedTiles<'_>,
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        if output.len() != raster.pixels() {
            return Err(ImportError::UnsupportedMember);
        }
        for object in self.objects(tiles)? {
            object.draw(raster, tiles, output)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Axis {
    code_start: u16,
    count: u32,
    flip: bool,
    mirror: bool,
    scale: u32,
    start: i32,
}

impl Axis {
    fn bounds(self) -> Result<Range<i32>, ImportError> {
        let last = self.part(self.count.saturating_sub(1))?;
        Ok(self.start..last.end)
    }

    fn part(self, cell: u32) -> Result<Part, ImportError> {
        let boundary = |index: u32| {
            i32::try_from(
                self.scale.saturating_mul(index).saturating_add(2048) >> 12u32,
            )
            .map(|value| self.start.saturating_add(value))
            .map_err(|_error| ImportError::UnsupportedMember)
        };
        let start = boundary(cell)?;
        let end = boundary(cell.saturating_add(1))?;
        let flip = if self.mirror {
            let first_half = cell.saturating_mul(2) < self.count;
            first_half != self.flip
        } else {
            self.flip
        };
        let index = if flip {
            self.count.saturating_sub(1).saturating_sub(cell)
        } else {
            cell
        };
        let tile = u16::try_from(index)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        Ok(Part {
            code: spread((self.code_start.wrapping_add(tile)) & 7),
            end,
            flip,
            start,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Part {
    code: u16,
    end: i32,
    flip: bool,
    start: i32,
}

impl Part {
    fn clip(self, limit: usize) -> Result<Range<i32>, ImportError> {
        let signed_limit = i32::try_from(limit)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        Ok(self.start.max(0)..self.end.min(signed_limit))
    }

    fn sample(self, position: i32) -> Result<usize, ImportError> {
        let extent = self.end.saturating_sub(self.start);
        let step = (16i32 << 16u32)
            .checked_div(extent)
            .ok_or(ImportError::UnsupportedMember)?;
        let offset = if self.flip {
            self.end.saturating_sub(1).saturating_sub(position)
        } else {
            position.saturating_sub(self.start)
        };
        usize::try_from(offset.saturating_mul(step) >> 16u32)
            .map_err(|_error| ImportError::UnsupportedMember)
    }
}

#[derive(Clone, Copy, Debug)]
struct Ink {
    bank: u16,
    blocked: u8,
    shadow: bool,
}

impl Ink {
    fn apply(self, pixel: &mut Pixel, pen: u8) -> Result<(), ImportError> {
        if pen > 15 {
            return Err(ImportError::UnsupportedMember);
        }
        if pen == 0 {
            return Ok(());
        }
        let visible =
            pixel.priority & 31 != 31 && pixel.priority & self.blocked == 0;
        if pen == 15 && self.shadow {
            if visible && pixel.priority & 128 == 0 {
                pixel.shadow = true;
                pixel.priority |= 128;
            }
        } else {
            if visible {
                pixel.pen = (self.bank << 4u32) | u16::from(pen);
                pixel.shadow = false;
            }
            pixel.priority = 31;
        }
        Ok(())
    }

    fn new(attributes: u16, mixer: VideoPriority) -> Result<Self, ImportError> {
        let mut blocked = 0;
        let priority = (attributes & 0x0f80) >> 6u32;
        for (layer, mask) in mixer.ordered_layers().into_iter().zip([1, 2, 4]) {
            let input = mixer
                .layers
                .get(layer)
                .ok_or(ImportError::UnsupportedMember)?;
            if priority > u16::from(input.priority) {
                blocked |= mask;
            }
        }
        Ok(Self {
            bank: u16::from(mixer.sprites.palette_bank)
                .saturating_add(attributes & 31),
            blocked,
            shadow: attributes & 0x0c00 != 0,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Object {
    code: u16,
    ink: Ink,
    x: Axis,
    y: Axis,
}

impl Object {
    fn draw(
        self,
        raster: Raster,
        tiles: &IndexedTiles<'_>,
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        for row in 0..self.y.count {
            let vertical = self.y.part(row)?;
            for column in 0..self.x.count {
                let horizontal = self.x.part(column)?;
                let code =
                    self.code | horizontal.code | (vertical.code << 1u32);
                let index = u32::from(code)
                    .checked_rem(tiles.tiles().get())
                    .ok_or(ImportError::UnsupportedMember)?;
                let tile =
                    tiles.tile(index).ok_or(ImportError::UnsupportedMember)?;
                TileDraw {
                    double_y: self.y.mirror && self.y.count == 1,
                    horizontal,
                    ink: self.ink,
                    vertical,
                }
                .draw(raster, tile, output)?;
            }
        }
        Ok(())
    }

    fn new(
        entry: ObjectEntry<'_>,
        display: Display,
        mixer: VideoPriority,
    ) -> Result<Self, ImportError> {
        let [flags, code, raw_y, raw_x, zoom_y, zoom_x, attributes, _] =
            entry.words();
        if attributes & 0x4000 != 0 {
            return Err(ImportError::UnsupportedMember);
        }
        let ink = Ink::new(attributes, mixer)?;
        let size_x = 1u32 << u32::from((flags >> 8u32) & 3);
        let size_y = 1u32 << u32::from((flags >> 10u32) & 3);
        let scale_y = scale(zoom_y)?;
        let scale_x = if flags & 0x4000 != 0 {
            scale_y
        } else {
            scale(zoom_x)?
        };
        let center_x = wrap(
            i32::from(raw_x & 1023).saturating_sub(display.offset_x),
            640,
        )
        .saturating_add(display.wiring.x);
        let center_y = wrap(
            i32::from(raw_y & 1023)
                .saturating_neg()
                .saturating_sub(display.offset_y),
            512,
        )
        .saturating_add(display.wiring.y);
        Ok(Self {
            code: code & !63,
            ink,
            x: Axis {
                code_start: compact(code),
                count: size_x,
                flip: flags & 0x1000 != 0,
                mirror: false,
                scale: scale_x,
                start: start(center_x, scale_x, size_x)?,
            },
            y: Axis {
                code_start: compact(code >> 1u32),
                count: size_y,
                flip: flags & 0x2000 != 0,
                mirror: attributes & 0x8000 != 0,
                scale: scale_y,
                start: start(center_y, scale_y, size_y)?,
            },
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct TileDraw {
    double_y: bool,
    horizontal: Part,
    ink: Ink,
    vertical: Part,
}

impl TileDraw {
    fn draw(
        self,
        raster: Raster,
        tile: &[u8],
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        self.draw_face(raster, tile, output)?;
        if self.double_y {
            Self {
                vertical: Part {
                    flip: !self.vertical.flip,
                    ..self.vertical
                },
                ..self
            }
            .draw_face(raster, tile, output)?;
        }
        Ok(())
    }

    fn draw_face(
        self,
        raster: Raster,
        tile: &[u8],
        output: &mut [Pixel],
    ) -> Result<(), ImportError> {
        let origin_y = i32::try_from(raster.origin_y)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        let vertical = Part {
            end: self.vertical.end.saturating_sub(origin_y),
            start: self.vertical.start.saturating_sub(origin_y),
            ..self.vertical
        };
        for row in vertical.clip(raster.height)? {
            let source_row = vertical.sample(row)? << 4u32;
            let destination_row = usize::try_from(row)
                .map_err(|_error| ImportError::UnsupportedMember)?
                .saturating_mul(raster.width);
            let end = destination_row.saturating_add(raster.width);
            let pixels = output
                .get_mut(destination_row..end)
                .ok_or(ImportError::UnsupportedMember)?;
            self.draw_row(tile, pixels, source_row)?;
        }
        Ok(())
    }

    fn draw_row(
        self,
        tile: &[u8],
        output: &mut [Pixel],
        source_row: usize,
    ) -> Result<(), ImportError> {
        for column in self.horizontal.clip(output.len())? {
            let source =
                source_row.saturating_add(self.horizontal.sample(column)?);
            let destination = usize::try_from(column)
                .map_err(|_error| ImportError::UnsupportedMember)?;
            let pen =
                *tile.get(source).ok_or(ImportError::UnsupportedMember)?;
            let pixel = output
                .get_mut(destination)
                .ok_or(ImportError::UnsupportedMember)?;
            self.ink.apply(pixel, pen)?;
        }
        Ok(())
    }
}

const fn compact(code: u16) -> u16 {
    (code & 1) | ((code & 4) >> 1u32) | ((code & 16) >> 2u32)
}

fn scale(raw: u16) -> Result<u32, ImportError> {
    let value = u32::from(raw & 1023);
    if value == 0 {
        return Ok(0x80_0000);
    }
    0x40_0000u32
        .saturating_add(value >> 1u32)
        .checked_div(value)
        .ok_or(ImportError::UnsupportedMember)
}

const fn spread(index: u16) -> u16 {
    (index & 1) | ((index & 2) << 1u32) | ((index & 4) << 2u32)
}

fn start(center: i32, scale: u32, count: u32) -> Result<i32, ImportError> {
    let half = i32::try_from(scale.saturating_mul(count) >> 13u32)
        .map_err(|_error| ImportError::UnsupportedMember)?;
    Ok(center.saturating_sub(half))
}

const fn wrap(value: i32, cutoff: i32) -> i32 {
    let position = value & 1023i32;
    if position >= cutoff {
        position.saturating_sub(1024)
    } else {
        position
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use games_assets::{AssetFormat, AssetRecord, ContentHash};

    use super::*;
    use crate::mame::Machine;
    use crate::mame::objects::RAM_BYTES;

    fn controls() -> Result<[u8; 44], ImportError> {
        let mut bytes = [0; 44];
        bytes
            .get_mut(40..)
            .ok_or(ImportError::UnsupportedMember)?
            .fill(255);
        Ok(bytes)
    }

    fn record(
        ram: &mut [u8],
        index: usize,
        words: [u16; 8],
    ) -> Result<(), ImportError> {
        let start = index.saturating_mul(16);
        let bytes = ram
            .get_mut(start..start.saturating_add(16))
            .ok_or(ImportError::UnsupportedMember)?;
        for (pair, word) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(words) {
            pair.copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }

    fn sheet(pens: &[u8; 512]) -> Result<IndexedTiles<'_>, ImportError> {
        let side = NonZeroU32::new(16).ok_or(ImportError::UnsupportedMember)?;
        let tiles = NonZeroU32::new(2).ok_or(ImportError::UnsupportedMember)?;
        let record = AssetRecord {
            digest: ContentHash::calculate(pens),
            format: AssetFormat::IndexedTiles8 {
                height: side,
                tiles,
                width: side,
            },
            key: String::from("graphics/sprites"),
            size: 512,
        };
        Ok(IndexedTiles::new(&record, pens)?)
    }

    #[test]
    fn opaque_occlusion_and_default_shadow() -> Result<(), ImportError> {
        let ink = Ink {
            bank: 1,
            blocked: 4,
            shadow: true,
        };
        let mut pixel = Pixel {
            pen: 1,
            priority: 2,
            shadow: false,
        };
        ink.apply(&mut pixel, 0)?;
        if pixel
            != (Pixel {
                pen: 1,
                priority: 2,
                shadow: false,
            })
        {
            return Err(ImportError::UnsupportedMember);
        }
        ink.apply(&mut pixel, 15)?;
        ink.apply(&mut pixel, 15)?;
        let palette = [[0, 0, 0, 255], [255, 132, 41, 255]];
        if pixel.rgba(&palette) != Some([153, 79, 24, 255]) {
            return Err(ImportError::UnsupportedMember);
        }
        // An opaque lower sprite replaces a low-priority shadow.
        ink.apply(&mut pixel, 3)?;
        ink.apply(&mut pixel, 4)?;
        if pixel
            != (Pixel {
                pen: 19,
                priority: 31,
                shadow: false,
            })
        {
            return Err(ImportError::UnsupportedMember);
        }
        // Even a blocked opaque sprite occludes sprites behind it.
        let mut blocked = Pixel {
            pen: 7,
            priority: 4,
            shadow: false,
        };
        ink.apply(&mut blocked, 2)?;
        Ink { blocked: 0, ..ink }.apply(&mut blocked, 5)?;
        if blocked.pen() != 7 || blocked.priority != 31 {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }

    #[test]
    fn overlay_mirrored_single_height_preserves_both_faces()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pens = [0; 512];
        pens.get_mut(..2)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[1, 2]);
        pens.get_mut(240..242)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[0, 3]);
        let tiles = sheet(&pens)?;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let raster = Raster {
            height: 16,
            layer_x: [0; 3],
            origin_y: 16,
            width: 16,
        };
        let mut ram = [0; RAM_BYTES];
        let controls = controls()?;
        record(&mut ram, 0, [0x8001, 0, 977, 51, 64, 64, 0x8000, 0])?;
        let mut output = [Pixel::default(); 256];
        machine.sprite_snapshot(&ram, &controls, mixer)?.overlay(
            raster,
            &tiles,
            &mut output,
        )?;
        let result = output.map(Pixel::pen);
        let mut expected = [0; 256];
        expected
            .get_mut(..2)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[1, 3]);
        expected
            .get_mut(240..242)
            .ok_or(ImportError::UnsupportedMember)?
            .copy_from_slice(&[1, 2]);
        if result != expected {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }

    #[test]
    fn overlay_scaled_multitile_flip_and_clipping() -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pens = [1; 512];
        pens.get_mut(256..)
            .ok_or(ImportError::UnsupportedMember)?
            .fill(2);
        let tiles = sheet(&pens)?;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let raster = Raster {
            height: 1,
            layer_x: [0; 3],
            origin_y: 16,
            width: 16,
        };
        let controls = controls()?;
        let mut ram = [0; RAM_BYTES];
        // Two horizontal tiles at half scale, clipped by two pixels.
        // Raw centre (49, 981) maps to (6, 20) after global wiring.
        for (flags, expected) in [
            (0x8101u16, [
                1u16, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0,
            ]),
            (0x9101u16, [
                2u16, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0,
            ]),
        ] {
            record(&mut ram, 0, [flags, 0, 981, 49, 128, 128, 0, 0])?;
            let mut output = [Pixel::default(); 16];
            machine.sprite_snapshot(&ram, &controls, mixer)?.overlay(
                raster,
                &tiles,
                &mut output,
            )?;
            if output.map(Pixel::pen) != expected {
                return Err(ImportError::UnsupportedMember);
            }
        }
        Ok(())
    }

    #[test]
    fn isolated_group_retains_offscreen_scaled_flipped_pixels()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mut pens = [1; 512];
        pens.get_mut(256..)
            .ok_or(ImportError::UnsupportedMember)?
            .fill(2);
        let tiles = sheet(&pens)?;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let raster = Raster {
            height: 4,
            layer_x: [0; 3],
            origin_y: 18,
            width: 14,
        };
        let controls = controls()?;
        let mut ram = [0; RAM_BYTES];
        let budget = NonZeroUsize::new(128).ok_or(ImportError::TooLarge)?;
        for (raw_x, origin_x) in [(49u16, -2i32), (76u16, 25i32)] {
            record(&mut ram, 0, [0x9101, 0, 981, raw_x, 128, 128, 0, 0])?;
            let snapshot = machine.sprite_snapshot(&ram, &controls, mixer)?;
            let group = snapshot.isolate(raster, &tiles, budget)?;
            if group.extent != (16, 8)
                || group.origin != (origin_x, -2i32)
                || group.pixels.len() != budget.get()
            {
                return Err(ImportError::UnsupportedMember);
            }
            let row = [2; 8].into_iter().chain([1; 8]);
            if !group.pixels.as_chunks::<16>().0.iter().all(|pixels| {
                pixels.iter().copied().map(Pixel::pen).eq(row.clone())
            }) {
                return Err(ImportError::UnsupportedMember);
            }
            let mut visible = [Pixel::from_pen(u16::MAX); 56];
            snapshot.overlay(raster, &tiles, &mut visible)?;
            let expected = if origin_x < 0i32 {
                [2; 6].into_iter().chain([1; 8]).collect::<Vec<_>>()
            } else {
                vec![u16::MAX; 14]
            };
            if !visible.as_chunks::<14>().0.iter().all(|pixels| {
                pixels
                    .iter()
                    .copied()
                    .map(Pixel::pen)
                    .eq(expected.iter().copied())
            }) {
                return Err(ImportError::UnsupportedMember);
            }
            if !matches!(
                snapshot.isolate(raster, &tiles, NonZeroUsize::MIN),
                Err(ImportError::TooLarge)
            ) {
                return Err(ImportError::UnsupportedMember);
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_malformed_observation_and_horizontal_mirror()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let controls = controls()?;
        let mut ram = [0; RAM_BYTES];
        for (object_bytes, control_bytes) in [
            (
                ram.as_slice(),
                controls.get(..43).ok_or(ImportError::UnsupportedMember)?,
            ),
            (
                ram.get(..RAM_BYTES.saturating_sub(1))
                    .ok_or(ImportError::UnsupportedMember)?,
                controls.as_slice(),
            ),
        ] {
            if machine
                .sprite_snapshot(object_bytes, control_bytes, mixer)
                .is_ok()
            {
                return Err(ImportError::UnsupportedMember);
            }
        }
        let tiles = sheet(&[1; 512])?;
        let raster = Raster {
            height: 1,
            layer_x: [0; 3],
            origin_y: 16,
            width: 1,
        };
        let state = machine.sprite_snapshot(&ram, &controls, mixer)?;
        if state.overlay(raster, &tiles, &mut []).is_ok() {
            return Err(ImportError::UnsupportedMember);
        }
        record(&mut ram, 0, [0x8001, 0, 981, 49, 64, 64, 0x4000, 0])?;
        if machine
            .sprite_snapshot(&ram, &controls, mixer)?
            .overlay(raster, &tiles, &mut [Pixel::default()])
            .is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }

    #[test]
    fn rejects_unmodelled_controls_and_ambiguous_depth()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let mixer = machine.decode_video_priority(&[0; 16])?;
        let mut controls = controls()?;
        let mut ram = [0; RAM_BYTES];
        *controls.get_mut(5).ok_or(ImportError::UnsupportedMember)? = 1;
        if machine.sprite_snapshot(&ram, &controls, mixer).is_ok() {
            return Err(ImportError::UnsupportedMember);
        }
        *controls.get_mut(5).ok_or(ImportError::UnsupportedMember)? = 0;
        let tiles = sheet(&[1; 512])?;
        let raster = Raster {
            height: 1,
            layer_x: [0; 3],
            origin_y: 16,
            width: 1,
        };
        record(&mut ram, 0, [0x8001, 0, 981, 49, 64, 64, 0, 0])?;
        record(&mut ram, 1, [0x8001, 1, 981, 49, 64, 64, 0, 0])?;
        let mut output = [Pixel::default()];
        if machine
            .sprite_snapshot(&ram, &controls, mixer)?
            .overlay(raster, &tiles, &mut output)
            .is_ok()
        {
            return Err(ImportError::UnsupportedMember);
        }
        Ok(())
    }
}
