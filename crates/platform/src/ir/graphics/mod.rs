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
//   - Portable window, frame and callback-scoped rendering capability.
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

/// Ordered scene presentation and portable geometry observations.
pub mod scene_renderer;

use std::fmt::{self, Debug, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};

use games::presentation::{DisplaySettings, PresentationMode};

use self::scene_renderer::{SceneBackend, SceneSession};
use super::frame::{FrameLayout, RenderError, validate_extent};
use super::hardware::InputSample;
use super::input::{InputPort, InputStartup};
use super::scene::SceneLimits;

pub(crate) type FrameRun<'run> =
    dyn FnMut(&mut dyn FrameBackend) -> Result<(), RenderError> + 'run;
pub(crate) type SceneRun<'run> =
    dyn FnMut(&mut dyn SceneBackend) -> Result<(), RenderError> + 'run;

pub(crate) type WindowPort = Box<dyn WindowBackend>;

/// Private window adapter. Frame and scene callbacks are scoped to this port.
pub(crate) trait WindowBackend: Debug {
    fn apply_display_settings(
        &mut self,
        settings: DisplaySettings,
    ) -> Result<(), RenderError>;
    fn input(&self, limit: NonZeroUsize) -> Result<InputPort, RenderError>;
    fn renderer_name(&self) -> String;
    fn set_presentation_mode(&mut self, mode: PresentationMode);
    fn with_frame(
        &mut self,
        layout: FrameLayout,
        run: &mut FrameRun<'_>,
    ) -> Result<(), RenderError>;
    fn with_scene(
        &mut self,
        layout: FrameLayout,
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
        self.backend.apply_display_settings(settings)
    }

    /// Open input for this window with an explicit controller capacity.
    /// # Errors
    /// Reports unavailable input, enumeration or device opening.
    pub fn input(
        &self,
        limit: NonZeroUsize,
    ) -> Result<InputStartup, RenderError> {
        InputStartup::new(self.backend.input(limit)?)
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
        Ok(Self { backend })
    }

    /// Diagnostic name of the selected renderer.
    #[must_use]
    pub fn renderer_name(&self) -> String {
        self.backend.renderer_name()
    }

    /// Set a presentation policy without changing logical design dimensions.
    pub fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.backend.set_presentation_mode(mode);
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
        run_once(run, |once| {
            self.backend.with_frame(layout, &mut |backend| {
                let mut session = FrameSession::new(backend, layout);
                once.call(|callback| callback(&mut session))
            })
        })
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
        run_once(run, |once| {
            self.backend.with_scene(layout, &mut |backend| {
                let mut session = SceneSession::new(backend, layout, limits)?;
                once.call(|callback| callback(&mut session))
            })
        })
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
}

/// Private presentation port scoped to the owning window.
pub(crate) trait FrameBackend: Debug {
    fn poll_event(&mut self) -> Option<InputSample>;
    fn present(&mut self, pixels: &[u8]) -> Result<bool, RenderError>;
}

/// Persistent frame resources borrowed for one rendering session.
pub struct FrameSession<'session, 'backend> {
    backend: &'session mut (dyn FrameBackend + 'backend),
    layout: FrameLayout,
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
    ) -> Self {
        Self { backend, layout }
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
        if pixels.len() != self.layout.bytes() {
            return Err(RenderError::InvalidLength);
        }
        self.backend.present(pixels)
    }
}

fn callback_error() -> RenderError {
    RenderError::Backend(String::from("render session must run exactly once"))
}

fn run_once<Output, Run>(
    run: Run,
    invoke: impl FnOnce(&mut OnceRun<Run, Output>) -> Result<(), RenderError>,
) -> Result<Output, RenderError> {
    let mut once = OnceRun {
        outcome: None,
        run: Some(run),
    };
    invoke(&mut once)?;
    once.outcome.unwrap_or_else(|| Err(callback_error()))
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{
        FrameBackend, FrameLayout, FrameSession, InputSample, NativeWindow,
        RenderError, callback_error, run_once,
    };
    use crate::ir::check;

    #[derive(Debug, Default)]
    struct TestFrame {
        presented: bool,
    }

    impl FrameBackend for TestFrame {
        fn poll_event(&mut self) -> Option<InputSample> {
            None
        }

        fn present(&mut self, _pixels: &[u8]) -> Result<bool, RenderError> {
            self.presented = true;
            Ok(true)
        }
    }

    #[test]
    fn wrong_frame_length_never_reaches_adapter() -> Result<(), RenderError> {
        let layout = FrameLayout::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroUsize::MAX,
        )?;
        let mut frame = TestFrame::default();
        {
            let mut session = FrameSession::new(&mut frame, layout);
            check(&session.present(&[0; 3]), &Err(RenderError::InvalidLength))?;
            check(&session.poll_event(), &None)?;
        }
        check(&frame.presented, &false)?;
        check(
            &FrameSession::new(&mut frame, layout).present(&[0; 4]),
            &Ok(true),
        )?;
        check(&frame.presented, &true)?;
        Ok(())
    }

    #[test]
    fn session_requires_exactly_one_callback() {
        assert_eq!(
            run_once(|| Ok(42i32), |once| once.call(|run| run())),
            Ok(42i32)
        );
        assert_eq!(
            run_once::<i32, _>(|| Ok::<i32, RenderError>(42i32), |_| Ok(())),
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
