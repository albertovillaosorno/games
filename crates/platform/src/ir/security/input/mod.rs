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
//   - Bounded controller identity admission and routing.
// - Must-Not:
//   - Enumerate devices or manage native controller resources.
// - Allows:
//   - Inputs: Discovered attachments and translated host events.
//   - Outputs: Admitted attachment batches and controller transitions.
//   - Side effects: Portable session identity bookkeeping only.
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

//! Bounded admission of controller observations.

use std::num::NonZeroUsize;

use games::input::InputEvent;

use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputTransition};

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
