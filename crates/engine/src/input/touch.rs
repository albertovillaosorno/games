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
//   - Normalized touch geometry and bounded contact admission.
// - Must-Not:
//   - Open native devices, assign title actions or advance simulation.
// - Allows:
//   - Inputs: Ordered zones and host contact transitions.
//   - Outputs: Admitted contact identities and matching zone identifiers.
//   - Side effects: Tracks active contact identities in memory.
// - Split-When:
//   - Gesture recognition requires an independent contract.
// - Merge-When:
//   - Another shared input component duplicates contact tracking.
// - Summary:
//   - Configurable contact geometry for digital touch controls.
// - Description:
//   - Rejects excess contacts and motion without a preceding press.
// - Usage:
//   - Install a title layout before processing input transitions.
// - Defaults:
//   - No contacts are admitted without an explicit layout.

//! Bounded touch contact geometry and lifetime tracking.

use std::collections::BTreeSet;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

use crate::presentation::Viewport;

/// A position in the normalized control surface, inclusive at both edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TouchPoint {
    /// Horizontal coordinate, from zero to `u16::MAX`.
    pub x: u16,
    /// Vertical coordinate, from zero to `u16::MAX`.
    pub y: u16,
}

impl TouchPoint {
    /// Project surface-normalized coordinates into a fitted viewport.
    /// Returns None for letterbox areas or invalid viewport geometry.
    #[must_use]
    pub fn in_viewport(
        self,
        viewport: Viewport,
        surface_width: NonZeroU32,
        surface_height: NonZeroU32,
    ) -> Option<Self> {
        Some(Self {
            x: project_axis(self.x, viewport.x, viewport.width, surface_width)?,
            y: project_axis(
                self.y,
                viewport.y,
                viewport.height,
                surface_height,
            )?,
        })
    }
}

/// A contact identity scoped to the originating touch device.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContactId {
    /// Host-local touch device identity.
    pub device: i64,
    /// Host-local finger identity on that device.
    pub finger: i64,
}

/// Contact transitions, including positions outside the control surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContactPhase {
    /// A new contact at a valid position.
    Down(TouchPoint),
    /// Move an existing contact; None indicates an invalid position.
    Motion(Option<TouchPoint>),
    /// End a contact, independently of its reported position.
    Up,
}

/// A rectangular touch zone with inclusive normalized bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TouchZone {
    id: u16,
    max: TouchPoint,
    min: TouchPoint,
}

impl TouchZone {
    const fn contains(self, point: TouchPoint) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Create a non-inverted zone, returning None for invalid bounds.
    #[must_use]
    pub const fn new(
        id: u16,
        min: TouchPoint,
        max: TouchPoint,
    ) -> Option<Self> {
        if min.x > max.x || min.y > max.y {
            return None;
        }
        Some(Self { id, max, min })
    }
}

/// Title-selected touch zones and a maximum simultaneous contact count.
#[derive(Clone, Debug)]
pub struct TouchLayout {
    limit: NonZeroUsize,
    zones: Vec<TouchZone>,
}

impl TouchLayout {
    /// Create an ordered layout; the first matching zone wins overlaps.
    ///
    /// Multiple zones may share one identifier. Positions without a zone
    /// do not hold an action but still occupy a contact slot until release.
    #[must_use]
    pub const fn new(zones: Vec<TouchZone>, limit: NonZeroUsize) -> Self {
        Self { limit, zones }
    }

    /// Look up a zone on the entire normalized control surface.
    #[must_use]
    pub fn zone_at(&self, point: TouchPoint) -> Option<u16> {
        self.zones
            .iter()
            .find(|zone| zone.contains(point))
            .map(|zone| zone.id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContactUpdate {
    Ignored,
    Zone(Option<u16>),
}

#[derive(Debug)]
pub(super) struct Contacts {
    active: BTreeSet<ContactId>,
}

impl Contacts {
    pub(super) fn clear(&mut self) {
        self.active.clear();
    }

    pub(super) const fn new() -> Self {
        Self { active: BTreeSet::new() }
    }

    pub(super) fn update(
        &mut self,
        layout: &TouchLayout,
        contact: ContactId,
        phase: ContactPhase,
    ) -> ContactUpdate {
        let point = match phase {
            ContactPhase::Down(point) => {
                if self.active.len() >= layout.limit.get()
                    || !self.active.insert(contact)
                {
                    return ContactUpdate::Ignored;
                }
                Some(point)
            },
            ContactPhase::Motion(point) => {
                if !self.active.contains(&contact) {
                    return ContactUpdate::Ignored;
                }
                point
            },
            ContactPhase::Up => {
                if !self.active.remove(&contact) {
                    return ContactUpdate::Ignored;
                }
                None
            },
        };
        ContactUpdate::Zone(point.and_then(|position| layout.zone_at(position)))
    }
}

fn project_axis(
    point: u16,
    offset: u32,
    extent: u32,
    surface: NonZeroU32,
) -> Option<u16> {
    let end = offset.checked_add(extent)?;
    if end > surface.get() || extent == 0 {
        return None;
    }
    let scale = u64::from(u16::MAX);
    let coordinate =
        u64::from(point).checked_mul(NonZeroU64::from(surface).get())?;
    let start = u64::from(offset).checked_mul(scale)?;
    let relative = coordinate.checked_sub(start)?;
    let logical = relative.checked_div(u64::from(extent))?;
    if coordinate > u64::from(end).checked_mul(scale)? {
        return None;
    }
    u16::try_from(logical).ok()
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{ContactId, ContactPhase, TouchLayout, TouchPoint, TouchZone};
    use crate::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use crate::presentation::Viewport;

    const LEFT: TouchPoint = TouchPoint { x: 100, y: 0 };
    const RIGHT: TouchPoint = TouchPoint { x: 500, y: 0 };
    const OUTSIDE: TouchPoint = TouchPoint { x: 900, y: 0 };

    fn zone(id: u16, start: u16, end: u16) -> TouchZone {
        TouchZone {
            id,
            max: TouchPoint { x: end, y: u16::MAX },
            min: TouchPoint { x: start, y: 0 },
        }
    }

    fn router(limit: usize) -> InputRouter<&'static str> {
        let mut mapping = InputMapping::new();
        mapping.bind_touch(1, "left");
        mapping.bind_touch(2, "right");
        mapping.bind_touch(3, "left");
        mapping.bind_key(42, "left");
        mapping.bind_button(7, "right");
        mapping.set_touch_layout(TouchLayout::new(
            vec![zone(1, 0, 199), zone(3, 200, 399), zone(2, 400, 699)],
            NonZeroUsize::new(limit).unwrap_or(NonZeroUsize::MIN),
        ));
        InputRouter::new(mapping)
    }

    const fn event(
        device: i64,
        finger: i64,
        phase: ContactPhase,
    ) -> InputEvent {
        InputEvent::Contact {
            id: ContactId { device, finger },
            phase,
        }
    }

    const fn action(
        action: &'static str,
        down: bool,
    ) -> LogicalAction<&'static str> {
        LogicalAction { action, down }
    }

    #[test]
    fn projection_rejects_letterbox_and_preserves_viewport_edges() {
        let width = NonZeroU32::new(800).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(400).unwrap_or(NonZeroU32::MIN);
        let view = Viewport::fit(width, height, height, height);
        assert_eq!(
            TouchPoint { x: 0, y: 32768 }.in_viewport(view, width, height),
            None
        );
        assert_eq!(
            TouchPoint { x: 65535, y: 0 }.in_viewport(view, width, height),
            None
        );
        assert_eq!(
            TouchPoint { x: 32768, y: 0 }.in_viewport(view, width, height),
            Some(TouchPoint { x: 32768, y: 0 })
        );
        assert_eq!(
            TouchPoint { x: 16384, y: u16::MAX }
                .in_viewport(view, width, height),
            Some(TouchPoint { x: 0, y: u16::MAX })
        );
        assert_eq!(
            TouchPoint { x: 49152, y: 0 }.in_viewport(view, width, height),
            None
        );
        let invalid = Viewport { width: u32::MAX, ..view };
        assert_eq!(LEFT.in_viewport(invalid, width, height), None);
    }

    #[test]
    fn zones_have_explicit_edges_priority_and_invalid_bounds() {
        let layout = TouchLayout::new(
            vec![zone(1, 0, 100), zone(2, 100, u16::MAX)],
            NonZeroUsize::MIN,
        );
        assert_eq!(layout.zone_at(LEFT), Some(1));
        assert_eq!(layout.zone_at(TouchPoint { x: 101, y: u16::MAX }), Some(2));
        assert_eq!(layout.zone_at(TouchPoint { x: u16::MAX, y: 0 }), Some(2));
        assert_eq!(TouchZone::new(1, RIGHT, LEFT), None);
    }

    #[test]
    fn simultaneous_fingers_aggregate_and_move_between_actions() {
        let mut router = router(2);
        assert_eq!(router.handle(event(1, 3, ContactPhase::Down(LEFT))), [
            action("left", true)
        ]);
        assert_eq!(router.handle(event(2, 3, ContactPhase::Down(LEFT))), []);
        assert_eq!(router.handle(event(1, 3, ContactPhase::Up)), []);
        assert_eq!(
            router.handle(event(
                2,
                3,
                ContactPhase::Motion(Some(TouchPoint { x: 300, y: 0 }))
            )),
            []
        );
        assert_eq!(
            router.handle(event(2, 3, ContactPhase::Motion(Some(RIGHT)))),
            [action("left", false), action("right", true)]
        );
        assert_eq!(router.handle(event(2, 3, ContactPhase::Motion(None))), [
            action("right", false)
        ]);
        assert_eq!(
            router.handle(event(2, 3, ContactPhase::Motion(Some(LEFT)))),
            [action("left", true)]
        );
        assert_eq!(router.handle(event(2, 3, ContactPhase::Up)), [action(
            "left", false
        )]);
    }

    #[test]
    fn unknown_duplicate_and_over_limit_contacts_are_not_admitted() {
        let mut router = router(1);
        for input in [
            event(1, 1, ContactPhase::Motion(Some(LEFT))),
            event(1, 1, ContactPhase::Down(OUTSIDE)),
            event(1, 1, ContactPhase::Down(LEFT)),
            event(1, 2, ContactPhase::Down(LEFT)),
            event(1, 2, ContactPhase::Motion(Some(LEFT))),
            event(1, 2, ContactPhase::Up),
            event(1, 1, ContactPhase::Up),
            event(1, 2, ContactPhase::Motion(Some(LEFT))),
        ] {
            assert_eq!(router.handle(input), [], "{input:?}");
        }
        assert_eq!(router.handle(event(1, 2, ContactPhase::Down(RIGHT))), [
            action("right", true)
        ]);
    }

    #[test]
    fn focus_loss_clears_contacts_without_releasing_controller() {
        let mut router = router(2);
        assert_eq!(router.handle(InputEvent::Attached { device: 5 }), []);
        assert_eq!(
            router.handle(InputEvent::Button {
                device: 5,
                code: 7,
                down: true
            }),
            [action("right", true)]
        );
        assert_eq!(router.handle(event(1, 1, ContactPhase::Down(RIGHT))), []);
        assert_eq!(router.handle(InputEvent::Key { code: 42, down: true }), [
            action("left", true)
        ]);
        assert_eq!(router.handle(event(1, 2, ContactPhase::Down(LEFT))), []);
        assert_eq!(router.handle(InputEvent::FocusLost), [action(
            "left", false
        )]);
        assert_eq!(
            router.handle(event(1, 2, ContactPhase::Motion(Some(LEFT)))),
            []
        );
        assert_eq!(router.handle(event(1, 2, ContactPhase::Down(LEFT))), [
            action("left", true)
        ]);
        assert_eq!(router.handle(InputEvent::Detached { device: 5 }), [
            action("right", false)
        ]);
    }

    #[test]
    fn raw_contacts_are_disabled_without_an_explicit_layout() {
        let mut mapping = InputMapping::new();
        mapping.bind_touch(1, "left");
        let mut router = InputRouter::new(mapping);
        assert_eq!(router.handle(event(1, 1, ContactPhase::Down(LEFT))), []);
        assert_eq!(router.handle(InputEvent::Touch { zone: 1, down: true }), [
            action("left", true)
        ]);
    }
}
