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

use std::num::{NonZeroU32, NonZeroUsize};

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
