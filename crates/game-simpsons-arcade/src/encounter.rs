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
const ENEMY_STRIKE: u8 = 14;
const ENEMY_DAMAGE: u8 = 3;
/// Number of fixed-step ticks in the initial enemy punch.
pub const ENEMY_ATTACK_TICKS: u8 = 34;
/// Number of fixed-step ticks in the initial enemy walking cycle.
pub const ENEMY_WALK_TICKS: u8 = 48;
const STAGGER_TICKS: u8 = 12;

/// Observable enemy action in the experimental encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnemyAction {
    /// Moving toward a nearby active player.
    Approach {
        /// Zero-based walking cycle tick.
        tick: u8,
    },
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
    /// Initial spawn position, used when all stations expire.
    pub spawn: Point,
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
            EnemyAction::Approach { tick } => {
                self.approach(layout, slots, tick);
            },
            EnemyAction::Attack { tick } => {
                self.attack(tick, slots);
            },
            EnemyAction::Stagger { remaining } => {
                self.action = if remaining <= 1 {
                    EnemyAction::Approach { tick: 0 }
                } else {
                    EnemyAction::Stagger {
                        remaining: remaining.saturating_sub(1),
                    }
                };
            },
        }
    }

    fn approach(&mut self, layout: StreetLayout, slots: &[Slot; 4], tick: u8) {
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
        let before = self.position;
        self.position.x = step_toward(self.position.x, target.x)
            .clamp(layout.min.x, layout.max.x);
        self.position.y = step_toward(self.position.y, target.y)
            .clamp(layout.min.y, layout.max.y);
        if self.position != before {
            self.action = EnemyAction::Approach {
                tick: if tick.saturating_add(1) >= ENEMY_WALK_TICKS {
                    0
                } else {
                    tick.saturating_add(1)
                },
            };
        }
    }

    fn attack(&mut self, tick: u8, slots: &mut [Slot; 4]) {
        let next = tick.saturating_add(1);
        if next == ENEMY_STRIKE {
            for slot in slots.iter_mut().filter(|slot| {
                slot.station.can_fight()
                    && !slot.station.maneuver.airborne()
                    && in_range(
                        self.position,
                        self.facing,
                        slot.station.position,
                        (22, 10),
                    )
            }) {
                slot.damage(ENEMY_DAMAGE);
            }
        }
        self.action = if next >= ENEMY_ATTACK_TICKS {
            EnemyAction::Approach { tick: 0 }
        } else {
            EnemyAction::Attack { tick: next }
        };
    }

    pub(super) const fn new(position: Point) -> Self {
        Self {
            action: EnemyAction::Approach { tick: 0 },
            facing: Facing::Left,
            health: ENEMY_HEALTH,
            position,
            spawn: position,
        }
    }

    fn receive_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        let before = self.health;
        for slot in slots {
            let station = &mut slot.station;
            if station.can_fight()
                && in_range(
                    station.position,
                    station.facing,
                    self.position,
                    (32, 12),
                )
                && (matches!(station.maneuver, Maneuver::Attack {
                    tick: PLAYER_STRIKE,
                    ..
                }) || station.maneuver.consume_air_strike())
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

    pub(super) const fn restart(self) -> Self {
        Self::new(self.spawn)
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
    use crate::{
        Action, CONTINUE_TICKS, Cabinet, Control, LayoutError, PLAYER_HEALTH,
        PLAYER_STOCKS, Participation, Phase, Player,
    };

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
        cabinet.input_step(2, &pulse(Control::Start));
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
        cabinet.input_step(1, &[]);
        cabinet.input_step(2, &[]);
        assert_eq!(cabinet.enemy().map(|enemy| enemy.position), Some(spawn));
        cabinet.input_step(3, &pulse(Control::Start));
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
        for (start, health) in [(3u64, 2), (23, 1), (43, 0)] {
            cabinet.input_step(start, &pulse(Control::Primary));
            for tick in start.saturating_add(1)..start.saturating_add(20) {
                cabinet.input_step(tick, &[]);
            }
            assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(health));
        }
        let defeated = cabinet.enemy();
        for tick in 63..120 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy(), defeated);
        assert_eq!(
            cabinet.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH)
        );
    }

    #[test]
    fn moving_ground_attack_retains_one_strike() {
        let result = playing(Point { x: 132, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [press, release] = pulse(Control::Right);
        cabinet.input_step(3, &pulse(Control::Primary));
        cabinet.input_step(4, &[press]);
        cabinet.input_step(5, &[release]);
        assert_eq!(
            cabinet.stations().first().map(|slot| slot.maneuver),
            Some(Maneuver::Attack { moving: true, tick: 2 })
        );
        for tick in 6..23 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(2));
    }

    #[test]
    fn aerial_attack_hits_only_once_per_flight() {
        let result = playing(Point { x: 132, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(3, &pulse(Control::Secondary));
        for tick in 4..8 {
            cabinet.input_step(tick, &[]);
        }
        cabinet.input_step(8, &pulse(Control::Primary));
        for tick in 9..30 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(2));
        assert_eq!(
            cabinet.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH)
        );
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
    fn enemy_punch_damages_once_and_repeats_after_recovery() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 0..ENEMY_ATTACK_TICKS {
            assert_eq!(
                cabinet.enemy().map(|enemy| enemy.action),
                Some(EnemyAction::Attack { tick })
            );
            let expected = if tick < ENEMY_STRIKE {
                PLAYER_HEALTH
            } else {
                PLAYER_HEALTH.saturating_sub(ENEMY_DAMAGE)
            };
            assert_eq!(
                cabinet.stations().first().map(|slot| slot.health),
                Some(expected)
            );
            cabinet.input_step(u64::from(tick).saturating_add(3), &[]);
        }
        assert_eq!(
            cabinet.enemy().map(|enemy| enemy.action),
            Some(EnemyAction::Approach { tick: 0 })
        );
        cabinet.input_step(37, &[]);
        assert_eq!(
            cabinet.enemy().map(|enemy| enemy.action),
            Some(EnemyAction::Attack { tick: 0 })
        );
    }

    #[test]
    fn enemy_walk_cycle_wraps_and_freezes_during_pause() {
        let result = playing(Point { x: 250, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for step in 1..=ENEMY_WALK_TICKS {
            let tick = if step == ENEMY_WALK_TICKS {
                0
            } else {
                step
            };
            assert_eq!(
                cabinet.enemy().map(|enemy| enemy.action),
                Some(EnemyAction::Approach { tick })
            );
            cabinet.input_step(u64::from(step).saturating_add(2), &[]);
        }
        let walking = cabinet.enemy();
        cabinet.input_step(51, &pulse(Control::Pause));
        for step in 52..60 {
            cabinet.input_step(step, &[]);
        }
        assert_eq!(cabinet.enemy(), walking);
        cabinet.input_step(60, &pulse(Control::Pause));
        assert_eq!(
            cabinet.enemy().map(|enemy| enemy.action),
            Some(EnemyAction::Approach { tick: 2 })
        );
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
        for tick in 4..17 {
            grounded.input_step(tick, &[]);
            jumping.input_step(tick, &[]);
        }
        assert_eq!(
            grounded.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH.saturating_sub(ENEMY_DAMAGE))
        );
        assert_eq!(
            jumping.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH)
        );
    }

    #[test]
    fn knocked_out_station_cannot_move_or_attack() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 3..1600 {
            if matches!(
                cabinet
                    .stations()
                    .first()
                    .map(|station| station.participation),
                Some(Participation::Continuing { .. })
            ) {
                break;
            }
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
        let [previous, ..] = before;
        let [current, ..] = cabinet.stations();
        assert_eq!(
            (
                current.position,
                current.health,
                current.maneuver,
                current.attack_cues
            ),
            (previous.position, 0, Maneuver::Idle, previous.attack_cues),
        );
        assert!(matches!(
            current.participation,
            Participation::Continuing { .. }
        ));
    }

    fn knocked_out() -> Result<Cabinet, LayoutError> {
        let mut cabinet = playing(Point { x: 128, y: 190 })?;
        for tick in 3..1600 {
            let [marge, ..] = cabinet.stations();
            if matches!(marge.participation, Participation::Continuing { .. }) {
                break;
            }
            cabinet.input_step(tick, &[]);
        }
        Ok(cabinet)
    }

    #[test]
    fn free_continue_requires_only_a_new_start() {
        let result = knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [initial, homer, bart, lisa] = cabinet.stations();
        assert_eq!(initial.participation, Participation::Continuing {
            ticks_left: CONTINUE_TICKS,
        });
        for tick in 100..112 {
            cabinet.input_step(tick, &[]);
        }
        let [waiting, ..] = cabinet.stations();
        assert_eq!(
            (waiting.health, waiting.participation),
            (0, Participation::Continuing {
                ticks_left: CONTINUE_TICKS.saturating_sub(12),
            })
        );
        cabinet.input_step(112, &pulse(Control::Start));
        let [continued, other_homer, other_bart, other_lisa] =
            cabinet.stations();
        assert_eq!(
            (
                continued.participation,
                continued.health,
                continued.position,
                continued.attack_cues
            ),
            (
                Participation::Playing,
                PLAYER_HEALTH,
                Point { x: 110, y: 190 },
                initial.attack_cues
            ),
        );
        assert_eq!([other_homer, other_bart, other_lisa], [homer, bart, lisa]);
    }

    #[test]
    fn expiry_resets_empty_world_and_start_admits_a_new_game() {
        let result = knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 0..u64::from(CONTINUE_TICKS.saturating_sub(1)) {
            cabinet.input_step(tick, &[]);
        }
        let [last, ..] = cabinet.stations();
        assert_eq!(last.participation, Participation::Continuing {
            ticks_left: 1
        });
        assert_eq!(cabinet.phase(), Phase::Playing);
        cabinet.input_step(1200, &[]);
        assert_eq!(cabinet.phase(), Phase::MainMenu);
        assert_eq!(cabinet.enemy(), Some(Enemy::new(Point { x: 128, y: 190 })));
        cabinet.input_step(1201, &pulse(Control::Start));
        assert_eq!(cabinet.phase(), Phase::Playing);
        let [rejoined, ..] = cabinet.stations();
        assert_eq!(
            (rejoined.participation, rejoined.health),
            (Participation::Playing, PLAYER_HEALTH)
        );
    }

    #[test]
    fn one_station_can_continue_during_another_stations_play() {
        let layout = StreetLayout {
            spawn: [
                Point { x: 110, y: 190 },
                Point { x: 260, y: 146 },
                Point { x: 132, y: 178 },
                Point { x: 154, y: 178 },
            ],
            ..StreetLayout::DOWNTOWN
        };
        let result = Cabinet::with_encounter(layout, Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let homer = pulse(Control::Start).map(|event| LogicalAction {
            action: Action {
                player: Player::Homer,
                ..event.action
            },
            ..event
        });
        cabinet.input_step(2, &pulse(Control::Start));
        for tick in 3..1600 {
            let [marge, ..] = cabinet.stations();
            if matches!(marge.participation, Participation::Continuing { .. }) {
                break;
            }
            cabinet.input_step(tick, &[]);
        }
        cabinet.input_step(99, &homer);
        let [knocked_out, before, ..] = cabinet.stations();
        assert_eq!(knocked_out.health, 0);
        assert_eq!(
            (before.participation, before.health),
            (Participation::Playing, PLAYER_HEALTH)
        );
        cabinet.input_step(100, &pulse(Control::Start));
        let [continued, after, ..] = cabinet.stations();
        assert_eq!(
            (continued.health, continued.participation),
            (PLAYER_HEALTH, Participation::Playing)
        );
        assert_eq!(after, before);
    }

    #[test]
    fn held_start_cannot_automatically_continue_after_knockout() {
        let result = Cabinet::with_encounter(StreetLayout::DOWNTOWN, Point {
            x: 128,
            y: 190,
        });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let action = Action {
            control: Control::Start,
            player: Player::Marge,
        };
        cabinet.input_step(0, &[LogicalAction { action, down: true }]);
        for tick in 1..1600 {
            if cabinet.stations().first().is_some_and(|slot| {
                matches!(slot.participation, Participation::Continuing { .. })
            }) {
                break;
            }
            cabinet.input_step(tick, &[]);
        }
        cabinet.input_step(100, &[LogicalAction { action, down: true }]);
        let [waiting, ..] = cabinet.stations();
        assert_eq!(waiting.health, 0);
        cabinet.input_step(101, &[LogicalAction { action, down: false }]);
        cabinet.input_step(102, &pulse(Control::Start));
        let [continued, ..] = cabinet.stations();
        assert_eq!(continued.health, PLAYER_HEALTH);
    }

    #[test]
    fn start_on_final_tick_continues_the_same_game() {
        let result = knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 0..u64::from(CONTINUE_TICKS.saturating_sub(1)) {
            cabinet.input_step(tick, &[]);
        }
        let [waiting, ..] = cabinet.stations();
        assert_eq!(waiting.participation, Participation::Continuing {
            ticks_left: 1,
        });
        cabinet.input_step(1201, &pulse(Control::Start));
        let [continued, ..] = cabinet.stations();
        assert_eq!(cabinet.phase(), Phase::Playing);
        assert_eq!(continued.participation, Participation::Playing);
        assert_eq!(continued.health, PLAYER_HEALTH);
    }

    #[test]
    fn meter_drains_at_observed_cadence_and_pause_freezes_it() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 3..17 {
            cabinet.input_step(tick, &[]);
        }
        let [damaged, ..] = cabinet.stations();
        assert_eq!(damaged.health, PLAYER_HEALTH - ENEMY_DAMAGE);
        assert_eq!(damaged.display_health, PLAYER_HEALTH);
        assert_eq!(damaged.maneuver, Maneuver::Hurt { tick: 0 });
        cabinet.input_step(17, &[]);
        let [first, ..] = cabinet.stations();
        assert_eq!(first.display_health, PLAYER_HEALTH - 1);
        cabinet.input_step(18, &[]);
        let [between, ..] = cabinet.stations();
        assert_eq!(between, crate::Station {
            maneuver: Maneuver::Hurt { tick: 2 },
            ..first
        });
        check_meter_pause(&mut cabinet, between);
    }

    fn check_meter_pause(cabinet: &mut Cabinet, between: crate::Station) {
        cabinet.input_step(19, &pulse(Control::Pause));
        for tick in 20..100 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().first(), Some(&between));
        cabinet.input_step(100, &pulse(Control::Pause));
        let [second, ..] = cabinet.stations();
        assert_eq!(second.display_health, PLAYER_HEALTH - 2);
        assert_eq!(second.maneuver, Maneuver::Hurt { tick: 3 });
    }

    fn advance_until(
        cabinet: &mut Cabinet,
        predicate: impl Fn(crate::Station) -> bool,
    ) -> crate::Station {
        for tick in 0..1600 {
            let [station, ..] = cabinet.stations();
            if predicate(station) {
                return station;
            }
            cabinet.input_step(tick, &[]);
        }
        let [station, ..] = cabinet.stations();
        assert!(predicate(station));
        station
    }

    #[test]
    fn first_stock_refills_without_start_before_final_continue() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let empty = advance_until(&mut cabinet, |station| {
            station.stocks == 1 && station.display_health == 0
        });
        assert_eq!(empty.health, 0);
        assert!(!empty.can_fight());
        assert!(matches!(
            empty.participation,
            Participation::Recovering { .. }
        ));
        cabinet.input_step(0, &pulse(Control::Start));
        let [still_empty, ..] = cabinet.stations();
        assert_eq!(still_empty.stocks, 1);
        assert_eq!(still_empty.health, 0);
        verify_refill_and_continue(&mut cabinet);
    }

    fn verify_refill_and_continue(cabinet: &mut Cabinet) {
        let refill = advance_until(cabinet, |station| {
            station.display_health == PLAYER_HEALTH.div_ceil(2)
                && station.health == PLAYER_HEALTH
        });
        assert_eq!(refill.stocks, 1);
        assert!(!refill.can_fight());
        let recovered = advance_until(cabinet, crate::Station::can_fight);
        assert_eq!(recovered.display_health, PLAYER_HEALTH);
        assert_eq!(recovered.stocks, 1);
        let final_empty = advance_until(cabinet, |station| {
            matches!(station.participation, Participation::Continuing { .. })
        });
        assert_eq!(final_empty.stocks, 0);
        assert_eq!(final_empty.display_health, 0);
        cabinet.input_step(0, &pulse(Control::Start));
        let [continued, ..] = cabinet.stations();
        assert_eq!(continued.stocks, PLAYER_STOCKS);
        assert_eq!(continued.display_health, PLAYER_HEALTH);
    }

    #[test]
    fn paused_world_keeps_continue_opportunity_and_enemy_state() {
        let result = knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(100, &pulse(Control::Pause));
        assert_eq!(cabinet.phase(), Phase::Paused);
        let frozen = cabinet.stations();
        let enemy = cabinet.enemy();
        for tick in 101..1500 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations(), frozen);
        assert_eq!(cabinet.enemy(), enemy);
        cabinet.input_step(1500, &pulse(Control::Pause));
        assert_eq!(cabinet.phase(), Phase::Playing);
        cabinet.input_step(1501, &pulse(Control::Start));
        assert_eq!(
            cabinet.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH)
        );
    }
}
