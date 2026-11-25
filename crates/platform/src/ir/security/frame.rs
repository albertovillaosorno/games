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

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{FrameLayout, RenderError};

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
