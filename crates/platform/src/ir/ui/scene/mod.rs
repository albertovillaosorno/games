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

use std::num::NonZeroUsize;

pub use games::presentation::SpriteRect;

use crate::ir::frame::{FrameLayout, RenderError};

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
