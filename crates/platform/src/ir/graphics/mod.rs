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
//   - Portable window, frame and borrowed or resident scene capability.
// - Must-Not:
//   - Select native APIs, leak native handles or advance simulation.
// - Allows:
//   - Inputs: Admitted dimensions, display settings and RGBA frames.
//   - Outputs: Typed render outcomes and portable input observations.
//   - Side effects: Delegated window creation and presentation.
// - Split-When:
//   - A rendering capability requires independent scene state.
// - Merge-When:
//   - Existing portable frame admission covers the same responsibility.
// - Summary:
//   - A host-neutral window with private frame and scene adapter ports.
// - Description:
//   - Keeps textures and event queues scoped to the owning window.
// - Usage:
//   - Open once and reuse a bounded frame or scene session.
// - Defaults:
//   - Explicit failure when accelerated rendering is unavailable.

//! Portable window and frame presentation sessions.

#[cfg(any(test, feature = "test-fixtures"))]
/// Explicit isolated host scene diagnostics.
pub mod fixtures;
/// Host animation frame scheduling without a blocking render loop.
pub mod frame_loop;
/// Ordered scene presentation and portable geometry observations.
pub mod scene_renderer;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{DisplaySettings, PresentationMode, WindowMode};

use self::scene_renderer::{SceneBackend, SceneSession};
use super::frame::{AdmittedRgba, FrameLayout, RenderError, validate_extent};
use super::hardware::InputSample;
use super::input::{InputPort, InputStartup};
use super::scene::SceneLimits;
use super::security::frame::validate_window_mode;

pub(crate) type FrameRun<'run> =
    dyn FnMut(&mut dyn FrameBackend) -> Result<(), RenderError> + 'run;
pub(crate) type ResidentScenePort = Box<dyn SceneBackend>;
pub(crate) type SceneRun<'run> =
    dyn FnMut(&mut dyn SceneBackend) -> Result<(), RenderError> + 'run;

pub(crate) type WindowPort = Box<dyn WindowBackend>;

/// Private window adapter for scoped frame and scene resources.
pub(crate) trait WindowBackend: Debug {
    fn apply_window_mode(
        &mut self,
        mode: WindowMode,
    ) -> Result<WindowMode, RenderError>;
    fn input(&self, limit: NonZeroUsize) -> Result<InputPort, RenderError>;
    /// Consume the window into an independently owned scene when supported.
    /// Borrowing-only adapters report unsupported.
    fn into_scene(
        self: Box<Self>,
        _layout: FrameLayout,
        _mode: PresentationMode,
    ) -> Result<ResidentScenePort, RenderError>;
    fn renderer_name(&self) -> String;
    fn with_frame(
        &mut self,
        layout: FrameLayout,
        mode: PresentationMode,
        run: &mut FrameRun<'_>,
    ) -> Result<(), RenderError>;
    fn with_scene(
        &mut self,
        layout: FrameLayout,
        mode: PresentationMode,
        run: &mut SceneRun<'_>,
    ) -> Result<(), RenderError>;
}

/// Platform selection is private and cannot leak a renderer or native window.
pub(crate) trait WindowFactory {
    const SUPPORTED: bool;

    fn open(
        title: &str,
        width: NonZeroU32,
        height: NonZeroU32,
        hidden: bool,
    ) -> Result<WindowPort, RenderError>;
}

/// A window and its selected accelerated rendering capability.
pub struct NativeWindow {
    backend: WindowPort,
    mode: PresentationMode,
}

impl Debug for NativeWindow {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWindow").finish_non_exhaustive()
    }
}

impl NativeWindow {
    /// Apply display settings before starting a rendering session.
    /// # Errors
    /// Reports an unsupported or rejected display transition.
    pub fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError> {
        let observed = self.backend.apply_window_mode(settings.window)?;
        validate_window_mode(settings.window, observed)?;
        self.mode = settings.presentation;
        Ok(())
    }

    /// Open input for this window with an explicit controller capacity.
    /// # Errors
    /// Reports unavailable input, enumeration or device opening.
    pub fn input(
        &self,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        InputStartup::new(self.backend.input(limit)?, limit)
    }

    /// Consume this window into a scene whose resources survive host callbacks.
    /// # Errors
    /// Reports unsupported ownership, allocation or inconsistent geometry.
    pub fn into_scene(
        self,
        layout: FrameLayout,
        limits: SceneLimits,
    ) -> Result<SceneSession<'static, 'static>, RenderError> {
        let backend = self.backend.into_scene(layout, self.mode)?;
        SceneSession::resident(backend, layout, limits, self.mode)
    }

    /// Open a resizable, accelerated window, optionally hidden for diagnostics.
    /// # Errors
    /// Rejects invalid dimensions or an unavailable renderer.
    pub fn open(
        title: &str,
        width: NonZeroU32,
        height: NonZeroU32,
        hidden: bool,
    ) -> Result<Self, RenderError> {
        validate_extent(width, height)?;
        let backend =
            <() as WindowFactory>::open(title, width, height, hidden)?;
        Ok(Self {
            backend,
            mode: PresentationMode::default(),
        })
    }

    /// Diagnostic name of the selected renderer.
    #[must_use]
    pub fn renderer_name(&self) -> String {
        self.backend.renderer_name()
    }

    /// Set a presentation policy without changing logical design dimensions.
    pub const fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.mode = mode;
    }

    /// Whether the selected host implements window presentation.
    /// Actual device availability is determined by opening the window.
    #[must_use]
    pub const fn supported() -> bool {
        <() as WindowFactory>::SUPPORTED
    }

    /// Reuse a frame allocation for the duration of the callback.
    /// # Errors
    /// Reports an unavailable session, failed presentation or callback error.
    pub fn with_frame<Output, Run>(
        &mut self,
        layout: FrameLayout,
        run: Run,
    ) -> Result<Output, RenderError>
    where
        Run: FnOnce(&mut FrameSession<'_, '_>) -> Result<Output, RenderError>,
    {
        let mode = self.mode;
        let mut once = OnceRun::new(run);
        self.backend.with_frame(layout, mode, &mut |backend| {
            once.call(|callback| {
                let mut session = FrameSession::new(backend, layout, mode)?;
                callback(&mut session)
            })
        })?;
        once.finish()
    }

    /// Reuse a bounded ordered scene for the duration of the callback.
    /// # Errors
    /// Reports unavailable scene resources or callback failure.
    pub fn with_scene<Output, Run>(
        &mut self,
        layout: FrameLayout,
        limits: SceneLimits,
        run: Run,
    ) -> Result<Output, RenderError>
    where
        Run: FnOnce(&mut SceneSession<'_, '_>) -> Result<Output, RenderError>,
    {
        let mode = &mut self.mode;
        let mut once = OnceRun::new(run);
        self.backend.with_scene(layout, *mode, &mut |backend| {
            once.call(|callback| {
                let mut session =
                    SceneSession::new(backend, layout, limits, *mode)?;
                let result = callback(&mut session);
                *mode = session.mode();
                result
            })
        })?;
        once.finish()
    }
}

struct OnceRun<Run, Output> {
    outcome: Option<Result<Output, RenderError>>,
    run: Option<Run>,
}

impl<Run, Output> OnceRun<Run, Output> {
    fn call(
        &mut self,
        invoke: impl FnOnce(Run) -> Result<Output, RenderError>,
    ) -> Result<(), RenderError> {
        let outcome =
            self.run.take().ok_or_else(callback_error).and_then(invoke);
        let status = outcome.as_ref().map(|_| ()).map_err(Clone::clone);
        self.outcome = Some(outcome);
        status
    }

    fn finish(self) -> Result<Output, RenderError> {
        self.outcome.unwrap_or_else(|| Err(callback_error()))
    }

    const fn new(run: Run) -> Self {
        Self {
            outcome: None,
            run: Some(run),
        }
    }
}

/// Private presentation port scoped to the owning window.
pub(crate) trait FrameBackend: Debug {
    fn layout(&self) -> FrameLayout;
    fn poll_event(&mut self) -> Option<InputSample>;
    fn present(&mut self, frame: AdmittedRgba<'_>)
    -> Result<bool, RenderError>;
    fn presentation_mode(&self) -> PresentationMode;
}

/// Persistent frame resources borrowed for one rendering session.
pub struct FrameSession<'session, 'backend> {
    backend: &'session mut (dyn FrameBackend + 'backend),
    layout: FrameLayout,
    mode: PresentationMode,
}

impl Debug for FrameSession<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSession")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl<'session, 'backend> FrameSession<'session, 'backend> {
    pub(crate) fn new(
        backend: &'session mut (dyn FrameBackend + 'backend),
        layout: FrameLayout,
        mode: PresentationMode,
    ) -> Result<Self, RenderError> {
        if backend.layout() != layout || backend.presentation_mode() != mode {
            return Err(RenderError::InvalidDimensions);
        }
        Ok(Self { backend, layout, mode })
    }

    /// Poll a normalized observation, without exposing a native event type.
    #[must_use]
    pub fn poll_event(&mut self) -> Option<InputSample> {
        self.backend.poll_event()
    }

    /// Present one complete logical frame. False means no drawable surface.
    /// # Errors
    /// Rejects incorrect byte extent or backend presentation failure.
    pub fn present(&mut self, pixels: &[u8]) -> Result<bool, RenderError> {
        let admitted = self.layout.admit(pixels)?;
        if self.backend.layout() != self.layout
            || self.backend.presentation_mode() != self.mode
        {
            return Err(RenderError::InvalidDimensions);
        }
        let result = self.backend.present(admitted);
        if self.backend.layout() != self.layout
            || self.backend.presentation_mode() != self.mode
        {
            return Err(RenderError::InvalidDimensions);
        }
        result
    }
}

fn callback_error() -> RenderError {
    RenderError::Backend(String::from("render session must run exactly once"))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::rc::Rc;

    use super::{
        AdmittedRgba, DisplaySettings, FrameBackend, FrameLayout, FrameRun,
        FrameSession, InputPort, InputSample, NativeWindow, OnceRun,
        PresentationMode, RenderError, SceneRun, WindowBackend, WindowMode,
        callback_error,
    };
    use crate::ir::check;

    #[derive(Debug)]
    struct TestFrame {
        layout: FrameLayout,
        layout_after_poll: Option<FrameLayout>,
        layout_after_present: Option<FrameLayout>,
        mode: PresentationMode,
        mode_after_poll: Option<PresentationMode>,
        mode_after_present: Option<PresentationMode>,
        presented: Vec<u8>,
    }

    impl FrameBackend for TestFrame {
        fn layout(&self) -> FrameLayout {
            self.layout
        }

        fn poll_event(&mut self) -> Option<InputSample> {
            if let Some(mode) = self.mode_after_poll.take() {
                self.mode = mode;
            }
            if let Some(layout) = self.layout_after_poll.take() {
                self.layout = layout;
            }
            None
        }

        fn present(
            &mut self,
            frame: AdmittedRgba<'_>,
        ) -> Result<bool, RenderError> {
            self.presented = frame.pixels().to_vec();
            if let Some(mode) = self.mode_after_present.take() {
                self.mode = mode;
            }
            if let Some(layout) = self.layout_after_present.take() {
                self.layout = layout;
            }
            Ok(true)
        }

        fn presentation_mode(&self) -> PresentationMode {
            self.mode
        }
    }

    #[derive(Debug)]
    struct TestWindow {
        frame_attempts: Vec<PresentationMode>,
        observed: Rc<Cell<PresentationMode>>,
        window_mode: WindowMode,
    }

    impl WindowBackend for TestWindow {
        fn apply_window_mode(
            &mut self,
            mode: WindowMode,
        ) -> Result<WindowMode, RenderError> {
            if mode == WindowMode::Fullscreen {
                Err(RenderError::Unsupported)
            } else {
                Ok(self.window_mode)
            }
        }

        fn input(
            &self,
            _limit: NonZeroUsize,
        ) -> Result<InputPort, RenderError> {
            Err(RenderError::Unsupported)
        }

        fn into_scene(
            self: Box<Self>,
            _layout: FrameLayout,
            _mode: PresentationMode,
        ) -> Result<super::ResidentScenePort, RenderError> {
            Err(RenderError::Unsupported)
        }

        fn renderer_name(&self) -> String {
            String::from("test")
        }

        fn with_frame(
            &mut self,
            layout: FrameLayout,
            mode: PresentationMode,
            run: &mut FrameRun<'_>,
        ) -> Result<(), RenderError> {
            self.observed.set(mode);
            let mut frame = TestFrame {
                layout,
                layout_after_poll: None,
                layout_after_present: None,
                mode,
                mode_after_poll: None,
                mode_after_present: None,
                presented: Vec::new(),
            };
            if self.frame_attempts.is_empty() {
                return run(&mut frame);
            }
            for attempt in &self.frame_attempts {
                frame.mode = *attempt;
                let _ignored = run(&mut frame);
            }
            Ok(())
        }

        fn with_scene(
            &mut self,
            _layout: FrameLayout,
            mode: PresentationMode,
            _run: &mut SceneRun<'_>,
        ) -> Result<(), RenderError> {
            self.observed.set(mode);
            Err(RenderError::Unsupported)
        }
    }

    fn run_once<Output, Run, Invoke>(
        run: Run,
        invoke: Invoke,
    ) -> Result<Output, RenderError>
    where
        Invoke: FnOnce(&mut OnceRun<Run, Output>) -> Result<(), RenderError>,
    {
        let mut once = OnceRun::new(run);
        invoke(&mut once)?;
        once.finish()
    }

    #[test]
    fn window_dispatches_only_admitted_presentation_mode()
    -> Result<(), RenderError> {
        let observed = Rc::new(Cell::new(PresentationMode::Adaptive));
        let mut window = NativeWindow {
            backend: Box::new(TestWindow {
                frame_attempts: Vec::new(),
                observed: Rc::clone(&observed),
                window_mode: WindowMode::Windowed,
            }),
            mode: PresentationMode::default(),
        };
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        for mode in [PresentationMode::AspectFit, PresentationMode::Integer] {
            window.set_presentation_mode(mode);
            check(
                &window.with_frame(layout, |frame| frame.present(&[0; 4])),
                &Ok(true),
            )?;
            check(&observed.get(), &mode)?;
        }
        check(
            &window.apply_display_settings(DisplaySettings {
                presentation: PresentationMode::Fill,
                window: WindowMode::Fullscreen,
            }),
            &Err(RenderError::Unsupported),
        )?;
        window.with_frame(layout, |_| Ok(()))?;
        check(&observed.get(), &PresentationMode::Integer)?;
        window.apply_display_settings(DisplaySettings {
            presentation: PresentationMode::Stretch,
            window: WindowMode::Windowed,
        })?;
        let limits = super::SceneLimits {
            atlas_bytes: NonZeroUsize::MIN,
            atlases: NonZeroUsize::MIN,
            sprites: NonZeroUsize::MIN,
        };
        check(
            &window.with_scene(layout, limits, |_| Ok(())),
            &Err(RenderError::Unsupported),
        )?;
        check(&observed.get(), &PresentationMode::Stretch)?;
        Ok(())
    }

    #[test]
    fn unconfirmed_window_transition_retains_previous_presentation_policy()
    -> Result<(), RenderError> {
        let observed = Rc::new(Cell::new(PresentationMode::Adaptive));
        let mut window = NativeWindow {
            backend: Box::new(TestWindow {
                frame_attempts: Vec::new(),
                observed: Rc::clone(&observed),
                window_mode: WindowMode::Fullscreen,
            }),
            mode: PresentationMode::Integer,
        };
        check(
            &window.apply_display_settings(DisplaySettings {
                presentation: PresentationMode::Stretch,
                window: WindowMode::Windowed,
            }),
            &Err(RenderError::InvalidWindowMode),
        )?;
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        window.with_frame(layout, |_| Ok(()))?;
        check(&observed.get(), &PresentationMode::Integer)
    }

    #[test]
    fn frame_session_rejects_inconsistent_adapter_extent()
    -> Result<(), RenderError> {
        let size = NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN);
        let layout =
            FrameLayout::new(size, NonZeroU32::MIN, NonZeroUsize::MAX)?;
        let wrong = FrameLayout::new(NonZeroU32::MIN, size, NonZeroUsize::MAX)?;
        let mut frame = TestFrame {
            layout: wrong,
            layout_after_poll: None,
            layout_after_present: None,
            mode: PresentationMode::AspectFit,
            mode_after_poll: None,
            mode_after_present: None,
            presented: Vec::new(),
        };
        check(
            &FrameSession::new(&mut frame, layout, PresentationMode::AspectFit)
                .err(),
            &Some(RenderError::InvalidDimensions),
        )?;
        check(&frame.presented.is_empty(), &true)?;
        frame.layout = layout;
        check(
            &FrameSession::new(&mut frame, layout, PresentationMode::Stretch)
                .err(),
            &Some(RenderError::InvalidDimensions),
        )?;
        Ok(())
    }

    #[test]
    fn frame_geometry_changes_do_not_admit_presentation()
    -> Result<(), RenderError> {
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        let wider = FrameLayout::new(
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        let changed = Some(PresentationMode::Stretch);
        for (after_poll, after_present, mode_poll, mode_present) in [
            (Some(wider), None, None, None),
            (None, Some(wider), None, None),
            (None, None, changed, None),
            (None, None, None, changed),
        ] {
            let mut frame = TestFrame {
                layout,
                layout_after_poll: after_poll,
                layout_after_present: after_present,
                mode: PresentationMode::AspectFit,
                mode_after_poll: mode_poll,
                mode_after_present: mode_present,
                presented: Vec::new(),
            };
            {
                let mut session = FrameSession::new(
                    &mut frame,
                    layout,
                    PresentationMode::AspectFit,
                )?;
                check(&session.poll_event(), &None)?;
                check(
                    &session.present(&[0; 4]),
                    &Err(RenderError::InvalidDimensions),
                )?;
                check(
                    &session.present(&[1; 4]),
                    &Err(RenderError::InvalidDimensions),
                )?;
            }
            let expected: &[u8] = if after_poll.is_some() || mode_poll.is_some()
            {
                &[]
            } else {
                &[0; 4]
            };
            check(&frame.presented.as_slice(), &expected)?;
        }
        Ok(())
    }

    #[test]
    fn wrong_frame_length_never_reaches_adapter() -> Result<(), RenderError> {
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        let mut frame = TestFrame {
            layout,
            layout_after_poll: None,
            layout_after_present: None,
            mode: PresentationMode::AspectFit,
            mode_after_poll: None,
            mode_after_present: None,
            presented: Vec::new(),
        };
        {
            let mut session = FrameSession::new(
                &mut frame,
                layout,
                PresentationMode::AspectFit,
            )?;
            check(&session.present(&[0; 3]), &Err(RenderError::InvalidLength))?;
            check(&session.poll_event(), &None)?;
        }
        check(&frame.presented.is_empty(), &true)?;
        check(
            &FrameSession::new(
                &mut frame,
                layout,
                PresentationMode::AspectFit,
            )?
            .present(&[1, 2, 3, 4]),
            &Ok(true),
        )?;
        check(&frame.presented, &[1, 2, 3, 4])?;
        Ok(())
    }

    #[test]
    fn callback_attempts_include_rejected_session_construction()
    -> Result<(), RenderError> {
        let mode = PresentationMode::Integer;
        let wrong = PresentationMode::Stretch;
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        for (attempts, expected, calls) in [
            (vec![wrong], RenderError::InvalidDimensions, 0),
            (vec![wrong, mode], callback_error(), 0),
            (vec![mode, wrong], callback_error(), 1),
        ] {
            let mut window = NativeWindow {
                backend: Box::new(TestWindow {
                    frame_attempts: attempts,
                    observed: Rc::new(Cell::new(mode)),
                    window_mode: WindowMode::Windowed,
                }),
                mode,
            };
            let invoked = Cell::new(0u8);
            check(
                &window.with_frame(layout, |_| {
                    invoked.set(invoked.get().saturating_add(1));
                    Ok(())
                }),
                &Err(expected),
            )?;
            check(&invoked.get(), &calls)?;
        }
        Ok(())
    }

    #[test]
    fn session_requires_exactly_one_callback() {
        assert_eq!(
            run_once(|| Ok(42i32), |once| once.call(|run| run())),
            Ok(42i32)
        );
        assert_eq!(
            run_once::<i32, _, _>(|| Ok::<i32, RenderError>(42i32), |_| Ok(())),
            Err(callback_error())
        );
        assert_eq!(
            run_once(
                || Ok(42i32),
                |once| {
                    once.call(|run| run())?;
                    let _second = once.call(|run| run());
                    Ok(())
                }
            ),
            Err(callback_error())
        );
        assert_eq!(
            run_once(
                || Err::<(), _>(RenderError::InvalidLength),
                |once| {
                    let _failure = once.call(|run| run());
                    Ok(())
                }
            ),
            Err(RenderError::InvalidLength)
        );
    }

    #[test]
    fn invalid_window_extent_is_rejected_before_backend_selection() {
        assert!(matches!(
            NativeWindow::open("test", NonZeroU32::MAX, NonZeroU32::MIN, true),
            Err(RenderError::InvalidDimensions)
        ));
        if !NativeWindow::supported() {
            assert!(matches!(
                NativeWindow::open(
                    "test",
                    NonZeroU32::MIN,
                    NonZeroU32::MIN,
                    true
                ),
                Err(RenderError::Unsupported)
            ));
        }
    }
}
