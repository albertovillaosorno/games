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
//   - Bounded neutral first-room contact and delayed reactions.
// - Must-Not:
//   - Enable campaign combat, admit lethal damage or render sprites.
// - Allows:
//   - Inputs: Station state and observed strike or leap geometry.
//   - Outputs: One station debit and subsequent reaction.
//   - Side effects: Deterministic in-memory station updates only.
// - Split-When:
//   - Additional encounter profiles require independent damage policies.
// - Merge-When:
//   - A shared combat policy supports these verified contact semantics.
// - Summary:
//   - Verify inclusive three-axis collision and its two-step response.
// - Description:
//   - Admit only the observed idle, right-facing neutral station.
// - Usage:
//   - Step once after admitted pursuit advancement.
// - Defaults:
//   - No pending reaction.

use crate::{
    Facing, Maneuver, PendingManeuver, Player, Point, Slot, hurt_slot,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Attack {
    Leap { point: Point, elevation: i16 },
    Strike(Point),
}

impl Attack {
    const fn damage(self) -> u8 {
        match self {
            Self::Strike(_) => 4,
            Self::Leap { .. } => 8,
        }
    }

    const fn volume(self) -> Volume {
        match self {
            Self::Strike(point) => Volume::strike(point),
            Self::Leap { point, elevation } => Volume::leap(point, elevation),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Preview {
    pending: bool,
}

impl Preview {
    pub(super) fn advance(
        &mut self,
        slot: &mut Slot,
        player: Player,
        candidate: Option<Attack>,
    ) {
        if self.pending {
            self.pending = false;
            // Health was debited on the preceding contact step.
            hurt_slot(slot, 0);
            slot.station.display_health = slot.station.health;
        } else if matches!(slot.station.maneuver, Maneuver::Hurt { .. }) {
            slot.advance_maneuver(player, PendingManeuver::default());
        } else if let Some(attack) = candidate
            && can_debit(slot, attack)
        {
            slot.station.health =
                slot.station.health.saturating_sub(attack.damage());
            self.pending = true;
        }
    }
}

struct Volume {
    center: [i16; 3],
    radii: [u8; 3],
}

impl Volume {
    const fn leap(point: Point, elevation: i16) -> Self {
        Self {
            center: [
                point.x.saturating_sub(8),
                point.y,
                elevation.saturating_add(24),
            ],
            radii: [8, 4, 8],
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.center
            .into_iter()
            .zip(other.center)
            .zip(self.radii.into_iter().zip(other.radii))
            .all(|((left, right), (r1, r2))| {
                left.abs_diff(right)
                    <= u16::from(r1).saturating_add(u16::from(r2))
            })
    }

    const fn station(point: Point, narrow: bool) -> Self {
        Self {
            center: [point.x, point.y, 24],
            radii: if narrow {
                [14, 6, 24]
            } else {
                [16, 8, 24]
            },
        }
    }

    const fn strike(point: Point) -> Self {
        Self {
            center: [point.x.saturating_sub(16), point.y, 40],
            radii: [16, 4, 12],
        }
    }
}

fn can_debit(slot: &Slot, attack: Attack) -> bool {
    let station = slot.station;
    station.can_fight()
        && station.health > attack.damage()
        && station.facing == Facing::Right
        && station.maneuver == Maneuver::Idle
        && station.walking.is_none()
        && attack
            .volume()
            .overlaps(&Volume::station(station.position, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_strike_matches_observed_station_contact_controls() {
        let strike = Volume::strike(Point { x: 329, y: 154 });
        for (horizontal, depth, narrow, expected) in [
            (300, 153, false, true),
            (320, 153, false, true),
            (292, 153, true, true),
            (284, 153, true, true),
            (282, 153, true, false),
            (280, 153, true, false),
            (278, 153, true, false),
            (276, 153, true, false),
            (274, 153, true, false),
            (300, 161, true, true),
            (300, 163, true, true),
            (300, 165, true, false),
            (300, 167, true, false),
            (300, 169, true, false),
            (300, 177, true, false),
            (300, 179, true, false),
        ] {
            assert_eq!(
                strike.overlaps(&Volume::station(
                    Point { x: horizontal, y: depth },
                    narrow
                )),
                expected,
                "{horizontal} {depth}"
            );
        }
    }

    #[test]
    fn all_three_axes_include_touching_boundaries() {
        let origin = Volume {
            center: [0; 3],
            radii: [2; 3],
        };
        for center in [[4, 4, 4], [-4, -4, -4]] {
            assert!(origin.overlaps(&Volume { center, radii: [2; 3] }));
        }
        for center in [[5, 0, 0], [0, 5, 0], [0, 0, 5]] {
            assert!(!origin.overlaps(&Volume { center, radii: [2; 3] }));
        }
    }

    #[test]
    fn descending_leap_reaches_inclusive_horizontal_boundary() {
        let station = Volume::station(Point { x: 172, y: 207 }, false);
        let before = Volume::leap(Point { x: 206, y: 207 }, 30);
        let contact = Volume::leap(Point { x: 204, y: 207 }, 28);
        assert!(!before.overlaps(&station));
        assert!(contact.overlaps(&station));
        assert!(!Volume::leap(Point { x: 204, y: 220 }, 28).overlaps(&station));
        assert!(!Volume::leap(Point { x: 204, y: 207 }, 33).overlaps(&station));
    }
}
