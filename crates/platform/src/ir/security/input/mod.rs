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
//   - Bounded controller identity, window, clock and lifecycle admission.
// - Must-Not:
//   - Enumerate devices or manage native controller resources.
// - Allows:
//   - Inputs: Discovered attachments, scoped events and clock observations.
//   - Outputs: Admitted attachment batches, transitions and elapsed time.
//   - Side effects: Portable session identity and clock bookkeeping only.
// - Split-When:
//   - Another device family requires distinct admission semantics.
// - Merge-When:
//   - An existing portable controller identity contract can be reused.
// - Summary:
//   - Track admitted controller identities independently of native handles.
// - Description:
//   - Reject invalid discovery batches atomically and unknown transitions.
// - Usage:
//   - Admit discoveries and filter translated input before routing.
// - Defaults:
//   - No controller is admitted before discovery.

//! Bounded admission of controller, scope and clock observations.

use std::cell::Cell;
use std::mem;
use std::num::NonZeroUsize;
use std::time::Duration;

use games::input::InputEvent;
use games::presentation::PresentationLayout;

use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputSample, InputTransition};

/// Session-relative time and window scope over raw monotonic observations.
#[derive(Debug)]
pub(crate) struct InputClock {
    origin: Duration,
    previous: Cell<Duration>,
    scope: u64,
}

impl InputClock {
    pub(crate) fn elapsed(
        &self,
        now: Duration,
    ) -> Result<Duration, RenderError> {
        if now < self.previous.get() {
            return Err(RenderError::InvalidTimestamp);
        }
        let elapsed = now
            .checked_sub(self.origin)
            .ok_or(RenderError::InvalidTimestamp)?;
        self.previous.set(now);
        Ok(elapsed)
    }

    pub(crate) fn event_elapsed(
        &self,
        event: &InputSample,
        elapsed: Duration,
    ) -> Option<Duration> {
        if !self.matches(event) {
            return None;
        }
        event
            .elapsed_since(self.origin)
            .filter(|timestamp| *timestamp <= elapsed)
    }

    pub(crate) fn matches(&self, event: &InputSample) -> bool {
        event.matches_window(self.scope)
    }

    pub(crate) const fn new(origin: Duration, scope: u64) -> Self {
        Self {
            origin,
            previous: Cell::new(origin),
            scope,
        }
    }
}

/// Foreground feedback permission follows the admitted session lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeedbackFocus {
    Background,
    Closed,
    Foreground,
}

impl FeedbackFocus {
    pub(crate) const fn allows_feedback(self) -> bool {
        matches!(self, Self::Foreground)
    }

    pub(crate) const fn new(foreground: bool) -> Self {
        if foreground {
            Self::Foreground
        } else {
            Self::Background
        }
    }

    pub(crate) fn observe(&mut self, transition: InputTransition) {
        if *self == Self::Closed {
            return;
        }
        if transition == InputTransition::Event(HostEvent::Quit) {
            *self = Self::Closed;
        } else if transition == InputTransition::FocusGained {
            *self = Self::Foreground;
        } else if transition
            == InputTransition::Event(HostEvent::Input(InputEvent::FocusLost))
        {
            *self = Self::Background;
        }
    }
}

/// Touch admission is tied to the most recently observed presented layout.
#[derive(Debug, Default)]
pub(crate) struct TouchGeometry {
    cutoff: Duration,
    layout: Option<PresentationLayout>,
    pending_cancellation: bool,
}

impl TouchGeometry {
    pub(crate) fn accepts(&self, stamp: Option<Duration>) -> bool {
        self.layout.is_some()
            && !self.pending_cancellation
            && stamp.is_some_and(|elapsed| elapsed >= self.cutoff)
    }

    /// Invalid observations withdraw geometry immediately. Retain any
    /// required cancellation until the next successful synchronization.
    pub(crate) const fn invalidate(&mut self) {
        self.pending_cancellation |= self.layout.take().is_some();
    }

    pub(crate) fn observe(
        &mut self,
        layout: Option<PresentationLayout>,
        elapsed: Duration,
    ) -> bool {
        if self.layout != layout {
            self.invalidate();
            self.layout = layout;
            self.cutoff = elapsed;
        }
        mem::take(&mut self.pending_cancellation)
    }
}

#[derive(Debug)]
pub(crate) struct ControllerAdmission {
    devices: Vec<u64>,
    limit: NonZeroUsize,
}

impl ControllerAdmission {
    pub(crate) fn admit(
        &mut self,
        events: Vec<InputEvent>,
    ) -> Result<Vec<InputEvent>, RenderError> {
        let previous = self.devices.len();
        if events.len() > self.limit.get().saturating_sub(previous) {
            return Err(RenderError::TooLarge);
        }
        self.devices
            .try_reserve(events.len())
            .map_err(|_error| RenderError::TooLarge)?;
        for event in &events {
            if let InputEvent::Attached { device } = *event
                && !self.contains(device)
            {
                self.devices.push(device);
            } else {
                self.devices.truncate(previous);
                return Err(RenderError::Backend(String::from(
                    "invalid controller attachment batch",
                )));
            }
        }
        Ok(events)
    }

    pub(crate) fn contains(&self, device: u64) -> bool {
        self.devices.contains(&device)
    }

    pub(crate) const fn new(limit: NonZeroUsize) -> Self {
        Self {
            devices: Vec::new(),
            limit,
        }
    }

    pub(crate) fn route(
        &mut self,
        observed: InputTransition,
        event: HostEvent,
    ) -> Option<HostEvent> {
        let expected = match observed {
            InputTransition::Disconnected(device) => {
                HostEvent::Input(InputEvent::Detached { device })
            },
            InputTransition::Event(HostEvent::Input(
                InputEvent::Detached { .. },
            ))
            | InputTransition::DiscoveryHint
            | InputTransition::FocusGained
            | InputTransition::Ignored => return None,
            InputTransition::Event(routed) => routed,
        };
        if event != expected {
            return None;
        }
        match event {
            HostEvent::Input(InputEvent::Attached { .. }) => None,
            HostEvent::Input(InputEvent::Detached { device }) => {
                let index = self.devices.iter().position(|id| *id == device)?;
                let _removed = self.devices.swap_remove(index);
                Some(event)
            },
            HostEvent::Input(
                InputEvent::Axis { device, .. }
                | InputEvent::Button { device, .. }
                | InputEvent::ControllerRemapped { device },
            ) => self.contains(device).then_some(event),
            HostEvent::Input(
                InputEvent::Contact { .. }
                | InputEvent::FocusLost
                | InputEvent::Key { .. }
                | InputEvent::Mouse { .. }
                | InputEvent::Touch { .. }
                | InputEvent::TouchCancelled,
            )
            | HostEvent::Quit => Some(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::check;

    #[test]
    fn malformed_discoveries_preserve_capacity() -> Result<(), RenderError> {
        let mut admission = ControllerAdmission::new(
            NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN),
        );
        let first = InputEvent::Attached { device: 1 };
        let second = InputEvent::Attached { device: 2 };
        let third = InputEvent::Attached { device: 3 };
        check(&admission.admit(vec![first])?, &vec![first])?;
        for invalid in [vec![second, first], vec![second, second], vec![
            second,
            InputEvent::FocusLost,
        ]] {
            check(&admission.admit(invalid).is_err(), &true)?;
            check(&admission.contains(1), &true)?;
            check(&admission.contains(2), &false)?;
        }
        check(
            &admission.admit(vec![first, second, third]),
            &Err(RenderError::TooLarge),
        )?;
        check(&admission.admit(vec![third, second])?, &vec![third, second])?;
        Ok(())
    }

    #[test]
    fn translation_must_preserve_observed_identity() -> Result<(), RenderError>
    {
        let mut admission = ControllerAdmission::new(NonZeroUsize::MIN);
        let attached = InputEvent::Attached { device: 1 };
        check(&admission.admit(vec![attached])?, &vec![attached])?;
        let detach = HostEvent::Input(InputEvent::Detached { device: 1 });
        let button = HostEvent::Input(InputEvent::Button {
            device: 1,
            code: 1,
            down: true,
        });
        for observed in [
            InputTransition::Disconnected(2),
            InputTransition::Event(button),
            InputTransition::Event(detach),
            InputTransition::DiscoveryHint,
            InputTransition::FocusGained,
            InputTransition::Ignored,
        ] {
            check(&admission.route(observed, detach), &None)?;
            check(&admission.contains(1), &true)?;
        }
        check(
            &admission.route(InputTransition::Disconnected(1), button),
            &None,
        )?;
        check(
            &admission.route(InputTransition::Event(button), button),
            &Some(button),
        )?;
        check(
            &admission.route(InputTransition::Disconnected(1), detach),
            &Some(detach),
        )?;
        check(&admission.contains(1), &false)?;
        Ok(())
    }
}
