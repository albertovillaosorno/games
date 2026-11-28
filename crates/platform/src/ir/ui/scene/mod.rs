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
//   - Portable sprite command and scene resource limit values.
// - Must-Not:
//   - Allocate textures, poll events or access native renderer handles.
// - Allows:
//   - Inputs: Scene-local atlas identities and integer rectangles.
//   - Outputs: Ordered sprite commands and bounded event outcomes.
//   - Side effects: None.
// - Split-When:
//   - Additional command families require independent resource policies.
// - Merge-When:
//   - Other graphics adapters duplicate these command value types.
// - Summary:
//   - Share scene commands across graphics adapters.
// - Description:
//   - Retain atlas identity scope and explicit caller-owned limits.
// - Usage:
//   - Submit ordered draw commands to a scene session.
// - Defaults:
//   - Atlas identities are issued by their owning scene session.

//! Backend-independent scene command values.

use std::num::{NonZeroI64, NonZeroU32, NonZeroUsize};

pub use games::presentation::SpriteRect;
use games::presentation::{Dimensions, Viewport};

use crate::ir::frame::{FrameLayout, RenderError, validate_rectangle};

/// An atlas identifier scoped to a scene session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasId(pub(crate) usize);

/// Limits on atlas allocation and per-frame command count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneLimits {
    /// Total logical RGBA bytes permitted across loaded atlases.
    pub atlas_bytes: NonZeroUsize,
    /// Maximum number of simultaneously loaded atlases.
    pub atlases: NonZeroUsize,
    /// Maximum number of draw commands per frame.
    pub sprites: NonZeroUsize,
}

/// Result of processing one bounded batch of native events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventBatch {
    /// At least one event was delivered before a poll returned no event.
    Drained,
    /// The initial poll returned no event.
    Empty,
    /// The event budget was reached; more events may be waiting.
    Limited,
    /// The caller requested termination from its event handler.
    Stopped,
}

/// One ordered sprite command; later commands are drawn over earlier ones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpriteDraw {
    /// Atlas to sample.
    pub atlas: AtlasId,
    /// Destination in logical scene pixels (may be clipped).
    pub destination: SpriteRect,
    /// Reverse horizontal pixel order.
    pub flip_horizontal: bool,
    /// Reverse vertical pixel order.
    pub flip_vertical: bool,
    /// Source rectangle entirely within the atlas.
    pub source: SpriteRect,
}

impl SpriteDraw {
    /// Clip an unscaled horizontal draw to the half-open logical interval.
    ///
    /// Preserve source registration and horizontal reflection. An empty
    /// intersection, horizontal scaling or overflow yields no draw.
    #[must_use]
    pub fn clipped_horizontal(self, left: i32, right: i32) -> Option<Self> {
        if self.source.width != self.destination.width || left >= right {
            return None;
        }
        let x = i64::from(self.destination.x);
        let end =
            x.checked_add(NonZeroI64::from(self.destination.width).get())?;
        let start = x.max(i64::from(left));
        let stop = end.min(i64::from(right));
        let width =
            NonZeroU32::new(u32::try_from(stop.checked_sub(start)?).ok()?)?;
        let offset = if self.flip_horizontal {
            end.checked_sub(stop)?
        } else {
            start.checked_sub(x)?
        };
        let source_x =
            i32::try_from(i64::from(self.source.x).checked_add(offset)?)
                .ok()?;
        Some(Self {
            destination: SpriteRect {
                x: i32::try_from(start).ok()?,
                width,
                ..self.destination
            },
            source: SpriteRect {
                x: source_x,
                width,
                ..self.source
            },
            ..self
        })
    }

    /// Draw a one-period wrapped atlas view without scaling or overlap.
    ///
    /// The atlas contains exactly the supplied period, rooted at zero.
    /// At most four nonempty commands cover the destination at zero.
    /// # Errors
    /// Rejects invalid view extents or unrepresentable sprite coordinates.
    pub fn wrapped(
        atlas: AtlasId,
        period: Dimensions,
        view: Viewport,
    ) -> Result<impl Iterator<Item = Self>, RenderError> {
        let regions = view
            .wrapped_regions(period)
            .ok_or(RenderError::InvalidDimensions)?;
        let mut commands = [None; 4];
        for (command, candidate) in commands.iter_mut().zip(regions) {
            if let Some(region) = candidate {
                *command = Some(Self {
                    atlas,
                    destination: sprite_rect(region.destination)?,
                    flip_horizontal: false,
                    flip_vertical: false,
                    source: sprite_rect(region.source)?,
                });
            }
        }
        Ok(commands.into_iter().flatten())
    }
}

/// Port for uploading a bounded RGBA atlas into a single scene.
pub trait AtlasUpload {
    /// Admit an atlas and return its scene-local identifier.
    ///
    /// # Errors
    /// Rejects invalid data, exhausted capacity or backend allocation failure.
    fn upload_atlas(
        &mut self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError>;
}

fn sprite_rect(view: Viewport) -> Result<SpriteRect, RenderError> {
    let rectangle = SpriteRect {
        height: NonZeroU32::new(view.height)
            .ok_or(RenderError::InvalidDimensions)?,
        width: NonZeroU32::new(view.width)
            .ok_or(RenderError::InvalidDimensions)?,
        x: i32::try_from(view.x)
            .map_err(|_error| RenderError::InvalidDimensions)?,
        y: i32::try_from(view.y)
            .map_err(|_error| RenderError::InvalidDimensions)?,
    };
    validate_rectangle(rectangle)?;
    Ok(rectangle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    fn rect(
        horizontal: i32,
        vertical: i32,
        width: u32,
        height: u32,
    ) -> SpriteRect {
        SpriteRect {
            height: dim(height),
            width: dim(width),
            x: horizontal,
            y: vertical,
        }
    }

    #[test]
    fn clipped_horizontal_preserves_source_and_reflection() {
        let draw = SpriteDraw {
            atlas: AtlasId(3),
            destination: rect(13, 190, 30, 8),
            flip_horizontal: false,
            flip_vertical: true,
            source: rect(7, 3, 30, 8),
        };
        for (flip, source_x) in [(false, 10i32), (true, 11i32)] {
            let oriented = SpriteDraw {
                flip_horizontal: flip,
                ..draw
            };
            let expected = SpriteDraw {
                destination: rect(16, 190, 23, 8),
                source: rect(source_x, 3, 23, 8),
                ..oriented
            };
            assert_eq!(oriented.clipped_horizontal(16, 39), Some(expected));
            assert_eq!(oriented.clipped_horizontal(0, 50), Some(oriented));
            assert_eq!(oriented.clipped_horizontal(43, 50), None);
            assert_eq!(oriented.clipped_horizontal(0, 13), None);
            assert_eq!(oriented.clipped_horizontal(20, 20), None);
        }
        let scaled = SpriteDraw {
            destination: rect(13, 190, 60, 8),
            ..draw
        };
        assert_eq!(scaled.clipped_horizontal(16, 39), None);
    }

    #[test]
    fn clipped_horizontal_excludes_both_black_margins() {
        let cases = [
            (-14i32, None),
            (13i32, Some((16i32, 3i32, 27u32))),
            (16i32, Some((16i32, 0i32, 30u32))),
            (270i32, Some((270i32, 0i32, 30u32))),
            (274i32, Some((274i32, 0i32, 30u32))),
            (302i32, Some((302i32, 0i32, 2u32))),
            (304i32, None),
            (306i32, None),
        ];
        for (x, expected) in cases {
            let draw = SpriteDraw {
                atlas: AtlasId(3),
                destination: rect(x, 190, 30, 1),
                flip_horizontal: false,
                flip_vertical: false,
                source: rect(0, 0, 30, 1),
            };
            let actual = draw.clipped_horizontal(16, 304).map(|clipped| {
                (
                    clipped.destination.x,
                    clipped.source.x,
                    clipped.destination.width.get(),
                )
            });
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn wrapped_draws_preserve_order_and_exact_pixel_sizes() {
        let period = Dimensions::new(dim(7), dim(5));
        let view = Viewport {
            height: 4,
            width: 6,
            x: 6,
            y: 4,
        };
        let atlas = AtlasId(3);
        let expected = [
            (rect(6, 4, 1, 1), rect(0, 0, 1, 1)),
            (rect(0, 4, 5, 1), rect(1, 0, 5, 1)),
            (rect(6, 0, 1, 3), rect(0, 1, 1, 3)),
            (rect(0, 0, 5, 3), rect(1, 1, 5, 3)),
        ]
        .map(|(source, destination)| SpriteDraw {
            atlas,
            destination,
            flip_horizontal: false,
            flip_vertical: false,
            source,
        });
        let actual = SpriteDraw::wrapped(atlas, period, view)
            .map(Iterator::collect::<Vec<_>>);
        assert_eq!(actual, Ok(Vec::from(expected)));
    }

    #[test]
    fn wrapped_draws_reject_invalid_and_unrepresentable_views() {
        let period = Dimensions::new(dim(7), dim(5));
        let view = Viewport {
            height: 4,
            width: 6,
            x: 6,
            y: 4,
        };
        let atlas = AtlasId(3);
        assert!(
            SpriteDraw::wrapped(atlas, period, Viewport { width: 8, ..view })
                .is_err()
        );
        let large = Dimensions::new(dim(u32::MAX), dim(5));
        assert!(
            SpriteDraw::wrapped(atlas, large, Viewport {
                x: u32::MAX.saturating_sub(1),
                ..view
            })
            .is_err()
        );
    }
}
