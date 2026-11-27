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
//   - Bounded bitmap text and static UI atlas creation.
// - Must-Not:
//   - Own control mapping, title gameplay or menu policy.
// - Allows:
//   - Inputs: Bitmap rows and bounded text plate descriptors.
//   - Outputs: Pixel coverage and ordered static text sprites.
//   - Side effects: Bounded atlas upload through an explicit port.
// - Split-When:
//   - Independent font shaping or dynamic layout is introduced.
// - Merge-When:
//   - Another platform helper duplicates text atlas generation.
// - Summary:
//   - Share portable compact UI bitmap and text atlas rendering.
// - Description:
//   - Reads in-width bits; creates centered static text plates.
// - Usage:
//   - Supply explicit line descriptors and a caller-owned byte limit.
// - Defaults:
//   - Empty or invalid glyph coordinates produce no ink.

//! Bounded compact bitmap UI and reusable static text plates.
/// Shared digital control editor presentation.
pub mod editor;
mod glyphs;
/// Portable menu layout and artwork.
pub mod menu;
/// Portable scene rendering commands.
pub mod scene;

use std::num::{NonZeroU32, NonZeroUsize};

use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::scene::{AtlasId, AtlasUpload, SpriteDraw, SpriteRect};

type Rasterized = (FrameLayout, Vec<u8>);

/// Geometry and allocation budget for a static text atlas.
#[derive(Clone, Copy, Debug)]
pub struct PlateLayout {
    /// Background RGBA pixels.
    pub background: [u8; 4],
    /// Height of one plate in pixels.
    pub height: u16,
    /// Maximum allocation for all plates.
    pub max_bytes: NonZeroUsize,
    /// Width of one plate in pixels.
    pub width: u16,
}

/// One horizontally centered line of compact bitmap text.
#[derive(Clone, Copy, Debug)]
pub struct TextLine<'text> {
    /// Opaque or transparent RGBA ink color.
    pub color: [u8; 4],
    /// Integer scale of one glyph pixel.
    pub scale: u16,
    /// Text using compact uppercase English and Mexican Spanish glyphs.
    pub text: &'text str,
    /// Vertical offset within the plate.
    pub top: u16,
}

impl TextLine<'_> {
    fn extent(self) -> Option<u16> {
        self.unit_extent()?.checked_mul(self.scale)
    }

    /// Clamp the glyph scale to fit this plate width.
    #[must_use]
    pub fn fit_width(mut self, width: u16) -> Self {
        let extent = self.unit_extent().unwrap_or_default();
        self.scale = width
            .checked_div(extent)
            .unwrap_or_default()
            .min(self.scale);
        self
    }

    fn fits(self, width: u16, height: u16) -> bool {
        self.scale > 0
            && self.extent().is_some_and(|extent| extent <= width)
            && self
                .scale
                .checked_mul(5)
                .and_then(|size| self.top.checked_add(size))
                .is_some_and(|bottom| bottom <= height)
    }

    fn left(self, width: u16) -> Option<u16> {
        width.saturating_sub(self.extent()?).checked_div(2)
    }

    fn paint_row(
        self,
        pixels: &mut [[u8; 4]],
        row: u16,
        width: u16,
    ) -> Result<(), RenderError> {
        let Some(glyph_y) = row
            .checked_sub(self.top)
            .and_then(|offset| offset.checked_div(self.scale))
            .filter(|offset| *offset < 5)
        else {
            return Ok(());
        };
        let mut left = usize::from(
            self.left(width).ok_or(RenderError::InvalidDimensions)?,
        );
        let scale = usize::from(self.scale);
        for letter in glyphs::display(self.text) {
            let rows = glyphs::rows(letter);
            for bit in (0..3u16).filter(|bit| ink(&rows, 3, *bit, glyph_y)) {
                let start =
                    left.saturating_add(usize::from(bit).saturating_mul(scale));
                let span = pixels
                    .get_mut(start..start.saturating_add(scale))
                    .ok_or(RenderError::InvalidDimensions)?;
                span.fill(self.color);
            }
            left = left.saturating_add(scale.saturating_mul(4));
        }
        Ok(())
    }

    /// Test ink at a pixel relative to a centered text plate.
    #[must_use]
    pub fn pixel(self, column: u16, row: u16, width: u16) -> bool {
        let Some(horizontal) =
            self.left(width).and_then(|left| column.checked_sub(left))
        else {
            return false;
        };
        let Some(vertical) = row.checked_sub(self.top) else {
            return false;
        };
        let Some(glyph_x) = horizontal.checked_div(self.scale) else {
            return false;
        };
        let Some(glyph_y) = vertical.checked_div(self.scale) else {
            return false;
        };
        let character = glyph_x.checked_div(4).unwrap_or_default();
        let bit = glyph_x.checked_rem(4).unwrap_or_default();
        glyphs::display(self.text)
            .nth(usize::from(character))
            .is_some_and(|letter| ink(&glyphs::rows(letter), 3, bit, glyph_y))
    }

    fn unit_extent(self) -> Option<u16> {
        u16::try_from(glyphs::display(self.text).count())
            .ok()?
            .checked_mul(4)?
            .checked_sub(1)
    }
}

/// Static text plates sharing a single scene atlas and bounded allocation.
#[derive(Clone, Copy, Debug)]
pub struct TextAtlas {
    atlas: AtlasId,
    count: usize,
    height: NonZeroU32,
    width: NonZeroU32,
}

impl TextAtlas {
    /// Prepare a sprite for one plate at the requested logical position.
    ///
    /// # Errors
    /// Rejects a tile index outside the loaded atlas.
    pub fn draw(
        &self,
        tile: usize,
        left: i32,
        top: i32,
    ) -> Result<SpriteDraw, RenderError> {
        if tile >= self.count {
            return Err(RenderError::InvalidSprite);
        }
        let source_y = u32::try_from(tile)
            .ok()
            .and_then(|index| index.checked_mul(self.height.get()))
            .and_then(|offset| i32::try_from(offset).ok())
            .ok_or(RenderError::InvalidDimensions)?;
        let source = SpriteRect {
            height: self.height,
            width: self.width,
            x: 0,
            y: source_y,
        };
        Ok(SpriteDraw {
            atlas: self.atlas,
            destination: SpriteRect {
                x: left,
                y: top,
                ..source
            },
            flip_horizontal: false,
            flip_vertical: false,
            source,
        })
    }

    /// Rasterize and upload static plates once, not once per frame.
    ///
    /// # Errors
    /// Rejects empty plates, clipped text, overflowing dimensions and
    /// allocations exceeding the supplied byte budget.
    pub fn load<Upload>(
        scene: &mut Upload,
        spec: PlateLayout,
        plates: &[Vec<TextLine<'_>>],
    ) -> Result<Self, RenderError>
    where
        Upload: AtlasUpload,
    {
        let (layout, pixels) = rasterize(spec, plates)?;
        Ok(Self {
            atlas: scene.upload_atlas(layout, &pixels)?,
            count: plates.len(),
            height: dimension(u32::from(spec.height))?,
            width: dimension(u32::from(spec.width))?,
        })
    }
}

fn dimension(value: u32) -> Result<NonZeroU32, RenderError> {
    NonZeroU32::new(value).ok_or(RenderError::InvalidDimensions)
}

/// Test a glyph bit, most significant in-width bit first.
#[must_use]
pub fn ink(rows: &[u8], width: u16, column: u16, row: u16) -> bool {
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

fn rasterize(
    spec: PlateLayout,
    plates: &[Vec<TextLine<'_>>],
) -> Result<Rasterized, RenderError> {
    let PlateLayout {
        background,
        height,
        max_bytes,
        width,
    } = spec;
    let atlas_height = u32::try_from(plates.len())
        .ok()
        .and_then(|count| count.checked_mul(u32::from(height)))
        .ok_or(RenderError::TooLarge)?;
    let layout = FrameLayout::new(
        dimension(u32::from(width))?,
        dimension(atlas_height)?,
        max_bytes,
    )?;
    if plates.iter().any(|lines| {
        lines.is_empty() || lines.iter().any(|line| !line.fits(width, height))
    }) {
        return Err(RenderError::InvalidDimensions);
    }
    let count = layout.bytes().checked_div(4).ok_or(RenderError::TooLarge)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_error| RenderError::TooLarge)?;
    pixels.resize(count, background);
    let mut rows = pixels.chunks_exact_mut(usize::from(width));
    for lines in plates {
        for row in 0..height {
            let row_pixels = rows.next().ok_or(RenderError::InvalidLength)?;
            for line in lines {
                line.paint_row(row_pixels, row, width)?;
            }
        }
    }
    Ok((layout, pixels.into_flattened()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::slice;

    use super::*;

    fn glyph_pixels(line: TextLine<'_>) -> Vec<bool> {
        (0..5)
            .flat_map(|row| {
                (0..3).map(move |column| line.pixel(column, row, 3))
            })
            .collect()
    }

    #[test]
    fn glyph_ink_rejects_outside_and_preserves_spacing() {
        assert!(ink(&[0b101], 3, 0, 0));
        assert!(!ink(&[0b101], 3, 1, 0));
        assert!(ink(&[0b101], 3, 2, 0));
        assert!(!ink(&[0b101], 3, 3, 0));
        assert!(!ink(&[0b101], 3, 0, 1));
    }

    #[test]
    fn printable_punctuation_has_distinct_visible_glyphs() {
        let mut seen = BTreeSet::new();
        for symbol in (b'!'..=b'~').filter(u8::is_ascii_punctuation) {
            let text = char::from(symbol).to_string();
            let line = TextLine {
                color: [255; 4],
                scale: 1,
                text: &text,
                top: 0,
            };
            let pixels = glyph_pixels(line);
            assert!(pixels.contains(&true), "{text} is blank");
            assert!(seen.insert(pixels), "{text} duplicates punctuation");
        }
        assert_eq!(seen.len(), 32);
    }

    #[test]
    fn rasterized_text_is_centered_and_bounded() {
        let color = [255, 255, 255, 255];
        let background = [0, 0, 0, 255];
        let plates = [vec![TextLine {
            color,
            scale: 1,
            text: "I",
            top: 1,
        }]];
        let budget = NonZeroUsize::new(196).unwrap_or(NonZeroUsize::MIN);
        let spec = PlateLayout {
            background,
            height: 7,
            max_bytes: budget,
            width: 7,
        };
        let result = rasterize(spec, &plates);
        assert_eq!(result.as_ref().map(|(layout, _)| layout.bytes()), Ok(196));
        let top = result
            .as_ref()
            .ok()
            .and_then(|(_, pixels)| pixels.get(28..56))
            .unwrap_or_default();
        assert_eq!(top.as_chunks::<4>().0, [
            background, background, color, color, color, background,
            background,
        ]);
        assert_eq!(
            rasterize(PlateLayout { width: 2, ..spec }, &plates),
            Err(RenderError::InvalidDimensions),
        );
        assert_eq!(
            rasterize(
                PlateLayout {
                    max_bytes: NonZeroUsize::MIN,
                    ..spec
                },
                &plates
            ),
            Err(RenderError::TooLarge),
        );
    }

    #[test]
    fn spanish_text_measures_composed_glyphs_and_shows_missing_glyphs() {
        let text = TextLine {
            color: [255; 4],
            scale: 2,
            text: "\u{bf}Qu\u{e9}?",
            top: 1,
        };
        let decomposed = TextLine {
            text: "\u{bf}QUE\u{301}?",
            ..text
        };
        assert_eq!(text.fit_width(38).scale, 2);
        assert_eq!(decomposed.fit_width(38).scale, 2);
        assert_eq!(text.fit_width(37).scale, 1);
        let spec = PlateLayout {
            background: [0; 4],
            height: 12,
            max_bytes: NonZeroUsize::new(38 * 12 * 4)
                .unwrap_or(NonZeroUsize::MIN),
            width: 38,
        };
        let raster = |line| rasterize(spec, &[vec![line]]);
        assert_eq!(raster(text), raster(decomposed));
        assert_ne!(raster(text), raster(TextLine { text: "?QUE?", ..text }));
        assert_eq!(
            raster(TextLine {
                text: "\u{bf}Q\u{5b57}?",
                ..text
            }),
            raster(TextLine {
                text: "\u{bf}Q\u{fffd}?",
                ..text
            })
        );
        assert_ne!(
            raster(TextLine {
                text: "\u{bf}Q\u{5b57}?",
                ..text
            }),
            raster(TextLine {
                text: "\u{bf}Q??",
                ..text
            })
        );
    }

    #[test]
    fn compact_spanish_glyphs_are_distinct_from_unaccented_forms() {
        for (plain, accented) in [
            ("A", "\u{c1}"),
            ("E", "\u{c9}"),
            ("I", "\u{cd}"),
            ("O", "\u{d3}"),
            ("U", "\u{da}"),
            ("U", "\u{dc}"),
            ("N", "\u{d1}"),
            ("!", "\u{a1}"),
            ("?", "\u{bf}"),
        ] {
            let line = TextLine {
                color: [255; 4],
                scale: 1,
                text: accented,
                top: 0,
            };
            let plain_line = TextLine { text: plain, ..line };
            let pixels = glyph_pixels(line);
            assert!(pixels.contains(&true), "{accented} is blank");
            assert_ne!(pixels, glyph_pixels(plain_line));
        }
    }

    #[test]
    fn row_rasterization_matches_centered_pixel_and_overdraw_order()
    -> Result<(), RenderError> {
        let spec = PlateLayout {
            background: [9, 8, 7, 255],
            height: 14,
            max_bytes: NonZeroUsize::new(48 * 14 * 4)
                .ok_or(RenderError::TooLarge)?,
            width: 48,
        };
        let lines = vec![
            TextLine {
                color: [255, 255, 255, 255],
                scale: 2,
                text: "A\u{d1}O 3",
                top: 1,
            },
            TextLine {
                color: [255, 0, 0, 255],
                scale: 1,
                text: "\u{a1}S\u{ed}!",
                top: 4,
            },
        ];
        let (_, actual) = rasterize(spec, slice::from_ref(&lines))?;
        let expected: Vec<_> = (0..spec.height)
            .flat_map(|row| {
                let line_refs = &lines;
                (0..spec.width).map(move |column| {
                    line_refs
                        .iter()
                        .rev()
                        .find(|line| line.pixel(column, row, spec.width))
                        .map_or(spec.background, |line| line.color)
                })
            })
            .collect();
        if actual == expected.into_flattened() {
            Ok(())
        } else {
            Err(RenderError::InvalidLength)
        }
    }
}
