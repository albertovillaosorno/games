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
//   - Fixed-step second-stage linked encounter and distinct child recovery.
// - Must-Not:
//   - Render, interpret source media or perform host I/O.
// - Allows:
//   - Inputs: Camera, enemy counts, station state and street bounds.
//   - Outputs: Observable parent and child states and resolved hits.
//   - Side effects: In-memory transitions only.
// - Split-When:
//   - Additional linked enemy strategies require independent ownership.
// - Merge-When:
//   - Another module duplicates the same linked encounter schedule.
// - Summary:
//   - Bounded optional linked encounters on the second-stage boardwalk.
// - Description:
//   - Reuses regular combat with separately timed admission and recovery.
// - Usage:
//   - Enable only with the corresponding second-stage art.
// - Defaults:
//   - Disabled; interaction parity remains provisional.

//! Optional second-stage linked encounter.

use super::{
    Enemy, EnemyAction, LINKED_CHILD_CAPACITY as CHILD_CAPACITY,
    LINKED_PARENT_CAPACITY as PARENT_CAPACITY, LinkedEncounter, Point, Slot,
    StreetLayout,
};

const ACTIVATION: i16 = 1248;
const RECOVERY: u8 = 40;
const CHILD_HEALTH: u8 = 12;
const CHILD_DAMAGE: u8 = 4;
const GROUND: i16 = 132;

const SCHEDULES: [Schedule; PARENT_CAPACITY] = [
    Schedule {
        admission: 1042,
        delays: &[16, 32],
        initial: 48,
        x: 1376,
    },
    Schedule {
        admission: 1106,
        delays: &[64, 64, 32],
        initial: 16,
        x: 1440,
    },
    Schedule {
        admission: 1170,
        delays: &[128, 224],
        initial: 192,
        x: 1504,
    },
];

type ChildHits<'slots> = (&'slots mut [Slot; 4], [bool; CHILD_CAPACITY]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Schedule {
    admission: i16,
    delays: &'static [u8],
    initial: u8,
    x: i16,
}

/// Sparse verified appearance of a linked parent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParentPose {
    /// Closed entry.
    Closed,
    /// Intermediate opening or closing.
    Intermediate,
    /// Open entry.
    Open,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentPhase {
    Closing(u16),
    Countdown(u8),
    Dormant,
    Hold(u8),
    Opening { remaining: u8, terminal: bool },
    Retired,
    Waiting,
}

/// Observable linked parent placement and animation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkedParent {
    next: usize,
    phase: ParentPhase,
    schedule: Schedule,
}

impl LinkedParent {
    fn advance(&mut self, camera: i16, counted: usize) -> bool {
        self.phase = match self.phase {
            ParentPhase::Dormant if camera >= self.schedule.admission => {
                ParentPhase::Waiting
            },
            ParentPhase::Waiting if camera >= ACTIVATION => {
                ParentPhase::Countdown(self.schedule.initial)
            },
            ParentPhase::Countdown(0 | 1) if counted <= 4 => {
                let terminal = self.schedule.delays.get(self.next).is_none();
                self.phase = ParentPhase::Opening { remaining: 9, terminal };
                return !terminal;
            },
            ParentPhase::Countdown(ticks) => {
                ParentPhase::Countdown(ticks.saturating_sub(1).max(1))
            },
            ParentPhase::Opening {
                remaining: 1,
                terminal: true,
            } => ParentPhase::Retired,
            ParentPhase::Opening {
                remaining: 1,
                terminal: false,
            } => ParentPhase::Hold(96),
            ParentPhase::Opening { remaining, terminal } => {
                ParentPhase::Opening {
                    remaining: remaining.saturating_sub(1),
                    terminal,
                }
            },
            ParentPhase::Hold(1) => ParentPhase::Closing(285),
            ParentPhase::Hold(ticks) => {
                ParentPhase::Hold(ticks.saturating_sub(1))
            },
            ParentPhase::Closing(1) => {
                let delay = self
                    .schedule
                    .delays
                    .get(self.next)
                    .copied()
                    .unwrap_or_default();
                self.next = self.next.saturating_add(1);
                ParentPhase::Countdown(delay)
            },
            ParentPhase::Closing(ticks) => {
                ParentPhase::Closing(ticks.saturating_sub(1))
            },
            phase @ (ParentPhase::Dormant
            | ParentPhase::Waiting
            | ParentPhase::Retired) => phase,
        };
        false
    }

    const fn new(schedule: Schedule) -> Self {
        Self {
            next: 0,
            phase: ParentPhase::Dormant,
            schedule,
        }
    }

    /// Current sparse appearance, absent outside its admitted lifetime.
    #[must_use]
    pub const fn pose(self) -> Option<ParentPose> {
        match self.phase {
            ParentPhase::Dormant | ParentPhase::Retired => None,
            ParentPhase::Waiting | ParentPhase::Countdown(_) => {
                Some(ParentPose::Closed)
            },
            ParentPhase::Opening { .. } | ParentPhase::Closing(277..=285) => {
                Some(ParentPose::Intermediate)
            },
            ParentPhase::Hold(_) => Some(ParentPose::Open),
            ParentPhase::Closing(_) => Some(ParentPose::Closed),
        }
    }

    /// Horizontal world anchor.
    #[must_use]
    pub const fn x(self) -> i16 {
        self.schedule.x
    }
}

/// Sparse verified appearance of the linked child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildPose {
    /// Active ground behavior.
    Active,
    /// Initial emergence.
    Emerging,
    /// Second emergence hold.
    Holding,
    /// Hit recovery (also used provisionally for defeat).
    Hurt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildPhase {
    Defeated(u8),
    Entering(u8),
    Fighting,
    Recovering(u8),
    Retired,
}

/// Observable state of a separately counted linked child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkedChild {
    enemy: Enemy,
    phase: ChildPhase,
}

impl LinkedChild {
    fn advance(&mut self, mut layout: StreetLayout, slots: &mut [Slot; 4]) {
        layout.min.y = layout.min.y.min(GROUND);
        self.phase = match self.phase {
            ChildPhase::Entering(99) => ChildPhase::Fighting,
            ChildPhase::Entering(age) => {
                ChildPhase::Entering(age.saturating_add(1))
            },
            ChildPhase::Fighting => {
                self.enemy.advance(layout, slots, (false, false));
                ChildPhase::Fighting
            },
            ChildPhase::Recovering(age) => {
                if age.saturating_add(1) >= RECOVERY {
                    self.enemy.action = EnemyAction::Approach { tick: 0 };
                    ChildPhase::Fighting
                } else {
                    ChildPhase::Recovering(age.saturating_add(1))
                }
            },
            ChildPhase::Defeated(47) => {
                self.enemy.award_credit(slots);
                ChildPhase::Retired
            },
            ChildPhase::Defeated(age) => {
                ChildPhase::Defeated(age.saturating_add(1))
            },
            ChildPhase::Retired => ChildPhase::Retired,
        };
    }

    /// Whether the child is vulnerable to a player strike.
    #[must_use]
    pub const fn can_fight(self) -> bool {
        matches!(self.phase, ChildPhase::Fighting)
    }

    /// World position, facing and health of the child.
    #[must_use]
    pub const fn enemy(self) -> Enemy {
        self.enemy
    }

    const fn new(x: i16) -> Self {
        let mut enemy = Enemy::new(Point { x, y: GROUND });
        enemy.health = CHILD_HEALTH;
        Self {
            enemy,
            phase: ChildPhase::Entering(0),
        }
    }

    /// Current sparse appearance; retired or blinking children are hidden.
    #[must_use]
    pub const fn pose(self) -> Option<ChildPose> {
        match self.phase {
            ChildPhase::Entering(0..=65) => Some(ChildPose::Emerging),
            ChildPhase::Entering(_) => Some(ChildPose::Holding),
            ChildPhase::Fighting => Some(ChildPose::Active),
            ChildPhase::Recovering(_) | ChildPhase::Defeated(0..=32) => {
                Some(ChildPose::Hurt)
            },
            ChildPhase::Defeated(age) if !age.is_multiple_of(2) => {
                Some(ChildPose::Hurt)
            },
            ChildPhase::Defeated(_) | ChildPhase::Retired => None,
        }
    }

    fn resolve_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        if !self.can_fight() || !self.enemy.receive_hit(slots, CHILD_DAMAGE) {
            return false;
        }
        self.phase = if self.enemy.health == 0 {
            ChildPhase::Defeated(0)
        } else {
            ChildPhase::Recovering(0)
        };
        true
    }
}

impl LinkedEncounter {
    pub(super) fn advance(
        &mut self,
        layout: StreetLayout,
        (slots, hits): ChildHits<'_>,
        (camera, regular_count): (i16, usize),
    ) -> u64 {
        let mut defeated = 0u64;
        for (entry, hit) in self.children.iter_mut().zip(hits) {
            let Some(child) = entry else {
                continue;
            };
            if hit {
                defeated =
                    defeated.saturating_add(u64::from(child.enemy.health == 0));
            } else {
                child.advance(layout, slots);
            }
        }
        let mut counted = regular_count.saturating_add(
            self.children()
                .filter(|child| child.phase != ChildPhase::Retired)
                .count(),
        );
        for parent in &mut self.parents {
            if parent.advance(camera, counted)
                && let Some(entry) =
                    self.children.iter_mut().find(|entry| entry.is_none())
            {
                *entry = Some(LinkedChild::new(parent.x()));
                counted = counted.saturating_add(1);
            }
        }
        defeated
    }

    pub(super) fn children(&self) -> impl Iterator<Item = LinkedChild> + '_ {
        self.children.iter().flatten().copied()
    }

    // A diagnostic player waits for the finite encounter rather than
    // walking past an actor that is still emerging or recovering.
    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn diagnostic_wait(&self, camera: i16) -> bool {
        camera >= ACTIVATION
            && (self
                .parents()
                .any(|parent| parent.phase != ParentPhase::Retired)
                || self
                    .children()
                    .any(|child| child.phase != ChildPhase::Retired))
    }

    pub(super) const fn new() -> Self {
        let [first, second, third] = SCHEDULES;
        Self {
            children: [None; CHILD_CAPACITY],
            parents: [
                LinkedParent::new(first),
                LinkedParent::new(second),
                LinkedParent::new(third),
            ],
        }
    }

    pub(super) fn parents(&self) -> impl Iterator<Item = LinkedParent> + '_ {
        self.parents.iter().copied()
    }

    pub(super) fn resolve_strikes(
        &mut self,
        slots: &mut [Slot; 4],
    ) -> [bool; CHILD_CAPACITY] {
        self.children.each_mut().map(|entry| {
            entry
                .as_mut()
                .is_some_and(|child| child.resolve_strikes(slots))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_parent_wait(
        parent: &mut LinkedParent,
        camera: i16,
        count: usize,
        ticks: u16,
    ) {
        for _ in 0..ticks {
            assert!(!parent.advance(camera, count));
        }
    }

    #[test]
    fn parent_waits_for_travel_and_primary_capacity() {
        let [_, schedule, _] = SCHEDULES;
        let mut parent = LinkedParent::new(schedule);
        assert!(!parent.advance(1105, 0));
        assert_eq!(parent.pose(), None);
        assert!(!parent.advance(1106, 0));
        assert_parent_wait(&mut parent, 1247, 0, 100);
        assert!(!parent.advance(1248, 5));
        assert_parent_wait(&mut parent, 1248, 5, 100);
        assert!(parent.advance(1248, 4));
        assert_eq!(parent.pose(), Some(ParentPose::Intermediate));
        assert_parent_wait(&mut parent, 1248, 0, 9);
        assert_eq!(parent.pose(), Some(ParentPose::Open));
        assert_parent_wait(&mut parent, 1248, 0, 96);
        assert_eq!(parent.pose(), Some(ParentPose::Intermediate));
    }

    #[test]
    fn finite_parent_schedules_admit_seven_children() {
        let mut parents = SCHEDULES.map(LinkedParent::new);
        let mut spawned = 0usize;
        for _ in 0u16..3000 {
            for parent in &mut parents {
                spawned = spawned
                    .saturating_add(usize::from(parent.advance(1248, 0)));
            }
        }
        assert_eq!(spawned, CHILD_CAPACITY);
        assert!(parents.iter().all(|parent| parent.pose().is_none()));
    }

    fn advance_child(
        child: &mut LinkedChild,
        slots: &mut [Slot; 4],
        ticks: u8,
    ) {
        for _ in 0..ticks {
            child.advance(StreetLayout::DOWNTOWN, slots);
        }
    }

    fn assert_child_recovery(
        child: &mut LinkedChild,
        slots: &mut [Slot; 4],
        health: u8,
    ) {
        assert!(child.resolve_strikes(slots));
        assert_eq!(child.enemy().health, health);
        assert_eq!(child.pose(), Some(ChildPose::Hurt));
        for _ in 0..RECOVERY {
            assert!(!child.resolve_strikes(slots));
            child.advance(StreetLayout::DOWNTOWN, slots);
        }
        assert!(child.can_fight());
    }

    #[test]
    fn child_emerges_then_requires_three_separated_strikes() {
        use crate::{Facing, Maneuver, PLAYER_STRIKE, Participation};
        let mut child = LinkedChild::new(150);
        let layout = StreetLayout::DOWNTOWN;
        let mut slots = layout.spawn.map(Slot::new);
        let [marge, ..] = &mut slots;
        marge.station.participation = Participation::Playing;
        marge.station.position = Point { x: 130, y: GROUND };
        marge.station.facing = Facing::Right;
        marge.station.maneuver = Maneuver::Attack {
            moving: false,
            tick: PLAYER_STRIKE,
        };
        assert!(!child.resolve_strikes(&mut slots));
        advance_child(&mut child, &mut slots, 66);
        assert_eq!(child.pose(), Some(ChildPose::Holding));
        advance_child(&mut child, &mut slots, 34);
        assert!(child.can_fight());
        assert_child_recovery(&mut child, &mut slots, 8);
        assert_child_recovery(&mut child, &mut slots, 4);
        assert!(child.resolve_strikes(&mut slots));
        assert_eq!(child.enemy().health, 0);
        advance_child(&mut child, &mut slots, 48);
        assert_eq!(child.pose(), None);
        let [winner, ..] = slots;
        assert_eq!(winner.station.score, 1);
    }
}
