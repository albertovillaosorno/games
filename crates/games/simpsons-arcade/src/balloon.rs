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
//   - Optional second-stage balloon boss and delayed victory state.
// - Must-Not:
//   - Render assets, decode source media or implement stage three.
// - Allows:
//   - Inputs: Fixed playing ticks, arena bounds and station maneuvers.
//   - Outputs: Actor state, station damage and one defeat event.
//   - Side effects: Deterministic in-memory gameplay transitions only.
// - Split-When:
//   - Linked projectile behaviour needs independent ownership.
// - Merge-When:
//   - A shared boss contract subsumes this title-local strategy.
// - Summary:
//   - Bounded attack-dependent boardwalk boss encounter.
// - Description:
//   - Observed quantity and timing landmarks, provisional combat policy.
// - Usage:
//   - Opt in through Cabinet::with_balloon_boss with local art.
// - Defaults:
//   - No actor without the complete endpoint and linked encounter.

//! Bounded experimental boardwalk balloon boss.

use super::{
    Facing, Player, Point, Slot, StreetLayout, distance, in_range,
    player_strike, step_toward, strike_grounded_stations,
};

/// Initial quantity and observed decrement per successful primary response.
pub const BALLOON_QUANTITY: u8 = 160;
const HIT_QUANTITY: u8 = 4;
// Paired reference runs establish the initialization and three timers.
const VISIBLE_TICK: u16 = 3;
const INTRO_HOLD_END: u16 = 131;
const INTRO_SHRINK_END: u16 = 323;
const INTRO_EXPAND_END: u16 = 403;
const INTRO_END: u16 = 485;
// The shared readiness gate, steering, contact and terminal credit policy
// are not recovered. These deterministic timings are provisional.
const CONTACT_STRIKE: u8 = 16;
const CONTACT_END: u8 = 48;
const TARGET_HOLD: u8 = 64;
const DEFEAT_END: u8 = 140;
// Both controls rose from 32 to 128 in 32 ticks, then stopped for
// one tick. The 32-tick hold ends at tick 99; acknowledgement differs.
const TERMINAL_RISE_END: u8 = 32;
const TERMINAL_SETTLE_START: u8 = 34;
const TERMINAL_SCALE_END: u8 = 66;
const TERMINAL_FALL_START: u8 = DEFEAT_END.saturating_sub(27);
const ACTIVE_ELEVATION: i16 = 32;
const TERMINAL_APEX: i16 = 128;
// Boardwalk actor coordinates are displaced 43 pixels from the screen.
const RECOIL_LEFT: i16 = 21;
const RECOIL_RIGHT: i16 = 340;
const SETTLE_CENTER: i16 = 165;

/// Current fixed-step balloon boss phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BalloonAction {
    /// Steering toward a retained active station.
    Approach,
    /// Encounter completed; the subsequent stage is not implemented.
    Cleared,
    /// Telegraphed close contact and recovery.
    Contact {
        /// Elapsed fixed contact ticks.
        tick: u8,
    },
    /// Entrance before the actor can receive strikes.
    Intro {
        /// Elapsed fixed playing ticks since arena admission.
        tick: u16,
    },
    /// Quantity response with a bounded four-phase scale cycle.
    Response {
        /// Current scale adjustment phase.
        phase: u8,
    },
    /// Terminal response before the delayed defeat point.
    Terminal {
        /// Elapsed fixed terminal ticks.
        tick: u8,
    },
}

/// Read-only balloon boss state in world coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BalloonBoss {
    /// Current encounter action.
    pub action: BalloonAction,
    /// Last admitted striker, retained until the delayed award.
    pub defeat_credit: Option<Player>,
    /// Direction of movement and contact.
    pub facing: Facing,
    /// Ground anchor for collision and depth ordering.
    pub position: Point,
    /// Remaining observed quantity, not a recovered collision model.
    pub quantity: u8,
    /// Horizontal and vertical reference scale parameters.
    pub scale: (u8, u8),
    /// Retained target identity until the hold expires.
    pub target: Option<Player>,
    /// Remaining target retention ticks.
    pub target_hold: u8,
}

impl BalloonBoss {
    pub(super) fn advance(
        &mut self,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
    ) -> bool {
        if matches!(
            self.action,
            BalloonAction::Approach | BalloonAction::Contact { .. }
        ) && self.receive_strikes(slots)
        {
            return self.quantity == 0;
        }
        self.action = match self.action {
            BalloonAction::Intro { tick } => self.entrance(tick),
            BalloonAction::Approach => self.approach(layout, slots),
            BalloonAction::Contact { tick } => self.contact(tick, slots),
            BalloonAction::Response { phase } => self.respond(phase),
            BalloonAction::Terminal { tick } => {
                self.terminal(tick, layout, slots)
            },
            BalloonAction::Cleared => BalloonAction::Cleared,
        };
        false
    }

    fn approach(
        &mut self,
        layout: StreetLayout,
        slots: &[Slot; 4],
    ) -> BalloonAction {
        let Some(target) = self.refresh_target(slots) else {
            return BalloonAction::Approach;
        };
        self.facing = if target.x < self.position.x {
            Facing::Left
        } else {
            Facing::Right
        };
        if in_range(self.position, self.facing, target, (38, 12)) {
            return BalloonAction::Contact { tick: 0 };
        }
        self.position.x = step_toward(self.position.x, target.x)
            .clamp(layout.min.x, layout.max.x);
        self.position.y = step_toward(self.position.y, target.y)
            .clamp(layout.min.y, layout.max.y);
        BalloonAction::Approach
    }

    fn contact(&self, tick: u8, slots: &mut [Slot; 4]) -> BalloonAction {
        let next = tick.saturating_add(1);
        if next == CONTACT_STRIKE {
            strike_grounded_stations(
                slots,
                (self.position, self.facing),
                (42, 14),
                4,
            );
        }
        if next >= CONTACT_END {
            BalloonAction::Approach
        } else {
            BalloonAction::Contact { tick: next }
        }
    }

    fn entrance(&mut self, tick: u16) -> BalloonAction {
        let next = tick.saturating_add(1);
        let raw_scale = match next {
            INTRO_HOLD_END..INTRO_SHRINK_END => 96u16.saturating_sub(
                next.saturating_sub(INTRO_HOLD_END).div_euclid(4),
            ),
            INTRO_SHRINK_END..INTRO_EXPAND_END => 48u16.saturating_add(
                next.saturating_sub(INTRO_SHRINK_END).div_euclid(4),
            ),
            INTRO_EXPAND_END.. => 64,
            _ => 96,
        };
        let scale = u8::try_from(raw_scale).unwrap_or(64);
        self.scale = (scale, scale);
        if next >= INTRO_END {
            BalloonAction::Approach
        } else {
            BalloonAction::Intro { tick: next }
        }
    }

    pub(super) const fn new(camera: i16) -> Self {
        Self {
            action: BalloonAction::Intro { tick: 0 },
            defeat_credit: None,
            facing: Facing::Right,
            position: Point {
                x: camera.saturating_add(146),
                y: 200,
            },
            quantity: BALLOON_QUANTITY,
            scale: (96, 96),
            target: None,
            target_hold: 0,
        }
    }

    fn receive_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        let mut struck = false;
        for (player, slot) in Player::ALL.into_iter().zip(slots) {
            if !player_strike(&mut slot.station, self.position, (42, 16)) {
                continue;
            }
            self.quantity = self.quantity.saturating_sub(HIT_QUANTITY);
            struck = true;
            if self.quantity == 0 {
                self.start_terminal(player, slot.station.position.x);
                return true;
            }
        }
        if struck {
            self.action = BalloonAction::Response { phase: 0 };
        }
        struck
    }

    fn refresh_target(&mut self, slots: &[Slot; 4]) -> Option<Point> {
        if self.target_hold != 0
            && let Some(target) = self.target
            && let Some(slot) = slots.get(target.index())
            && slot.station.can_fight()
        {
            self.target_hold = self.target_hold.saturating_sub(1);
            return Some(slot.station.position);
        }
        let nearest = Player::ALL
            .into_iter()
            .zip(slots)
            .filter(|(_, slot)| slot.station.can_fight())
            .min_by_key(|(_, slot)| {
                distance(self.position, slot.station.position)
            });
        self.target = nearest.map(|(player, _)| player);
        self.target_hold = TARGET_HOLD;
        nearest.map(|(_, slot)| slot.station.position)
    }

    const fn respond(&mut self, phase: u8) -> BalloonAction {
        let (horizontal, vertical) = self.scale;
        let complete = match phase {
            0 if horizontal < 96 => {
                self.scale.0 = horizontal.saturating_add(4);
                false
            },
            1 if horizontal > 64 => {
                self.scale.0 = horizontal.saturating_sub(4);
                false
            },
            2 if horizontal > 32 => {
                self.scale =
                    (horizontal.saturating_sub(4), vertical.saturating_add(4));
                false
            },
            3 if horizontal < 64 => {
                self.scale =
                    (horizontal.saturating_add(4), vertical.saturating_sub(4));
                false
            },
            _ => true,
        };
        if !complete {
            BalloonAction::Response { phase }
        } else if phase < 3 {
            BalloonAction::Response {
                phase: phase.saturating_add(1),
            }
        } else {
            self.scale = (64, 64);
            BalloonAction::Approach
        }
    }

    const fn start_terminal(&mut self, player: Player, striker_x: i16) {
        self.defeat_credit = Some(player);
        // The last admitted strike supplies the recoil side.
        self.facing = if self.position.x > striker_x {
            Facing::Right
        } else {
            Facing::Left
        };
        self.action = BalloonAction::Terminal { tick: 0 };
    }

    fn terminal(
        &mut self,
        tick: u8,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
    ) -> BalloonAction {
        let next = tick.saturating_add(1);
        self.terminal_horizontal(next, layout);
        if (TERMINAL_SETTLE_START..=TERMINAL_SCALE_END).contains(&next) {
            self.scale = (
                self.scale.0.saturating_sub(1),
                self.scale.1.saturating_sub(1),
            );
        }
        if next < DEFEAT_END {
            return BalloonAction::Terminal { tick: next };
        }
        if let Some(player) = self.defeat_credit.take()
            && let Some(slot) = slots.get_mut(player.index())
        {
            slot.station.award_defeat_point();
        }
        BalloonAction::Cleared
    }

    /// Reference elevation during the terminal response.
    ///
    /// The portable readiness gate retains the provisional defeat duration.
    #[must_use]
    pub fn terminal_elevation(self) -> i16 {
        match self.action {
            BalloonAction::Terminal { tick } if tick <= TERMINAL_RISE_END => {
                ACTIVE_ELEVATION
                    .saturating_add(i16::from(tick).saturating_mul(3))
            },
            BalloonAction::Terminal { tick } if tick > TERMINAL_FALL_START => {
                terminal_fall(tick.saturating_sub(TERMINAL_FALL_START))
            },
            BalloonAction::Terminal { .. } => TERMINAL_APEX,
            BalloonAction::Cleared => 0,
            BalloonAction::Approach
            | BalloonAction::Contact { .. }
            | BalloonAction::Intro { .. }
            | BalloonAction::Response { .. } => ACTIVE_ELEVATION,
        }
    }

    const fn terminal_horizontal(&mut self, tick: u8, layout: StreetLayout) {
        let camera = layout.min.x.saturating_sub(StreetLayout::DOWNTOWN.min.x);
        let speed = if tick <= TERMINAL_RISE_END {
            if self.position.x < camera.saturating_add(RECOIL_LEFT) {
                self.facing = Facing::Right;
            } else if self.position.x > camera.saturating_add(RECOIL_RIGHT) {
                self.facing = Facing::Left;
            }
            6
        } else if tick >= TERMINAL_SETTLE_START {
            self.facing =
                if self.position.x <= camera.saturating_add(SETTLE_CENTER) {
                    Facing::Right
                } else {
                    Facing::Left
                };
            4
        } else {
            return;
        };
        self.position.x = match self.facing {
            Facing::Left => self.position.x.saturating_sub(speed),
            Facing::Right => self.position.x.saturating_add(speed),
        };
    }

    /// Whether the boss contributes a sprite to the arena.
    #[must_use]
    pub const fn visible(self) -> bool {
        match self.action {
            BalloonAction::Intro { tick } => tick >= VISIBLE_TICK,
            BalloonAction::Cleared => false,
            BalloonAction::Approach
            | BalloonAction::Contact { .. }
            | BalloonAction::Response { .. }
            | BalloonAction::Terminal { .. } => true,
        }
    }
}

fn terminal_fall(tick: u8) -> i16 {
    // The paired controls differ by at most one pixel of fractional phase.
    // Integrate the observed fractional gravity and limit the velocity byte.
    let mut elevation = i32::from(TERMINAL_APEX)
        .saturating_mul(256)
        .saturating_add(127);
    let mut velocity = 0i32;
    for _ in 0..tick.min(27) {
        velocity = velocity.saturating_sub(96);
        if velocity < -2048i32 {
            velocity = velocity.saturating_add(256);
        }
        elevation = elevation.saturating_add(velocity);
    }
    i16::try_from(elevation.max(0).div_euclid(256)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Maneuver, PLAYER_HEALTH, PLAYER_STRIKE, Participation};

    fn active_slots() -> [Slot; 4] {
        let mut slots = StreetLayout::DOWNTOWN.spawn.map(Slot::new);
        for slot in &mut slots {
            slot.station.participation = Participation::Playing;
            slot.station.health = PLAYER_HEALTH;
            slot.station.position = Point { x: 136, y: 200 };
            slot.station.facing = Facing::Right;
        }
        slots
    }

    #[test]
    fn entrance_tracks_observed_timer_landmarks() {
        let mut boss = BalloonBoss::new(0);
        let mut slots = active_slots();
        for tick in 0..INTRO_END {
            assert_eq!(boss.action, BalloonAction::Intro { tick });
            assert_eq!(boss.visible(), tick >= VISIBLE_TICK);
            assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
            assert_entrance_scale(boss, tick.saturating_add(1));
        }
        assert_eq!(boss.action, BalloonAction::Approach);
        assert_eq!(boss.quantity, BALLOON_QUANTITY);
        assert_eq!(slots.map(|slot| slot.station.score), [0; 4]);
    }

    fn assert_entrance_scale(boss: BalloonBoss, tick: u16) {
        match tick {
            INTRO_HOLD_END => assert_eq!(boss.scale, (96, 96)),
            INTRO_SHRINK_END => assert_eq!(boss.scale, (48, 48)),
            INTRO_EXPAND_END => assert_eq!(boss.scale, (64, 64)),
            _ => {},
        }
    }

    #[test]
    fn forty_strikes_yield_one_delayed_point() {
        let mut boss = BalloonBoss::new(0);
        boss.action = BalloonAction::Approach;
        boss.scale = (64, 64);
        let mut slots = active_slots();
        for strike in 0..40u8 {
            set_striker(&mut slots, strike);
            assert_eq!(
                boss.advance(StreetLayout::DOWNTOWN, &mut slots),
                strike == 39
            );
            assert_eq!(
                boss.quantity,
                156u8.saturating_sub(strike.saturating_mul(4))
            );
            for slot in &mut slots {
                slot.station.maneuver = Maneuver::Idle;
            }
            if strike != 39 {
                assert_response(&mut boss, &mut slots);
            }
        }
        assert_terminal(&mut boss, &mut slots);
    }

    fn set_striker(slots: &mut [Slot; 4], strike: u8) {
        let index = usize::from(strike.checked_rem(4).unwrap_or_default());
        if let Some(slot) = slots.get_mut(index) {
            slot.station.maneuver = Maneuver::Attack {
                moving: false,
                tick: PLAYER_STRIKE,
            };
        }
    }

    fn assert_response(boss: &mut BalloonBoss, slots: &mut [Slot; 4]) {
        for _ in 0..36u8 {
            assert!(!boss.advance(StreetLayout::DOWNTOWN, slots));
        }
        assert_eq!(boss.action, BalloonAction::Approach);
        assert_eq!(boss.scale, (64, 64));
    }

    fn assert_terminal(boss: &mut BalloonBoss, slots: &mut [Slot; 4]) {
        for tick in 0..DEFEAT_END {
            assert_eq!(boss.action, BalloonAction::Terminal { tick });
            assert_eq!(slots.map(|slot| slot.station.score), [0; 4]);
            assert!(!boss.advance(StreetLayout::DOWNTOWN, slots));
        }
        assert_eq!(boss.action, BalloonAction::Cleared);
        assert!(!boss.visible());
        assert_eq!(slots.map(|slot| slot.station.score), [0, 0, 0, 1]);
        assert!(!boss.advance(StreetLayout::DOWNTOWN, slots));
        assert_eq!(slots.map(|slot| slot.station.score), [0, 0, 0, 1]);
    }

    #[test]
    fn terminal_recoil_preserves_both_reference_paths() {
        assert_recoil(226, Facing::Left, [
            220, 184, 178, 34, 34, 166, 162, 162, 162,
        ]);
        assert_recoil(300, Facing::Right, [
            306, 342, 336, 192, 192, 164, 168, 168, 168,
        ]);
    }

    fn assert_recoil(start: i16, facing: Facing, samples: [i16; 9]) {
        let mut slots = active_slots();
        let mut boss = BalloonBoss::new(0);
        boss.position.x = start;
        boss.facing = facing;
        boss.quantity = 0;
        boss.scale = (64, 64);
        boss.action = BalloonAction::Terminal { tick: 0 };
        let mut tick = 0u8;
        for (sample, expected) in
            [1, 7, 8, 32, 33, 66, 67, 99, 113].into_iter().zip(samples)
        {
            for _ in tick..sample {
                assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
            }
            assert_eq!(boss.position.x, expected, "{sample}");
            tick = sample;
        }
    }

    #[test]
    fn terminal_fall_stays_within_paired_fractional_phase() {
        let first = [
            128i16, 127, 127, 126, 124, 122, 120, 117, 114, 111, 107, 103, 99,
            94, 88, 83, 77, 70, 64, 57, 49, 41, 34, 26, 18, 11, 3, 0,
        ];
        let second = [
            128i16, 128, 127, 126, 124, 122, 120, 117, 114, 111, 107, 103, 99,
            94, 89, 83, 77, 71, 64, 57, 49, 41, 34, 26, 18, 11, 3, 0,
        ];
        for (tick, (low, high)) in (0..=27u8).zip(first.into_iter().zip(second))
        {
            let actual = terminal_fall(tick);
            assert!((low..=high).contains(&actual), "{tick}");
        }
    }

    #[test]
    fn response_scale_retains_the_observed_four_phase_cycle() {
        let mut slots = active_slots();
        let mut boss = BalloonBoss::new(0);
        boss.action = BalloonAction::Response { phase: 0 };
        boss.scale = (64, 64);
        for tick in 1..=36u8 {
            assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
            match tick {
                1 => assert_eq!(boss.scale, (68, 64)),
                8 | 9 => assert_eq!(boss.scale, (96, 64)),
                17 | 18 | 35 | 36 => assert_eq!(boss.scale, (64, 64)),
                26 | 27 => assert_eq!(boss.scale, (32, 96)),
                _ => {},
            }
        }
        assert_eq!(boss.action, BalloonAction::Approach);
    }

    #[test]
    fn contact_can_be_evaded_and_no_attack_means_no_defeat() {
        let mut boss = BalloonBoss::new(0);
        boss.action = BalloonAction::Contact {
            tick: CONTACT_STRIKE.saturating_sub(1),
        };
        boss.facing = Facing::Left;
        let mut slots = active_slots();
        let [_, evading, ..] = &mut slots;
        evading.station.position.y = 146;
        assert!(!boss.advance(StreetLayout::DOWNTOWN, &mut slots));
        let [first, second, ..] = slots;
        assert_eq!(first.station.health, PLAYER_HEALTH.saturating_sub(4));
        assert_eq!(second.station.health, PLAYER_HEALTH);
        assert_eq!(boss.quantity, BALLOON_QUANTITY);
    }
}
