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

use std::collections::VecDeque;
use std::mem;
use std::num::NonZeroUsize;
use std::time::Duration;

use games::input::InputEvent;
use games::input::physical::{ControllerAxis, ControllerButton};

use crate::ir::frame::RenderError;
use crate::ir::hardware::{HostEvent, InputSample, InputTransition};

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

/// One event feed shared by the scoped surface and its controller port.
/// A real focus-loss observation may arrive before the next frame poll.
pub(super) struct ControllerEvents {
    registry: Option<Registry>,
    samples: VecDeque<InputSample>,
    scope: u64,
}

impl ControllerEvents {
    pub(super) fn cancel(&mut self, stamp: Option<Duration>) {
        if let Some(registry) = &mut self.registry {
            let scope = self.scope;
            registry.neutralize(|edge| {
                self.samples.push_back(InputSample::new(
                    stamp,
                    edge,
                    Some(scope),
                ));
            });
        }
    }

    fn close(&mut self) {
        self.samples.clear();
        self.registry = None;
    }

    fn disconnect(&mut self, index: u32) {
        if let Some(registry) = &mut self.registry {
            registry.disconnect(index);
        }
    }

    #[cfg(all(feature = "test-fixtures", target_arch = "wasm32"))]
    pub(super) fn fixture_hold(
        &mut self,
        stamp: Duration,
    ) -> Result<Vec<InputEvent>, RenderError> {
        let state = State::from_values(
            [1.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64],
            [false; BUTTONS.len()],
        )
        .ok_or(RenderError::InvalidLength)?;
        let observed = [(0, state)];
        if self.registry.is_none() {
            self.open(NonZeroUsize::MIN)?;
            let attached = self.poll(Some(&observed), stamp)?;
            let _observed = self.poll(Some(&observed), stamp)?;
            Ok(attached)
        } else {
            self.poll(Some(&observed), stamp)
        }
    }

    pub(super) const fn new(scope: u64) -> Self {
        Self {
            registry: None,
            samples: VecDeque::new(),
            scope,
        }
    }

    fn open(&mut self, limit: NonZeroUsize) -> Result<(), RenderError> {
        if self.registry.is_some() {
            return Err(RenderError::Unsupported);
        }
        let registry = Registry::new(limit)?;
        // One polled batch and one cancellation batch may coexist. Further
        // cancellation is idempotent until all samples are drained.
        self.samples
            .try_reserve_exact(
                registry
                    .limit
                    .saturating_mul(MAX_TRANSITIONS)
                    .saturating_mul(2),
            )
            .map_err(|_error| RenderError::TooLarge)?;
        self.registry = Some(registry);
        Ok(())
    }

    fn poll(
        &mut self,
        observations: Option<&[Observation]>,
        stamp: Duration,
    ) -> Result<Vec<InputEvent>, RenderError> {
        if !self.samples.is_empty() {
            return Ok(Vec::new());
        }
        let registry =
            self.registry.as_mut().ok_or(RenderError::Unsupported)?;
        let scope = self.scope;
        registry.poll(observations, |edge| {
            self.samples.push_back(InputSample::new(
                Some(stamp),
                edge,
                Some(scope),
            ));
        })
    }

    pub(super) fn pop(&mut self) -> Option<InputSample> {
        self.samples.pop_front()
    }

    fn remove(&mut self, id: u64) -> Option<HostEvent> {
        self.registry.as_mut()?.remove(id)
    }
}

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

    fn neutralize(&mut self, mut emit: impl FnMut(InputTransition)) {
        for device in &mut self.devices {
            let state = mem::take(&mut device.state);
            for edge in State::default().changes(state, device.id) {
                emit(edge);
            }
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

    #[test]
    fn lifecycle_cancellation_precedes_the_next_focused_snapshot()
    -> Result<(), RenderError> {
        let scope = 7;
        let mut feed = ControllerEvents::new(scope);
        let limit = NonZeroUsize::MIN.saturating_add(1);
        feed.open(limit)?;
        check(feed.open(limit).is_err(), "one owning controller port")?;
        let state = State::from_values([1.0f64; 6], [true; BUTTONS.len()])
            .ok_or(RenderError::InvalidLength)?;
        let observed = [(0, state), (1, state)];
        let stamp = |ms| Duration::from_millis(ms);
        check(
            feed.poll(Some(&observed), stamp(1))?
                == [InputEvent::Attached { device: 1 }, InputEvent::Attached {
                    device: 2,
                }],
            "both controllers admitted",
        )?;
        check(
            feed.poll(Some(&observed), stamp(2))?.is_empty(),
            "held observations queued",
        )?;
        // Blur and refocus can occur between polls. Preserve all queued
        // edges, append the neutral state at the loss time exactly once,
        // and never admit a new snapshot ahead of those releases.
        feed.cancel(Some(stamp(3)));
        feed.cancel(Some(stamp(4)));
        check(
            feed.poll(Some(&observed), stamp(5))?.is_empty(),
            "snapshot deferred while cancellation is queued",
        )?;
        for (old, new, time) in [
            (State::default(), state, stamp(2)),
            (state, State::default(), stamp(3)),
        ] {
            verify_samples(&mut feed, (old, new), time, scope)?;
        }
        check(feed.pop().is_none(), "duplicate cancellation")?;
        check(
            feed.poll(Some(&observed), stamp(6))?.is_empty(),
            "same device identities on refocus",
        )?;
        verify_samples(&mut feed, (State::default(), state), stamp(6), scope)
    }

    #[test]
    fn feed_ownership_and_disconnect_are_scoped() -> Result<(), RenderError> {
        let scope = 7;
        let mut feed = ControllerEvents::new(scope);
        let limit = NonZeroUsize::MIN.saturating_add(1);
        feed.open(limit)?;
        let observed = [(0, State::default()), (1, State::default())];
        let stamp = Duration::from_millis;
        check(feed.poll(Some(&observed), stamp(1))?.len() == 2, "attached")?;
        feed.disconnect(0);
        check(
            feed.poll(Some(&observed), stamp(7))?.is_empty()
                && feed.pop()
                    == Some(InputSample::new(
                        Some(stamp(7)),
                        InputTransition::Disconnected(1),
                        Some(scope),
                    )),
            "disconnect still ordered",
        )?;
        check(
            feed.remove(1)
                == Some(HostEvent::Input(InputEvent::Detached { device: 1 })),
            "detach still observed",
        )?;
        feed.close();
        feed.cancel(Some(stamp(8)));
        check(feed.pop().is_none(), "closed feed releases ownership")?;
        feed.open(limit)?;
        check(
            feed.poll(Some(&observed), stamp(9))?
                == [InputEvent::Attached { device: 1 }, InputEvent::Attached {
                    device: 2,
                }],
            "independent reopened session",
        )
    }

    fn verify_samples(
        feed: &mut ControllerEvents,
        (old, new): (State, State),
        stamp: Duration,
        scope: u64,
    ) -> Result<(), RenderError> {
        for device in [1, 2] {
            let edges: Vec<_> = new.changes(old, device).collect();
            check(
                edges.len() == MAX_TRANSITIONS,
                "complete controller snapshot",
            )?;
            for edge in edges {
                check(
                    feed.pop()
                        == Some(InputSample::new(
                            Some(stamp),
                            edge,
                            Some(scope),
                        )),
                    "ordered, scoped controller edge",
                )?;
            }
        }
        Ok(())
    }

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
