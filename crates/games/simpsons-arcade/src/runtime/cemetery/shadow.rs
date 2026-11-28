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
//   - Exact palette shadow resolution for registered cemetery draws.
// - Must-Not:
//   - Advance simulation, decode source media or infer sprite priorities.
// - Allows:
//   - Inputs: Admitted canonical pixels and ordered registered draws.
//   - Outputs: Sparse opaque correction pixels and one overlay command.
//   - Side effects: Replace a bounded persistent atlas on sample changes.
// - Split-When:
//   - Other scene effects require distinct palette rules.
// - Merge-When:
//   - Multiple titles require the same palette shadow correction.
// - Summary:
//   - Correct alpha rounding and shadow overlap without GPU readback.
// - Description:
//   - Resolve only mask pixels against the opaque actors and map layers.
// - Usage:
//   - Queue after the room actors and before interface artwork.
// - Defaults:
//   - No correction until the protected room is admitted.

use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;

use super::super::{Art, dimension, failure};

const HEIGHT: usize = 196;
pub(super) const MASK: [u8; 4] = [0, 0, 0, 102];
const TOP: i32 = 28;
const WIDTH: usize = 320;

pub(in crate::runtime) const ATLAS_BYTES: usize = WIDTH * HEIGHT * 4;

type Color = [u8; 4];
type Point = (i32, i32);
type DrawSource<'source> = (SpriteDraw, &'source Source);

/// Source pixels needed only while the protected room is selected.
#[derive(Debug)]
pub(in crate::runtime) struct Source {
    art: Art,
    mask: Vec<Point>,
    pixels: Vec<Color>,
}

impl Source {
    pub(in crate::runtime) fn new(
        art: Art,
        bytes: &[u8],
    ) -> Result<Self, RenderError> {
        let (pixels, remainder) = bytes.as_chunks::<4>();
        let width = usize::try_from(art.rect.width.get()).map_err(failure)?;
        let height = usize::try_from(art.rect.height.get()).map_err(failure)?;
        if !remainder.is_empty()
            || width.checked_mul(height) != Some(pixels.len())
        {
            return Err(RenderError::InvalidLength);
        }
        let mut mask = Vec::new();
        for (index, &pixel) in pixels.iter().enumerate() {
            if pixel == MASK {
                mask.push((
                    i32::try_from(
                        index
                            .checked_rem(width)
                            .ok_or(RenderError::InvalidLength)?,
                    )
                    .map_err(failure)?,
                    i32::try_from(
                        index
                            .checked_div(width)
                            .ok_or(RenderError::InvalidLength)?,
                    )
                    .map_err(failure)?,
                ));
            } else if !matches!(pixel, [_, _, _, 0 | 255]) {
                return Err(failure("unexpected cemetery source alpha"));
            }
        }
        Ok(Self {
            art,
            mask,
            pixels: pixels.to_vec(),
        })
    }

    fn sample(
        &self,
        draw: SpriteDraw,
        (horizontal, vertical): Point,
    ) -> Option<Color> {
        let sx = sample_axis(
            (draw.source.x, draw.source.width.get()),
            (draw.destination.x, draw.destination.width.get()),
            draw.flip_horizontal,
            horizontal,
        )?;
        let sy = sample_axis(
            (draw.source.y, draw.source.height.get()),
            (draw.destination.y, draw.destination.height.get()),
            draw.flip_vertical,
            vertical,
        )?;
        let width = usize::try_from(self.art.rect.width.get()).ok()?;
        let height = usize::try_from(self.art.rect.height.get()).ok()?;
        if sx >= width || sy >= height {
            return None;
        }
        self.pixels
            .get(sy.checked_mul(width)?.checked_add(sx)?)
            .copied()
    }
}

/// One reusable room-sized correction texture.
#[derive(Debug)]
pub(super) struct Shadow {
    art: Art,
    has_pixels: bool,
    pixels: Vec<Color>,
    previous: Vec<SpriteDraw>,
    previous_backgrounds: usize,
    sources: Vec<Source>,
}

impl Shadow {
    pub(super) fn is_overlay(&self, draw: SpriteDraw) -> bool {
        draw.atlas == self.art.atlas
    }

    pub(super) fn new(
        scene: &mut SceneSession<'_, '_>,
        sources: Vec<Source>,
    ) -> Result<Self, RenderError> {
        let pixels = vec![[0; 4]; WIDTH * HEIGHT];
        let art = Art::upload(
            scene,
            (
                dimension(u32::try_from(WIDTH).map_err(failure)?)?,
                dimension(u32::try_from(HEIGHT).map_err(failure)?)?,
            ),
            pixels.as_flattened(),
        )?;
        Ok(Self {
            art,
            has_pixels: false,
            pixels,
            previous: Vec::new(),
            previous_backgrounds: 0,
            sources,
        })
    }

    pub(super) fn queue(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        draws: &[SpriteDraw],
        background_count: usize,
    ) -> Result<Option<SpriteDraw>, RenderError> {
        if self.previous != draws
            || self.previous_backgrounds != background_count
        {
            let sources = draws
                .iter()
                .map(|&draw| {
                    self.sources
                        .iter()
                        .find(|source| source.art.atlas == draw.atlas)
                        .map(|source| (draw, source))
                        .ok_or(RenderError::InvalidSprite)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (backgrounds, actors) = sources
                .split_at_checked(background_count)
                .ok_or(RenderError::InvalidSprite)?;
            self.previous.clear();
            self.previous_backgrounds = 0;
            self.has_pixels = false;
            self.pixels.fill([0; 4]);
            let has_pixels = correct(&mut self.pixels, backgrounds, actors)?;
            if has_pixels {
                scene.replace_atlas(
                    self.art.atlas,
                    self.pixels.as_flattened(),
                )?;
            }
            self.has_pixels = has_pixels;
            self.previous.extend_from_slice(draws);
            self.previous_backgrounds = background_count;
        }
        if self.has_pixels {
            let mut destination = self.art.rect;
            destination.y = TOP;
            return Ok(Some(self.art.draw(destination, false)));
        }
        Ok(None)
    }
}

fn correct(
    pixels: &mut [Color],
    backgrounds: &[DrawSource<'_>],
    actors: &[DrawSource<'_>],
) -> Result<bool, RenderError> {
    let mut any = false;
    for &(draw, source) in actors {
        for &(source_x, source_y) in &source.mask {
            let point = (
                project_axis(
                    (draw.source.x, draw.source.width.get()),
                    (draw.destination.x, draw.destination.width.get()),
                    draw.flip_horizontal,
                    source_x,
                ),
                project_axis(
                    (draw.source.y, draw.source.height.get()),
                    (draw.destination.y, draw.destination.height.get()),
                    draw.flip_vertical,
                    source_y,
                ),
            );
            let (Some(horizontal), Some(vertical)) = point else {
                continue;
            };
            let Some(index) = room_index((horizontal, vertical)) else {
                continue;
            };
            let pixel =
                pixels.get_mut(index).ok_or(RenderError::InvalidLength)?;
            if pixel.last() == Some(&255) {
                continue;
            }
            *pixel = resolve(
                samples_at(actors, (horizontal, vertical)),
                samples_at(backgrounds, (horizontal, vertical)),
            )
            .ok_or(RenderError::InvalidSprite)?;
            any = true;
        }
    }
    Ok(any)
}

// Mask sources are always registered one-to-one, including reflection.
fn project_axis(
    (source, source_len): (i32, u32),
    (destination, destination_len): (i32, u32),
    flipped: bool,
    value: i32,
) -> Option<i32> {
    if source_len != destination_len {
        return None;
    }
    let width = i32::try_from(source_len).ok()?;
    let local = value.checked_sub(source)?;
    if !(0i32..width).contains(&local) {
        return None;
    }
    destination.checked_add(if flipped {
        width.saturating_sub(1).saturating_sub(local)
    } else {
        local
    })
}

fn resolve(
    mut actors: impl Iterator<Item = Color>,
    mut backgrounds: impl Iterator<Item = Color>,
) -> Option<Color> {
    // A shadow never darkens an opaque actor, including a lower-priority
    // actor, nor does overlapping shadow coverage attenuate twice.
    if let Some(color) = actors.find(|pixel| pixel.last() == Some(&255)) {
        return Some(color);
    }
    let [red, green, blue, _] =
        backgrounds.find(|pixel| pixel.last() == Some(&255))?;
    let channel = |value| {
        u8::try_from(u16::from(value).saturating_mul(3).checked_div(5)?).ok()
    };
    Some([channel(red)?, channel(green)?, channel(blue)?, 255])
}

fn room_index((horizontal, vertical): Point) -> Option<usize> {
    let column = usize::try_from(horizontal).ok()?;
    let row = usize::try_from(vertical.checked_sub(TOP)?).ok()?;
    if column >= WIDTH || row >= HEIGHT {
        return None;
    }
    row.checked_mul(WIDTH)?.checked_add(column)
}

// Room layers are either unscaled subrectangles or the constant one-pixel
// backdrop. Do not silently invent a sampling rule for other scaling.
fn sample_axis(
    (source, source_len): (i32, u32),
    (destination, destination_len): (i32, u32),
    flipped: bool,
    value: i32,
) -> Option<usize> {
    let local = u32::try_from(value.checked_sub(destination)?).ok()?;
    if local >= destination_len {
        return None;
    }
    let sample = if source_len == 1 {
        0
    } else if source_len == destination_len {
        if flipped {
            source_len.saturating_sub(1).saturating_sub(local)
        } else {
            local
        }
    } else {
        return None;
    };
    usize::try_from(source.checked_add(i32::try_from(sample).ok()?)?).ok()
}

fn samples_at<'source>(
    draws: &'source [DrawSource<'source>],
    point: Point,
) -> impl Iterator<Item = Color> + 'source {
    draws
        .iter()
        .rev()
        .filter_map(move |&(draw, source)| source.sample(draw, point))
}

#[cfg(test)]
mod tests {
    use super::{MASK, project_axis, resolve, room_index, sample_axis};

    #[test]
    fn shadow_uses_floor_palette_once_and_never_darkens_an_actor() {
        let background = [148, 123, 109, 255];
        let clear = [0; 4];
        let front = [82, 82, 82, 255];
        let back = [100, 100, 100, 255];
        assert_eq!(
            resolve(
                [MASK, MASK, clear].into_iter(),
                [clear, background].into_iter()
            ),
            Some([88, 73, 65, 255])
        );
        assert_eq!(
            resolve(
                [MASK, front, back, MASK].into_iter(),
                [background].into_iter()
            ),
            Some(front)
        );
        assert_eq!(resolve([MASK].into_iter(), [clear].into_iter()), None);
    }

    #[test]
    fn clipped_reflected_masks_keep_pixel_registration() {
        let source = (7i32, 5u32);
        let destination = (-2i32, 5u32);
        for (flip, value, expected) in [
            (false, 9i32, Some(0i32)),
            (true, 9i32, Some(0i32)),
            (true, 11i32, Some(-2i32)),
            (true, 6i32, None),
        ] {
            assert_eq!(
                project_axis(source, destination, flip, value),
                expected
            );
        }
    }

    #[test]
    fn clipped_reflected_and_wrapped_samples_keep_pixel_registration() {
        let source = (7i32, 5u32);
        let destination = (-2i32, 5u32);
        for (flip, value, expected) in [
            (true, 0i32, Some(9usize)),
            (false, 2i32, Some(11)),
            (true, 3i32, None),
        ] {
            assert_eq!(sample_axis(source, destination, flip, value), expected);
        }
        assert_eq!(sample_axis((0i32, 1), (0i32, 320), false, 319), Some(0));
        assert_eq!(
            sample_axis((96i32, 320), (0i32, 320), false, 319),
            Some(415)
        );
    }

    #[test]
    fn correction_texture_clips_to_the_room() {
        for (point, expected) in [
            ((0i32, 27i32), None),
            ((319i32, 223i32), Some(62_719usize)),
            ((320i32, 223i32), None),
        ] {
            assert_eq!(room_index(point), expected);
        }
    }

    #[test]
    fn shadow_palette_covers_every_byte_without_double_attenuation() {
        let mut expected = 0u8;
        let mut remainder = 0u8;
        for channel in 0..=u8::MAX {
            let background = [channel, 255, 0, 255];
            assert_eq!(
                resolve(
                    [MASK, [0; 4], MASK, MASK].into_iter(),
                    [[0; 4], background].into_iter(),
                ),
                Some([expected, 153, 0, 255]),
                "palette channel {channel}"
            );
            // Accumulate the rational step independently of multiplication.
            remainder = remainder.saturating_add(3);
            if remainder >= 5 {
                remainder = remainder.saturating_sub(5);
                expected = expected.saturating_add(1);
            }
        }
    }

    #[test]
    fn shadow_preserves_frontmost_opaque_actor_in_every_overlap_order() {
        for front in 0usize..4 {
            assert_opaque_priority(front);
        }
    }

    fn assert_opaque_priority(front: usize) {
        let first = [12, 34, 56, 255];
        let second = [78, 90, 123, 255];
        let background = [148, 123, 109, 255];
        for back in 0usize..4 {
            if front == back {
                continue;
            }
            let mut actors = [MASK; 4];
            if let Some(pixel) = actors.get_mut(front) {
                *pixel = first;
            }
            if let Some(pixel) = actors.get_mut(back) {
                *pixel = second;
            }
            let expected = if front < back {
                first
            } else {
                second
            };
            assert_eq!(
                resolve(actors.into_iter(), [background].into_iter()),
                Some(expected),
                "opaque indices {front} and {back}"
            );
        }
    }

    #[test]
    fn shadow_projection_and_sampling_are_inverse_on_registered_crops() {
        for width in [1u32, 2, 3, 7, 31, 196, 320] {
            for (origin, target) in [
                (0i32, -17i32),
                (0i32, 0i32),
                (0i32, 319i32),
                (7i32, -17i32),
                (7i32, 0i32),
                (7i32, 319i32),
                (96i32, -17i32),
                (96i32, 0i32),
                (96i32, 319i32),
            ] {
                let source = (origin, width);
                let destination = (target, width);
                assert_axis_inverse(source, destination, false);
                assert_axis_inverse(source, destination, true);
            }
        }
    }

    fn assert_axis_inverse(
        source: (i32, u32),
        destination: (i32, u32),
        flip: bool,
    ) {
        let width = i32::try_from(source.1).unwrap_or_default();
        for offset in 0i32..width {
            let pixel = source.0.saturating_add(offset);
            let projected = project_axis(source, destination, flip, pixel);
            let expected = if flip {
                destination.0.saturating_add(
                    width.saturating_sub(1).saturating_sub(offset),
                )
            } else {
                destination.0.saturating_add(offset)
            };
            assert_eq!(projected, Some(expected));
            assert_eq!(
                projected.and_then(|point| {
                    sample_axis(source, destination, flip, point)
                }),
                usize::try_from(pixel).ok()
            );
        }
        assert_eq!(
            project_axis(
                source,
                destination,
                flip,
                source.0.saturating_add(width)
            ),
            None
        );
        assert_eq!(
            sample_axis(
                source,
                destination,
                flip,
                destination.0.saturating_sub(1)
            ),
            None
        );
    }
}
