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
//   - Portable atlas, sprite and presentation receipt admission.
// - Must-Not:
//   - Allocate textures, select renderers or submit native commands.
// - Allows:
//   - Inputs: Scene limits, loaded layouts, draw commands and receipts.
//   - Outputs: Checked resource capacity and current presentation geometry.
//   - Side effects: Retains admitted scene metadata in bounded memory.
// - Split-When:
//   - Another rendering resource requires independent budget admission.
// - Merge-When:
//   - Equivalent rendering security rules already exist.
// - Summary:
//   - Shared bounded ordered scene admission before native dispatch.
// - Description:
//   - Validates byte budgets, counts and all referenced atlas rectangles.
// - Usage:
//   - Commit capacity only after a successful native allocation and upload.
// - Defaults:
//   - Reject invalid data or exceeded resource budgets before drawing.

//! Bounded scene resource, draw and presentation admission.

use std::cell::Cell;

use games::presentation::{Dimensions, PresentationLayout};

use crate::ir::frame::{
    AdmittedRgba, FrameLayout, RenderError, validate_rectangle,
};
use crate::ir::scene::{AtlasId, SceneLimits, SpriteDraw, SpriteRect};

/// Borrowed draw list admitted against the owning scene's resource budget.
///
/// The private rendering port consumes this value without copying commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedDraws<'draws> {
    sprites: &'draws [SpriteDraw],
}

impl<'draws> AdmittedDraws<'draws> {
    /// The exact admitted ordered commands.
    #[must_use]
    pub const fn as_slice(self) -> &'draws [SpriteDraw] {
        self.sprites
    }
}

/// One atlas upload admitted against the owning scene budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedAtlas<'pixels> {
    frame: AdmittedRgba<'pixels>,
    id: AtlasId,
}

impl<'pixels> AdmittedAtlas<'pixels> {
    /// The exact admitted frame and layout.
    #[must_use]
    pub const fn frame(self) -> AdmittedRgba<'pixels> {
        self.frame
    }

    /// The scene-local identity being loaded or replaced.
    #[must_use]
    pub const fn id(self) -> AtlasId {
        self.id
    }
}

/// Scene-local atlas metadata and capacity, independent of native textures.
#[derive(Debug)]
pub(crate) struct SceneAdmission {
    layouts: Vec<FrameLayout>,
    limits: SceneLimits,
    remaining_bytes: usize,
}

impl SceneAdmission {
    /// Admit the complete ordered draw list against committed resources.
    pub(crate) fn draws<'draws>(
        &self,
        sprites: &'draws [SpriteDraw],
    ) -> Result<AdmittedDraws<'draws>, RenderError> {
        admit_draws(self.limits, sprites, |id| self.layouts.get(id.0).copied())
    }

    /// Admit and reserve metadata before asking the adapter to allocate.
    pub(crate) fn load<'pixels, Upload>(
        &mut self,
        layout: FrameLayout,
        pixels: &'pixels [u8],
        upload: Upload,
    ) -> Result<AtlasId, RenderError>
    where
        Upload: FnOnce(AdmittedAtlas<'pixels>) -> Result<(), RenderError>,
    {
        let frame = layout.admit(pixels)?;
        let remaining = admit_atlas(
            self.limits,
            AtlasUsage {
                loaded: self.layouts.len(),
                remaining_bytes: self.remaining_bytes,
            },
            frame,
        )?;
        self.layouts
            .try_reserve(1)
            .map_err(|_error| RenderError::TooLarge)?;
        let id = AtlasId(self.layouts.len());
        upload(AdmittedAtlas { frame, id })?;
        self.layouts.push(layout);
        self.remaining_bytes = remaining;
        Ok(id)
    }

    pub(crate) const fn new(limits: SceneLimits) -> Self {
        Self {
            layouts: Vec::new(),
            limits,
            remaining_bytes: limits.atlas_bytes.get(),
        }
    }

    /// Admit replacement bytes against an existing atlas's exact layout.
    pub(crate) fn replace<'pixels, Upload>(
        &self,
        id: AtlasId,
        pixels: &'pixels [u8],
        upload: Upload,
    ) -> Result<(), RenderError>
    where
        Upload: FnOnce(AdmittedAtlas<'pixels>) -> Result<(), RenderError>,
    {
        let layout =
            *self.layouts.get(id.0).ok_or(RenderError::InvalidSprite)?;
        upload(AdmittedAtlas {
            frame: layout.admit(pixels)?,
            id,
        })
    }
}

/// A receipt remains current only until an observed or dispatched change.
#[derive(Debug, Default)]
pub(crate) struct PresentationAdmission {
    last: Cell<Option<PresentationLayout>>,
}

impl PresentationAdmission {
    pub(crate) fn complete<Expected>(
        &self,
        receipt: Option<PresentationLayout>,
        expected: Expected,
    ) -> Result<bool, RenderError>
    where
        Expected: FnOnce(Dimensions) -> Result<PresentationLayout, RenderError>,
    {
        self.invalidate();
        if let Some(presented) = receipt
            && presented != expected(presented.surface)?
        {
            return Err(RenderError::InvalidDimensions);
        }
        self.last.set(receipt);
        Ok(receipt.is_some())
    }

    pub(crate) fn invalidate(&self) {
        self.last.set(None);
    }

    pub(crate) const fn last(&self) -> Option<PresentationLayout> {
        self.last.get()
    }

    pub(crate) fn observe_surface(
        &self,
        surface: Result<Option<Dimensions>, RenderError>,
    ) -> Result<Option<Dimensions>, RenderError> {
        if self.last().is_some_and(|layout| {
            surface.as_ref().ok().copied().flatten() != Some(layout.surface)
        }) {
            self.invalidate();
        }
        surface
    }

    pub(crate) fn visible<Observe>(
        &self,
        observe: Observe,
    ) -> Result<Option<PresentationLayout>, RenderError>
    where
        Observe: FnOnce() -> Result<Option<PresentationLayout>, RenderError>,
    {
        let Some(last) = self.last() else {
            return Ok(None);
        };
        let observed = observe();
        if observed
            .as_ref()
            .is_ok_and(|current| *current == Some(last))
        {
            return observed;
        }
        self.invalidate();
        observed.map(|_layout| None)
    }
}

/// Current atlas usage before admitting an additional resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasUsage {
    /// Number of resident atlases.
    pub loaded: usize,
    /// Unallocated bytes under the admitted atlas budget.
    pub remaining_bytes: usize,
}

/// Admit one allocation and return the prospective remaining byte budget.
///
/// # Errors
/// Rejects exceeded count and byte limits for admitted pixel data.
pub fn admit_atlas(
    limits: SceneLimits,
    usage: AtlasUsage,
    frame: AdmittedRgba<'_>,
) -> Result<usize, RenderError> {
    if usage.loaded >= limits.atlases.get() {
        return Err(RenderError::TooLarge);
    }
    usage
        .remaining_bytes
        .min(limits.atlas_bytes.get())
        .checked_sub(frame.layout().bytes())
        .ok_or(RenderError::TooLarge)
}

/// Admit the entire ordered draw list before modifying a rendered frame.
///
/// # Errors
/// Rejects exceeded command limits, unknown atlases and invalid rectangles.
pub fn admit_draws<Lookup>(
    limits: SceneLimits,
    sprites: &[SpriteDraw],
    mut atlas_layout: Lookup,
) -> Result<AdmittedDraws<'_>, RenderError>
where
    Lookup: FnMut(AtlasId) -> Option<FrameLayout>,
{
    if sprites.len() > limits.sprites.get() {
        return Err(RenderError::TooLarge);
    }
    for sprite in sprites {
        let layout =
            atlas_layout(sprite.atlas).ok_or(RenderError::InvalidSprite)?;
        admit_source(sprite.source, layout)?;
        validate_rectangle(sprite.destination)?;
    }
    Ok(AdmittedDraws { sprites })
}

fn admit_source(
    rectangle: SpriteRect,
    layout: FrameLayout,
) -> Result<(), RenderError> {
    let left = u32::try_from(rectangle.x)
        .map_err(|_error| RenderError::InvalidSprite)?;
    let top = u32::try_from(rectangle.y)
        .map_err(|_error| RenderError::InvalidSprite)?;
    if left
        .checked_add(rectangle.width.get())
        .is_none_or(|right| right > layout.width().get())
        || top
            .checked_add(rectangle.height.get())
            .is_none_or(|bottom| bottom > layout.height().get())
    {
        return Err(RenderError::InvalidSprite);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{
        AdmittedDraws, AtlasId, AtlasUsage, FrameLayout, RenderError,
        SceneLimits, SpriteDraw, SpriteRect, admit_atlas, admit_draws,
    };
    use crate::ir::check;

    fn limits() -> SceneLimits {
        SceneLimits {
            atlas_bytes: NonZeroUsize::new(16).unwrap_or(NonZeroUsize::MIN),
            atlases: NonZeroUsize::MIN,
            sprites: NonZeroUsize::MIN,
        }
    }

    fn layout() -> Result<FrameLayout, RenderError> {
        FrameLayout::new(
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            limits().atlas_bytes,
        )
    }

    #[test]
    fn allocation_admission_checks_length_count_and_bytes()
    -> Result<(), RenderError> {
        let layout = layout()?;
        let bytes = [0u8; 16];
        for (loaded, remaining, pixels, expected) in [
            (0, 16, 15, Err(RenderError::InvalidLength)),
            (1, 16, 16, Err(RenderError::TooLarge)),
            (0, 15, 16, Err(RenderError::TooLarge)),
            (0, 16, 16, Ok(0)),
            (0, usize::MAX, 16, Ok(0)),
        ] {
            check(
                &layout
                    .admit(
                        bytes
                            .get(..pixels)
                            .ok_or(RenderError::InvalidLength)?,
                    )
                    .and_then(|frame| {
                        admit_atlas(
                            limits(),
                            AtlasUsage {
                                loaded,
                                remaining_bytes: remaining,
                            },
                            frame,
                        )
                    }),
                &expected,
            )?;
        }
        Ok(())
    }

    #[test]
    fn draw_admission_checks_every_reference_and_source_extent()
    -> Result<(), RenderError> {
        let layout = layout()?;
        let rectangle = SpriteRect {
            x: 0,
            y: 0,
            width: layout.width(),
            height: layout.height(),
        };
        let sprite = SpriteDraw {
            atlas: AtlasId(0),
            destination: SpriteRect { x: -1, ..rectangle },
            flip_horizontal: false,
            flip_vertical: false,
            source: rectangle,
        };
        let atlas = |id| (id == AtlasId(0)).then_some(layout);
        check(
            &admit_draws(limits(), &[sprite], atlas)
                .map(AdmittedDraws::as_slice),
            &Ok([sprite].as_slice()),
        )?;
        check(
            &admit_draws(limits(), &[sprite, sprite], atlas),
            &Err(RenderError::TooLarge),
        )?;
        for (atlas_id, x, y) in [
            (AtlasId(1), 0i32, 0i32),
            (AtlasId(0), -1i32, 0i32),
            (AtlasId(0), 0i32, -1i32),
            (AtlasId(0), 1i32, 0i32),
            (AtlasId(0), 0i32, 1i32),
            (AtlasId(0), i32::MAX, i32::MAX),
        ] {
            let bad = SpriteDraw {
                atlas: atlas_id,
                source: SpriteRect { x, y, ..rectangle },
                ..sprite
            };
            check(
                &admit_draws(limits(), &[bad], atlas),
                &Err(RenderError::InvalidSprite),
            )?;
        }
        Ok(())
    }

    #[test]
    fn draw_admission_preserves_clipping_and_rejects_unsafe_origins()
    -> Result<(), RenderError> {
        let layout = layout()?;
        let rectangle = SpriteRect {
            x: 0,
            y: 0,
            width: layout.width(),
            height: layout.height(),
        };
        let sprite = SpriteDraw {
            atlas: AtlasId(0),
            destination: rectangle,
            flip_horizontal: false,
            flip_vertical: false,
            source: rectangle,
        };
        let atlas = |id| (id == AtlasId(0)).then_some(layout);
        let lower = -1_073_741_824i32;
        let upper = 1_073_741_823i32;
        for (x, y, valid) in [
            (lower, upper, true),
            (upper, lower, true),
            (-1i32, 0i32, true),
            (lower.saturating_sub(1), 0i32, false),
            (0i32, lower.saturating_sub(1), false),
            (upper.saturating_add(1), 0i32, false),
            (0i32, upper.saturating_add(1), false),
            (i32::MAX, i32::MIN, false),
        ] {
            let draw = SpriteDraw {
                destination: SpriteRect { x, y, ..rectangle },
                ..sprite
            };
            check(&admit_draws(limits(), &[draw], atlas).is_ok(), &valid)?;
        }
        Ok(())
    }
}
