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
use games::presentation::{
    Dimensions, DisplaySettings, PresentationLayout, PresentationMode,
};

use crate::ir::frame::{AdaptiveWidthRequest, FrameLayout, RenderError};
use crate::ir::hardware::InputSample;
use crate::ir::scene::{
    AtlasId, AtlasUpload, EventBatch, SceneLimits, SpriteDraw,
};
use crate::ir::security::scene::{
    AdmittedAtlas, AdmittedDraws, PresentationAdmission, SceneAdmission,
};

/// Private, callback-scoped ordered rendering port.
pub(crate) trait SceneBackend: Debug {
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError>;
    fn drawable_available(&self) -> bool;
    fn layout(&self) -> FrameLayout;
    // Commit the supplied atlas identity only after a successful upload.
    // An error must leave the previously admitted resources unchanged.
    fn load_atlas(
        &mut self,
        upload: AdmittedAtlas<'_>,
    ) -> Result<(), RenderError>;
    fn poll_event(&mut self, wait_ms: u32) -> Option<InputSample>;
    fn present(
        &mut self,
        background: [u8; 3],
        sprites: AdmittedDraws<'_>,
    ) -> Result<Option<PresentationLayout>, RenderError>;
    fn presentation_mode(&self) -> PresentationMode;
    fn readback(&mut self) -> Result<Vec<u8>, RenderError>;
    fn resize_target(&mut self, layout: FrameLayout)
    -> Result<(), RenderError>;
    fn surface_dimensions(&self) -> Result<Option<Dimensions>, RenderError>;
}

/// A read-only view of scene geometry during event dispatch.
#[derive(Clone, Copy)]
pub struct SceneView<'session> {
    backend: &'session dyn SceneBackend,
    presentation: &'session PresentationAdmission,
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
        self.presentation.visible(|| {
            if !self.backend.drawable_available() {
                return Ok(None);
            }
            self.backend
                .surface_dimensions()?
                .map(|surface| {
                    self.layout()
                        .presentation(self.backend.presentation_mode(), surface)
                })
                .transpose()
        })
    }
}

/// Persistent ordered scene resources scoped to the window's callback.
pub struct SceneSession<'session, 'backend> {
    admission: SceneAdmission,
    backend: &'session mut (dyn SceneBackend + 'backend),
    base: FrameLayout,
    presentation: PresentationAdmission,
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
        self.presentation.invalidate();
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
        let request = AdaptiveWidthRequest::new(self.base, maximum)?;
        let mode = self.backend.presentation_mode();
        let surface = if request.needs_surface(mode) {
            self.presentation
                .observe_surface(self.backend.surface_dimensions())?
        } else {
            None
        };
        let layout = request.select(mode, surface)?;
        if self.layout() == layout {
            return Ok(layout);
        }
        self.presentation.invalidate();
        self.backend.resize_target(layout)?;
        if self.layout() != layout {
            return Err(RenderError::InvalidDimensions);
        }
        Ok(layout)
    }

    /// Last successful drawable receipt not invalidated by a later change.
    #[must_use]
    pub const fn last_presented_layout(&self) -> Option<PresentationLayout> {
        self.presentation.last()
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
        self.admission
            .load(layout, pixels, |upload| self.backend.load_atlas(upload))
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
        base: FrameLayout,
        limits: SceneLimits,
    ) -> Result<Self, RenderError> {
        if backend.layout() != base {
            return Err(RenderError::InvalidDimensions);
        }
        Ok(Self {
            admission: SceneAdmission::new(limits),
            backend,
            base,
            presentation: PresentationAdmission::default(),
        })
    }

    /// Present ordered sprite commands over the opaque background.
    /// # Errors
    /// Rejects invalid commands, inconsistent geometry or backend rendering.
    pub fn present(
        &mut self,
        background: [u8; 3],
        sprites: &[SpriteDraw],
    ) -> Result<bool, RenderError> {
        let admitted = self.admission.draws(sprites)?;
        self.presentation.invalidate();
        let receipt = self.backend.present(background, admitted)?;
        self.presentation.complete(
            receipt,
            self.layout(),
            self.backend.presentation_mode(),
        )
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
        SceneView {
            backend: self.backend,
            presentation: &self.presentation,
        }
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
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use super::{
        AdmittedAtlas, AdmittedDraws, AtlasId, Dimensions, DisplaySettings,
        EventBatch, FrameLayout, InputSample, PresentationAdmission,
        PresentationLayout, PresentationMode, RenderError, SceneBackend,
        SceneLimits, SceneSession, SceneView, SpriteDraw,
    };
    use crate::ir::check;
    use crate::ir::hardware::{HostEvent, InputTransition};
    use crate::ir::scene::SpriteRect;

    type PresentationResult = Result<Option<PresentationLayout>, RenderError>;
    type RecordedUpload = (AtlasId, FrameLayout, Vec<u8>);

    #[derive(Debug)]
    struct TestScene {
        drawable: bool,
        draws: usize,
        events: VecDeque<InputSample>,
        fail_upload: bool,
        layout: FrameLayout,
        mode: PresentationMode,
        presentations: VecDeque<PresentationResult>,
        resize_outcome: Result<bool, RenderError>,
        resizes: Vec<FrameLayout>,
        settings_outcome: Result<(), RenderError>,
        surface: Option<Dimensions>,
        surface_queries: Cell<usize>,
        uploads: Vec<RecordedUpload>,
        waits: Vec<u32>,
    }

    impl TestScene {
        fn new(count: u64) -> Result<Self, RenderError> {
            Ok(Self {
                drawable: true,
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
                mode: PresentationMode::AspectFit,
                presentations: VecDeque::new(),
                resize_outcome: Ok(true),
                resizes: Vec::new(),
                settings_outcome: Ok(()),
                surface: None,
                surface_queries: Cell::new(0),
                uploads: Vec::new(),
                waits: Vec::new(),
            })
        }
    }

    impl SceneBackend for TestScene {
        fn apply_display_settings(
            &mut self,
            settings: DisplaySettings,
        ) -> Result<(), RenderError> {
            self.mode = settings.presentation;
            self.settings_outcome.clone()
        }

        fn drawable_available(&self) -> bool {
            self.drawable
        }

        fn layout(&self) -> FrameLayout {
            self.layout
        }

        fn load_atlas(
            &mut self,
            upload: AdmittedAtlas<'_>,
        ) -> Result<(), RenderError> {
            if self.fail_upload {
                self.fail_upload = false;
                return Err(RenderError::Unsupported);
            }
            let frame = upload.frame();
            self.uploads.push((
                upload.id(),
                frame.layout(),
                frame.pixels().to_vec(),
            ));
            Ok(())
        }

        fn poll_event(&mut self, wait_ms: u32) -> Option<InputSample> {
            self.waits.push(wait_ms);
            self.events.pop_front()
        }

        fn present(
            &mut self,
            _background: [u8; 3],
            _sprites: AdmittedDraws<'_>,
        ) -> Result<Option<PresentationLayout>, RenderError> {
            self.draws = self.draws.saturating_add(1);
            self.presentations.pop_front().unwrap_or(Ok(None))
        }

        fn presentation_mode(&self) -> PresentationMode {
            self.mode
        }

        fn readback(&mut self) -> Result<Vec<u8>, RenderError> {
            Ok(Vec::new())
        }

        fn resize_target(
            &mut self,
            layout: FrameLayout,
        ) -> Result<(), RenderError> {
            self.resizes.push(layout);
            if self.resize_outcome.clone()? {
                self.layout = layout;
            }
            Ok(())
        }

        fn surface_dimensions(
            &self,
        ) -> Result<Option<Dimensions>, RenderError> {
            self.surface_queries
                .set(self.surface_queries.get().saturating_add(1));
            Ok(self.surface)
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
    fn adaptive_session_admits_extent_and_restores_base()
    -> Result<(), RenderError> {
        let width = NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let base = FrameLayout::new(height, height, NonZeroUsize::MAX)?;
        let maximum = FrameLayout::new(width, height, NonZeroUsize::MAX)?;
        let short =
            FrameLayout::new(NonZeroU32::MIN, height, NonZeroUsize::MAX)?;
        let wrong_height =
            FrameLayout::new(width, NonZeroU32::MIN, NonZeroUsize::MAX)?;
        let mut backend = TestScene::new(0)?;
        backend.layout = base;
        backend.surface = Some(Dimensions::new(width, height));
        backend.mode = PresentationMode::Adaptive;
        {
            let mut scene = SceneSession::new(&mut backend, base, limits())?;
            for invalid in [short, wrong_height] {
                check(
                    &scene.fit_adaptive_width(invalid),
                    &Err(RenderError::InvalidDimensions),
                )?;
            }
            check(&scene.fit_adaptive_width(base), &Ok(base))?;
            check(&scene.fit_adaptive_width(maximum), &Ok(maximum))?;
            check(&scene.fit_adaptive_width(maximum), &Ok(maximum))?;
            scene.apply_display_settings(DisplaySettings {
                presentation: PresentationMode::AspectFit,
                ..DisplaySettings::default()
            })?;
            check(&scene.fit_adaptive_width(maximum), &Ok(base))?;
            check(&scene.layout(), &base)?;
        }
        check(&backend.surface_queries.get(), &2)?;
        check(&backend.resizes, &[maximum, base])?;
        Ok(())
    }

    #[test]
    fn adaptive_session_rejects_inconsistent_backend_extent()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let maximum = FrameLayout::new(
            NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            base.height(),
            NonZeroUsize::MAX,
        )?;
        check(
            &SceneSession::new(&mut backend, maximum, limits()).err(),
            &Some(RenderError::InvalidDimensions),
        )?;
        for (outcome, expected) in [
            (Ok(false), RenderError::InvalidDimensions),
            (Err(RenderError::Unsupported), RenderError::Unsupported),
        ] {
            backend.mode = PresentationMode::Adaptive;
            backend.surface =
                Some(Dimensions::new(maximum.width(), maximum.height()));
            backend.resize_outcome = outcome;
            {
                let mut scene =
                    SceneSession::new(&mut backend, base, limits())?;
                check(&scene.fit_adaptive_width(maximum), &Err(expected))?;
                check(&scene.layout(), &base)?;
            }
        }
        backend.resize_outcome = Ok(true);
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
        check(&scene.fit_adaptive_width(maximum), &Ok(maximum))?;
        Ok(())
    }

    #[test]
    fn atlas_and_draw_admission_precedes_dispatch() -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        backend.fail_upload = true;
        let layout = backend.layout;
        let mut scene = SceneSession::new(&mut backend, layout, limits())?;
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
        let id = scene.load_atlas(layout, &[1, 2, 3, 4])?;
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
        let invalid_origin = SpriteDraw {
            destination: SpriteRect { x: i32::MAX, ..rectangle },
            ..sprite
        };
        check(
            &scene.present([0; 3], &[invalid_origin]),
            &Err(RenderError::InvalidSprite),
        )?;
        check(&scene.present([0; 3], &[sprite]), &Ok(false))?;
        check(&backend.uploads, &[(AtlasId(0), layout, vec![1, 2, 3, 4])])?;
        check(&backend.draws, &1)?;
        Ok(())
    }

    #[test]
    fn bounded_batches_preserve_order_queue_tail_and_first_wait()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(5)?;
        let mut delivered = Vec::new();
        let budget = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
        let base = backend.layout;
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
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
        let base = backend.layout;
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
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

    #[test]
    fn presentation_receipts_require_exact_admitted_geometry()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let width = NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let surface = Dimensions::new(width, height);
        let expected =
            base.presentation(PresentationMode::AspectFit, surface)?;
        let wrong = base.presentation(PresentationMode::Stretch, surface)?;
        backend.surface = Some(surface);
        backend.presentations = VecDeque::from([
            Ok(Some(expected)),
            Err(RenderError::Unsupported),
            Ok(Some(wrong)),
            Ok(Some(expected)),
            Ok(None),
        ]);
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
        check(&scene.last_presented_layout(), &None)?;
        check(&scene.visible_layout()?, &None)?;
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        let rect = SpriteRect {
            x: 0,
            y: 0,
            width: NonZeroU32::MIN,
            height: NonZeroU32::MIN,
        };
        let invalid = SpriteDraw {
            atlas: AtlasId(0),
            destination: rect,
            flip_horizontal: false,
            flip_vertical: false,
            source: rect,
        };
        check(
            &scene.present([0; 3], &[invalid]),
            &Err(RenderError::InvalidSprite),
        )?;
        check(&scene.visible_layout()?, &Some(expected))?;
        for (result, last) in [
            (Err(RenderError::Unsupported), None),
            (Err(RenderError::InvalidDimensions), None),
            (Ok(true), Some(expected)),
            (Ok(false), None),
        ] {
            check(&scene.present([0; 3], &[]), &result)?;
            check(&scene.last_presented_layout(), &last)?;
            check(&scene.visible_layout()?, &last)?;
        }
        Ok(())
    }

    #[test]
    fn dispatched_geometry_changes_require_fresh_presentation()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let width = NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN);
        let maximum =
            FrameLayout::new(width, base.height(), NonZeroUsize::MAX)?;
        let surface = Dimensions::new(width, base.height());
        let expected =
            base.presentation(PresentationMode::AspectFit, surface)?;
        let adaptive =
            base.presentation(PresentationMode::Adaptive, surface)?;
        backend.surface = Some(surface);
        backend.presentations = VecDeque::from([
            Ok(Some(expected)),
            Ok(Some(adaptive)),
            Ok(Some(expected)),
        ]);
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        check(&scene.visible_layout()?, &Some(expected))?;
        scene.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Stretch,
            ..DisplaySettings::default()
        })?;
        scene.apply_display_settings(DisplaySettings::default())?;
        check(&scene.last_presented_layout(), &None)?;
        check(&scene.visible_layout()?, &None)?;
        scene.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Adaptive,
            ..DisplaySettings::default()
        })?;
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        check(&scene.visible_layout()?, &Some(adaptive))?;
        check(&scene.fit_adaptive_width(maximum), &Ok(maximum))?;
        check(&scene.last_presented_layout(), &None)?;
        scene.apply_display_settings(DisplaySettings::default())?;
        check(&scene.fit_adaptive_width(maximum), &Ok(base))?;
        check(&scene.visible_layout()?, &None)?;
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        check(&scene.visible_layout()?, &Some(expected))?;
        Ok(())
    }

    #[test]
    fn failed_geometry_transitions_invalidate_presented_layout()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let width = NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN);
        let maximum =
            FrameLayout::new(width, base.height(), NonZeroUsize::MAX)?;
        let surface = Dimensions::new(width, base.height());
        let expected =
            base.presentation(PresentationMode::Adaptive, surface)?;
        backend.surface = Some(surface);
        backend.mode = PresentationMode::Adaptive;
        backend.settings_outcome = Err(RenderError::Unsupported);
        backend.resize_outcome = Err(RenderError::Unsupported);
        backend.presentations =
            VecDeque::from([Ok(Some(expected)), Ok(Some(expected))]);
        let mut scene = SceneSession::new(&mut backend, base, limits())?;
        let settings = DisplaySettings {
            presentation: PresentationMode::Adaptive,
            ..DisplaySettings::default()
        };
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        check(
            &scene.apply_display_settings(settings),
            &Err(RenderError::Unsupported),
        )?;
        check(&scene.visible_layout()?, &None)?;
        check(&scene.present([0; 3], &[]), &Ok(true))?;
        check(
            &scene.fit_adaptive_width(maximum),
            &Err(RenderError::Unsupported),
        )?;
        check(&scene.visible_layout()?, &None)?;
        Ok(())
    }

    #[test]
    fn observed_stale_geometry_cannot_reappear_without_a_receipt()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let surface = Dimensions::new(base.width(), base.height());
        let expected =
            base.presentation(PresentationMode::AspectFit, surface)?;
        let presentation = PresentationAdmission::default();
        let visible = |adapter: &TestScene| {
            SceneView {
                backend: adapter,
                presentation: &presentation,
            }
            .visible_layout()
        };
        backend.surface = Some(surface);
        let changed = Dimensions::new(
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            base.height(),
        );
        for (unavailable, expected_observation) in [
            (None, Ok(None)),
            (Some(changed), Ok(None)),
            (
                Some(Dimensions::new(NonZeroU32::MAX, base.height())),
                Err(RenderError::InvalidDimensions),
            ),
        ] {
            let admitted = presentation.complete(
                Some(expected),
                base,
                PresentationMode::AspectFit,
            )?;
            check(&admitted, &true)?;
            check(&visible(&backend)?, &Some(expected))?;
            backend.surface = unavailable;
            check(&visible(&backend), &expected_observation)?;
            check(&presentation.last(), &None)?;
            backend.surface = Some(surface);
            let queries = backend.surface_queries.get();
            check(&visible(&backend)?, &None)?;
            check(&backend.surface_queries.get(), &queries)?;
        }
        Ok(())
    }

    #[test]
    fn visible_geometry_rejects_stale_or_unavailable_drawables()
    -> Result<(), RenderError> {
        let mut backend = TestScene::new(0)?;
        let base = backend.layout;
        let width = NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let surface = Dimensions::new(width, height);
        let expected =
            base.presentation(PresentationMode::AspectFit, surface)?;
        let visible = |adapter: &TestScene| {
            let presentation = PresentationAdmission::default();
            let admitted = presentation.complete(
                Some(expected),
                base,
                PresentationMode::AspectFit,
            )?;
            check(&admitted, &true)?;
            SceneView {
                backend: adapter,
                presentation: &presentation,
            }
            .visible_layout()
        };
        backend.surface = Some(surface);
        check(&visible(&backend), &Ok(Some(expected)))?;
        backend.mode = PresentationMode::Stretch;
        check(&visible(&backend), &Ok(None))?;
        backend.mode = PresentationMode::AspectFit;
        backend.layout = FrameLayout::new(width, height, NonZeroUsize::MAX)?;
        check(&visible(&backend), &Ok(None))?;
        backend.layout = base;
        backend.surface = Some(Dimensions::new(height, width));
        check(&visible(&backend), &Ok(None))?;
        backend.surface = None;
        check(&visible(&backend), &Ok(None))?;
        backend.surface = Some(Dimensions::new(NonZeroU32::MAX, height));
        check(&visible(&backend), &Err(RenderError::InvalidDimensions))?;
        backend.drawable = false;
        let queries = backend.surface_queries.get();
        check(&visible(&backend), &Ok(None))?;
        check(&backend.surface_queries.get(), &queries)?;
        Ok(())
    }
}
