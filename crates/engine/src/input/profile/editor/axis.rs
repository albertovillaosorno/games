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
//   - Selection and staged editing of signed digital controller axes.
// - Must-Not:
//   - Interpret analogue device samples, route actions or store profiles.
// - Allows:
//   - Inputs: Explicit menu commands, title action count and admitted axes.
//   - Outputs: A candidate axis profile or cancellation.
//   - Side effects: Deterministic draft state only.
// - Split-When:
//   - Hardware calibration requires device sampling.
// - Merge-When:
//   - Another axis editor duplicates the same selection and draft rules.
// - Summary:
//   - Edit both directions and hysteresis as one next-launch transaction.
// - Description:
//   - Keep thresholds ordered and never enable a trigger's negative side.
// - Usage:
//   - Admit and persist the full candidate before acknowledging success.
// - Defaults:
//   - First standard axis and no active draft.

//! Portable signed-axis editing, independent of native device samples.

use super::{EditorEffect, EditorNavigation, cycle};
use crate::input::physical::ControllerAxis;
use crate::input::profile::AxisProfile;

const THRESHOLD_STEP: u16 = 1000;
const MAX_MAGNITUDE: u16 = 32_767;

/// Editable field of one physical axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AxisField {
    /// Logical action on the negative side.
    Negative,
    /// Logical action on the positive side.
    Positive,
    /// Inclusive press magnitude.
    Press,
    /// Inclusive release magnitude.
    Release,
}

impl AxisField {
    const ALL: [Self; 4] =
        [Self::Negative, Self::Positive, Self::Press, Self::Release];

    fn next(self, axis: ControllerAxis, forward: bool) -> Self {
        let start = usize::from(is_trigger(axis));
        let count = Self::ALL.len().saturating_sub(start);
        let index = match self {
            Self::Negative => 0usize,
            Self::Positive => 1,
            Self::Press => 2,
            Self::Release => 3,
        };
        let next = cycle(index.saturating_sub(start), count, forward);
        Self::ALL
            .get(start.saturating_add(next))
            .copied()
            .unwrap_or(Self::Positive)
    }
}

/// Selected physical axis and its current or staged values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AxisEditorView {
    /// Standard physical axis.
    pub axis: ControllerAxis,
    /// Selected draft field, if editing.
    pub field: Option<AxisField>,
    /// Admitted or pending direction and threshold values.
    pub profile: AxisProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Draft {
    field: AxisField,
    profile: AxisProfile,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct AxisEditor {
    draft: Option<Draft>,
    index: usize,
}

impl AxisEditor {
    fn axis(self) -> Option<ControllerAxis> {
        ControllerAxis::ALL.get(self.index).copied()
    }

    pub(super) fn begin(&mut self, axes: &[AxisProfile]) -> bool {
        let Some(view) = self.view(axes) else {
            return false;
        };
        self.draft = Some(Draft {
            field: if is_trigger(view.axis) {
                AxisField::Positive
            } else {
                AxisField::Negative
            },
            profile: view.profile,
        });
        true
    }

    pub(super) const fn cancel(&mut self) {
        self.draft = None;
    }

    pub(super) fn capture(
        &mut self,
        command: EditorNavigation,
        actions: usize,
    ) -> EditorEffect {
        let Some(axis) = self.axis() else {
            return EditorEffect::None;
        };
        let Some(draft) = self.draft.as_mut() else {
            return EditorEffect::None;
        };
        match command {
            EditorNavigation::Up => draft.field = draft.field.next(axis, false),
            EditorNavigation::Down => {
                draft.field = draft.field.next(axis, true);
            },
            EditorNavigation::Left => change(draft, actions, false),
            EditorNavigation::Right => change(draft, actions, true),
            EditorNavigation::Clear => clear(draft),
            EditorNavigation::Confirm => {
                let profile = draft.profile;
                self.cancel();
                return EditorEffect::ProposeAxis { axis, profile };
            },
            EditorNavigation::Cancel | EditorNavigation::Family => {},
        }
        EditorEffect::None
    }

    pub(super) fn select(&mut self, forward: bool) {
        self.index = cycle(self.index, ControllerAxis::ALL.len(), forward);
    }

    pub(super) fn view(
        self,
        profiles: &[AxisProfile],
    ) -> Option<AxisEditorView> {
        let axis = self.axis()?;
        let admitted = *profiles.get(self.index)?;
        let (profile, field) = self.draft.map_or((admitted, None), |draft| {
            (draft.profile, Some(draft.field))
        });
        Some(AxisEditorView { axis, field, profile })
    }
}

fn change(draft: &mut Draft, actions: usize, forward: bool) {
    let profile = &mut draft.profile;
    match draft.field {
        AxisField::Negative => {
            profile.negative = next_action(profile.negative, actions, forward);
        },
        AxisField::Positive => {
            profile.positive = next_action(profile.positive, actions, forward);
        },
        AxisField::Press => {
            profile.press = step(profile.press, forward)
                .max(profile.release.saturating_add(1))
                .min(MAX_MAGNITUDE);
        },
        AxisField::Release => {
            profile.release = step(profile.release, forward)
                .min(profile.press.saturating_sub(1));
        },
    }
}

fn clear(draft: &mut Draft) {
    match draft.field {
        AxisField::Negative => draft.profile.negative = None,
        AxisField::Positive => draft.profile.positive = None,
        AxisField::Press => {
            draft.profile.press = AxisProfile::UNBOUND
                .press
                .max(draft.profile.release.saturating_add(1));
        },
        AxisField::Release => {
            draft.profile.release = AxisProfile::UNBOUND
                .release
                .min(draft.profile.press.saturating_sub(1));
        },
    }
}

const fn is_trigger(axis: ControllerAxis) -> bool {
    matches!(
        axis,
        ControllerAxis::TriggerLeft | ControllerAxis::TriggerRight
    )
}

fn next_action(
    slot: Option<usize>,
    actions: usize,
    forward: bool,
) -> Option<usize> {
    let index = slot.map_or(0, |value| value.saturating_add(1));
    cycle(index, actions.saturating_add(1), forward).checked_sub(1)
}

const fn step(value: u16, forward: bool) -> u16 {
    if forward {
        value.saturating_add(THRESHOLD_STEP)
    } else {
        value.saturating_sub(THRESHOLD_STEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AXES: [AxisProfile; 6] = [AxisProfile::UNBOUND; 6];

    #[test]
    fn trigger_skips_negative_field_and_draft_is_discarded_on_cancel() {
        let mut editor = AxisEditor::default();
        editor.select(false);
        assert!(editor.begin(&AXES));
        let initial = editor.view(&AXES);
        assert_eq!(
            initial.map(|view| view.axis),
            Some(ControllerAxis::TriggerRight)
        );
        assert_eq!(
            initial.and_then(|view| view.field),
            Some(AxisField::Positive)
        );
        assert_eq!(
            editor.capture(EditorNavigation::Right, 2),
            EditorEffect::None
        );
        assert_eq!(
            editor.view(&AXES).and_then(|view| view.profile.positive),
            Some(0)
        );
        editor.cancel();
        assert_eq!(
            editor.view(&AXES).map(|view| view.profile),
            Some(AxisProfile::UNBOUND)
        );
    }

    #[test]
    fn press_and_release_never_cross_at_extremes() {
        let mut draft = Draft {
            field: AxisField::Press,
            profile: AxisProfile {
                press: 32_767,
                release: 32_766,
                ..AxisProfile::UNBOUND
            },
        };
        change(&mut draft, 2, false);
        assert_eq!(draft.profile.press, 32_767);
        clear(&mut draft);
        assert_eq!(draft.profile.press, 32_767);
        draft.field = AxisField::Release;
        change(&mut draft, 2, true);
        assert_eq!(draft.profile.release, 32_766);
        draft.profile = AxisProfile {
            press: 1,
            release: 0,
            ..AxisProfile::UNBOUND
        };
        change(&mut draft, 2, true);
        assert_eq!(draft.profile.release, 0);
        clear(&mut draft);
        assert_eq!(draft.profile.release, 0);
    }
}
