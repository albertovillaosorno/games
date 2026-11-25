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

pub(crate) type InputPort = Box<dyn InputBackend>;

/// Private device adapter port.
pub(crate) trait InputBackend: Debug + Haptics {
    fn discover_after_drain(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn discover_available(&mut self) -> Result<Vec<InputEvent>, RenderError>;
    fn elapsed(&self) -> Duration;
    fn event_elapsed(&self, event: &InputSample) -> Option<Duration>;
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
    /// attachment events. Touch observations older than an invalidated
    /// geometry are rejected before routing.
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
        self.backend.submit(command)
    }

    fn support(&self, device: u64) -> Option<HapticSupport> {
        self.backend.support(device)
    }
}

#[derive(Debug, Default)]
struct TouchGeometry {
    cutoff: Option<Duration>,
    layout: Option<PresentationLayout>,
}

impl TouchGeometry {
    fn accepts(&self, stamp: Option<Duration>) -> bool {
        self.cutoff
            .is_none_or(|cutoff| stamp.is_some_and(|elapsed| elapsed >= cutoff))
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
        if invalidated || self.cutoff.is_some() {
            self.cutoff = Some(elapsed);
        }
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
    use std::num::NonZeroU32;
    use std::time::Duration;

    use games::presentation::{Dimensions, PresentationMode};

    use super::TouchGeometry;
    use crate::ir::check;

    #[derive(Debug)]
    struct TestInput {
        elapsed: Duration,
    }

    impl super::Haptics for TestInput {
        fn submit(
            &mut self,
            _command: super::HapticCommand,
        ) -> Result<(), super::HapticError> {
            Err(super::HapticError::Disconnected)
        }

        fn support(&self, _device: u64) -> Option<super::HapticSupport> {
            None
        }
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
    fn session_does_not_route_disabled_or_invalidated_touch()
    -> Result<(), super::RenderError> {
        use games::input::touch::{ContactId, ContactPhase, TouchPoint};

        let clock = Duration::from_millis(20);
        let mut session =
            super::InputStartup::new(Box::new(TestInput { elapsed: clock }))?
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
        let size = Dimensions::new(NonZeroU32::MIN, NonZeroU32::MIN);
        let geometry = PresentationMode::AspectFit.layout(size, size);
        check(&session.synchronize_touch(|| Ok(Some(geometry)))?, &None)?;
        check(&session.translate(&sample(Some(clock))), &Some(contact))?;
        check(&session.synchronize_touch(|| Ok(None))?, &Some(clock))?;
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
            (None, 1, false, [true; 4]),
            (first, 2, false, [true; 4]),
            (first, 3, false, [true; 4]),
            (second, 4, true, [false, false, true, true]),
            (second, 5, false, [false, false, true, true]),
            (None, 6, true, [false, false, false, true]),
            (None, 7, false, [false, false, false, true]),
            (first, 8, false, [false, false, false, true]),
        ] {
            assert_eq!(geometry.observe(layout, at(tick)), cancelled);
            let stamps = [None, Some(at(1)), Some(at(4)), Some(at(8))];
            assert_eq!(stamps.map(|stamp| geometry.accepts(stamp)), accepted);
        }
    }
}
