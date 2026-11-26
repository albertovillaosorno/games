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
//   - Bounded standard browser controller state and physical edges.
// - Must-Not:
//   - Assign player stations or filter title-specific dead zones.
// - Allows:
//   - Inputs: Standard mapped controller observations and disconnections.
//   - Outputs: Portable attachment, release and physical change events.
//   - Side effects: Bounded session-local controller state.
// - Split-When:
//   - Browser API acquisition and state reconciliation need separate tests.
// - Merge-When:
//   - A shared host adapter owns equivalent controller observations.
// - Summary:
//   - Preserve device identity across polls and release on loss.
// - Description:
//   - Convert finite standard values and emit only observed changes.
// - Usage:
//   - Poll after draining the owning window's event queue.
// - Defaults:
//   - Neutral axes and buttons, no connected controllers.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(super) mod browser;

use std::num::NonZeroUsize;

use games::input::InputEvent;
use games::input::physical::{ControllerAxis, ControllerButton};

use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputTransition};

const BUTTONS: [(u32, ControllerButton); 15] = [
    (0, ControllerButton::South),
    (1, ControllerButton::East),
    (2, ControllerButton::West),
    (3, ControllerButton::North),
    (4, ControllerButton::LeftShoulder),
    (5, ControllerButton::RightShoulder),
    (8, ControllerButton::Back),
    (9, ControllerButton::Start),
    (10, ControllerButton::LeftStick),
    (11, ControllerButton::RightStick),
    (12, ControllerButton::DPadUp),
    (13, ControllerButton::DPadDown),
    (14, ControllerButton::DPadLeft),
    (15, ControllerButton::DPadRight),
    (16, ControllerButton::Guide),
];

const MAX_DEVICES: usize = 64;
const MAX_TRANSITIONS: usize = BUTTONS.len() + ControllerAxis::ALL.len();

type Observation = (u32, State);

#[derive(Debug)]
struct Device {
    disconnected: bool,
    id: u64,
    index: u32,
    state: State,
}

#[derive(Debug)]
struct Registry {
    devices: Vec<Device>,
    limit: usize,
    next: u64,
}

impl Registry {
    fn attach(
        &mut self,
        states: &[Observation],
    ) -> Result<Vec<InputEvent>, RenderError> {
        let mut attached = Vec::new();
        attached
            .try_reserve_exact(
                self.limit
                    .saturating_sub(self.devices.len())
                    .min(states.len()),
            )
            .map_err(|_error| RenderError::TooLarge)?;
        for &(index, _) in states {
            if self.devices.len() == self.limit {
                break;
            }
            if self.devices.iter().any(|device| device.index == index) {
                continue;
            }
            let id = self.next;
            self.next = id.checked_add(1).ok_or(RenderError::TooLarge)?;
            self.devices.push(Device {
                disconnected: false,
                id,
                index,
                state: State::default(),
            });
            attached.push(InputEvent::Attached { device: id });
        }
        Ok(attached)
    }

    fn disconnect(&mut self, index: u32) {
        if let Some(device) =
            self.devices.iter_mut().find(|device| device.index == index)
        {
            device.disconnected = true;
        }
    }

    fn new(maximum: NonZeroUsize) -> Result<Self, RenderError> {
        let limit = maximum.get().min(MAX_DEVICES);
        let mut devices = Vec::new();
        devices
            .try_reserve_exact(limit)
            .map_err(|_error| RenderError::TooLarge)?;
        Ok(Self { devices, limit, next: 1 })
    }

    fn poll(
        &mut self,
        observations: Option<&[Observation]>,
        mut emit: impl FnMut(InputTransition),
    ) -> Result<Vec<InputEvent>, RenderError> {
        for device in &mut self.devices {
            let observed = observations.map(|states| {
                states
                    .iter()
                    .find(|(index, _)| *index == device.index)
                    .map(|(_, state)| *state)
            });
            if device.disconnected || observed == Some(None) {
                emit(InputTransition::Disconnected(device.id));
                continue;
            }
            let state = observed.flatten().unwrap_or_default();
            for edge in state.changes(device.state, device.id) {
                emit(edge);
            }
            device.state = state;
        }
        self.attach(observations.unwrap_or_default())
    }

    fn remove(&mut self, id: u64) -> Option<HostEvent> {
        let index = self.devices.iter().position(|device| device.id == id)?;
        let _removed = self.devices.remove(index);
        Some(HostEvent::Input(InputEvent::Detached { device: id }))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct State {
    axes: [i16; 6],
    buttons: [bool; BUTTONS.len()],
}

impl State {
    fn changes(
        self,
        previous: Self,
        device: u64,
    ) -> impl Iterator<Item = InputTransition> {
        let axes = ControllerAxis::ALL
            .into_iter()
            .zip(self.axes.into_iter().zip(previous.axes))
            .filter_map(move |(axis, (value, old))| {
                (value != old).then_some(InputEvent::Axis {
                    code: axis.code(),
                    device,
                    value,
                })
            });
        let buttons = BUTTONS
            .into_iter()
            .zip(self.buttons.into_iter().zip(previous.buttons))
            .filter_map(move |((_, button), (down, old))| {
                (down != old).then_some(InputEvent::Button {
                    code: button.code(),
                    device,
                    down,
                })
            });
        axes.chain(buttons)
            .map(|event| InputTransition::Event(HostEvent::Input(event)))
    }

    fn from_values(
        axes: [f64; 6],
        buttons: [bool; BUTTONS.len()],
    ) -> Option<Self> {
        let mut result = Self {
            buttons,
            ..Self::default()
        };
        for (index, (destination, value)) in
            result.axes.iter_mut().zip(axes).enumerate()
        {
            let minimum = if index < 4 {
                -1.0f64
            } else {
                0.0f64
            };
            if !value.is_finite() || !(minimum..=1.0f64).contains(&value) {
                return None;
            }
            *destination = signed_axis(value);
        }
        Some(result)
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "finite admitted signed unit values fit the i16 axis domain"
)]
fn signed_axis(value: f64) -> i16 {
    let scale = if value < 0.0f64 {
        32_768.0f64
    } else {
        32_767.0f64
    };
    (value * scale).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(device: u64, value: i16) -> InputTransition {
        input(InputEvent::Axis { code: 0, device, value })
    }

    fn check(condition: bool, message: &str) -> Result<(), RenderError> {
        condition
            .then_some(())
            .ok_or_else(|| RenderError::Backend(message.to_owned()))
    }

    fn input(event: InputEvent) -> InputTransition {
        InputTransition::Event(HostEvent::Input(event))
    }

    #[test]
    fn standard_mapping_preserves_signed_extents_and_rejects_invalid_values()
    -> Result<(), RenderError> {
        let mut buttons = [false; BUTTONS.len()];
        *buttons.get_mut(11).ok_or(RenderError::InvalidLength)? = true;
        let state = State::from_values(
            [-1.0f64, 1.0f64, 0.0f64, 0.0f64, 0.5f64, 1.0f64],
            buttons,
        )
        .ok_or(RenderError::InvalidLength)?;
        let events: Vec<_> = state.changes(State::default(), 7).collect();
        check(
            events
                == [
                    input(InputEvent::Axis {
                        code: 0,
                        device: 7,
                        value: i16::MIN,
                    }),
                    input(InputEvent::Axis {
                        code: 1,
                        device: 7,
                        value: i16::MAX,
                    }),
                    input(InputEvent::Axis {
                        code: 4,
                        device: 7,
                        value: 16_384,
                    }),
                    input(InputEvent::Axis {
                        code: 5,
                        device: 7,
                        value: i16::MAX,
                    }),
                    input(InputEvent::Button {
                        code: ControllerButton::DPadDown.code(),
                        device: 7,
                        down: true,
                    }),
                ],
            "standard edges",
        )?;
        for invalid in [f64::NAN, f64::INFINITY, -1.1f64, 1.1f64] {
            check(
                State::from_values([invalid; 6], buttons).is_none(),
                "invalid stick",
            )?;
        }
        check(
            State::from_values(
                [0.0f64, 0.0f64, 0.0f64, 0.0f64, -0.1f64, 0.0f64],
                buttons,
            )
            .is_none(),
            "negative trigger",
        )?;
        check(MAX_TRANSITIONS == 21, "transition bound")
    }

    #[test]
    fn disconnect_releases_identity_before_replacement_and_focus_neutralizes()
    -> Result<(), RenderError> {
        let mut registry = Registry::new(NonZeroUsize::MIN)?;
        let mut events = Vec::new();
        let state = State::from_values(
            [1.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64],
            [false; BUTTONS.len()],
        )
        .ok_or(RenderError::InvalidLength)?;
        let observed = [(0, state), (1, state)];
        check(
            registry.poll(Some(&observed), |event| events.push(event))?
                == [InputEvent::Attached { device: 1 }]
                && events.is_empty(),
            "bounded initial attachment",
        )?;
        check(
            registry
                .poll(Some(&observed), |event| events.push(event))?
                .is_empty()
                && events.pop() == Some(axis(1, i16::MAX)),
            "deferred axis",
        )?;
        check(
            registry
                .poll(Some(&observed), |event| events.push(event))?
                .is_empty()
                && events.is_empty(),
            "unchanged observation",
        )?;
        check(
            registry.poll(None, |event| events.push(event))?.is_empty()
                && events.pop() == Some(axis(1, 0)),
            "focus release",
        )?;
        registry.disconnect(0);
        check(
            registry
                .poll(Some(&observed), |event| events.push(event))?
                .is_empty()
                && events.pop() == Some(InputTransition::Disconnected(1)),
            "disconnect before reuse",
        )?;
        check(
            registry.remove(1)
                == Some(HostEvent::Input(InputEvent::Detached { device: 1 })),
            "detach",
        )?;
        check(
            registry.poll(Some(&observed), |event| events.push(event))?
                == [InputEvent::Attached { device: 2 }]
                && events.is_empty(),
            "new identity",
        )
    }
}
