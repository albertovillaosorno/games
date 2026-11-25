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

use crate::presentation::{PresentationLayout, Viewport};

/// A position in the normalized control surface, inclusive at both edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TouchPoint {
    /// Horizontal coordinate, from zero to `u16::MAX`.
    pub x: u16,
    /// Vertical coordinate, from zero to `u16::MAX`.
    pub y: u16,
}

impl TouchPoint {
    /// Project a contact through the rendered source and destination.
    ///
    /// Rejects letterbox space and invalid rectangles. Cropped content
    /// retains its position within the complete logical frame.
    #[must_use]
    pub fn in_presentation(self, layout: PresentationLayout) -> Option<Self> {
        let relative = self.in_viewport(
            layout.destination,
            layout.surface.width,
            layout.surface.height,
        )?;
        Some(Self {
            x: source_axis(
                relative.x,
                layout.source.x,
                layout.source.width,
                layout.design.width,
            )?,
            y: source_axis(
                relative.y,
                layout.source.y,
                layout.source.height,
                layout.design.height,
            )?,
        })
    }

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

    /// Project a visible logical point onto the physical surface.
    /// Returns None for cropped-out content or invalid geometry.
    #[must_use]
    pub fn on_presentation(self, layout: PresentationLayout) -> Option<Self> {
        self.in_presentation(PresentationLayout {
            design: layout.surface,
            destination: layout.source,
            source: layout.destination,
            surface: layout.design,
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

fn source_axis(
    point: u16,
    offset: u32,
    extent: u32,
    design: NonZeroU32,
) -> Option<u16> {
    if extent == 0 || offset.checked_add(extent)? > design.get() {
        return None;
    }
    let start = u64::from(offset).checked_mul(u64::from(u16::MAX))?;
    let relative = u64::from(point).checked_mul(u64::from(extent))?;
    let coordinate = start.checked_add(relative)?;
    u16::try_from(coordinate.checked_div(NonZeroU64::from(design).get())?).ok()
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{ContactId, ContactPhase, TouchLayout, TouchPoint, TouchZone};
    use crate::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
    use crate::presentation::{Dimensions, PresentationMode, Viewport};

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
    fn cropped_touch_maps_to_the_visible_source_and_stretch_maps_all() {
        let nz = |value| NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN);
        let surface = Dimensions::new(nz(400), nz(400));
        let design = Dimensions::new(nz(800), nz(400));
        let fill = PresentationMode::Fill.layout(surface, design);
        let stretch = PresentationMode::Stretch.layout(surface, design);
        let first = TouchPoint { x: 0, y: 0 };
        let last = TouchPoint { x: u16::MAX, y: u16::MAX };
        let center = TouchPoint { x: 32768, y: 32768 };
        assert_eq!(
            first.in_presentation(fill),
            Some(TouchPoint { x: 16383, y: 0 })
        );
        assert_eq!(
            last.in_presentation(fill),
            Some(TouchPoint { x: 49151, y: u16::MAX })
        );
        assert_eq!(
            center.in_presentation(fill),
            Some(TouchPoint { x: 32767, y: 32768 })
        );
        assert_eq!(first.on_presentation(fill), None);
        assert_eq!(last.on_presentation(fill), None);
        let visible = TouchPoint { x: 24000, y: 48000 };
        let round_trip = visible
            .on_presentation(fill)
            .and_then(|point| point.in_presentation(fill));
        assert!(
            round_trip.is_some_and(|point| point.x.abs_diff(visible.x) <= 2
                && point.y.abs_diff(visible.y) <= 2)
        );
        assert_eq!(first.in_presentation(stretch), Some(first));
        assert_eq!(last.in_presentation(stretch), Some(last));
        let mut invalid = fill;
        invalid.source.width = u32::MAX;
        assert_eq!(center.in_presentation(invalid), None);
    }

    #[test]
    fn integer_and_aspect_fit_reject_unrendered_surface_space() {
        let nz = |value| NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN);
        let surface = Dimensions::new(nz(800), nz(600));
        let design = Dimensions::new(nz(320), nz(224));
        for mode in [PresentationMode::AspectFit, PresentationMode::Integer] {
            let layout = mode.layout(surface, design);
            assert_eq!(
                TouchPoint { x: 32768, y: 0 }.in_presentation(layout),
                None
            );
            let center = TouchPoint { x: 32768, y: 32768 };
            assert_eq!(center.in_presentation(layout), Some(center));
        }
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
