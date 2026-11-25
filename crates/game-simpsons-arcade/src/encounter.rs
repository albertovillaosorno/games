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
//   - First-street enemy approach, strike, stagger and health rules.
// - Must-Not:
//   - Render, interpret original media or perform host I/O.
// - Allows:
//   - Inputs: Fixed-step station state and walkable bounds.
//   - Outputs: Enemy state and deterministic station damage.
//   - Side effects: In-memory gameplay transitions only.
// - Split-When:
//   - Distinct enemy strategies or stage waves require separate ownership.
// - Merge-When:
//   - Another title-local module duplicates encounter hit resolution.
// - Summary:
//   - One independently authored, provisional combat encounter.
// - Description:
//   - Resolves strikes once at defined ticks and respects facing and depth.
// - Usage:
//   - Opt in through Cabinet::with_encounter for the initial street.
// - Defaults:
//   - One approaching enemy; no original combat parity claim.

//! Provisional fixed-step encounter for the first street.

use super::{Facing, Maneuver, Point, Slot, StreetLayout};

const ENEMY_HEALTH: u8 = 3;
const PLAYER_STRIKE: u8 = 5;
const ENEMY_STRIKE: u8 = 8;
const ENEMY_RECOVERY: u8 = 18;
const STAGGER_TICKS: u8 = 12;

/// Observable enemy action in the experimental encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnemyAction {
    /// Moving toward a nearby active player.
    Approach,
    /// Windup and recovery of a ground strike.
    Attack {
        /// Zero-based attack tick.
        tick: u8,
    },
    /// Temporarily interrupted by a player strike.
    Stagger {
        /// Remaining recovery ticks.
        remaining: u8,
    },
}

/// Read-only state of the initial enemy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Enemy {
    /// Current action; a zero-health enemy no longer acts.
    pub action: EnemyAction,
    /// Current horizontal facing.
    pub facing: Facing,
    /// Remaining health; zero means defeated.
    pub health: u8,
    /// Ground position, independent of art dimensions.
    pub position: Point,
}

impl Enemy {
    pub(super) fn advance(
        &mut self,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
    ) {
        if self.health == 0 {
            return;
        }
        if self.receive_strikes(slots) {
            return;
        }
        match self.action {
            EnemyAction::Approach => self.approach(layout, slots),
            EnemyAction::Attack { tick } => {
                self.attack(tick, slots);
            },
            EnemyAction::Stagger { remaining } => {
                self.action = if remaining <= 1 {
                    EnemyAction::Approach
                } else {
                    EnemyAction::Stagger {
                        remaining: remaining.saturating_sub(1),
                    }
                };
            },
        }
    }

    fn approach(&mut self, layout: StreetLayout, slots: &[Slot; 4]) {
        let Some(target) = slots
            .iter()
            .filter(|slot| slot.station.can_fight())
            .min_by_key(|slot| distance(self.position, slot.station.position))
            .map(|slot| slot.station.position)
        else {
            return;
        };
        self.facing = if target.x < self.position.x {
            Facing::Left
        } else {
            Facing::Right
        };
        if in_range(self.position, self.facing, target, (18, 8)) {
            self.action = EnemyAction::Attack { tick: 0 };
            return;
        }
        self.position.x = step_toward(self.position.x, target.x)
            .clamp(layout.min.x, layout.max.x);
        self.position.y = step_toward(self.position.y, target.y)
            .clamp(layout.min.y, layout.max.y);
    }

    fn attack(&mut self, tick: u8, slots: &mut [Slot; 4]) {
        let next = tick.saturating_add(1);
        if next == ENEMY_STRIKE {
            for slot in slots.iter_mut().filter(|slot| {
                slot.station.can_fight()
                    && !matches!(slot.station.maneuver, Maneuver::Jump { .. })
                    && in_range(
                        self.position,
                        self.facing,
                        slot.station.position,
                        (22, 10),
                    )
            }) {
                slot.station.health = slot.station.health.saturating_sub(1);
                slot.station.maneuver = Maneuver::Idle;
                slot.station.walking_ticks = None;
            }
        }
        self.action = if next >= ENEMY_RECOVERY {
            EnemyAction::Approach
        } else {
            EnemyAction::Attack { tick: next }
        };
    }

    pub(super) const fn new(position: Point) -> Self {
        Self {
            action: EnemyAction::Approach,
            facing: Facing::Left,
            health: ENEMY_HEALTH,
            position,
        }
    }

    fn receive_strikes(&mut self, slots: &[Slot; 4]) -> bool {
        let before = self.health;
        for slot in slots {
            let station = slot.station;
            if station.can_fight()
                && station.maneuver
                    == (Maneuver::Attack { tick: PLAYER_STRIKE })
                && in_range(
                    station.position,
                    station.facing,
                    self.position,
                    (32, 12),
                )
            {
                self.health = self.health.saturating_sub(1);
            }
        }
        if before == self.health {
            return false;
        }
        self.action = EnemyAction::Stagger { remaining: STAGGER_TICKS };
        true
    }
}

fn distance(first: Point, second: Point) -> u32 {
    u32::from(first.x.abs_diff(second.x))
        .saturating_add(u32::from(first.y.abs_diff(second.y)))
}

const fn in_range(
    from: Point,
    facing: Facing,
    to: Point,
    extent: (u16, u16),
) -> bool {
    let ahead = match facing {
        Facing::Left => to.x <= from.x,
        Facing::Right => to.x >= from.x,
    };
    ahead
        && from.x.abs_diff(to.x) <= extent.0
        && from.y.abs_diff(to.y) <= extent.1
}

const fn step_toward(value: i16, target: i16) -> i16 {
    if value < target {
        value.saturating_add(1)
    } else if value > target {
        value.saturating_sub(1)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use games::input::LogicalAction;
    use games::replay::InputStep as _;

    use super::*;
    use crate::{Action, Cabinet, Control, LayoutError, Player};

    fn pulse(control: Control) -> [LogicalAction<Action>; 2] {
        let action = Action {
            control,
            player: Player::Marge,
        };
        [LogicalAction { action, down: true }, LogicalAction {
            action,
            down: false,
        }]
    }

    fn playing(spawn: Point) -> Result<Cabinet, LayoutError> {
        let mut cabinet =
            Cabinet::with_encounter(StreetLayout::DOWNTOWN, spawn)?;
        for (tick, control) in [
            (0, Control::Coin),
            (1, Control::Start),
            (2, Control::Primary),
        ] {
            cabinet.input_step(tick, &pulse(control));
        }
        Ok(cabinet)
    }

    #[test]
    fn enemy_approaches_only_after_entry() {
        let spawn = Point { x: 250, y: 190 };
        let result = Cabinet::with_encounter(StreetLayout::DOWNTOWN, spawn);
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &[]);
        assert_eq!(cabinet.enemy().map(|enemy| enemy.position), Some(spawn));
        cabinet.input_step(1, &pulse(Control::Coin));
        cabinet.input_step(2, &pulse(Control::Start));
        assert_eq!(cabinet.enemy().map(|enemy| enemy.position), Some(spawn));
        cabinet.input_step(3, &pulse(Control::Primary));
        assert_eq!(
            cabinet.enemy().map(|enemy| enemy.position),
            Some(Point { x: 249, y: 190 })
        );
    }

    #[test]
    fn three_distinct_strikes_defeat_without_repeat() {
        let result = playing(Point { x: 132, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for (start, health) in [(3u64, 2), (20, 1), (37, 0)] {
            cabinet.input_step(start, &pulse(Control::Primary));
            for tick in start.saturating_add(1)..start.saturating_add(17) {
                cabinet.input_step(tick, &[]);
            }
            assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(health));
        }
        let defeated = cabinet.enemy();
        for tick in 54..120 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy(), defeated);
        assert_eq!(cabinet.stations().first().map(|slot| slot.health), Some(3));
    }

    #[test]
    fn strikes_miss_behind_or_at_another_depth() {
        for spawn in [Point { x: 90, y: 190 }, Point { x: 130, y: 146 }] {
            let result = playing(spawn);
            assert!(result.is_ok());
            let Ok(mut cabinet) = result else {
                return;
            };
            cabinet.input_step(3, &pulse(Control::Primary));
            for tick in 4..9 {
                cabinet.input_step(tick, &[]);
            }
            assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(3));
        }
    }

    #[test]
    fn jumping_dodges_ground_strike() {
        let spawn = Point { x: 128, y: 190 };
        let result = playing(spawn);
        assert!(result.is_ok());
        let Ok(mut grounded) = result else {
            return;
        };
        let mut jumping = grounded;
        jumping.input_step(3, &pulse(Control::Secondary));
        grounded.input_step(3, &[]);
        for tick in 4..11 {
            grounded.input_step(tick, &[]);
            jumping.input_step(tick, &[]);
        }
        assert_eq!(
            grounded.stations().first().map(|slot| slot.health),
            Some(2)
        );
        assert_eq!(jumping.stations().first().map(|slot| slot.health), Some(3));
    }

    #[test]
    fn knocked_out_station_cannot_move_or_attack() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 3..100 {
            cabinet.input_step(tick, &[]);
        }
        let before = cabinet.stations();
        assert_eq!(before.first().map(|slot| slot.health), Some(0));
        cabinet.input_step(100, &pulse(Control::Primary));
        cabinet.input_step(101, &[LogicalAction {
            action: Action {
                control: Control::Right,
                player: Player::Marge,
            },
            down: true,
        }]);
        for tick in 102..120 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations(), before);
    }
}
