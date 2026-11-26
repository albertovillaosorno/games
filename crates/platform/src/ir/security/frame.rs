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
//   - Backend-independent RGBA frame layout and rendering error values.
// - Must-Not:
//   - Allocate native resources or choose a rendering backend.
// - Allows:
//   - Inputs: Explicit dimensions and caller-provided byte budget.
//   - Outputs: Checked pixel dimensions, stride and byte count.
//   - Side effects: None.
// - Split-When:
//   - Additional pixel formats need independent layout policies.
// - Merge-When:
//   - A shared graphics contract duplicates these layout checks.
// - Summary:
//   - Admit bounded tightly packed RGBA8 frames for portable rendering.
// - Description:
//   - Preserve representable rectangle and signed stride bounds.
// - Usage:
//   - Check layout before allocating or uploading frame data.
// - Defaults:
//   - Reject oversized dimensions, stride and total bytes.

//! Portable frame layout and rendering errors.

use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{Dimensions, PresentationMode, SpriteRect};

// Keep both ends of a signed rectangle representable.
const MAX_RECTANGLE_EXTENT: u32 = 1_073_741_823;

/// An explicit rendering failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// The native display or renderer rejected an operation.
    Backend(String),
    /// The pixel or surface dimensions are not representable.
    InvalidDimensions,
    /// The supplied frame is not exactly the requested RGBA extent.
    InvalidLength,
    /// An atlas or sprite rectangle is absent or outside its source.
    InvalidSprite,
    /// The frame requires more bytes than permitted.
    TooLarge,
    /// The selected platform has no implementation of this capability.
    Unsupported,
}

/// A bounded, tightly packed logical RGBA8 frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLayout {
    bytes: usize,
    height: NonZeroU32,
    pitch: usize,
    width: NonZeroU32,
}

impl FrameLayout {
    /// Number of bytes in one complete frame.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Logical height of the frame.
    #[must_use]
    pub const fn height(self) -> NonZeroU32 {
        self.height
    }

    /// Check dimensions and the caller's maximum texture upload size.
    ///
    /// # Errors
    /// Rejects unrepresentable dimensions, stride or total byte counts.
    pub fn new(
        width: NonZeroU32,
        height: NonZeroU32,
        max_bytes: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        validate_extent(width, height)?;
        let pitch = usize::try_from(width.get())
            .ok()
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RenderError::InvalidDimensions)?;
        if i32::try_from(pitch).is_err() {
            return Err(RenderError::InvalidDimensions);
        }
        let bytes = usize::try_from(height.get())
            .ok()
            .and_then(|rows| pitch.checked_mul(rows))
            .ok_or(RenderError::TooLarge)?;
        if bytes > max_bytes.get() {
            return Err(RenderError::TooLarge);
        }
        Ok(Self {
            bytes,
            height,
            pitch,
            width,
        })
    }

    /// Number of bytes in one tightly packed pixel row.
    #[must_use]
    pub const fn pitch(self) -> usize {
        self.pitch
    }

    /// Logical width of the frame.
    #[must_use]
    pub const fn width(self) -> NonZeroU32 {
        self.width
    }
}

/// An admitted bound for height-preserving scene extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdaptiveWidthRequest {
    base: FrameLayout,
    maximum: FrameLayout,
}

impl AdaptiveWidthRequest {
    /// Whether selecting the next extent requires a drawable observation.
    #[must_use]
    pub fn needs_surface(self, mode: PresentationMode) -> bool {
        mode == PresentationMode::Adaptive
            && self.maximum.width() > self.base.width()
    }

    /// Admit a maximum against the initial logical scene dimensions.
    /// # Errors
    /// Rejects a different height or a width smaller than the base.
    pub fn new(
        base: FrameLayout,
        maximum: FrameLayout,
    ) -> Result<Self, RenderError> {
        if maximum.height() != base.height() || maximum.width() < base.width() {
            return Err(RenderError::InvalidDimensions);
        }
        Ok(Self { base, maximum })
    }

    /// Choose the complete bounded logical frame for the next presentation.
    ///
    /// Missing drawable geometry and fixed presentation modes use the base.
    /// # Errors
    /// Rejects an unrepresentable selected extent.
    pub fn select(
        self,
        mode: PresentationMode,
        surface: Option<Dimensions>,
    ) -> Result<FrameLayout, RenderError> {
        let base = Dimensions::new(self.base.width(), self.base.height());
        let design = surface.map_or(base, |extent| {
            mode.design_for_surface(extent, base, self.maximum.width())
        });
        if design.width == base.width {
            return Ok(self.base);
        }
        let budget = NonZeroUsize::new(self.maximum.bytes())
            .ok_or(RenderError::TooLarge)?;
        FrameLayout::new(design.width, design.height, budget)
    }
}

/// Validate dimensions before opening a window or allocating a frame.
///
/// # Errors
/// Rejects dimensions outside the portable signed rectangle range.
pub fn validate_extent(
    width: NonZeroU32,
    height: NonZeroU32,
) -> Result<(), RenderError> {
    if width.get().max(height.get()) > MAX_RECTANGLE_EXTENT {
        Err(RenderError::InvalidDimensions)
    } else {
        Ok(())
    }
}

/// Admit a signed sprite rectangle without truncating its origin or extent.
///
/// Clipped destinations are permitted. Keeping each signed origin within
/// half of the coordinate range and each extent within the positive half
/// ensures both ends remain representable without backend-specific clamping.
/// # Errors
/// Rejects unrepresentable dimensions or origins.
pub fn validate_rectangle(rectangle: SpriteRect) -> Result<(), RenderError> {
    validate_extent(rectangle.width, rectangle.height)?;
    let origins = -1_073_741_824i32..=1_073_741_823i32;
    if !origins.contains(&rectangle.x) || !origins.contains(&rectangle.y) {
        return Err(RenderError::InvalidSprite);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{
        AdaptiveWidthRequest, Dimensions, FrameLayout, PresentationMode,
        RenderError,
    };
    use crate::ir::check;

    #[test]
    fn adaptive_selector_retains_height_and_bounded_world_extent()
    -> Result<(), RenderError> {
        let size = |value| NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN);
        let layout =
            |width| FrameLayout::new(size(width), size(4), NonZeroUsize::MAX);
        let base = layout(6)?;
        let maximum = layout(10)?;
        let request = AdaptiveWidthRequest::new(base, maximum)?;
        for mode in PresentationMode::ALL {
            let adaptive = mode == PresentationMode::Adaptive;
            check(&request.needs_surface(mode), &adaptive)?;
            check(&request.select(mode, None), &Ok(base))?;
            check(
                &request.select(mode, Some(Dimensions::new(size(3), size(4)))),
                &Ok(base),
            )?;
        }
        for (mode, columns, expected) in [
            (PresentationMode::Adaptive, 8, 8),
            (PresentationMode::Adaptive, 24, 10),
            (PresentationMode::AspectFit, 24, 6),
            (PresentationMode::Integer, 24, 6),
            (PresentationMode::Fill, 24, 6),
            (PresentationMode::Stretch, 24, 6),
        ] {
            let surface = Some(Dimensions::new(size(columns), size(4)));
            check(&request.select(mode, surface), &layout(expected))?;
        }
        Ok(())
    }

    #[test]
    fn layout_preserves_packed_extent_and_budget() {
        let width = NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let budget = NonZeroUsize::new(24).unwrap_or(NonZeroUsize::MIN);
        let layout = FrameLayout::new(width, height, budget);
        assert_eq!(layout.clone().map(FrameLayout::pitch), Ok(12));
        assert_eq!(layout.clone().map(FrameLayout::bytes), Ok(24));
        assert_eq!(layout.clone().map(FrameLayout::width), Ok(width));
        assert_eq!(layout.map(FrameLayout::height), Ok(height));
        assert_eq!(
            FrameLayout::new(width, height, NonZeroUsize::MIN),
            Err(RenderError::TooLarge)
        );
    }

    #[test]
    fn layout_rejects_unrepresentable_dimensions_and_stride() {
        let budget = NonZeroUsize::MAX;
        assert_eq!(
            FrameLayout::new(NonZeroU32::MIN, NonZeroU32::MAX, budget),
            Err(RenderError::InvalidDimensions)
        );
        let width = NonZeroU32::new(536_870_912).unwrap_or(NonZeroU32::MIN);
        assert_eq!(
            FrameLayout::new(width, NonZeroU32::MIN, budget),
            Err(RenderError::InvalidDimensions)
        );
    }
}
