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
//   - Optional first-stage wrestler encounter and victory state.
// - Must-Not:
//   - Render, decode reference media or implement the subsequent stage.
// - Allows:
//   - Inputs: Fixed playing ticks, walkable bounds and station maneuvers.
//   - Outputs: Wrestler state, station damage and one defeat event.
//   - Side effects: Deterministic in-memory gameplay transitions only.
// - Split-When:
//   - Additional independent boss strategies require separate ownership.
// - Merge-When:
//   - A shared boss contract replaces this title-local encounter.
// - Summary:
//   - Independently authored, provisional multi-hit wrestler fight.
// - Description:
//   - Holds the late arena until attack-dependent defeat; no parity claim.
// - Usage:
//   - Opt in to Cabinet::with_late_wrestler with the complete local art.
// - Defaults:
//   - No boss without opt-in; no subsequent-stage simulation.

//! Bounded experimental wrestler encounter.

use super::{
    Facing, Maneuver, Point, Slot, StreetLayout, WRESTLER_ATTACK_STRIKE,
    distance, hurt_slot, in_range, player_strike, step_toward,
};

// The entrance landmarks are approximate reference observations. The remaining
// constants are independent playability tuning, not recovered game rules.
const INTRO_VISIBLE: u16 = 135;
const INTRO_END: u16 = 251;
const HEALTH: u8 = 12;
const ATTACK_END: u8 = 48;
const STAGGER_END: u8 = 18;
const DEFEAT_END: u8 = 66;

/// Current fixed-step state of the optional wrestler encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WrestlerAction {
    /// Seeking an active station.
    Approach,
    /// Windup, strike and recovery.
    Attack {
        /// Elapsed fixed attack ticks.
        tick: u8,
    },
    /// Fight completed; subsequent stage is not yet implemented.
    Cleared,
    /// Final knockdown before disappearance.
    Defeat {
        /// Elapsed fixed defeat ticks.
        tick: u8,
    },
    /// Entrance time before the active fight.
    Intro {
        /// Elapsed playing ticks since arena admission.
        tick: u16,
    },
    /// Brief recovery after a successful player strike.
    Stagger {
        /// Elapsed fixed recovery ticks.
        tick: u8,
    },
}

/// Read-only wrestler state in world coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Wrestler {
    /// Current encounter action.
    pub action: WrestlerAction,
    /// Direction of the next or current strike.
    pub facing: Facing,
    /// Remaining provisional hit points.
    pub health: u8,
    /// Ground anchor used for combat and depth sorting.
    pub position: Point,
}

impl Wrestler {
    pub(super) fn advance(
        &mut self,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
    ) -> bool {
        if matches!(
            self.action,
            WrestlerAction::Approach | WrestlerAction::Attack { .. }
        ) && self.receive_strikes(slots)
        {
            return self.health == 0;
        }
        self.action = match self.action {
            WrestlerAction::Intro { tick } if tick < INTRO_END => {
                WrestlerAction::Intro {
                    tick: tick.saturating_add(1),
                }
            },
            WrestlerAction::Approach => self.approach(layout, slots),
            WrestlerAction::Attack { tick } => self.attack(tick, slots),
            WrestlerAction::Stagger { tick } if tick < STAGGER_END => {
                WrestlerAction::Stagger {
                    tick: tick.saturating_add(1),
                }
            },
            WrestlerAction::Intro { .. } | WrestlerAction::Stagger { .. } => {
                WrestlerAction::Approach
            },
            WrestlerAction::Defeat { tick } if tick < DEFEAT_END => {
                WrestlerAction::Defeat {
                    tick: tick.saturating_add(1),
                }
            },
            WrestlerAction::Defeat { .. } | WrestlerAction::Cleared => {
                WrestlerAction::Cleared
            },
        };
        false
    }

    fn approach(
        &mut self,
        layout: StreetLayout,
        slots: &[Slot; 4],
    ) -> WrestlerAction {
        let Some(target) = slots
            .iter()
            .filter(|slot| slot.station.can_fight())
            .min_by_key(|slot| distance(self.position, slot.station.position))
            .map(|slot| slot.station.position)
        else {
            return WrestlerAction::Approach;
        };
        self.facing = if target.x < self.position.x {
            Facing::Left
        } else {
            Facing::Right
        };
        if in_range(self.position, self.facing, target, (38, 12)) {
            return WrestlerAction::Attack { tick: 0 };
        }
        self.position.x = step_toward(self.position.x, target.x)
            .clamp(layout.min.x, layout.max.x);
        self.position.y = step_toward(self.position.y, target.y)
            .clamp(layout.min.y, layout.max.y);
        WrestlerAction::Approach
    }

    fn attack(&self, tick: u8, slots: &mut [Slot; 4]) -> WrestlerAction {
        let next = tick.saturating_add(1);
        if next == WRESTLER_ATTACK_STRIKE {
            for slot in slots.iter_mut().filter(|slot| {
                slot.station.can_fight()
                    && !slot.station.maneuver.airborne()
                    && slot.station.maneuver != Maneuver::Restrained
                    && in_range(
                        self.position,
                        self.facing,
                        slot.station.position,
                        (48, 16),
                    )
            }) {
                hurt_slot(slot, 6);
            }
        }
        if next >= ATTACK_END {
            WrestlerAction::Approach
        } else {
            WrestlerAction::Attack { tick: next }
        }
    }

    pub(super) const fn new(camera: i16) -> Self {
        Self {
            action: WrestlerAction::Intro { tick: 0 },
            facing: Facing::Right,
            health: HEALTH,
            position: Point {
                x: camera.saturating_add(160),
                y: 190,
            },
        }
    }

    fn receive_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        let mut struck = false;
        for slot in slots {
            if !player_strike(&mut slot.station, self.position, (42, 16)) {
                continue;
            }
            self.health = self.health.saturating_sub(1);
            struck = true;
            if self.health == 0 {
                self.action = WrestlerAction::Defeat { tick: 0 };
                return true;
            }
        }
        if struck {
            self.action = WrestlerAction::Stagger { tick: 0 };
        }
        struck
    }

    /// Whether the encounter contributes a visible sprite.
    #[must_use]
    pub const fn visible(self) -> bool {
        match self.action {
            WrestlerAction::Intro { tick } => tick >= INTRO_VISIBLE,
            WrestlerAction::Cleared => false,
            WrestlerAction::Defeat { tick } => {
                tick < 52 || !tick.is_multiple_of(2)
            },
            WrestlerAction::Approach
            | WrestlerAction::Attack { .. }
            | WrestlerAction::Stagger { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PLAYER_HEALTH, Participation};

    fn active_slots() -> [Slot; 4] {
        let mut slots = StreetLayout::DOWNTOWN.spawn.map(Slot::new);
        let [first, second, ..] = &mut slots;
        for slot in [first, second] {
            slot.station.participation = Participation::Playing;
            slot.station.health = PLAYER_HEALTH;
            slot.station.position = Point { x: 145, y: 190 };
            slot.station.facing = Facing::Right;
        }
        slots
    }

    #[test]
    fn entrance_never_awards_a_defeat_without_attacks() {
        let mut boss = Wrestler::new(0);
        let mut slots = StreetLayout::DOWNTOWN.spawn.map(Slot::new);
        assert!(!boss.visible());
        for _ in 0..INTRO_VISIBLE {
            assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
        }
        assert!(boss.visible());
        for _ in 0u16..500 {
            assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
        }
        assert_eq!(boss.action, WrestlerAction::Approach);
        assert_eq!(boss.health, HEALTH);
    }

    #[test]
    fn cooperative_hits_and_stagger_lead_to_one_defeat() {
        let mut boss = Wrestler::new(0);
        boss.action = WrestlerAction::Approach;
        let mut slots = active_slots();
        for remaining in (0..HEALTH).step_by(2).rev() {
            set_strikers(&mut slots, Maneuver::Attack {
                moving: false,
                tick: 10,
            });
            assert_eq!(
                boss.advance(StreetLayout::DOWNTOWN, &mut slots),
                remaining == 0
            );
            assert_eq!(boss.health, remaining);
            advance_without_defeat(
                &mut boss,
                &mut slots,
                STAGGER_END.saturating_add(1),
            );
            set_strikers(&mut slots, Maneuver::Idle);
        }
        advance_without_defeat(
            &mut boss,
            &mut slots,
            DEFEAT_END.saturating_add(1),
        );
        assert_eq!(boss.action, WrestlerAction::Cleared);
        assert!(!boss.visible());
    }

    fn advance_without_defeat(
        boss: &mut Wrestler,
        slots: &mut [Slot; 4],
        ticks: u8,
    ) {
        for _ in 0..ticks {
            assert!(!boss.advance(StreetLayout::DOWNTOWN, slots));
        }
    }

    fn set_strikers(slots: &mut [Slot; 4], maneuver: Maneuver) {
        let [first, second, ..] = slots;
        first.station.maneuver = maneuver;
        second.station.maneuver = maneuver;
    }

    #[test]
    fn telegraphed_strike_can_be_evaded() {
        let mut boss = Wrestler::new(0);
        boss.facing = Facing::Left;
        boss.action = WrestlerAction::Attack {
            tick: WRESTLER_ATTACK_STRIKE.saturating_sub(1),
        };
        let mut slots = active_slots();
        let [_, evading, ..] = &mut slots;
        evading.station.position.y = 146;
        assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
        let [first, second, ..] = slots;
        assert_eq!(first.station.health, PLAYER_HEALTH.saturating_sub(6));
        assert_eq!(second.station.health, PLAYER_HEALTH);
    }
}
