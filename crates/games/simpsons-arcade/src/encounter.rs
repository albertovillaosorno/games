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
//   - First-street enemy admission, approach, strike and defeat rules.
// - Must-Not:
//   - Render, interpret original media or perform host I/O.
// - Allows:
//   - Inputs: Fixed-step station state, camera and walkable bounds.
//   - Outputs: Enemy state and deterministic station damage.
//   - Side effects: In-memory gameplay transitions only.
// - Split-When:
//   - Distinct enemy strategies or stage waves require separate ownership.
// - Merge-When:
//   - Another title-local module duplicates encounter hit resolution.
// - Summary:
//   - Independently authored, bounded provisional street encounters.
// - Description:
//   - Admits enemies at observed travel gates; resolves individual strikes.
// - Usage:
//   - Opt in through Cabinet::with_opening_street or with_encounter.
// - Defaults:
//   - At most twelve enemies; no original combat parity claim.

//! Provisional fixed-step encounter for the first street.

use super::{
    BASE_WIDTH, ENEMY_CAPACITY, Encounter, FAR_STREET_ADMISSION, Facing,
    LATER_COMBAT_GATE, MID_STREET_ADMISSION, MID_STREET_COMBAT_GATE,
    MID_STREET_ENEMY_COUNT, Maneuver, OPENING_ENEMY_COUNT,
    OPPOSING_ENEMY_COUNT, Player, Point, Slot, StreetLayout, distance,
    hurt_slot, in_range, player_strike, step_toward, strip_width, view_origin,
    viewport_width,
};

const ENEMY_HEALTH: u8 = 1;
const ENEMY_STRIKE: u8 = 14;
const ENEMY_DAMAGE: u8 = 3;
/// Number of fixed-step ticks in the initial enemy punch.
pub const ENEMY_ATTACK_TICKS: u8 = 34;
/// Number of fixed-step ticks in the initial enemy walking cycle.
pub const ENEMY_WALK_TICKS: u8 = 48;
/// Number of fixed-step ticks before the defeated enemy disappears.
pub const ENEMY_DEFEAT_TICKS: u8 = 66;
const DEFEAT_KNOCKBACK_TICKS: u8 = 33;
const DEFEAT_BLINK_TICK: u8 = 52;
const REPELLED_KNOCKBACK_TICKS: u8 = 28;
const REPELLED_CENTER_TICKS: u8 = 91;
const REPELLED_BOUNDARY_TICKS: u8 = 103;
const RESTRAINT_EDGES: u8 = 12;
const RESTRAINT_RELEASE_TICKS: u8 = 4;
const RESTRAINT_FIRST_DAMAGE: u8 = 25;
const RESTRAINT_DAMAGE_INTERVAL: u8 = 48;

// First observed sprite-group admissions during rightward travel.
// Anchors are approximate; the observed arrivals start outside the view.
const RESTRAINT_THRESHOLD: i16 = 154;
const OPENING_COMBAT_LIMIT: i16 = 362;
const REINFORCEMENTS: [(i16, Point); ENEMY_CAPACITY - 1] = [
    (50, Point { x: 334, y: 176 }),
    (82, Point { x: -18, y: 208 }),
    (RESTRAINT_THRESHOLD, Point { x: 334, y: 192 }),
    (210, Point { x: 334, y: 208 }),
    (274, Point { x: -18, y: 176 }),
    (OPENING_COMBAT_LIMIT, Point { x: 334, y: 160 }),
    (LATER_COMBAT_GATE, Point { x: 334, y: 192 }),
    (LATER_COMBAT_GATE, Point { x: -18, y: 192 }),
    (MID_STREET_ADMISSION, Point { x: -18, y: 192 }),
    (MID_STREET_COMBAT_GATE, Point { x: -18, y: 176 }),
    (FAR_STREET_ADMISSION, Point { x: -18, y: 152 }),
];

// Provisional regular-enemy waves at the observed second-stage holds.
const STAGE_TWO_REINFORCEMENTS: [(i16, Point); 4] = [
    (0, Point { x: 334, y: 176 }),
    (66, Point { x: -18, y: 192 }),
    (128, Point { x: 334, y: 192 }),
    (210, Point { x: 334, y: 176 }),
];

impl Encounter {
    pub(super) fn admit(&mut self, camera: i16) {
        if !self.progressive {
            return;
        }
        let [_, remaining @ ..] = &mut self.enemies;
        let arrivals = if self.stage_two {
            STAGE_TWO_REINFORCEMENTS.as_slice()
        } else {
            REINFORCEMENTS.as_slice()
        };
        for (entry, &(trigger, anchor)) in remaining.iter_mut().zip(arrivals) {
            if entry.is_some() || camera < trigger {
                continue;
            }
            let mut enemy = Enemy::new(Point {
                x: arrival_x(self.art_travel, trigger, anchor.x),
                y: anchor.y,
            });
            if anchor.x < 0 {
                enemy.facing = Facing::Right;
            }
            *entry = Some(enemy);
        }
    }

    pub(super) fn advance(
        &mut self,
        view: StreetLayout,
        (base, scroll_limit): (StreetLayout, i16),
        (slots, restraint_edge): (&mut [Slot; 4], bool),
    ) -> u64 {
        let street = StreetLayout {
            max: Point {
                x: base
                    .max
                    .x
                    .saturating_add(scroll_limit.max(self.art_travel))
                    .saturating_add(46),
                ..base.max
            },
            min: Point {
                x: base.min.x.saturating_sub(50),
                ..base.min
            },
            ..base
        };
        // Resolve admitted player strikes before any same-tick counterstrike
        // can change a station's maneuver. Ground strikes may hit multiple
        // targets; aerial and special strikes retain their one-hit budget.
        let [marge, ..] = *slots;
        let was_held = marge.station.maneuver == Maneuver::Restrained;
        let hits = self.enemies.each_mut().map(|entry| {
            entry
                .as_mut()
                .is_some_and(|enemy| enemy.resolve_strikes(slots))
        });
        let defeated = hits.into_iter().map(u64::from).sum();
        for ((index, entry), hit) in
            self.enemies.iter_mut().enumerate().zip(hits)
        {
            let bounds = if index == 0 {
                view
            } else {
                street
            };
            if let Some(enemy) = entry
                && !hit
                && (enemy.health == 0
                    || slots.iter().any(|slot| slot.station.can_fight()))
            {
                let restraint = self.marge_restraint
                    && !self.stage_two
                    && !was_held
                    && index != 0
                    && view.max.x
                        >= base.max.x.saturating_add(RESTRAINT_THRESHOLD);
                enemy.advance(bounds, slots, (restraint, restraint_edge));
            }
        }
        defeated
    }

    pub(super) fn enemies(&self) -> impl Iterator<Item = Enemy> + '_ {
        self.enemies.iter().flatten().copied()
    }

    pub(super) fn enter_stage_two(&mut self, travel: i16) {
        self.stage_two = true;
        self.art_travel = travel;
        let [initial, remaining @ ..] = &mut self.enemies;
        *initial = Some(Enemy::new(Point { x: 246, y: 190 }));
        remaining.fill(None);
        self.admit(0);
    }

    pub(super) const fn initial(self) -> Option<Enemy> {
        let [initial, ..] = self.enemies;
        initial
    }

    pub(super) fn limit_travel(&self, travel: i16) -> i16 {
        if !self.progressive {
            return travel;
        }
        let gates = if self.stage_two {
            [(66, 2), (128, 3), (210, 4)]
        } else {
            [
                (OPENING_COMBAT_LIMIT, OPENING_ENEMY_COUNT),
                (LATER_COMBAT_GATE, OPPOSING_ENEMY_COUNT),
                (MID_STREET_COMBAT_GATE, MID_STREET_ENEMY_COUNT),
            ]
        };
        gates
            .into_iter()
            .find(|&(_, count)| {
                !self
                    .enemies
                    .iter()
                    .take(count)
                    .all(|entry| entry.is_some_and(|enemy| enemy.health == 0))
            })
            .map_or(travel, |(gate, _)| travel.min(gate))
    }

    pub(super) const fn new(spawn: Point, progressive: bool) -> Self {
        let mut enemies = [None; ENEMY_CAPACITY];
        let [initial, ..] = &mut enemies;
        *initial = Some(Enemy::new(spawn));
        Self {
            art_travel: 0,
            enemies,
            marge_restraint: false,
            opening_spawn: spawn,
            progressive,
            stage_two: false,
        }
    }

    pub(super) fn restart(&mut self) {
        self.stage_two = false;
        let [initial, remaining @ ..] = &mut self.enemies;
        *initial = Some(Enemy::new(self.opening_spawn));
        remaining.fill(None);
    }
}

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
    /// Knockback, landing and disappearance after a lethal strike.
    Defeat {
        /// Zero-based defeat tick, capped at disappearance.
        tick: u8,
    },
    /// Recovering from a successful player escape, with health retained.
    Repelled {
        /// Zero-based knockback and recovery tick.
        tick: u8,
    },
    /// Holding Marge in the optional reinforcement encounter.
    Restrain(RestraintMotion),
}

/// Fixed-step restraint timeline for the observed Marge interaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RestraintMotion {
    age: u8,
    progress: u8,
}

impl RestraintMotion {
    const fn advance(&mut self, edge: bool) -> bool {
        self.age = if self.age == 80 {
            33
        } else {
            self.age.saturating_add(1)
        };
        if self.progress >= RESTRAINT_EDGES || edge {
            self.progress = self.progress.saturating_add(1);
        }
        self.progress >= RESTRAINT_EDGES.saturating_add(RESTRAINT_RELEASE_TICKS)
    }

    // The first repeated hit was observed at offset 25. Later hits
    // varied between trials/intervals; use a representative cadence.
    const fn damage_due(self) -> bool {
        self.age >= RESTRAINT_FIRST_DAMAGE
            && self
                .age
                .saturating_sub(RESTRAINT_FIRST_DAMAGE)
                .is_multiple_of(RESTRAINT_DAMAGE_INTERVAL)
    }

    /// Display phase in the six locally observed combined pose keys.
    #[must_use]
    pub const fn pose_tick(self) -> u8 {
        match self.age {
            0..=6 => 0,
            7..=13 => 7,
            14..=19 | 26 => 14,
            20..=25 => 20,
            27..=32 => 27,
            age => match age.saturating_sub(33).checked_rem(24) {
                Some(0..=5) => 20,
                Some(12..=17) => 45,
                _ => 14,
            },
        }
    }
}

/// Read-only state of an admitted enemy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Enemy {
    /// Current action, including the noninteractive defeat timeline.
    pub action: EnemyAction,
    /// Successful character awaiting score credit at disappearance.
    pub defeat_credit: Option<Player>,
    /// Current horizontal facing.
    pub facing: Facing,
    /// Remaining health; zero means defeated.
    pub health: u8,
    /// Ground position, independent of art dimensions.
    pub position: Point,
    /// Initial spawn position for this enemy.
    pub spawn: Point,
}

impl Enemy {
    pub(super) fn advance(
        &mut self,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
        (restraint, restraint_edge): (bool, bool),
    ) {
        if self.health == 0 {
            self.advance_defeat(layout, slots);
            return;
        }
        match self.action {
            EnemyAction::Approach { tick } => {
                self.approach_or_restrain(layout, slots, (tick, restraint));
            },
            EnemyAction::Attack { tick } => {
                self.attack(tick, slots);
            },
            EnemyAction::Defeat { .. } => {},
            EnemyAction::Repelled { tick } => {
                self.advance_repelled(layout, tick);
            },
            EnemyAction::Restrain(motion) => {
                self.advance_restraint(slots, motion, restraint_edge);
            },
        }
    }

    fn advance_defeat(&mut self, layout: StreetLayout, slots: &mut [Slot; 4]) {
        let EnemyAction::Defeat { tick } = self.action else {
            return;
        };
        if tick >= ENEMY_DEFEAT_TICKS {
            return;
        }
        let next = tick.saturating_add(1);
        self.knockback(layout, next, DEFEAT_KNOCKBACK_TICKS);
        self.action = EnemyAction::Defeat { tick: next };
        if next == ENEMY_DEFEAT_TICKS
            && let Some(player) = self.defeat_credit.take()
            && let Some(slot) = slots.get_mut(player.index())
        {
            slot.station.award_defeat_point();
        }
    }

    fn advance_repelled(&mut self, layout: StreetLayout, tick: u8) {
        let next = tick.saturating_add(1);
        self.knockback(layout, next, REPELLED_KNOCKBACK_TICKS);
        let duration = match self.facing {
            Facing::Right => REPELLED_CENTER_TICKS,
            Facing::Left => REPELLED_BOUNDARY_TICKS,
        };
        self.action = if next >= duration {
            EnemyAction::Approach { tick: 0 }
        } else {
            EnemyAction::Repelled { tick: next }
        };
    }

    fn advance_restraint(
        &mut self,
        slots: &mut [Slot; 4],
        mut motion: RestraintMotion,
        edge: bool,
    ) {
        let [marge, ..] = slots;
        if marge.station.maneuver != Maneuver::Restrained
            || !marge.station.can_fight()
        {
            self.action = EnemyAction::Approach { tick: 0 };
            return;
        }
        let escaped = motion.advance(edge);
        if motion.damage_due() && !restrain_damage(marge) {
            self.action = EnemyAction::Approach { tick: 0 };
        } else if escaped {
            release_marge(slots);
            self.action = EnemyAction::Repelled { tick: 0 };
        } else {
            self.action = EnemyAction::Restrain(motion);
        }
    }

    fn approach(
        &mut self,
        layout: StreetLayout,
        slots: &[Slot; 4],
        (tick, restraint): (u8, bool),
    ) {
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
        let strike_range = if restraint {
            (4, 4)
        } else {
            (18, 8)
        };
        if in_range(self.position, self.facing, target, strike_range) {
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

    fn approach_or_restrain(
        &mut self,
        layout: StreetLayout,
        slots: &mut [Slot; 4],
        (tick, enabled): (u8, bool),
    ) {
        let [marge, ..] = *slots;
        let candidate = enabled
            && marge.station.can_fight()
            && marge.station.maneuver == Maneuver::Idle
            && marge.station.facing == Facing::Right
            && slots.iter().filter(|slot| slot.station.can_fight()).count()
                == 1;
        let facing = if marge.station.position.x < self.position.x {
            Facing::Left
        } else {
            Facing::Right
        };
        if candidate
            && in_range(self.position, facing, marge.station.position, (4, 4))
        {
            self.facing = facing;
            self.position = marge.station.position;
            let [held_slot, ..] = slots;
            if restrain_damage(held_slot) {
                self.action = EnemyAction::Restrain(RestraintMotion::default());
            }
            return;
        }
        self.approach(layout, slots, (tick, candidate));
    }

    fn attack(&mut self, tick: u8, slots: &mut [Slot; 4]) {
        let next = tick.saturating_add(1);
        if next == ENEMY_STRIKE {
            for slot in slots.iter_mut().filter(|slot| {
                slot.station.can_fight()
                    && !slot.station.maneuver.airborne()
                    && slot.station.maneuver != Maneuver::Restrained
                    && in_range(
                        self.position,
                        self.facing,
                        slot.station.position,
                        (22, 10),
                    )
            }) {
                hurt_slot(slot, ENEMY_DAMAGE);
            }
        }
        self.action = if next >= ENEMY_ATTACK_TICKS {
            EnemyAction::Approach { tick: 0 }
        } else {
            EnemyAction::Attack { tick: next }
        };
    }

    fn knockback(&mut self, layout: StreetLayout, tick: u8, duration: u8) {
        if tick <= duration {
            let displacement = match self.facing {
                Facing::Left => 2,
                Facing::Right => -2,
            };
            self.position.x = self
                .position
                .x
                .saturating_add(displacement)
                .clamp(layout.min.x, layout.max.x);
        }
    }

    pub(super) const fn new(position: Point) -> Self {
        Self {
            action: EnemyAction::Approach { tick: 0 },
            defeat_credit: None,
            facing: Facing::Left,
            health: ENEMY_HEALTH,
            position,
            spawn: position,
        }
    }

    fn receive_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        for (player, slot) in Player::ALL.into_iter().zip(slots) {
            let station = &mut slot.station;
            if player_strike(station, self.position, (32, 12)) {
                self.health = 0;
                // The original observed rightward knockback faces left.
                self.facing = match station.facing {
                    Facing::Left => Facing::Right,
                    Facing::Right => Facing::Left,
                };
                self.action = EnemyAction::Defeat { tick: 0 };
                self.defeat_credit = Some(player);
                return true;
            }
        }
        false
    }

    fn resolve_strikes(&mut self, slots: &mut [Slot; 4]) -> bool {
        let held = matches!(self.action, EnemyAction::Restrain(_));
        let hit = self.health > 0 && self.receive_strikes(slots);
        if held && hit {
            release_marge(slots);
        }
        hit
    }

    /// Whether the enemy still contributes a visible world sprite.
    #[must_use]
    pub const fn visible(self) -> bool {
        match self.action {
            EnemyAction::Defeat { tick } => {
                tick < DEFEAT_BLINK_TICK
                    || (tick < ENEMY_DEFEAT_TICKS && !tick.is_multiple_of(2))
            },
            EnemyAction::Approach { .. }
            | EnemyAction::Attack { .. }
            | EnemyAction::Repelled { .. }
            | EnemyAction::Restrain(_) => self.health > 0,
        }
    }
}

fn release_marge(slots: &mut [Slot; 4]) {
    let [marge, ..] = slots;
    if marge.station.maneuver == Maneuver::Restrained {
        marge.station.maneuver = Maneuver::Idle;
    }
}

const fn restrain_damage(marge: &mut Slot) -> bool {
    marge.damage(ENEMY_DAMAGE);
    if marge.station.can_fight() {
        marge.station.maneuver = Maneuver::Restrained;
        true
    } else {
        false
    }
}

fn arrival_x(travel: i16, camera: i16, offset: i16) -> i16 {
    if travel == 0 {
        return camera.saturating_add(offset);
    }
    let strip = strip_width(travel);
    let width = viewport_width(strip);
    let origin = view_origin(strip, width, camera).unwrap_or(camera);
    let original = camera.saturating_add(offset);
    if offset < 0 {
        original.min(origin.saturating_add(offset))
    } else {
        let extra =
            i16::try_from(width.saturating_sub(BASE_WIDTH)).unwrap_or_default();
        original.max(origin.saturating_add(offset).saturating_add(extra))
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use games::input::LogicalAction;
    use games::replay::InputStep as _;

    use super::*;
    use crate::{
        Action, CONTINUE_TICKS, Cabinet, Control, LayoutError, PLAYER_HEALTH,
        PLAYER_STOCKS, PLAYER_STRIKE, Participation, Phase, Station,
    };

    fn pulse(control: Control) -> [LogicalAction<Action>; 2] {
        pulse_for(Player::Marge, control)
    }

    fn pulse_for(
        player: Player,
        control: Control,
    ) -> [LogicalAction<Action>; 2] {
        let action = Action { control, player };
        [LogicalAction { action, down: true }, LogicalAction {
            action,
            down: false,
        }]
    }

    #[test]
    fn reinforcement_admission_is_monotonic_and_resets() {
        let spawn = Point { x: 246, y: 190 };
        let mut encounter = Encounter::new(spawn, true);
        for (camera, count) in [
            (0, 1),
            (49, 1),
            (50, 2),
            (81, 2),
            (82, 3),
            (153, 3),
            (154, 4),
            (209, 4),
            (210, 5),
            (273, 5),
            (274, 6),
            (361, 6),
            (362, 7),
            (457, 7),
            (458, 9),
            (458, 9),
            (561, 9),
            (562, 10),
            (769, 10),
            (770, 11),
            (833, 11),
            (834, 12),
            (0, 12),
        ] {
            encounter.admit(camera);
            assert_eq!(encounter.enemies().count(), count);
        }
        assert!(encounter.enemies().map(|enemy| enemy.spawn).eq([
            spawn,
            Point { x: 384, y: 176 },
            Point { x: 64, y: 208 },
            Point { x: 488, y: 192 },
            Point { x: 544, y: 208 },
            Point { x: 256, y: 176 },
            Point { x: 696, y: 160 },
            Point { x: 792, y: 192 },
            Point { x: 440, y: 192 },
            Point { x: 544, y: 192 },
            Point { x: 752, y: 176 },
            Point { x: 816, y: 152 },
        ]));
        assert_eq!(
            encounter.enemies().nth(2).map(|enemy| enemy.facing),
            Some(Facing::Right)
        );
        encounter.restart();
        assert!(encounter.enemies().eq([Enemy::new(spawn)]));
        encounter.admit(50);
        assert_eq!(encounter.enemies().count(), 2);
    }

    #[test]
    fn second_stage_waves_gate_only_their_own_travel() {
        let mut encounter = Encounter::new(Point { x: 246, y: 190 }, true);
        encounter.enter_stage_two(270);
        assert_eq!(encounter.enemies().count(), 2);
        assert_eq!(encounter.limit_travel(270), 66);
        defeat_prefix(&mut encounter, 2);
        encounter.admit(66);
        assert_eq!(encounter.enemies().count(), 3);
        assert_eq!(encounter.limit_travel(270), 128);
        defeat_prefix(&mut encounter, 3);
        encounter.admit(128);
        assert_eq!(encounter.enemies().count(), 4);
        assert_eq!(encounter.limit_travel(270), 210);
        defeat_prefix(&mut encounter, 4);
        encounter.admit(210);
        assert_eq!(encounter.enemies().count(), 5);
        assert_eq!(encounter.limit_travel(270), 270);
        encounter.restart();
        assert_eq!(encounter.enemies().count(), 1);
    }

    fn defeat_prefix(encounter: &mut Encounter, count: usize) {
        for enemy in encounter.enemies.iter_mut().flatten().take(count) {
            enemy.health = 0;
        }
    }

    #[test]
    fn later_pair_gates_travel_until_both_are_defeated() {
        let mut encounter = Encounter::new(Point { x: 246, y: 190 }, true);
        encounter.admit(OPENING_COMBAT_LIMIT);
        assert_eq!(encounter.limit_travel(1350), OPENING_COMBAT_LIMIT);
        defeat_prefix(&mut encounter, OPENING_ENEMY_COUNT);
        assert_eq!(
            (encounter.limit_travel(444), encounter.limit_travel(1350)),
            (444, LATER_COMBAT_GATE)
        );
        encounter.admit(LATER_COMBAT_GATE);
        assert_eq!(encounter.enemies().count(), OPPOSING_ENEMY_COUNT);
        defeat_prefix(&mut encounter, OPPOSING_ENEMY_COUNT - 1);
        assert_eq!(encounter.limit_travel(1350), LATER_COMBAT_GATE);
        defeat_prefix(&mut encounter, OPPOSING_ENEMY_COUNT);
        encounter.admit(LATER_COMBAT_GATE);
        assert_eq!(encounter.limit_travel(1350), MID_STREET_COMBAT_GATE);
        assert!(encounter.enemies().all(|enemy| enemy.health == 0));
        encounter.restart();
        assert_eq!(
            (encounter.enemies().count(), encounter.limit_travel(1350)),
            (1, OPENING_COMBAT_LIMIT)
        );
    }

    #[test]
    fn mid_street_gate_preserves_intermediate_and_far_admissions() {
        let mut encounter = Encounter::new(Point { x: 246, y: 190 }, true);
        encounter.admit(MID_STREET_ADMISSION);
        defeat_prefix(&mut encounter, OPPOSING_ENEMY_COUNT);
        assert_eq!(encounter.limit_travel(700), 700);
        assert_eq!(encounter.limit_travel(1350), MID_STREET_COMBAT_GATE);
        encounter.admit(MID_STREET_COMBAT_GATE);
        defeat_prefix(&mut encounter, MID_STREET_ENEMY_COUNT - 1);
        assert_eq!(encounter.limit_travel(1350), MID_STREET_COMBAT_GATE);
        defeat_prefix(&mut encounter, MID_STREET_ENEMY_COUNT);
        encounter.admit(FAR_STREET_ADMISSION);
        assert_eq!(encounter.limit_travel(1350), 1350);
        assert_eq!(
            encounter.enemies().last().map(|enemy| enemy.health),
            Some(1)
        );
    }

    #[test]
    fn later_pair_approaches_from_opposite_edges() {
        let mut encounter = Encounter::new(Point { x: 246, y: 190 }, true);
        encounter.admit(LATER_COMBAT_GATE);
        assert!(
            encounter
                .enemies()
                .skip(OPENING_ENEMY_COUNT)
                .map(|enemy| enemy.facing)
                .eq([Facing::Left, Facing::Right])
        );
    }

    #[test]
    fn simultaneous_player_strike_survives_other_enemy_counterstrike() {
        let result = Cabinet::with_opening_street();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &pulse(Control::Start));
        cabinet.input_step(1, &pulse(Control::Primary));
        for tick in 2..11 {
            cabinet.input_step(tick, &[]);
        }
        let Some(encounter) = &mut cabinet.encounter else {
            return;
        };
        encounter.admit(50);
        let [initial, reinforcement, ..] = &mut encounter.enemies;
        *initial = Some(Enemy {
            action: EnemyAction::Attack { tick: 13 },
            facing: Facing::Right,
            ..Enemy::new(Point { x: 90, y: 190 })
        });
        *reinforcement = Some(Enemy::new(Point { x: 130, y: 190 }));
        cabinet.input_step(11, &[]);
        assert!(cabinet.enemies().map(|enemy| enemy.health).eq([1, 0]));
        assert_eq!(cabinet.impact_cues(), 1);
        let [marge, ..] = cabinet.stations();
        assert_eq!(marge.health, PLAYER_HEALTH.saturating_sub(ENEMY_DAMAGE));
        let Some(outcome) = &mut cabinet.encounter else {
            return;
        };
        outcome.admit(50);
        assert_eq!(outcome.enemies().nth(1).map(|enemy| enemy.health), Some(0));
    }

    #[test]
    fn one_ground_strike_emits_one_impact_per_defeated_enemy() {
        let result = Cabinet::with_opening_street();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &pulse(Control::Start));
        cabinet.input_step(1, &pulse(Control::Primary));
        for tick in 2..11 {
            cabinet.input_step(tick, &[]);
        }
        let Some(encounter) = &mut cabinet.encounter else {
            return;
        };
        encounter.admit(50);
        let [initial, reinforcement, ..] = &mut encounter.enemies;
        *initial = Some(Enemy::new(Point { x: 130, y: 190 }));
        *reinforcement = Some(Enemy::new(Point { x: 132, y: 190 }));
        assert_eq!(cabinet.impact_cues(), 0);
        cabinet.input_step(11, &[]);
        assert!(cabinet.enemies().map(|enemy| enemy.health).eq([0, 0]));
        assert_eq!(cabinet.stations().map(|station| station.score), [0; 4]);
        assert_eq!(cabinet.impact_cues(), 2);
        for tick in 12..100 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|station| station.score), [
            2, 0, 0, 0
        ]);
        assert_eq!(cabinet.impact_cues(), 2);
    }

    #[test]
    fn single_encounter_never_admits_reinforcements() {
        let mut encounter = Encounter::new(Point { x: 246, y: 190 }, false);
        encounter.admit(154);
        assert_eq!(encounter.enemies().count(), 1);
    }

    #[test]
    fn reinforcements_enter_beyond_every_supported_view() {
        for travel in [50i16, 82, 154, 362, 444, 458, 562, 770, 834, 1350] {
            for (gate, anchor) in REINFORCEMENTS
                .into_iter()
                .filter(|(gate, _)| *gate <= travel)
            {
                assert_arrival_outside_view(travel, gate, anchor.x);
            }
        }
    }

    fn assert_arrival_outside_view(travel: i16, gate: i16, offset: i16) {
        let strip = strip_width(travel);
        let x = i32::from(arrival_x(travel, gate, offset));
        for width in BASE_WIDTH..=viewport_width(strip) {
            let origin =
                i32::from(view_origin(strip, width, gate).unwrap_or_default());
            if offset < 0 {
                assert!(x < origin);
            } else {
                assert!(
                    x >= origin.saturating_add(
                        i32::try_from(width).unwrap_or_default()
                    )
                );
            }
        }
    }

    #[test]
    fn opening_street_admits_reinforcements_during_play() {
        let result = Cabinet::with_opening_street()
            .and_then(|cabinet| cabinet.with_scroll_limit(444));
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &pulse(Control::Start));
        let [press, release] = pulse(Control::Right);
        cabinet.input_step(1, slice::from_ref(&press));
        for tick in 2..40 {
            let [marge, ..] = cabinet.stations();
            if cabinet.enemy().is_some_and(|enemy| {
                enemy.position.x.abs_diff(marge.position.x) <= 28
            }) {
                break;
            }
            cabinet.input_step(tick, &[]);
        }
        cabinet.input_step(40, &[release]);
        cabinet.input_step(41, &pulse(Control::Primary));
        for tick in 42..62 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(0));
        cabinet.input_step(62, &[press]);
        assert_opening_arrivals(&mut cabinet);
        assert_combat_travel_bound(&mut cabinet);
    }

    #[test]
    fn opening_street_clear_releases_remaining_verified_travel() {
        for (limit, expected) in [
            (80, 2),
            (154, 4),
            (444, 7),
            (458, 9),
            (562, 10),
            (770, 11),
            (834, 12),
            (1350, 12),
        ] {
            let result = Cabinet::with_opening_street()
                .and_then(|cabinet| cabinet.with_scroll_limit(limit));
            assert!(result.is_ok());
            let Ok(mut cabinet) = result else {
                return;
            };
            cabinet.input_step(0, &pulse(Control::Start));
            clear_street(&mut cabinet, limit);
            assert_eq!(cabinet.enemies().count(), expected);
            assert!(cabinet.enemies().all(|enemy| enemy.health == 0));
            assert_eq!(
                cabinet.camera_origin(),
                i16::try_from(limit).unwrap_or_default()
            );
        }
    }

    fn clear_street(cabinet: &mut Cabinet, limit: u16) {
        let mut held = None;
        let end = i16::try_from(limit).unwrap_or_default();
        for tick in 1..3000 {
            let living = cabinet.enemies().any(|enemy| enemy.health > 0);
            if (OPENING_COMBAT_LIMIT < cabinet.camera_origin()
                && cabinet.camera_origin() < LATER_COMBAT_GATE)
                || (LATER_COMBAT_GATE < cabinet.camera_origin()
                    && cabinet.camera_origin() < MID_STREET_ADMISSION)
                || (MID_STREET_COMBAT_GATE < cabinet.camera_origin()
                    && cabinet.camera_origin() < FAR_STREET_ADMISSION)
            {
                assert!(!living);
            }
            if !living && cabinet.camera_origin() == end {
                return;
            }
            let (direction, requested) = cabinet.diagnostic_intent();
            let mut actions = direction_changes(&mut held, direction);
            if let Some(action) = requested {
                actions.extend(pulse(action));
            }
            cabinet.input_step(tick, &actions);
            assert_eq!(cabinet.phase(), Phase::Playing);
        }
    }

    fn direction_changes(
        held: &mut Option<Control>,
        direction: Option<Control>,
    ) -> Vec<LogicalAction<Action>> {
        if *held == direction {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if let Some(previous) = *held {
            let [_, release] = pulse(previous);
            actions.push(release);
        }
        if let Some(next) = direction {
            let [press, _] = pulse(next);
            actions.push(press);
        }
        *held = direction;
        actions
    }

    #[test]
    fn opening_street_rejects_art_outside_world_coordinates() {
        let result = Cabinet::with_opening_street()
            .and_then(|cabinet| cabinet.with_scroll_limit(32_767));
        assert_eq!(result, Err(LayoutError::ScrollOverflow));
    }

    fn assert_opening_arrivals(cabinet: &mut Cabinet) {
        for tick in 63..363 {
            cabinet.input_step(tick, &[]);
            let camera = cabinet.camera_origin();
            let count = match camera {
                0..=49 => 1,
                50..=81 => 2,
                82..=153 => 3,
                154..=209 => 4,
                210..=273 => 5,
                274..=361 => 6,
                _ => 7,
            };
            assert_eq!(cabinet.enemies().count(), count);
            if camera == 362 {
                break;
            }
        }
        assert_eq!(cabinet.camera_origin(), 362);
    }

    fn assert_combat_travel_bound(cabinet: &mut Cabinet) {
        for tick in 363..403 {
            cabinet.input_step(tick, &[]);
            assert_eq!(cabinet.camera_origin(), 362);
        }
        let [_, release] = pulse(Control::Right);
        cabinet.input_step(403, &[release]);
        cabinet.input_step(404, &pulse(Control::Pause));
        let frozen = *cabinet;
        cabinet.input_step(405, &[]);
        assert_eq!(*cabinet, frozen);
        cabinet.reset_world();
        assert_eq!(cabinet.camera_origin(), 0);
        assert_eq!(cabinet.enemies().count(), 1);
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(1));
    }

    fn restraint_fixture(enabled: bool) -> Result<Cabinet, LayoutError> {
        let mut cabinet = Cabinet::with_opening_street()?
            .with_scroll_limit(154)?
            .with_marge_restraint(enabled);
        cabinet.input_step(0, &pulse(Control::Start));
        cabinet.camera = 154;
        let [marge, ..] = &mut cabinet.slots;
        marge.restore_position(154);
        let position = marge.station.position;
        if let Some(encounter) = &mut cabinet.encounter {
            encounter.admit(154);
            let [initial, first, second, third, ..] = &mut encounter.enemies;
            for enemy in [initial, second, third].into_iter().flatten() {
                enemy.health = 0;
                enemy.action = EnemyAction::Defeat { tick: ENEMY_DEFEAT_TICKS };
            }
            *first = Some(Enemy::new(Point {
                x: position.x.saturating_add(4),
                ..position
            }));
        }
        cabinet.input_step(1, &[]);
        Ok(cabinet)
    }

    fn held(cabinet: &Cabinet) -> bool {
        let [marge, ..] = cabinet.stations();
        marge.maneuver == Maneuver::Restrained
            && cabinet
                .enemies()
                .any(|enemy| matches!(enemy.action, EnemyAction::Restrain(_)))
    }

    #[test]
    fn restraint_escape_requires_twelve_individual_edge_ticks() {
        let result = restraint_fixture(true);
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        assert!(held(&cabinet));
        let [initial, ..] = cabinet.stations();
        assert_eq!(initial.health, PLAYER_HEALTH - ENEMY_DAMAGE);
        let [direction, release] = pulse(Control::Left);
        cabinet.input_step(2, &[direction]);
        for tick in 3..15 {
            // Two pulses in one tick are a chord, even if neither is held
            // when the fixed step begins.
            let [primary_down, primary_up] = pulse(Control::Primary);
            let [secondary_down, secondary_up] = pulse(Control::Secondary);
            cabinet.input_step(tick, &[
                primary_down,
                primary_up,
                secondary_down,
                secondary_up,
            ]);
        }
        assert!(held(&cabinet));
        assert_restraint_escape(&mut cabinet, initial, release);
    }

    fn assert_restraint_escape(
        cabinet: &mut Cabinet,
        initial: Station,
        release: LogicalAction<Action>,
    ) {
        for (tick, control) in (15..26)
            .zip([Control::Secondary, Control::Primary].into_iter().cycle())
        {
            cabinet.input_step(tick, &pulse(control));
        }
        assert!(held(cabinet));
        cabinet.input_step(26, &pulse(Control::Secondary));
        for tick in 27..30 {
            cabinet.input_step(tick, &[]);
            assert!(held(cabinet));
        }
        cabinet.input_step(30, &[release]);
        let [escaped, ..] = cabinet.stations();
        assert_eq!(escaped.maneuver, Maneuver::Idle);
        assert_eq!(escaped.position, initial.position);
        assert_eq!(escaped.facing, initial.facing);
        assert_eq!(escaped.attack_cues, initial.attack_cues);
        assert!(cabinet.enemies().any(|enemy| {
            enemy.health > 0
                && enemy.action == EnemyAction::Repelled { tick: 0 }
        }));
    }

    #[test]
    fn repelled_enemy_recovers_without_defeat_or_blink() {
        for (facing, end, duration) in
            [(Facing::Right, 144, 91), (Facing::Left, 256, 103)]
        {
            let layout = StreetLayout::DOWNTOWN;
            let mut enemy = Enemy::new(Point { x: 200, y: 190 });
            enemy.facing = facing;
            enemy.action = EnemyAction::Repelled { tick: 0 };
            let health = enemy.health;
            for tick in 0..duration {
                assert_eq!(enemy.action, EnemyAction::Repelled { tick });
                assert!(enemy.visible());
                enemy.advance_repelled(layout, tick);
            }
            assert_eq!(enemy.position, Point { x: end, y: 190 });
            assert_eq!(enemy.health, health);
            assert_eq!(enemy.action, EnemyAction::Approach { tick: 0 });
        }
    }

    #[test]
    fn restraint_accepts_staggered_edges_and_preserves_pause() {
        let result = restraint_fixture(true);
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [press, release] = pulse(Control::Primary);
        cabinet.input_step(2, slice::from_ref(&press));
        for tick in 3..20 {
            cabinet.input_step(tick, slice::from_ref(&press));
        }
        for tick in 20..30 {
            cabinet.input_step(tick, &pulse(Control::Secondary));
        }
        assert!(held(&cabinet));
        cabinet.input_step(30, &pulse(Control::Pause));
        let frozen = cabinet;
        for tick in 31..40 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet, frozen);
        cabinet.input_step(40, &pulse(Control::Pause));
        cabinet.input_step(41, &pulse(Control::Secondary));
        for tick in 42..45 {
            cabinet.input_step(tick, &[]);
            assert!(held(&cabinet));
        }
        cabinet.input_step(45, &[release]);
        assert!(!held(&cabinet));
    }

    #[test]
    fn restraint_depletion_releases_station_and_preserves_stock_recovery() {
        let result = restraint_fixture(true);
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [slot, ..] = &mut cabinet.slots;
        slot.station.health = ENEMY_DAMAGE;
        for tick in 2..26 {
            cabinet.input_step(tick, &[]);
            assert!(held(&cabinet));
        }
        cabinet.input_step(26, &[]);
        assert_restraint_depleted(&mut cabinet);
    }

    fn assert_restraint_depleted(cabinet: &mut Cabinet) {
        let [marge, ..] = cabinet.stations();
        assert_eq!(marge.health, 0);
        assert_eq!(marge.hurt_cues, 0);
        assert_eq!(marge.stocks, PLAYER_STOCKS - 1);
        assert!(matches!(
            marge.participation,
            Participation::Recovering { .. }
        ));
        assert_eq!(marge.maneuver, Maneuver::Idle);
        assert!(!held(cabinet));
        assert!(cabinet.enemies().all(|enemy| {
            !matches!(enemy.action, EnemyAction::Restrain(_))
        }));
        cabinet.reset_world();
        assert!(!held(cabinet));
    }

    #[test]
    fn legacy_encounter_keeps_the_standard_punch_at_restraint_contact() {
        let result = restraint_fixture(false);
        assert!(result.is_ok());
        let Ok(cabinet) = result else {
            return;
        };
        assert!(!held(&cabinet));
        assert!(
            cabinet.enemies().any(|enemy| {
                enemy.action == EnemyAction::Attack { tick: 0 }
            })
        );
    }

    #[test]
    fn opening_street_can_reach_restraint_through_play() {
        let result = Cabinet::with_opening_street()
            .and_then(|cabinet| cabinet.with_scroll_limit(154))
            .map(|cabinet| cabinet.with_marge_restraint(true));
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &pulse(Control::Start));
        let [press, release] = pulse(Control::Right);
        cabinet.input_step(1, slice::from_ref(&press));
        for tick in 2..40 {
            let [marge, ..] = cabinet.stations();
            if cabinet.enemy().is_some_and(|enemy| {
                enemy.position.x.abs_diff(marge.position.x) <= 28
            }) {
                break;
            }
            cabinet.input_step(tick, &[]);
        }
        cabinet.input_step(40, &[release]);
        cabinet.input_step(41, &pulse(Control::Primary));
        for tick in 42..62 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(0));
        cabinet.input_step(62, slice::from_ref(&press));
        for tick in 63..500 {
            cabinet.input_step(tick, &[]);
            if held(&cabinet) {
                break;
            }
        }
        assert_eq!(cabinet.camera_origin(), 154);
        assert!(held(&cabinet));
    }

    fn playing(spawn: Point) -> Result<Cabinet, LayoutError> {
        let mut cabinet =
            Cabinet::with_encounter(StreetLayout::DOWNTOWN, spawn)?;
        cabinet.input_step(2, &pulse(Control::Start));
        Ok(cabinet)
    }

    #[test]
    fn midgame_join_fills_each_station_without_delaying_control() {
        for player in [Player::Homer, Player::Bart, Player::Lisa] {
            let result = playing(Point { x: 288, y: 208 });
            assert!(result.is_ok());
            let Ok(mut cabinet) = result else {
                return;
            };
            cabinet.input_step(3, &pulse_for(player, Control::Start));
            let stations = cabinet.stations();
            assert!(stations.get(player.index()).is_some());
            let Some(joined) = stations.get(player.index()) else {
                return;
            };
            assert!(joined.can_fight());
            assert_eq!(joined.display_health, 0);
            cabinet.input_step(4, &[LogicalAction {
                action: Action {
                    control: Control::Left,
                    player,
                },
                down: true,
            }]);
            let moved = cabinet.stations();
            assert!(moved.get(player.index()).is_some_and(|station| {
                station.position.x < joined.position.x
                    && station.display_health == 0
            }));
            verify_join_meter(&mut cabinet, player);
        }
    }

    fn verify_join_meter(cabinet: &mut Cabinet, player: Player) {
        for offset in 2..=66u8 {
            let tick = u64::from(offset).saturating_add(3);
            if offset == 8 {
                cabinet.input_step(tick, &pulse_for(player, Control::Start));
            } else {
                cabinet.input_step(tick, &[]);
            }
            let expected = if offset < 4 {
                0
            } else {
                offset
                    .saturating_sub(4)
                    .checked_div(2)
                    .unwrap_or_default()
                    .saturating_add(1)
            };
            let stations = cabinet.stations();
            assert!(stations.get(player.index()).is_some_and(|station| {
                station.health == PLAYER_HEALTH
                    && station.display_health == expected
            }));
        }
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
    fn defeat_lifetime_blinks_and_then_stays_absent() {
        let result = playing(Point { x: 132, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(3, &pulse(Control::Primary));
        for tick in 4..14 {
            cabinet.input_step(tick, &[]);
        }
        let outcome = cabinet.enemy();
        assert!(outcome.is_some());
        let Some(impact) = outcome else {
            return;
        };
        assert_eq!(impact.action, EnemyAction::Defeat { tick: 0 });
        assert_eq!(impact.health, 0);
        verify_defeat_timeline(&mut cabinet, impact);
        let defeated = cabinet.enemy();
        for tick in 81..120 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.enemy(), defeated);
        assert_eq!(
            cabinet.stations().first().map(|slot| slot.health),
            Some(PLAYER_HEALTH)
        );
        assert_eq!(cabinet.impact_cues(), 1);
        cabinet.reset_world();
        assert_eq!(cabinet.impact_cues(), 1);
    }

    fn verify_defeat_timeline(cabinet: &mut Cabinet, impact: Enemy) {
        for tick in 0..=ENEMY_DEFEAT_TICKS {
            let state = cabinet.enemy();
            assert!(state.is_some());
            let Some(enemy) = state else {
                return;
            };
            assert_eq!(enemy.action, EnemyAction::Defeat { tick });
            assert_eq!(enemy.health, 0);
            assert_eq!(cabinet.stations().map(|station| station.score), [
                u16::from(tick == ENEMY_DEFEAT_TICKS),
                0,
                0,
                0
            ]);
            assert_eq!(
                enemy.visible(),
                tick < 52
                    || (53..66).contains(&tick) && !tick.is_multiple_of(2)
            );
            assert_eq!(
                enemy.position.x,
                impact
                    .position
                    .x
                    .saturating_add(i16::from(tick.min(33)).saturating_mul(2))
            );
            cabinet.input_step(14u64.saturating_add(u64::from(tick)), &[]);
        }
    }

    #[test]
    fn defeated_enemy_pauses_and_cannot_be_struck_again() {
        let layout = StreetLayout {
            max: Point { x: 138, y: 208 },
            spawn: [
                Point { x: 110, y: 190 },
                Point { x: 88, y: 190 },
                Point { x: 132, y: 178 },
                Point { x: 132, y: 178 },
            ],
            ..StreetLayout::DOWNTOWN
        };
        let result = Cabinet::with_encounter(layout, Point { x: 132, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(2, &pulse(Control::Start));
        cabinet.input_step(3, &pulse(Control::Primary));
        for tick in 4..24 {
            cabinet.input_step(tick, &[]);
        }
        let frozen = cabinet.enemy();
        assert_eq!(
            frozen.map(|enemy| enemy.action),
            Some(EnemyAction::Defeat { tick: 10 })
        );
        assert_eq!(frozen.map(|enemy| enemy.position.x), Some(138));
        cabinet.input_step(24, &pulse(Control::Pause));
        for tick in 25..40 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|station| station.score), [0; 4]);
        assert_eq!(cabinet.enemy(), frozen);
        assert_resume_defeat(&mut cabinet);
    }

    fn assert_resume_defeat(cabinet: &mut Cabinet) {
        cabinet.input_step(40, &pulse(Control::Pause));
        cabinet.input_step(41, &pulse(Control::Primary));
        for tick in 42..52 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(
            cabinet.enemy().map(|enemy| enemy.action),
            Some(EnemyAction::Defeat { tick: 22 })
        );
        assert_eq!(
            cabinet.stations().first().map(|station| station.health),
            Some(PLAYER_HEALTH)
        );
    }

    #[test]
    fn leftward_defeat_mirrors_and_stops_at_the_boundary() {
        let result = playing(Point { x: 90, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [press, release] = pulse(Control::Left);
        cabinet.input_step(3, &[press]);
        cabinet.input_step(4, &[release]);
        cabinet.input_step(5, &pulse(Control::Primary));
        for tick in 6..49 {
            cabinet.input_step(tick, &[]);
        }
        let defeated = cabinet.enemy();
        assert_eq!(
            defeated.map(|enemy| (enemy.action, enemy.facing, enemy.position)),
            Some((EnemyAction::Defeat { tick: 33 }, Facing::Right, Point {
                x: StreetLayout::DOWNTOWN.min.x,
                y: 190
            }))
        );
    }

    #[test]
    fn all_station_ground_strikes_resolve_at_observed_offset() {
        for player in Player::ALL {
            let layout = StreetLayout::DOWNTOWN;
            let Some(position) = layout.spawn.get(player.index()).copied()
            else {
                continue;
            };
            let result = Cabinet::with_encounter(layout, Point {
                x: position.x.saturating_add(18),
                y: position.y,
            });
            assert!(result.is_ok());
            let Ok(mut cabinet) = result else {
                return;
            };
            cabinet.input_step(2, &pulse_for(player, Control::Start));
            cabinet.input_step(3, &pulse_for(player, Control::Primary));
            for tick in 0..PLAYER_STRIKE {
                assert_eq!(cabinet.impact_cues(), 0);
                assert_eq!(
                    cabinet.enemy().map(|enemy| enemy.health),
                    Some(ENEMY_HEALTH)
                );
                cabinet.input_step(u64::from(tick).saturating_add(4), &[]);
            }
            assert_eq!(
                cabinet.enemy().map(|enemy| enemy.health),
                Some(ENEMY_HEALTH.saturating_sub(1))
            );
            assert_eq!(cabinet.impact_cues(), 1);
            assert_station_defeat_credit(&mut cabinet, player);
        }
    }

    fn assert_station_defeat_credit(cabinet: &mut Cabinet, player: Player) {
        let expected = Player::ALL.map(|seat| u16::from(seat == player));
        for tick in 14..79 {
            assert_eq!(cabinet.stations().map(|station| station.score), [0; 4]);
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|station| station.score), [0; 4]);
        cabinet.input_step(79, &[]);
        assert_eq!(cabinet.stations().map(|station| station.score), expected);
        cabinet.input_step(80, &[]);
        assert_eq!(cabinet.stations().map(|station| station.score), expected);
        cabinet.reset_world();
        assert_eq!(cabinet.stations().map(|station| station.score), [0; 4]);
    }

    #[test]
    fn earned_score_survives_continue_and_resets_on_new_campaign() {
        let result = scored_and_knocked_out();
        assert!(result.is_ok());
        let Ok(cabinet) = result else {
            return;
        };
        let [waiting, ..] = cabinet.stations();
        assert_eq!(waiting.score, 1);
        assert!(matches!(
            waiting.participation,
            Participation::Continuing { .. }
        ));
        verify_scored_continue(cabinet, waiting.score);
        verify_scored_expiry(cabinet);
    }

    fn verify_scored_continue(mut continued: Cabinet, earned: u16) {
        continued.input_step(2000, &pulse(Control::Start));
        let [restored, ..] = continued.stations();
        assert_eq!(restored.score, earned);
        assert_eq!(restored.health, PLAYER_HEALTH);
        assert_eq!(restored.participation, Participation::Playing);
        continued.input_step(2001, &pulse_for(Player::Homer, Control::Start));
        assert_eq!(continued.stations().map(|station| station.score), [
            1, 0, 0, 0
        ]);
    }

    fn verify_scored_expiry(mut expired: Cabinet) {
        for tick in 0..u64::from(CONTINUE_TICKS) {
            expired.input_step(tick, &[]);
        }
        assert_eq!(expired.phase(), Phase::MainMenu);
        expired.input_step(2000, &pulse(Control::Start));
        assert_eq!(expired.phase(), Phase::Playing);
        assert_eq!(expired.stations().map(|station| station.score), [0; 4]);
    }

    #[test]
    fn expired_station_rejoins_without_resetting_teammate_score() {
        let result = scored_and_knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let [waiting, ..] = cabinet.stations();
        assert_eq!(waiting.score, 1);
        assert!(matches!(
            waiting.participation,
            Participation::Continuing { .. }
        ));
        // Isolate a second legal encounter beside the newly joining station.
        if let Some(encounter) = &mut cabinet.encounter {
            let [_, second, ..] = &mut encounter.enemies;
            *second = Some(Enemy::new(Point { x: 106, y: 190 }));
        }
        cabinet.input_step(2000, &pulse_for(Player::Homer, Control::Start));
        cabinet.input_step(2001, &pulse_for(Player::Homer, Control::Primary));
        assert_multiplayer_expiry_and_rejoin(&mut cabinet);
    }

    fn assert_multiplayer_expiry_and_rejoin(cabinet: &mut Cabinet) {
        for tick in 2002..2080 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|station| station.score), [
            1, 1, 0, 0
        ]);
        let end = 2080u64.saturating_add(u64::from(CONTINUE_TICKS));
        for tick in 2080..end {
            cabinet.input_step(tick, &[]);
        }
        let [expired, teammate, ..] = cabinet.stations();
        assert_eq!(cabinet.phase(), Phase::Playing);
        assert_eq!(expired.participation, Participation::Absent);
        assert_eq!(expired.score, 1);
        assert_eq!(teammate.participation, Participation::Playing);
        assert_eq!(teammate.score, 1);
        assert_local_rejoin(cabinet, end, teammate);
    }

    fn assert_local_rejoin(
        cabinet: &mut Cabinet,
        tick: u64,
        teammate: Station,
    ) {
        cabinet.input_step(tick, &pulse(Control::Start));
        let [rejoined, preserved, ..] = cabinet.stations();
        assert_eq!(rejoined.participation, Participation::Playing);
        assert_eq!(rejoined.score, 0);
        assert_eq!(preserved, teammate);
    }

    fn scored_and_knocked_out() -> Result<Cabinet, LayoutError> {
        let mut cabinet = playing(Point { x: 132, y: 190 })?;
        // Keep a second, initially out-of-range opponent alive to exhaust
        // the station after a genuine input-driven defeat earns a point.
        if let Some(encounter) = &mut cabinet.encounter {
            let [_, second, ..] = &mut encounter.enemies;
            *second = Some(Enemy::new(Point { x: 250, y: 190 }));
        }
        cabinet.input_step(3, &pulse(Control::Primary));
        for tick in 4..2000 {
            cabinet.input_step(tick, &[]);
            if matches!(
                cabinet.stations().first().map(|slot| slot.participation),
                Some(Participation::Continuing { .. })
            ) {
                break;
            }
        }
        Ok(cabinet)
    }

    #[test]
    fn score_retains_continue_but_resets_new_participation() {
        let mut slot = Slot::new(Point { x: 132, y: 190 });
        slot.start(0, false);
        slot.station.score = 9_999;
        slot.station.award_defeat_point();
        assert_eq!(slot.station.score, 0);
        slot.station.award_defeat_point();
        slot.station.participation =
            Participation::Continuing { ticks_left: 1 };
        slot.start(0, true);
        assert_eq!(slot.station.score, 1);
        slot.station.participation =
            Participation::Continuing { ticks_left: 1 };
        slot.advance_participation(0);
        assert_eq!(slot.station.participation, Participation::Absent);
        slot.start(0, true);
        assert_eq!(slot.station.score, 0);
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
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(0));
    }

    #[test]
    fn aerial_attack_can_defeat_the_first_enemy() {
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
        assert_eq!(cabinet.enemy().map(|enemy| enemy.health), Some(0));
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
            for tick in 4..14 {
                cabinet.input_step(tick, &[]);
            }
            assert_eq!(
                cabinet.enemy().map(|enemy| enemy.health),
                Some(ENEMY_HEALTH)
            );
            assert_eq!(cabinet.impact_cues(), 0);
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
            assert_eq!(
                cabinet.stations().first().map(|slot| slot.hurt_cues),
                Some(u64::from(tick >= ENEMY_STRIKE))
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
        assert_eq!(grounded.stations().map(|slot| slot.hurt_cues), [
            1, 0, 0, 0
        ]);
        assert_eq!(jumping.stations().map(|slot| slot.hurt_cues), [0; 4]);
    }

    #[test]
    fn regular_stock_depletion_emits_one_reaction_cue() {
        for stocks in [PLAYER_STOCKS, 1] {
            let result = playing(Point { x: 128, y: 190 });
            assert!(result.is_ok());
            let Ok(mut cabinet) = result else {
                return;
            };
            let [marge_slot, ..] = &mut cabinet.slots;
            marge_slot.station.health = ENEMY_DAMAGE;
            marge_slot.station.stocks = stocks;
            for tick in 3..17 {
                cabinet.input_step(tick, &[]);
            }
            let [marge, ..] = cabinet.stations();
            assert_eq!(marge.health, 0);
            assert_eq!(marge.stocks, stocks - 1);
            assert_eq!(marge.hurt_cues, 1);
            for tick in 17..50 {
                cabinet.input_step(tick, &[]);
                let [recovering, ..] = cabinet.stations();
                assert_eq!(recovering.hurt_cues, 1);
            }
        }
    }

    #[test]
    fn hurt_cues_survive_reset_and_count_a_new_session_hit() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        for tick in 3..17 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|slot| slot.hurt_cues), [1, 0, 0, 0]);
        cabinet.reset_world();
        assert_eq!(cabinet.stations().map(|slot| slot.hurt_cues), [1, 0, 0, 0]);
        cabinet.input_step(17, &pulse(Control::Start));
        for tick in 18..32 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().map(|slot| slot.hurt_cues), [2, 0, 0, 0]);
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
                current.attack_cues,
                current.hurt_cues
            ),
            (
                previous.position,
                0,
                Maneuver::Idle,
                previous.attack_cues,
                previous.hurt_cues
            ),
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
        assert_eq!(continued.display_health, 0);
    }

    #[test]
    fn continuing_meter_refills_without_delaying_control() {
        let result = knocked_out();
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        cabinet.input_step(0, &pulse(Control::Start));
        let [continued, ..] = cabinet.stations();
        assert!(continued.can_fight());
        assert_eq!(continued.display_health, 0);
        cabinet.input_step(1, &[LogicalAction {
            action: Action {
                control: Control::Left,
                player: Player::Marge,
            },
            down: true,
        }]);
        let [moving, ..] = cabinet.stations();
        assert!(moving.position.x < continued.position.x);
        assert_eq!(moving.display_health, 0);
        cabinet.input_step(2, &[]);
        cabinet.input_step(3, &[]);
        let [before_refill, ..] = cabinet.stations();
        assert_eq!(before_refill.display_health, 0);
        verify_continue_meter_pause(&mut cabinet, before_refill);
    }

    fn verify_continue_meter_pause(
        cabinet: &mut Cabinet,
        before_refill: Station,
    ) {
        cabinet.input_step(4, &pulse(Control::Pause));
        for tick in 5..15 {
            cabinet.input_step(tick, &[]);
        }
        assert_eq!(cabinet.stations().first(), Some(&before_refill));
        cabinet.input_step(15, &pulse(Control::Pause));
        assert_eq!(
            cabinet
                .stations()
                .first()
                .map(|station| station.display_health),
            Some(1)
        );
        cabinet.input_step(16, &[]);
        assert_eq!(
            cabinet
                .stations()
                .first()
                .map(|station| station.display_health),
            Some(1)
        );
        cabinet.input_step(17, &[]);
        assert_eq!(
            cabinet
                .stations()
                .first()
                .map(|station| station.display_health),
            Some(2)
        );
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
            ticks_left: 1
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
        assert_eq!(between, Station {
            maneuver: Maneuver::Hurt { tick: 2 },
            ..first
        });
        check_meter_pause(&mut cabinet, between);
    }

    fn check_meter_pause(cabinet: &mut Cabinet, between: Station) {
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
        predicate: impl Fn(Station) -> bool,
    ) -> Station {
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
        let recovered = advance_until(cabinet, Station::can_fight);
        assert_eq!(recovered.display_health, PLAYER_HEALTH - 2);
        assert_eq!(recovered.stocks, 1);
        let final_empty = advance_until(cabinet, |station| {
            matches!(station.participation, Participation::Continuing { .. })
        });
        assert_eq!(final_empty.stocks, 0);
        assert_eq!(final_empty.display_health, 0);
        cabinet.input_step(0, &pulse(Control::Start));
        let [continued, ..] = cabinet.stations();
        assert_eq!(continued.stocks, PLAYER_STOCKS);
        assert_eq!(continued.display_health, 0);
    }

    #[test]
    fn automatic_refill_admits_held_movement_before_full_meter() {
        let result = playing(Point { x: 128, y: 190 });
        assert!(result.is_ok());
        let Ok(mut cabinet) = result else {
            return;
        };
        let empty = advance_until(&mut cabinet, |station| {
            station.stocks == 1 && station.display_health == 0
        });
        assert_eq!(empty.health, 0);
        let [press, _] = pulse(Control::Left);
        cabinet.input_step(0, slice::from_ref(&press));
        let restored = advance_until(&mut cabinet, |station| {
            station.health == PLAYER_HEALTH && station.display_health == 0
        });
        for offset in 0..=62u8 {
            cabinet.input_step(u64::from(offset), &[]);
            let [station, ..] = cabinet.stations();
            let distance = i16::from(offset.saturating_sub(58)) * 2;
            assert_eq!(station.display_health, (offset + 1).div_ceil(2));
            assert_eq!(station.position, Point {
                x: restored.position.x - distance,
                ..restored.position
            });
            assert_eq!(station.can_fight(), offset >= 59);
            assert_eq!(station.stocks, 1);
        }
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
