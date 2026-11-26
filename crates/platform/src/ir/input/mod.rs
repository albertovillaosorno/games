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
//   - Portable input session, touch admission and feedback exchange.
// - Must-Not:
//   - Open native devices or expose operating-system event handles.
// - Allows:
//   - Inputs: Normalized observations and a presented layout query.
//   - Outputs: Admitted input events, feedback outcomes and clock samples.
//   - Side effects: Delegated device discovery and feedback only.
// - Split-When:
//   - A separate device capability needs independent session state.
// - Merge-When:
//   - Shared device admission covers the same observation semantics.
// - Summary:
//   - Host-neutral input session over a private adapter port.
// - Description:
//   - Invalidates held contacts when displayed geometry changes.
// - Usage:
//   - Open through a window capability and route admitted events.
// - Defaults:
//   - Touch is disabled until explicitly enabled.

//! Host-neutral input sessions and keyboard profile names.

use std::fmt::{self, Debug, Formatter};
use std::time::Duration;

use games::input::physical::KeyboardKey;
use games::presentation::PresentationLayout;

use super::frame::RenderError;
pub use super::hardware::HostEvent;
use super::hardware::{
    HapticCommand, HapticError, HapticSupport, Haptics, InputEvent,
    InputSample, InputTransition,
};
use super::security::haptics::FeedbackRequest;

pub(crate) type InputPort = Box<dyn InputBackend>;

/// Private device adapter port.
pub(crate) trait InputBackend: Debug {
    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn elapsed(&self) -> Duration;
    fn event_elapsed(&self, event: &InputSample) -> Option<Duration>;
    fn feedback_allowed(&self) -> bool;
    fn feedback_support(&self, device: u64) -> Option<HapticSupport>;
    fn submit_feedback(
        &mut self,
        request: FeedbackRequest,
    ) -> Result<(), HapticError>;
    fn translate(&mut self, event: &InputSample) -> Option<HostEvent>;
}

/// Platform-specific resolution of existing physical keyboard names.
pub(crate) trait InputNames {
    fn key_from_name(name: &str) -> Option<KeyboardKey>;
    fn key_name(key: KeyboardKey) -> &'static str;
}

/// Input session and its initial controller attachment observations.
#[derive(Debug)]
pub struct InputStartup {
    /// Initial attachments, in discovery order.
    pub attached: Vec<InputEvent>,
    /// Device session retaining admitted resources privately.
    pub input: NativeInput,
}

impl InputStartup {
    pub(crate) fn new(mut backend: InputPort) -> Result<Self, RenderError> {
        let attached = backend.discover_available()?;
        Ok(Self {
            attached,
            input: NativeInput {
                backend,
                touch_enabled: false,
                touch_geometry: TouchGeometry::default(),
            },
        })
    }
}

/// An input capability with private native device ownership.
pub struct NativeInput {
    backend: InputPort,
    touch_enabled: bool,
    touch_geometry: TouchGeometry,
}

impl Debug for NativeInput {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInput")
            .field("touch_enabled", &self.touch_enabled)
            .finish_non_exhaustive()
    }
}

impl NativeInput {
    /// Discover controllers only after draining the event queue.
    ///
    /// # Errors
    /// Reports failed device enumeration or opening.
    pub fn discover_after_drain(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
        self.backend.discover_after_drain()
    }

    /// Discover available controllers in adapter discovery order.
    ///
    /// # Errors
    /// Reports failed device enumeration or opening.
    pub fn discover_available(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
        self.backend.discover_available()
    }

    /// Monotonic elapsed time since input startup.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.backend.elapsed()
    }

    /// Enable normalized touch observations for this window session.
    pub const fn enable_touch(&mut self) {
        self.touch_enabled = true;
    }

    /// Map a sample to the session clock without inventing missing time.
    #[must_use]
    pub fn event_elapsed(&self, event: &InputSample) -> Option<Duration> {
        self.backend.event_elapsed(event)
    }

    /// Cancel existing contacts when the actually presented layout changes.
    ///
    /// When touch is disabled, do not query the renderer.
    /// # Errors
    /// Reports a failed presentation query.
    pub fn synchronize_touch<Observe>(
        &mut self,
        observe: Observe,
    ) -> Result<Option<Duration>, RenderError>
    where
        Observe: FnOnce() -> Result<Option<PresentationLayout>, RenderError>,
    {
        if !self.touch_enabled {
            return Ok(None);
        }
        let layout = observe()?;
        let elapsed = self.elapsed();
        Ok(self
            .touch_geometry
            .observe(layout, elapsed)
            .then_some(elapsed))
    }

    /// Admit one portable input sample through the owning device session.
    ///
    /// Discovery notifications and unknown controllers do not fabricate
    /// attachment events. Touch requires a current presented geometry and
    /// a timestamp at or after its observation before routing.
    #[must_use]
    pub fn translate(&mut self, event: &InputSample) -> Option<HostEvent> {
        if matches!(
            event.transition(),
            InputTransition::Event(HostEvent::Input(
                InputEvent::Contact { .. }
            ))
        ) && (!self.touch_enabled
            || !self.touch_geometry.accepts(self.event_elapsed(event)))
        {
            return None;
        }
        self.backend.translate(event)
    }
}

impl Haptics for NativeInput {
    fn submit(&mut self, command: HapticCommand) -> Result<(), HapticError> {
        let request = FeedbackRequest::new(
            command,
            self.backend.feedback_support(command.device),
            self.backend.feedback_allowed(),
        )?;
        self.backend.submit_feedback(request)
    }

    fn support(&self, device: u64) -> Option<HapticSupport> {
        self.backend.feedback_support(device)
    }
}

#[derive(Debug, Default)]
struct TouchGeometry {
    cutoff: Duration,
    layout: Option<PresentationLayout>,
}

impl TouchGeometry {
    fn accepts(&self, stamp: Option<Duration>) -> bool {
        self.layout.is_some()
            && stamp.is_some_and(|elapsed| elapsed >= self.cutoff)
    }

    fn observe(
        &mut self,
        layout: Option<PresentationLayout>,
        elapsed: Duration,
    ) -> bool {
        if self.layout == layout {
            return false;
        }
        let invalidated = self.layout.is_some();
        self.layout = layout;
        self.cutoff = elapsed;
        invalidated
    }
}

/// Resolve a legacy physical keyboard profile name.
#[must_use]
pub fn key_from_name(name: &str) -> Option<KeyboardKey> {
    <() as InputNames>::key_from_name(name)
}

/// Display a physical keyboard profile name.
#[must_use]
pub fn key_name(key: KeyboardKey) -> &'static str {
    <() as InputNames>::key_name(key)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::num::{NonZeroU16, NonZeroU32};
    use std::rc::Rc;
    use std::time::Duration;

    use games::presentation::{Dimensions, PresentationMode};

    use super::TouchGeometry;
    use crate::ir::check;

    #[derive(Debug, Default)]
    struct TestInput {
        allowed: Rc<Cell<bool>>,
        elapsed: Duration,
        feedback: Rc<Cell<Option<super::HapticCommand>>>,
    }

    impl super::InputBackend for TestInput {
        fn discover_after_drain(
            &mut self,
        ) -> Result<Vec<super::InputEvent>, super::RenderError> {
            Ok(Vec::new())
        }

        fn discover_available(
            &mut self,
        ) -> Result<Vec<super::InputEvent>, super::RenderError> {
            Ok(Vec::new())
        }

        fn elapsed(&self) -> Duration {
            self.elapsed
        }

        fn event_elapsed(
            &self,
            event: &super::InputSample,
        ) -> Option<Duration> {
            event.elapsed_since(Duration::ZERO)
        }

        fn feedback_allowed(&self) -> bool {
            self.allowed.get()
        }

        fn feedback_support(
            &self,
            device: u64,
        ) -> Option<super::HapticSupport> {
            match device {
                1 => Some(super::HapticSupport::SingleMotor),
                2 | 4 => Some(super::HapticSupport::DualMotor),
                3 => Some(super::HapticSupport::Unsupported),
                _ => None,
            }
        }

        fn submit_feedback(
            &mut self,
            request: super::FeedbackRequest,
        ) -> Result<(), super::HapticError> {
            let command = request.command();
            self.feedback.set(Some(command));
            if command.device == 4 {
                Err(super::HapticError::Unavailable)
            } else {
                Ok(())
            }
        }

        fn translate(
            &mut self,
            event: &super::InputSample,
        ) -> Option<super::HostEvent> {
            if let super::InputTransition::Event(routed) = event.transition() {
                Some(routed)
            } else {
                None
            }
        }
    }

    #[test]
    fn feedback_admission_precedes_backend_dispatch()
    -> Result<(), super::RenderError> {
        use games::capability::HapticEffect;

        use super::{HapticCommand, HapticError, Haptics as _};

        let backend = TestInput::default();
        let feedback = Rc::clone(&backend.feedback);
        let allowed = Rc::clone(&backend.allowed);
        let mut input = super::InputStartup::new(Box::new(backend))?.input;
        let effect = HapticEffect::Rumble {
            low: 7,
            high: u16::MAX,
            milliseconds: NonZeroU16::MAX,
        };
        for (device, expected) in [
            (0, HapticError::Disconnected),
            (3, HapticError::Unsupported),
            (1, HapticError::Unavailable),
        ] {
            check(
                &input.submit(HapticCommand { device, effect }),
                &Err(expected),
            )?;
            check(&feedback.get(), &None)?;
        }
        let stop = HapticCommand {
            device: 1,
            effect: HapticEffect::Stop,
        };
        check(&input.submit(stop), &Ok(()))?;
        check(&feedback.get(), &Some(stop))?;
        allowed.set(true);
        for (device, admitted, result) in [
            (1, effect.single_motor(), Ok(())),
            (2, effect, Ok(())),
            (4, effect, Err(HapticError::Unavailable)),
        ] {
            check(&input.submit(HapticCommand { device, effect }), &result)?;
            check(
                &feedback.get(),
                &Some(HapticCommand { device, effect: admitted }),
            )?;
        }
        Ok(())
    }

    #[test]
    fn session_does_not_route_disabled_or_invalidated_touch()
    -> Result<(), super::RenderError> {
        use games::input::touch::{ContactId, ContactPhase, TouchPoint};

        let clock = Duration::from_millis(20);
        let mut session = super::InputStartup::new(Box::new(TestInput {
            elapsed: clock,
            ..TestInput::default()
        }))?
        .input;
        let contact = super::HostEvent::Input(super::InputEvent::Contact {
            id: ContactId { device: 1, finger: 1 },
            phase: ContactPhase::Down(TouchPoint { x: 10, y: 10 }),
        });
        let sample = |stamp| {
            super::InputSample::new(
                stamp,
                super::InputTransition::Event(contact),
                None,
            )
        };
        check(
            &session.synchronize_touch(|| Err(super::RenderError::Unsupported)),
            &Ok(None),
        )?;
        check(&session.translate(&sample(Some(clock))), &None)?;
        session.enable_touch();
        check(&session.translate(&sample(Some(clock))), &None)?;
        let size = Dimensions::new(NonZeroU32::MIN, NonZeroU32::MIN);
        let geometry = PresentationMode::AspectFit.layout(size, size);
        check(&session.synchronize_touch(|| Ok(Some(geometry)))?, &None)?;
        check(&session.translate(&sample(None)), &None)?;
        check(
            &session.translate(&sample(Some(Duration::from_millis(19)))),
            &None,
        )?;
        check(&session.translate(&sample(Some(clock))), &Some(contact))?;
        check(&session.synchronize_touch(|| Ok(None))?, &Some(clock))?;
        check(&session.translate(&sample(Some(clock))), &None)?;
        check(
            &session.translate(&sample(Some(Duration::from_millis(19)))),
            &None,
        )?;
        check(&session.translate(&sample(None)), &None)?;
        check(&session.synchronize_touch(|| Ok(Some(geometry)))?, &None)?;
        check(&session.translate(&sample(Some(clock))), &Some(contact))?;
        Ok(())
    }

    #[test]
    fn touch_geometry_cancels_once_and_rejects_stale_samples() {
        let surface = Dimensions::new(
            NonZeroU32::new(96).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(48).unwrap_or(NonZeroU32::MIN),
        );
        let design = Dimensions::new(NonZeroU32::MIN, NonZeroU32::MIN);
        let first = Some(PresentationMode::AspectFit.layout(surface, design));
        let second = Some(PresentationMode::Stretch.layout(surface, design));
        let mut geometry = TouchGeometry::default();
        let at = Duration::from_millis;
        for (layout, tick, cancelled, accepted) in [
            (None, 1, false, [false; 4]),
            (first, 2, false, [false, false, true, true]),
            (first, 3, false, [false, false, true, true]),
            (second, 4, true, [false, false, true, true]),
            (second, 5, false, [false, false, true, true]),
            (None, 6, true, [false; 4]),
            (None, 7, false, [false; 4]),
            (first, 8, false, [false, false, false, true]),
        ] {
            assert_eq!(geometry.observe(layout, at(tick)), cancelled);
            let stamps = [None, Some(at(1)), Some(at(4)), Some(at(8))];
            assert_eq!(stamps.map(|stamp| geometry.accepts(stamp)), accepted);
        }
    }
}
