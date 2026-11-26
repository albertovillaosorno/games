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
use std::num::NonZeroUsize;
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
use super::security::input::{ControllerAdmission, InputClock};

pub(crate) type InputPort = Box<dyn InputBackend>;

/// Private device adapter port.
pub(crate) trait InputBackend: Debug {
    fn clock(&self) -> Duration;
    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn feedback_allowed(&self) -> bool;
    fn feedback_support(&self, device: u64) -> Option<HapticSupport>;
    fn submit_feedback(
        &mut self,
        request: FeedbackRequest,
    ) -> Result<(), HapticError>;
    fn translate(&mut self, event: &InputSample) -> Option<HostEvent>;
    fn window_scope(&self) -> u64;
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
    pub(crate) fn new(
        mut backend: InputPort,
        limit: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        let clock = InputClock::new(backend.clock(), backend.window_scope());
        let mut controllers = ControllerAdmission::new(limit);
        let attached = controllers.admit(backend.discover_available()?)?;
        Ok(Self {
            attached,
            input: NativeInput {
                backend,
                clock,
                controllers,
                touch_enabled: false,
                touch_geometry: TouchGeometry::default(),
            },
        })
    }
}

/// An input capability with private native device ownership.
pub struct NativeInput {
    backend: InputPort,
    clock: InputClock,
    controllers: ControllerAdmission,
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
        self.controllers.admit(self.backend.discover_after_drain()?)
    }

    /// Discover available controllers in adapter discovery order.
    ///
    /// # Errors
    /// Reports failed device enumeration or opening.
    pub fn discover_available(
        &mut self,
    ) -> Result<Vec<InputEvent>, RenderError> {
        self.controllers.admit(self.backend.discover_available()?)
    }

    /// Monotonic elapsed time since input startup.
    ///
    /// # Errors
    /// Rejects a reversed adapter clock without advancing its observation.
    pub fn elapsed(&self) -> Result<Duration, RenderError> {
        self.clock.elapsed(self.backend.clock())
    }

    /// Enable normalized touch observations for this window session.
    pub const fn enable_touch(&mut self) {
        self.touch_enabled = true;
    }

    /// Map a sample to the session clock without inventing missing time.
    ///
    /// Foreign windows, reversed clocks and future samples are rejected.
    #[must_use]
    pub fn event_elapsed(&self, event: &InputSample) -> Option<Duration> {
        if !self.clock.matches(event) {
            return None;
        }
        self.clock.event_elapsed(event, self.elapsed().ok()?)
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
        let elapsed = self.elapsed()?;
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
        if !self.clock.matches(event) {
            return None;
        }
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
        self.controllers
            .route(event.transition(), self.backend.translate(event)?)
    }
}

impl Haptics for NativeInput {
    fn submit(&mut self, command: HapticCommand) -> Result<(), HapticError> {
        let request = FeedbackRequest::new(
            command,
            self.support(command.device),
            self.backend.feedback_allowed(),
        )?;
        self.backend.submit_feedback(request)
    }

    fn support(&self, device: u64) -> Option<HapticSupport> {
        if !self.controllers.contains(device) {
            return None;
        }
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
    use std::mem;
    use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};
    use std::rc::Rc;
    use std::time::Duration;

    use games::presentation::{Dimensions, PresentationMode};

    use super::TouchGeometry;
    use crate::ir::check;

    #[derive(Debug, Default)]
    struct TestInput {
        after_drain: Vec<super::InputEvent>,
        allowed: Rc<Cell<bool>>,
        attachments: Vec<super::InputEvent>,
        clock: Rc<Cell<Duration>>,
        feedback: Rc<Cell<Option<super::HapticCommand>>>,
        scope: u64,
    }

    impl super::InputBackend for TestInput {
        fn clock(&self) -> Duration {
            self.clock.get()
        }

        fn discover_after_drain(
            &mut self,
        ) -> Result<Vec<super::InputEvent>, super::RenderError> {
            Ok(mem::take(&mut self.after_drain))
        }

        fn discover_available(
            &mut self,
        ) -> Result<Vec<super::InputEvent>, super::RenderError> {
            Ok(mem::take(&mut self.attachments))
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
            match event.transition() {
                super::InputTransition::Event(routed) => Some(routed),
                super::InputTransition::Disconnected(device) => {
                    Some(super::HostEvent::Input(super::InputEvent::Detached {
                        device,
                    }))
                },
                super::InputTransition::DiscoveryHint
                | super::InputTransition::FocusGained
                | super::InputTransition::Ignored => None,
            }
        }

        fn window_scope(&self) -> u64 {
            self.scope
        }
    }

    #[test]
    fn feedback_admission_precedes_backend_dispatch()
    -> Result<(), super::RenderError> {
        use games::capability::HapticEffect;

        use super::{HapticCommand, HapticError, Haptics as _};

        let backend = TestInput {
            attachments: (1..=4)
                .map(|device| super::InputEvent::Attached { device })
                .collect(),
            ..TestInput::default()
        };
        let feedback = Rc::clone(&backend.feedback);
        let allowed = Rc::clone(&backend.allowed);
        let mut input = super::InputStartup::new(
            Box::new(backend),
            NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
        )?
        .input;
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
        let observation = Rc::new(Cell::new(Duration::ZERO));
        let mut session = super::InputStartup::new(
            Box::new(TestInput {
                clock: Rc::clone(&observation),
                ..TestInput::default()
            }),
            NonZeroUsize::MIN,
        )?
        .input;
        observation.set(clock);
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
            &session.translate(&sample(Some(Duration::from_millis(21)))),
            &None,
        )?;
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
    fn session_admits_scoped_elapsed_observations()
    -> Result<(), super::RenderError> {
        use super::{HostEvent, InputSample, InputTransition, RenderError};

        let ms = Duration::from_millis;
        let clock = Rc::new(Cell::new(ms(100)));
        let event = HostEvent::Input(super::InputEvent::FocusLost);
        let sample = |stamp, scope| {
            InputSample::new(stamp, InputTransition::Event(event), scope)
        };
        let mut input = super::InputStartup::new(
            Box::new(TestInput {
                clock: Rc::clone(&clock),
                scope: 7,
                ..TestInput::default()
            }),
            NonZeroUsize::MIN,
        )?
        .input;
        check(&input.elapsed(), &Ok(Duration::ZERO))?;
        clock.set(ms(120));
        check(&input.elapsed(), &Ok(ms(20)))?;
        for (stamp, scope, expected) in [
            (Some(ms(90)), Some(7), Some(Duration::ZERO)),
            (Some(ms(100)), None, Some(Duration::ZERO)),
            (Some(ms(110)), Some(7), Some(ms(10))),
            (Some(ms(120)), None, Some(ms(20))),
            (Some(ms(121)), Some(7), None),
            (Some(ms(110)), Some(8), None),
            (None, Some(7), None),
        ] {
            check(&input.event_elapsed(&sample(stamp, scope)), &expected)?;
        }
        check(&input.translate(&sample(Some(ms(110)), Some(8))), &None)?;
        check(
            &input.translate(&sample(Some(ms(110)), Some(7))),
            &Some(event),
        )?;
        clock.set(ms(119));
        check(&input.elapsed(), &Err(RenderError::InvalidTimestamp))?;
        check(&input.event_elapsed(&sample(Some(ms(110)), None)), &None)?;
        clock.set(ms(120));
        check(&input.elapsed(), &Ok(ms(20)))?;
        clock.set(ms(121));
        check(&input.elapsed(), &Ok(ms(21)))?;
        Ok(())
    }

    #[test]
    fn controller_routes_and_feedback_require_discovered_identity()
    -> Result<(), super::RenderError> {
        use super::{
            Haptics as _, HostEvent, InputEvent, InputSample, InputTransition,
        };

        let attachment = |device| InputEvent::Attached { device };
        let button = |device| {
            HostEvent::Input(InputEvent::Button {
                device,
                code: 1,
                down: true,
            })
        };
        let sample = |transition| InputSample::new(None, transition, None);
        let mut startup = super::InputStartup::new(
            Box::new(TestInput {
                after_drain: vec![attachment(2)],
                attachments: vec![attachment(1)],
                ..TestInput::default()
            }),
            NonZeroUsize::MIN,
        )?;
        check(&startup.attached, &vec![attachment(1)])?;
        let input = &mut startup.input;
        check(
            &input.translate(&sample(InputTransition::Event(button(1)))),
            &Some(button(1)),
        )?;
        check(
            &input.translate(&sample(InputTransition::Event(button(2)))),
            &None,
        )?;
        check(&input.support(2), &None)?;
        check(
            &input.translate(&sample(InputTransition::Event(
                HostEvent::Input(attachment(2)),
            ))),
            &None,
        )?;
        check(
            &input.translate(&sample(InputTransition::Disconnected(1))),
            &Some(HostEvent::Input(InputEvent::Detached { device: 1 })),
        )?;
        check(
            &input.translate(&sample(InputTransition::Disconnected(1))),
            &None,
        )?;
        check(
            &input.translate(&sample(InputTransition::Event(button(1)))),
            &None,
        )?;
        check(&input.support(1), &None)?;
        check(&input.discover_after_drain()?, &vec![attachment(2)])?;
        check(
            &input.translate(&sample(InputTransition::Event(button(2)))),
            &Some(button(2)),
        )?;
        check(&input.support(2), &Some(super::HapticSupport::DualMotor))?;
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
