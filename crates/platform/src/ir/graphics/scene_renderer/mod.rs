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
//   - Portable ordered scene session and read-only presentation geometry.
// - Must-Not:
//   - Expose renderer resources or interpret gameplay and source art.
// - Allows:
//   - Inputs: Bounded atlases, sprite commands and display settings.
//   - Outputs: Presented geometry, frame readback and input observations.
//   - Side effects: Delegated rendering and bounded event polling.
// - Split-When:
//   - Another render command family needs distinct admission.
// - Merge-When:
//   - Shared frame or scene contracts cover equivalent operations.
// - Summary:
//   - Callback-scoped ordered scene over a private rendering port.
// - Description:
//   - Project touch against the actual last presented scene geometry.
// - Usage:
//   - Upload atlases once and submit ordered bounded frames.
// - Defaults:
//   - Missing or stale presentation geometry does not admit touch.

//! Portable ordered rendering sessions and presentation geometry.

use std::fmt::{self, Debug, Formatter};
use std::num::NonZeroUsize;

use games::input::touch::TouchPoint;
use games::presentation::{DisplaySettings, PresentationLayout};

use crate::ir::frame::{FrameLayout, RenderError};
use crate::ir::hardware::InputSample;
use crate::ir::scene::{
    AtlasId, AtlasUpload, EventBatch, SceneLimits, SpriteDraw,
};
use crate::ir::security::scene::SceneAdmission;

/// Private, callback-scoped ordered rendering port.
pub(crate) trait SceneBackend: Debug {
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError>;
    fn fit_adaptive_width(
        &mut self,
        maximum: FrameLayout,
    ) -> Result<FrameLayout, RenderError>;
    fn last_presented_layout(&self) -> Option<PresentationLayout>;
    fn layout(&self) -> FrameLayout;
    // Commit the supplied atlas identity only after a successful upload.
    // An error must leave the previously admitted resources unchanged.
    fn load_atlas(
        &mut self,
        id: AtlasId,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<(), RenderError>;
    fn poll_event(&mut self, wait_ms: u32) -> Option<InputSample>;
    fn present(
        &mut self,
        background: [u8; 3],
        sprites: &[SpriteDraw],
    ) -> Result<bool, RenderError>;
    fn readback(&mut self) -> Result<Vec<u8>, RenderError>;
    fn visible_layout(&self)
    -> Result<Option<PresentationLayout>, RenderError>;
}

/// A read-only view of scene geometry during event dispatch.
#[derive(Clone, Copy)]
pub struct SceneView<'session> {
    backend: &'session dyn SceneBackend,
}

impl Debug for SceneView<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneView")
            .field("layout", &self.layout())
            .finish_non_exhaustive()
    }
}

impl SceneView<'_> {
    /// Logical extent of the current target, including adaptive extension.
    #[must_use]
    pub fn layout(self) -> FrameLayout {
        self.backend.layout()
    }

    /// Map surface-normalized input to the actually presented scene.
    /// # Errors
    /// Reports failure to observe current drawable geometry.
    pub fn logical_touch(
        self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        Ok(self
            .visible_layout()?
            .and_then(|layout| point.in_presentation(layout)))
    }

    /// Map logical input coordinates to the actually presented surface.
    /// # Errors
    /// Reports failure to observe current drawable geometry.
    pub fn surface_touch(
        self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        Ok(self
            .visible_layout()?
            .and_then(|layout| point.on_presentation(layout)))
    }

    /// Last presented layout, only if it matches current drawable geometry.
    /// # Errors
    /// Reports a failed drawable query.
    pub fn visible_layout(
        self,
    ) -> Result<Option<PresentationLayout>, RenderError> {
        self.backend.visible_layout()
    }
}

/// Persistent ordered scene resources scoped to the window's callback.
pub struct SceneSession<'session, 'backend> {
    admission: SceneAdmission,
    backend: &'session mut (dyn SceneBackend + 'backend),
}

impl Debug for SceneSession<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneSession")
            .field("layout", &self.layout())
            .finish_non_exhaustive()
    }
}

impl<'session, 'backend> SceneSession<'session, 'backend> {
    /// Change display settings without rebuilding resident atlases.
    /// # Errors
    /// Reports an unavailable or rejected display transition.
    pub fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError> {
        self.backend.apply_display_settings(settings)
    }

    /// Poll at most the caller's event budget, waiting only on the first poll.
    /// The callback observes a read-only view and can stop without draining.
    /// # Errors
    /// Propagates the first handler or backend error.
    pub fn dispatch_events<Dispatch>(
        &mut self,
        budget: NonZeroUsize,
        wait_ms: u32,
        mut dispatch: Dispatch,
    ) -> Result<EventBatch, RenderError>
    where
        Dispatch:
            FnMut(SceneView<'_>, &InputSample) -> Result<bool, RenderError>,
    {
        for index in 0..budget.get() {
            let first = index == 0;
            let wait = if first {
                wait_ms
            } else {
                0
            };
            let empty = if first {
                EventBatch::Empty
            } else {
                EventBatch::Drained
            };
            let Some(event) = self.backend.poll_event(wait) else {
                return Ok(empty);
            };
            if !dispatch(self.view(), &event)? {
                return Ok(EventBatch::Stopped);
            }
        }
        Ok(EventBatch::Limited)
    }

    /// Fit the adaptive logical target without rebuilding resident atlases.
    /// # Errors
    /// Reports invalid maximum dimensions or allocation failure.
    pub fn fit_adaptive_width(
        &mut self,
        maximum: FrameLayout,
    ) -> Result<FrameLayout, RenderError> {
        self.backend.fit_adaptive_width(maximum)
    }

    /// Exact layout of the last successful drawable presentation, if any.
    #[must_use]
    pub fn last_presented_layout(&self) -> Option<PresentationLayout> {
        self.backend.last_presented_layout()
    }

    /// Current logical render extent.
    #[must_use]
    pub fn layout(&self) -> FrameLayout {
        self.backend.layout()
    }

    /// Upload a bounded, tightly packed RGBA atlas.
    /// # Errors
    /// Rejects incorrect byte extent, exceeded budget or backend failure.
    pub fn load_atlas(
        &mut self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError> {
        self.admission.load(layout, pixels.len(), |id| {
            self.backend.load_atlas(id, layout, pixels)
        })
    }

    /// Map surface-normalized touch to the actually presented scene.
    /// # Errors
    /// Reports failure to observe drawable geometry.
    pub fn logical_touch(
        &self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        self.view().logical_touch(point)
    }

    pub(crate) fn new(
        backend: &'session mut (dyn SceneBackend + 'backend),
        limits: SceneLimits,
    ) -> Self {
        Self {
            admission: SceneAdmission::new(limits),
            backend,
        }
    }

    /// Present ordered sprite commands over the opaque background.
    /// # Errors
    /// Rejects invalid sprite commands or failed backend rendering.
    pub fn present(
        &mut self,
        background: [u8; 3],
        sprites: &[SpriteDraw],
    ) -> Result<bool, RenderError> {
        self.admission.draws(sprites)?;
        self.backend.present(background, sprites)
    }

    /// Whether the last presented frame still matches current geometry.
    /// # Errors
    /// Reports a failed drawable query.
    pub fn presentation_is_current(&self) -> Result<bool, RenderError> {
        Ok(self.visible_layout()?.is_some())
    }

    /// Read back a complete logical RGBA frame for diagnostics only.
    /// # Errors
    /// Rejects incorrect extent or backend readback failure.
    pub fn readback(&mut self) -> Result<Vec<u8>, RenderError> {
        let pixels = self.backend.readback()?;
        if pixels.len() != self.layout().bytes() {
            return Err(RenderError::InvalidLength);
        }
        Ok(pixels)
    }

    /// Project a logical touch point to the actually presented surface.
    /// # Errors
    /// Reports failure to observe drawable geometry.
    pub fn surface_touch(
        &self,
        point: TouchPoint,
    ) -> Result<Option<TouchPoint>, RenderError> {
        self.view().surface_touch(point)
    }

    /// Borrow read-only geometry from this session.
    #[must_use]
    pub fn view(&self) -> SceneView<'_> {
        SceneView { backend: self.backend }
    }

    /// Observe the last presented layout if it matches current geometry.
    /// # Errors
    /// Reports a failed drawable query.
    pub fn visible_layout(
        &self,
    ) -> Result<Option<PresentationLayout>, RenderError> {
        self.view().visible_layout()
    }
}

impl AtlasUpload for SceneSession<'_, '_> {
    fn upload_atlas(
        &mut self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<AtlasId, RenderError> {
        self.load_atlas(layout, pixels)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use super::{
        AtlasId, DisplaySettings, EventBatch, FrameLayout, InputSample,
        PresentationLayout, RenderError, SceneBackend, SceneLimits,
        SceneSession, SpriteDraw,
    };
    use crate::ir::check;
    use crate::ir::hardware::{HostEvent, InputTransition};
    use crate::ir::scene::SpriteRect;

    #[derive(Debug)]
    struct TestScene {
        draws: usize,
        events: VecDeque<InputSample>,
        fail_upload: bool,
        layout: FrameLayout,
        uploads: Vec<AtlasId>,
        waits: Vec<u32>,
    }

    impl TestScene {
        fn new(count: u64) -> Result<Self, RenderError> {
            Ok(Self {
                draws: 0,
                events: (1..=count)
                    .map(|tick| {
                        InputSample::new(
                            Some(Duration::from_millis(tick)),
                            InputTransition::Event(HostEvent::Quit),
                            None,
                        )
                    })
                    .collect(),
                fail_upload: false,
                layout: FrameLayout::new(
                    NonZeroU32::MIN,
                    NonZeroU32::MIN,
                    NonZeroUsize::MAX,
                )?,
                uploads: Vec::new(),
                waits: Vec::new(),
            })
        }
    }

    impl SceneBackend for TestScene {
        fn apply_display_settings(
            &mut self,
            _settings: DisplaySettings,
        ) -> Result<(), RenderError> {
            Ok(())
        }

        fn fit_adaptive_width(
            &mut self,
            _maximum: FrameLayout,
        ) -> Result<FrameLayout, RenderError> {
            Ok(self.layout)
        }

        fn last_presented_layout(&self) -> Option<PresentationLayout> {
            None
        }

        fn layout(&self) -> FrameLayout {
            self.layout
        }

        fn load_atlas(
            &mut self,
            id: AtlasId,
            _layout: FrameLayout,
            _pixels: &[u8],
        ) -> Result<(), RenderError> {
            if self.fail_upload {
                self.fail_upload = false;
                return Err(RenderError::Unsupported);
            }
            self.uploads.push(id);
            Ok(())
        }

        fn poll_event(&mut self, wait_ms: u32) -> Option<InputSample> {
            self.waits.push(wait_ms);
            self.events.pop_front()
        }

        fn present(
            &mut self,
            _background: [u8; 3],
            _sprites: &[SpriteDraw],
        ) -> Result<bool, RenderError> {
            self.draws = self.draws.saturating_add(1);
            Ok(false)
        }

        fn readback(&mut self) -> Result<Vec<u8>, RenderError> {
            Ok(Vec::new())
        }

        fn visible_layout(
            &self,
        ) -> Result<Option<PresentationLayout>, RenderError> {
            Ok(None)
        }
    }

    fn limits() -> SceneLimits {
        SceneLimits {
            atlas_bytes: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            atlases: NonZeroUsize::MIN,
            sprites: NonZeroUsize::MIN,
        }
    }

    #[test]
    fn atlas_and_draw_admission_precedes_dispatch() -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        backend.fail_upload = true;
        let layout = backend.layout;
        let mut scene = SceneSession::new(&mut backend, limits());
        check(
            &scene.load_atlas(layout, &[0; 3]),
            &Err(RenderError::InvalidLength),
        )?;
        check(
            &scene.load_atlas(layout, &[0; 4]),
            &Err(RenderError::Unsupported),
        )?;
        let rectangle = SpriteRect {
            x: 0,
            y: 0,
            width: NonZeroU32::MIN,
            height: NonZeroU32::MIN,
        };
        let sprite = SpriteDraw {
            atlas: AtlasId(0),
            destination: rectangle,
            flip_horizontal: false,
            flip_vertical: false,
            source: rectangle,
        };
        check(
            &scene.present([0; 3], &[sprite]),
            &Err(RenderError::InvalidSprite),
        )?;
        let id = scene.load_atlas(layout, &[0; 4])?;
        check(&id, &AtlasId(0))?;
        check(
            &scene.load_atlas(layout, &[0; 4]),
            &Err(RenderError::TooLarge),
        )?;
        check(
            &scene.present([0; 3], &[sprite, sprite]),
            &Err(RenderError::TooLarge),
        )?;
        let invalid = SpriteDraw {
            source: SpriteRect { x: 1, ..rectangle },
            ..sprite
        };
        check(
            &scene.present([0; 3], &[invalid]),
            &Err(RenderError::InvalidSprite),
        )?;
        check(&scene.present([0; 3], &[sprite]), &Ok(false))?;
        check(&backend.uploads, &[AtlasId(0)])?;
        check(&backend.draws, &1)?;
        Ok(())
    }

    #[test]
    fn bounded_batches_preserve_order_queue_tail_and_first_wait()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(5)?;
        let mut delivered = Vec::new();
        let budget = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
        let mut scene = SceneSession::new(&mut backend, limits());
        for (expected, count) in [
            (EventBatch::Limited, 2),
            (EventBatch::Limited, 4),
            (EventBatch::Drained, 5),
            (EventBatch::Empty, 5),
        ] {
            let result = scene.dispatch_events(budget, 12, |view, event| {
                check(&view.visible_layout()?, &None)?;
                delivered.push(event.elapsed_since(Duration::ZERO));
                Ok(true)
            })?;
            check(&result, &expected)?;
            check(&delivered.len(), &count)?;
        }
        check(
            &delivered,
            &(1..=5)
                .map(|tick| Some(Duration::from_millis(tick)))
                .collect::<Vec<_>>(),
        )?;
        check(&backend.waits, &[12, 0, 12, 0, 12, 0, 12])?;
        Ok(())
    }

    #[test]
    fn event_stop_or_failure_does_not_consume_next_sample()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(3)?;
        let budget = NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN);
        let mut scene = SceneSession::new(&mut backend, limits());
        check(
            &scene.dispatch_events(budget, 0, |_, _| Ok(false))?,
            &EventBatch::Stopped,
        )?;
        check(
            &scene.dispatch_events(budget, 0, |_, event| {
                check(
                    &event.elapsed_since(Duration::ZERO),
                    &Some(Duration::from_millis(2)),
                )?;
                Err(RenderError::InvalidLength)
            }),
            &Err(RenderError::InvalidLength),
        )?;
        check(
            &scene.dispatch_events(budget, 0, |_, event| {
                check(
                    &event.elapsed_since(Duration::ZERO),
                    &Some(Duration::from_millis(3)),
                )?;
                Ok(true)
            })?,
            &EventBatch::Drained,
        )?;
        check(&scene.readback(), &Err(RenderError::InvalidLength))?;
        Ok(())
    }
}
