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
//   - Side effects: Bounded texture allocation on explicit admission.
// - Split-When:
//   - Independent font shaping or dynamic layout is introduced.
// - Merge-When:
//   - Another platform helper duplicates text atlas generation.
// - Summary:
//   - Share native compact UI bitmap and text atlas rendering.
// - Description:
//   - Reads in-width bits; creates centered static text plates.
// - Usage:
//   - Supply explicit line descriptors and a caller-owned byte limit.
// - Defaults:
//   - Empty or invalid glyph coordinates produce no ink.

//! Bounded compact bitmap UI and reusable static text plates.

use std::num::{NonZeroU32, NonZeroUsize};

use super::sprites::{AtlasId, SceneSession, SpriteDraw, SpriteRect};
use super::{FrameLayout, RenderError};

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
    /// Text using uppercase ASCII glyphs; lowercase is accepted.
    pub text: &'text str,
    /// Vertical offset within the plate.
    pub top: u16,
}

impl TextLine<'_> {
    fn extent(self) -> Option<u16> {
        u16::try_from(self.text.len())
            .ok()?
            .checked_mul(4)?
            .checked_sub(1)?
            .checked_mul(self.scale)
    }

    /// Clamp the glyph scale to fit this plate width.
    #[must_use]
    pub fn fit_width(mut self, width: u16) -> Self {
        let extent = u16::try_from(self.text.len())
            .ok()
            .and_then(|length| length.checked_mul(4))
            .and_then(|length| length.checked_sub(1))
            .unwrap_or_default();
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

    /// Test ink at a pixel relative to a centered text plate.
    #[must_use]
    pub fn pixel(self, column: u16, row: u16, width: u16) -> bool {
        let Some(extent) = self.extent() else {
            return false;
        };
        let left = width
            .saturating_sub(extent)
            .checked_div(2)
            .unwrap_or_default();
        let Some(horizontal) = column.checked_sub(left) else {
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
        self.text
            .as_bytes()
            .get(usize::from(character))
            .is_some_and(|letter| {
                ink(&rows(letter.to_ascii_uppercase()), 3, bit, glyph_y)
            })
    }
}

/// Static text plates sharing a single GPU atlas and bounded allocation.
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
    pub fn load(
        scene: &mut SceneSession<'_, '_>,
        spec: PlateLayout,
        plates: &[Vec<TextLine<'_>>],
    ) -> Result<Self, RenderError> {
        let (layout, pixels) = rasterize(spec, plates)?;
        Ok(Self {
            atlas: scene.load_atlas(layout, &pixels)?,
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
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(layout.bytes())
        .map_err(|_error| RenderError::TooLarge)?;
    for lines in plates {
        for row in 0..height {
            for column in 0..width {
                let color = lines
                    .iter()
                    .rev()
                    .find(|line| line.pixel(column, row, width))
                    .map_or(background, |line| line.color);
                pixels.extend_from_slice(&color);
            }
        }
    }
    Ok((layout, pixels))
}

const fn rows(letter: u8) -> [u8; 5] {
    match letter {
        b'0' | b'O' => [7, 5, 5, 5, 7],
        b'1' => [2, 6, 2, 2, 7],
        b'2' => [7, 1, 7, 4, 7],
        b'3' => [7, 1, 7, 1, 7],
        b'4' => [5, 5, 7, 1, 1],
        b'5' | b'S' => [7, 4, 7, 1, 7],
        b'6' => [7, 4, 7, 5, 7],
        b'7' => [7, 1, 2, 2, 2],
        b'8' => [7, 5, 7, 5, 7],
        b'9' => [7, 5, 7, 1, 7],
        b'A' => [2, 5, 7, 5, 5],
        b'B' => [6, 5, 6, 5, 6],
        b'C' => [7, 4, 4, 4, 7],
        b'D' => [6, 5, 5, 5, 6],
        b'E' => [7, 4, 6, 4, 7],
        b'F' => [7, 4, 6, 4, 4],
        b'G' => [7, 4, 5, 5, 7],
        b'H' => [5, 5, 7, 5, 5],
        b'I' => [7, 2, 2, 2, 7],
        b'J' => [1, 1, 1, 5, 7],
        b'K' => [5, 5, 6, 5, 5],
        b'L' => [4, 4, 4, 4, 7],
        b'M' => [5, 7, 7, 5, 5],
        b'N' => [5, 7, 7, 7, 5],
        b'P' => [6, 5, 6, 4, 4],
        b'Q' => [7, 5, 5, 7, 1],
        b'R' => [6, 5, 6, 5, 5],
        b'T' => [7, 2, 2, 2, 2],
        b'U' => [5, 5, 5, 5, 7],
        b'V' => [5, 5, 5, 5, 2],
        b'W' => [5, 5, 7, 7, 5],
        b'X' => [5, 5, 2, 5, 5],
        b'Y' => [5, 5, 2, 2, 2],
        b'Z' => [7, 1, 2, 4, 7],
        b':' => [0, 2, 0, 2, 0],
        b'-' => [0, 0, 7, 0, 0],
        b'/' => [1, 1, 2, 4, 4],
        b'>' => [4, 2, 1, 2, 4],
        b'+' => [0, 2, 7, 2, 0],
        b' ' => [0; 5],
        _ => [7, 1, 2, 0, 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_ink_rejects_outside_and_preserves_spacing() {
        assert!(ink(&[0b101], 3, 0, 0));
        assert!(!ink(&[0b101], 3, 1, 0));
        assert!(ink(&[0b101], 3, 2, 0));
        assert!(!ink(&[0b101], 3, 3, 0));
        assert!(!ink(&[0b101], 3, 0, 1));
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
}
