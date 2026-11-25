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
//   - Title-specific four-player entry and initial street movement rules.
// - Must-Not:
//   - Access operating-system devices, render or interpret original ROMs.
// - Allows:
//   - Inputs: Ordered logical controls at a fixed simulation tick.
//   - Outputs: Cabinet phase, active slots, positions and facing.
//   - Side effects: Deterministic in-memory state transitions.
// - Split-When:
//   - Independent title combat or stage progression rules are implemented.
// - Merge-When:
//   - Another title module duplicates player entry or held-control state.
// - Summary:
//   - Initial four-player entry and street movement prototype.
// - Description:
//   - Models direct free-play admission and fixed-step street movement.
// - Usage:
//   - Drive with the shared InputRouter and StepClock or replay adapter.
// - Defaults:
//   - All players absent; no payment or platform activity.

//! Initial cabinet entry and street movement for the pinned four-player set.

mod encounter;
#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Native authenticated host input for the local arcade screen.
pub mod lan;
mod maneuver;
/// Host-authoritative remote input for the free-play cabinet.
pub mod remote;

#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Native prototype host for locally imported art.
pub mod native;
use std::mem;
use std::time::Duration;

pub use encounter::{Enemy, EnemyAction};
use games::input::LogicalAction;
use games::replay::InputStep;
pub use games::session::SessionPhase as Phase;
use games::session::{LocalSession, MenuInput, SessionEffect};
pub use maneuver::{AirAttackMotion, JumpMotion, Maneuver};

/// Fixed reference screen period: 384 by 264 clocks at 6 MHz.
pub const TICK_INTERVAL: Duration = Duration::from_micros(16_896);

/// Approximate twenty-second original continue opportunity in ticks.
pub const CONTINUE_TICKS: u16 = 1200;
const PLAYER_HEALTH: u8 = 3;
// Original ground-attack follow-up edges first take effect at offset 20.
const GROUND_ATTACK_RECOVERY_TICKS: u8 = 20;

/// Characters and default input seat order for the four-player edition.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Player {
    /// Third station.
    Bart,
    /// Second station.
    Homer,
    /// Fourth station.
    Lisa,
    /// First station.
    Marge,
}

impl Player {
    /// Stations in cabinet order.
    pub const ALL: [Self; 4] =
        [Self::Marge, Self::Homer, Self::Bart, Self::Lisa];

    /// Index in a complete four-station array.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Marge => 0,
            Self::Homer => 1,
            Self::Bart => 2,
            Self::Lisa => 3,
        }
    }
}

/// Public digital controls of one cabinet station.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Control {
    /// Down on the eight-way stick.
    Down,
    /// Left on the eight-way stick.
    Left,
    /// Suspend or resume the local session.
    Pause,
    /// First button; confirms a menu choice or starts a street attack.
    Primary,
    /// Right on the eight-way stick.
    Right,
    /// Second button; starts a jump in the street.
    Secondary,
    /// Start immediately, join or rejoin the street.
    Start,
    /// Up on the eight-way stick.
    Up,
}

impl Control {
    const fn mask(self) -> u8 {
        match self {
            Self::Pause => 1,
            Self::Start => 2,
            Self::Primary => 4,
            Self::Secondary => 8,
            Self::Left => 16,
            Self::Right => 32,
            Self::Up => 64,
            Self::Down => 128,
        }
    }
}

/// A logical action routed through exactly one input seat.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Action {
    /// Digital control being pressed or released.
    pub control: Control,
    /// Input seat receiving the action, independent of character choice.
    pub player: Player,
}

/// A station's current admission state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Participation {
    /// Not playing.
    Absent,
    /// Knocked out; a new Start edge can restore play for free.
    Continuing {
        /// Remaining countdown ticks.
        ticks_left: u16,
    },
    /// Station is active in the street.
    Playing,
}

/// Integer world-space foot position in the first street prototype.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    /// Horizontal location.
    pub x: i16,
    /// Vertical location.
    pub y: i16,
}

/// Horizontal character facing, retained during vertical movement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Facing {
    /// Facing toward the left.
    Left,
    /// Facing toward the right.
    Right,
}

/// Prototype walkable area and four initial foot positions.
///
/// Isolated original-machine observations support the two-pixel rate
/// during some unobstructed movement intervals for all four stations.
/// Boundaries, spawn offsets, diagonal motion and collisions remain
/// provisional until further independent comparisons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreetLayout {
    /// Maximum foot position.
    pub max: Point,
    /// Minimum foot position.
    pub min: Point,
    /// Spawn positions in cabinet order.
    pub spawn: [Point; 4],
}

impl StreetLayout {
    /// Initial downtown screen prototype, before camera and collision rules.
    pub const DOWNTOWN: Self = Self {
        max: Point { x: 288, y: 208 },
        min: Point { x: 32, y: 146 },
        spawn: [
            Point { x: 110, y: 190 },
            Point { x: 88, y: 190 },
            Point { x: 132, y: 178 },
            Point { x: 154, y: 178 },
        ],
    };

    fn contains(self, point: Point) -> bool {
        (self.min.x..=self.max.x).contains(&point.x)
            && (self.min.y..=self.max.y).contains(&point.y)
    }
}

/// Invalid prototype movement configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// The configured camera travel exceeds the coordinate range.
    ScrollOverflow,
    /// A spawn point is outside the walkable area.
    SpawnOutside,
}

/// A read-only view of an individual station.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Station {
    /// Cumulative attack cues at the observed provisional onset tick.
    pub attack_cues: u64,
    /// Current sprite facing.
    pub facing: Facing,
    /// Experimental encounter health; zero means the station is knocked out.
    pub health: u8,
    /// Current attack or jump, independent of rendering.
    pub maneuver: Maneuver,
    /// Whether the station has joined.
    pub participation: Participation,
    /// Current foot position.
    pub position: Point,
    /// Consecutive walking ticks, zero-based; None outside displaced walking.
    pub walking_ticks: Option<u64>,
}

impl Station {
    const fn can_fight(self) -> bool {
        matches!(self.participation, Participation::Playing) && self.health > 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Recovery {
    Ground(u8),
    Landing { turn: bool },
    Ready,
}

impl Recovery {
    const fn advance(self) -> Self {
        match self {
            Self::Ground(ticks) if ticks > 1 => {
                Self::Ground(ticks.saturating_sub(1))
            },
            Self::Ground(_) | Self::Landing { .. } | Self::Ready => Self::Ready,
        }
    }

    const fn allows_turn(self) -> bool {
        matches!(self, Self::Ready | Self::Landing { turn: true })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Slot {
    action_recovery: Recovery,
    attack_quarters: i16,
    held: u8,
    spawn: Point,
    station: Station,
}

impl Slot {
    fn advance_maneuver(&mut self, player: Player, queued: Option<Maneuver>) {
        self.action_recovery = self.action_recovery.advance();
        if self.station.can_fight()
            && matches!(queued, Some(Maneuver::Attack { .. }))
            && let Maneuver::Jump(jump) = self.station.maneuver
            && jump.accepts_attack()
        {
            self.station.maneuver = Maneuver::air_attack(jump);
            return;
        }
        let previous = self.station.maneuver;
        self.station.maneuver =
            previous.advance(player, self.held(Control::Secondary));
        if previous.airborne() && self.station.maneuver == Maneuver::Idle {
            // Landing is visible before new ground actions can begin.
            self.action_recovery = Recovery::Landing {
                turn: !matches!(previous, Maneuver::AirAttack(_)),
            };
        }
        if let Some(maneuver) = queued {
            self.begin_maneuver(maneuver);
        }
    }

    // Retain a quarter-pixel remainder and round the position downward.
    const fn attack_step(&mut self, horizontal: i16) -> i16 {
        let next = self.attack_quarters.saturating_add(horizontal.signum());
        if next < 0 {
            self.attack_quarters = next.saturating_add(4);
            -1
        } else if next >= 4 {
            self.attack_quarters = next.saturating_sub(4);
            1
        } else {
            self.attack_quarters = next;
            0
        }
    }

    fn begin_maneuver(&mut self, maneuver: Maneuver) {
        if self.station.can_fight()
            && self.station.maneuver == Maneuver::Idle
            && self.action_recovery == Recovery::Ready
        {
            if matches!(maneuver, Maneuver::Attack { .. }) {
                self.action_recovery =
                    Recovery::Ground(GROUND_ATTACK_RECOVERY_TICKS);
            }
            self.attack_quarters = 0;
            self.station.maneuver = maneuver;
            self.station.walking_ticks = None;
        }
    }

    const fn damage(&mut self) {
        self.attack_quarters = 0;
        self.action_recovery = Recovery::Ready;
        self.station.health = self.station.health.saturating_sub(1);
        self.station.maneuver = Maneuver::Idle;
        self.station.walking_ticks = None;
        if self.station.health == 0 {
            self.station.participation = Participation::Continuing {
                ticks_left: CONTINUE_TICKS,
            };
        }
    }

    fn edge(
        &mut self,
        control: Control,
        down: bool,
        pending: &mut Option<Maneuver>,
    ) -> bool {
        if control == Control::Secondary && !down {
            self.station.maneuver.release_jump();
            if let Some(maneuver) = pending {
                maneuver.release_jump();
            }
        }
        let mask = control.mask();
        let was_held = self.held & mask != 0;
        if down {
            self.held |= mask;
        } else {
            self.held &= !mask;
        }
        down && !was_held
    }

    const fn expire_continue(&mut self) {
        if let Participation::Continuing { ticks_left } =
            self.station.participation
        {
            self.station.participation = if ticks_left <= 1 {
                Participation::Absent
            } else {
                Participation::Continuing {
                    ticks_left: ticks_left.saturating_sub(1),
                }
            };
        }
    }

    const fn held(self, control: Control) -> bool {
        self.held & control.mask() != 0
    }

    fn maneuver_step(&mut self, dx: i16, dy: i16) -> (i16, i16) {
        match self.station.maneuver {
            Maneuver::AirAttack(motion) => {
                let forward = if self.station.facing == Facing::Left {
                    -1
                } else {
                    1
                };
                (motion.step().saturating_mul(forward), 0)
            },
            Maneuver::Special { .. } => (0, 0),
            Maneuver::Attack { .. } => (self.attack_step(dx), 0),
            // Original drift continues through offset 18, then stops.
            Maneuver::Idle
                if matches!(self.action_recovery, Recovery::Ground(2..)) =>
            {
                (self.attack_step(dx), 0)
            },
            Maneuver::Idle if self.action_recovery != Recovery::Ready => (0, 0),
            Maneuver::Idle | Maneuver::Jump(_) => {
                self.attack_quarters = 0;
                (dx, dy)
            },
        }
    }

    fn move_tick(&mut self, layout: StreetLayout) {
        if !self.station.can_fight() {
            return;
        }
        let dx = axis(self.held(Control::Left), self.held(Control::Right));
        let dy = axis(self.held(Control::Up), self.held(Control::Down));
        if !matches!(
            self.station.maneuver,
            Maneuver::Attack { tick: 1.. }
                | Maneuver::AirAttack(_)
                | Maneuver::Special { .. }
        ) && (self.station.maneuver != Maneuver::Idle
            || self.action_recovery.allows_turn())
            && dx != 0
        {
            self.station.facing = if dx < 0 {
                Facing::Left
            } else {
                Facing::Right
            };
        }
        let (step_x, step_y) = self.maneuver_step(dx, dy);
        let previous = self.station.position;
        let position = &mut self.station.position;
        position.x = position
            .x
            .saturating_add(step_x)
            .clamp(layout.min.x, layout.max.x);
        position.y = position
            .y
            .saturating_add(step_y)
            .clamp(layout.min.y, layout.max.y);
        self.station.walking_ticks = if *position == previous
            || self.station.maneuver != Maneuver::Idle
            || self.action_recovery != Recovery::Ready
        {
            None
        } else {
            Some(
                self.station
                    .walking_ticks
                    .map_or(0, |elapsed| elapsed.saturating_add(1)),
            )
        };
    }

    const fn new(position: Point) -> Self {
        Self {
            action_recovery: Recovery::Ready,
            attack_quarters: 0,
            held: 0,
            spawn: position,
            station: Station {
                attack_cues: 0,
                facing: Facing::Right,
                health: PLAYER_HEALTH,
                maneuver: Maneuver::Idle,
                participation: Participation::Absent,
                position,
                walking_ticks: None,
            },
        }
    }

    const fn start(&mut self, camera: i16) {
        if !matches!(
            self.station.participation,
            Participation::Absent | Participation::Continuing { .. }
        ) {
            return;
        }
        self.attack_quarters = 0;
        self.action_recovery = Recovery::Ready;
        self.station.health = PLAYER_HEALTH;
        self.station.position = Point {
            x: self.spawn.x.saturating_add(camera),
            ..self.spawn
        };
        self.station.facing = Facing::Right;
        self.station.maneuver = Maneuver::Idle;
        self.station.walking_ticks = None;
        self.station.participation = Participation::Playing;
    }
}

/// Deterministic first-street title state, independent of the host platform.
///
/// The prototype omits the original intro, full stage progression, collision
/// and scoring. Optional combat, continuing and maneuver timings are
/// provisional. It is not a gameplay parity claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cabinet {
    camera: i16,
    choices: [Player; 4],
    enemy: Option<Enemy>,
    layout: StreetLayout,
    scroll_limit: i16,
    session: LocalSession,
    slots: [Slot; 4],
}

impl Cabinet {
    fn advance_camera(&mut self) {
        if self.camera == self.scroll_limit {
            return;
        }
        let mut active =
            self.slots.iter().filter(|slot| slot.station.can_fight());
        let Some(first) = active.next() else {
            return;
        };
        let first_x = first.station.position.x;
        let (mut left, right) =
            active.fold((first_x, first_x), |(left, right), slot| {
                let x = slot.station.position.x;
                (left.min(x), right.max(x))
            });
        if let Some(enemy) = self.enemy.filter(|enemy| enemy.health > 0) {
            left = left.min(enemy.position.x);
        }
        // Provisional rightward anchor, not an original collision rule.
        let anchor =
            self.layout.max.x.saturating_sub(80).max(self.layout.min.x);
        let target = right
            .saturating_sub(anchor)
            .min(left.saturating_sub(self.layout.min.x))
            .min(self.scroll_limit);
        self.camera =
            self.camera.max(target.min(self.camera.saturating_add(2)));
    }

    fn advance_continue(&mut self) {
        if self.phase() != Phase::Playing || self.enemy.is_none() {
            return;
        }
        for slot in &mut self.slots {
            slot.expire_continue();
        }
        if self
            .slots
            .iter()
            .any(|slot| slot.station.participation != Participation::Absent)
        {
            return;
        }
        self.session.return_to_menu();
        self.reset_world();
    }

    fn apply(
        &mut self,
        change: &LogicalAction<Action>,
        pending: &mut [Option<Maneuver>; 4],
    ) {
        let seat = change.action.player;
        let Some(&character) = self.choices.get(seat.index()) else {
            return;
        };
        let index = character.index();
        let (Some(slot), Some(queued)) =
            (self.slots.get_mut(index), pending.get_mut(index))
        else {
            return;
        };
        let control = change.action.control;
        if !slot.edge(control, change.down, queued) {
            return;
        }
        let playing = self.session.phase() == Phase::Playing;
        if matches!(control, Control::Left | Control::Right)
            && slot.station.participation == Participation::Absent
            && matches!(self.session.phase(), Phase::MainMenu | Phase::Playing)
        {
            self.cycle_character(seat, character, control);
            return;
        }
        let command = match control {
            Control::Start => MenuInput::Start,
            Control::Pause => MenuInput::Pause,
            Control::Primary | Control::Secondary if playing => {
                if slot.station.can_fight() {
                    queue_maneuver(queued, slot, control);
                }
                return;
            },
            Control::Primary => MenuInput::Confirm,
            Control::Up if !playing => MenuInput::Previous,
            Control::Down if !playing => MenuInput::Next,
            Control::Down
            | Control::Left
            | Control::Right
            | Control::Secondary
            | Control::Up => return,
        };
        match self.session.input(command) {
            SessionEffect::Join => slot.start(self.camera),
            SessionEffect::Reset => self.reset_world(),
            SessionEffect::None | SessionEffect::Exit => {},
        }
        if self.phase() != Phase::Playing {
            *pending = [None; 4];
        }
    }

    /// Rightward travel of the original-width gameplay camera.
    #[must_use]
    pub const fn camera_origin(&self) -> i16 {
        self.camera
    }

    /// Selected character for each input seat, in physical seat order.
    #[must_use]
    pub const fn choices(&self) -> [Player; 4] {
        self.choices
    }

    fn cycle_character(
        &mut self,
        seat: Player,
        current: Player,
        direction: Control,
    ) {
        let count = Player::ALL.len();
        let offset = if direction == Control::Right {
            1
        } else {
            count.saturating_sub(1)
        };
        let mut index = current.index();
        for _ in 1..count {
            index = index
                .saturating_add(offset)
                .checked_rem(count)
                .unwrap_or_default();
            let Some(&candidate) = Player::ALL.get(index) else {
                return;
            };
            if self.swap_vacant_character(seat, current, candidate) {
                return;
            }
        }
    }

    /// Inspect the optional experimental enemy, including after defeat.
    #[must_use]
    pub const fn enemy(&self) -> Option<Enemy> {
        self.enemy
    }

    fn finish_tick(&mut self, pending: [Option<Maneuver>; 4]) {
        if self.phase() != Phase::Playing {
            return;
        }
        for ((player, slot), queued) in
            Player::ALL.into_iter().zip(&mut self.slots).zip(pending)
        {
            slot.advance_maneuver(player, queued);
        }
        self.advance_continue();
        if self.phase() == Phase::Playing {
            let layout = self.movement_layout();
            for slot in &mut self.slots {
                slot.move_tick(layout);
            }
            if let Some(enemy) = &mut self.enemy
                && self.slots.iter().any(|slot| slot.station.can_fight())
            {
                enemy.advance(layout, &mut self.slots);
            }
            self.advance_camera();
            self.record_attack_cues();
        }
    }

    const fn movement_layout(&self) -> StreetLayout {
        let mut layout = self.layout;
        layout.min.x = layout.min.x.saturating_add(self.camera);
        layout.max.x = layout.max.x.saturating_add(self.camera);
        layout
    }

    /// Construct a cabinet with explicit spawn and movement boundaries.
    ///
    /// # Errors
    /// Rejects invalid bounds or a spawn outside them.
    pub fn new(layout: StreetLayout) -> Result<Self, LayoutError> {
        if !layout.spawn.iter().all(|point| layout.contains(*point)) {
            return Err(LayoutError::SpawnOutside);
        }
        Ok(Self {
            camera: 0,
            choices: Player::ALL,
            enemy: None,
            layout,
            scroll_limit: 0,
            session: LocalSession::default(),
            slots: layout.spawn.map(Slot::new),
        })
    }

    /// Current cabinet phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.session.phase()
    }

    fn record_attack_cues(&mut self) {
        for (player, slot) in Player::ALL.into_iter().zip(&mut self.slots) {
            if slot.station.can_fight()
                && slot.station.maneuver.attack_cue(player)
            {
                slot.station.attack_cues =
                    slot.station.attack_cues.saturating_add(1);
            }
        }
    }

    fn reset_world(&mut self) {
        self.camera = 0;
        self.enemy = self.enemy.map(Enemy::restart);
        for slot in &mut self.slots {
            let held = slot.held;
            let cues = slot.station.attack_cues;
            *slot = Slot::new(slot.spawn);
            slot.held = held;
            slot.station.attack_cues = cues;
        }
    }

    /// Shared menu state used by native and future host presentations.
    #[must_use]
    pub const fn session(&self) -> LocalSession {
        self.session
    }

    /// Inspect all four stations in character order.
    #[must_use]
    pub fn stations(&self) -> [Station; 4] {
        self.slots.map(|slot| slot.station)
    }

    fn swap_vacant_character(
        &mut self,
        seat: Player,
        current: Player,
        candidate: Player,
    ) -> bool {
        let Some(other) =
            self.choices.iter().position(|&choice| choice == candidate)
        else {
            return false;
        };
        let Ok([first, second]) = self
            .slots
            .get_disjoint_mut([current.index(), candidate.index()])
        else {
            return false;
        };
        if second.station.participation != Participation::Absent {
            return false;
        }
        // Physical holds follow the input seat, not the chosen character.
        mem::swap(&mut first.held, &mut second.held);
        self.choices.swap(seat.index(), other);
        true
    }

    /// Enable one provisional enemy at a specified walkable ground position.
    ///
    /// # Errors
    /// Rejects an enemy spawn outside the configured movement bounds.
    pub fn with_encounter(
        layout: StreetLayout,
        spawn: Point,
    ) -> Result<Self, LayoutError> {
        let mut cabinet = Self::new(layout)?;
        if !layout.contains(spawn) {
            return Err(LayoutError::SpawnOutside);
        }
        cabinet.enemy = Some(Enemy::new(spawn));
        Ok(cabinet)
    }

    /// Enable bounded rightward opening-street travel.
    ///
    /// The host supplies the extent of verified additional world art.
    /// Rendering dimensions are not simulation inputs.
    /// # Errors
    /// Rejects travel outside the representable world coordinates.
    pub fn with_scroll_limit(
        mut self,
        travel: u16,
    ) -> Result<Self, LayoutError> {
        let limit = i16::try_from(travel)
            .map_err(|_error| LayoutError::ScrollOverflow)?;
        if self.layout.max.x.checked_add(limit).is_none() || limit < self.camera
        {
            return Err(LayoutError::ScrollOverflow);
        }
        self.scroll_limit = limit;
        Ok(self)
    }
}

impl InputStep for Cabinet {
    type Input = LogicalAction<Action>;

    fn input_step(&mut self, _tick: u64, actions: &[Self::Input]) {
        let mut pending = [None; 4];
        for action in actions {
            self.apply(action, &mut pending);
        }
        self.finish_tick(pending);
    }
}

const fn axis(negative: bool, positive: bool) -> i16 {
    match (negative, positive) {
        (true, false) => -2,
        (false, true) => 2,
        _ => 0,
    }
}

fn queue_maneuver(
    queued: &mut Option<Maneuver>,
    slot: &Slot,
    control: Control,
) {
    let (maneuver, other, combines) = if control == Control::Primary {
        (
            Maneuver::Attack { tick: 0 },
            Control::Secondary,
            matches!(queued, Some(Maneuver::Jump(_))),
        )
    } else {
        (
            Maneuver::jump(),
            Control::Primary,
            matches!(queued, Some(Maneuver::Attack { .. })),
        )
    };
    if slot.station.maneuver == Maneuver::Idle && combines && slot.held(other) {
        *queued = Some(Maneuver::Special { hit: false, tick: 0 });
    } else {
        let _first = queued.get_or_insert(maneuver);
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::ops::Range;

    use games::replay::{InputScript, Scripted};
    use games::timing::StepClock;

    use super::*;

    fn cabinet() -> Cabinet {
        let result = Cabinet::new(StreetLayout::DOWNTOWN);
        assert!(result.is_ok());
        // A valid constant layout cannot fail; keep fixture construction
        // independent from any mutable global or host input.
        result.unwrap_or_else(|_error| Cabinet {
            camera: 0,
            choices: Player::ALL,
            enemy: None,
            layout: StreetLayout::DOWNTOWN,
            scroll_limit: 0,
            session: LocalSession::default(),
            slots: StreetLayout::DOWNTOWN.spawn.map(Slot::new),
        })
    }

    fn change(
        player: Player,
        control: Control,
        down: bool,
    ) -> LogicalAction<Action> {
        LogicalAction {
            action: Action { control, player },
            down,
        }
    }

    fn pulse(player: Player, control: Control) -> [LogicalAction<Action>; 2] {
        [
            change(player, control, true),
            change(player, control, false),
        ]
    }

    #[test]
    fn start_enters_immediately_and_other_stations_can_join() {
        let mut state = cabinet();
        assert_eq!(state.phase(), Phase::MainMenu);
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        assert_eq!(state.phase(), Phase::Playing);
        let [marge, homer, ..] = state.stations();
        assert_eq!(marge.participation, Participation::Playing);
        assert_eq!(homer.participation, Participation::Absent);
        state.input_step(1, &pulse(Player::Homer, Control::Start));
        let [unchanged, joined, ..] = state.stations();
        assert_eq!(unchanged, marge);
        assert_eq!(joined.participation, Participation::Playing);
    }

    #[test]
    fn vacant_seat_selects_and_joins_its_character() {
        let mut state = cabinet();
        state.input_step(0, &pulse(Player::Marge, Control::Right));
        assert_eq!(state.choices(), [
            Player::Homer,
            Player::Marge,
            Player::Bart,
            Player::Lisa
        ]);
        state.input_step(1, &pulse(Player::Marge, Control::Start));
        let [marge, homer, ..] = state.stations();
        assert_eq!(marge.participation, Participation::Absent);
        assert_eq!(homer.participation, Participation::Playing);
        state.input_step(2, &[change(Player::Marge, Control::Right, true)]);
        let [_, moved, ..] = state.stations();
        assert_eq!(moved.position.x, homer.position.x + 2);
        state.input_step(3, &[change(Player::Marge, Control::Right, false)]);
        assert_eq!(
            state.stations().get(1).map(|station| station.position),
            Some(moved.position)
        );
    }

    #[test]
    fn selection_hold_follows_seat_until_release() {
        let mut state = cabinet();
        for tick in 0..2 {
            state.input_step(tick, &[change(
                Player::Marge,
                Control::Right,
                true,
            )]);
        }
        assert_eq!(state.choices().first(), Some(&Player::Homer));
        state.input_step(2, &[change(Player::Marge, Control::Right, false)]);
        state.input_step(3, &pulse(Player::Marge, Control::Start));
        let [_, homer, ..] = state.stations();
        assert_eq!(Some(&homer.position), StreetLayout::DOWNTOWN.spawn.get(1));
        state.input_step(4, &[]);
        assert_eq!(
            state.stations().get(1).map(|station| station.position),
            Some(homer.position)
        );
    }

    #[test]
    fn late_join_selection_skips_active_characters() {
        let mut state = cabinet();
        state.input_step(0, &pulse(Player::Homer, Control::Start));
        let [_, homer, ..] = state.stations();
        state.input_step(1, &pulse(Player::Marge, Control::Right));
        assert_eq!(state.choices(), [
            Player::Bart,
            Player::Homer,
            Player::Marge,
            Player::Lisa
        ]);
        state.input_step(2, &pulse(Player::Marge, Control::Start));
        let [_, unchanged, bart, ..] = state.stations();
        assert_eq!(unchanged, homer);
        assert_eq!(bart.participation, Participation::Playing);
    }

    fn four_playing() -> Cabinet {
        let mut state = cabinet();
        for player in Player::ALL {
            state.input_step(2, &pulse(player, Control::Start));
        }
        state
    }

    #[test]
    fn overlapping_buttons_start_one_stationary_special() {
        let mut state = four_playing();
        let original = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Marge, Control::Secondary, true),
            change(Player::Homer, Control::Secondary, true),
            change(Player::Homer, Control::Primary, true),
        ]);
        let [marge, homer, bart, lisa] = state.stations();
        assert_eq!(marge.maneuver, Maneuver::Special { hit: false, tick: 0 });
        assert_eq!(homer.maneuver, marge.maneuver);
        assert_eq!(bart.maneuver, Maneuver::Idle);
        assert_eq!(lisa.maneuver, Maneuver::Idle);
        for tick in 4..34 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations(), original);
    }

    #[test]
    fn sequential_pulses_do_not_start_special() {
        let mut state = four_playing();
        state.input_step(3, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Marge, Control::Primary, false),
            change(Player::Marge, Control::Secondary, true),
            change(Player::Marge, Control::Secondary, false),
        ]);
        let [marge, ..] = state.stations();
        assert_eq!(marge.maneuver, Maneuver::Attack { tick: 0 });
    }

    #[test]
    fn four_stations_move_independently_and_clamp_to_the_layout() {
        let mut state = four_playing();
        let original = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Left, true),
            change(Player::Homer, Control::Right, true),
            change(Player::Bart, Control::Up, true),
            change(Player::Lisa, Control::Down, true),
        ]);
        for tick in 4..8 {
            state.input_step(tick, &[]);
        }
        let [marge, homer, bart, lisa] = state.stations();
        let [old_marge, old_homer, old_bart, old_lisa] = original;
        assert_eq!(marge.position.x, old_marge.position.x - 10);
        assert_eq!(homer.position.x, old_homer.position.x + 10);
        assert_eq!(bart.position.y, old_bart.position.y - 10);
        assert_eq!(lisa.position.y, old_lisa.position.y + 10);
        assert_eq!(marge.facing, Facing::Left);
        state.input_step(8, &[change(Player::Marge, Control::Right, true)]);
        assert_eq!(
            state.stations().first().map(|slot| slot.position),
            Some(marge.position)
        );
        assert_clamped(state);
    }

    fn assert_clamped(mut state: Cabinet) {
        for tick in 9..300 {
            state.input_step(tick, &[]);
        }
        let [_, right, upper, lower] = state.stations();
        assert_eq!(right.position.x, StreetLayout::DOWNTOWN.max.x);
        assert_eq!(upper.position.y, StreetLayout::DOWNTOWN.min.y);
        assert_eq!(lower.position.y, StreetLayout::DOWNTOWN.max.y);
    }

    #[test]
    fn start_does_not_reset_an_already_active_station() {
        let mut state = four_playing();
        state.input_step(3, &[change(Player::Marge, Control::Right, true)]);
        state.input_step(4, &pulse(Player::Marge, Control::Start));
        let [marge, ..] = state.stations();
        assert_eq!(marge.position.x, 114);
        assert_eq!(marge.health, 3);
    }

    #[test]
    fn pause_suspends_motion_and_attack_until_resume() {
        let mut state = four_playing();
        state.input_step(3, &pulse(Player::Marge, Control::Primary));
        let frozen = state.stations();
        state.input_step(4, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.phase(), Phase::Paused);
        assert_eq!(state.stations(), frozen);
        for tick in 5..60 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations(), frozen);
        state.input_step(60, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.phase(), Phase::Playing);
        state.input_step(61, &[]);
        assert_ne!(state.stations(), frozen);
    }

    #[test]
    fn pause_at_the_attack_cue_does_not_skip_or_repeat_it() {
        let mut state = four_playing();
        state.input_step(3, &pulse(Player::Marge, Control::Primary));
        for tick in 4..7 {
            state.input_step(tick, &[]);
        }
        let mut uninterrupted = state;
        let before = state.stations();
        state.input_step(7, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.stations(), before);
        for tick in 8..20 {
            state.input_step(tick, &[]);
        }
        state.input_step(20, &pulse(Player::Marge, Control::Pause));
        uninterrupted.input_step(7, &[]);
        assert_eq!(state.stations(), uninterrupted.stations());
        state.input_step(21, &[]);
        uninterrupted.input_step(8, &[]);
        assert_eq!(state.stations(), uninterrupted.stations());
    }

    #[test]
    // Original reference: nine moving ticks on each component.
    fn diagonal_walk_preserves_both_observed_cardinal_components() {
        let mut state = four_playing();
        let [before, ..] = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Left, true),
            change(Player::Marge, Control::Up, true),
        ]);
        for tick in 4..12 {
            state.input_step(tick, &[]);
        }
        let [after, ..] = state.stations();
        assert_eq!(after.position.x, before.position.x - 18);
        assert_eq!(after.position.y, before.position.y - 18);
        assert_eq!(after.facing, Facing::Left);
    }

    #[test]
    fn attack_admits_initial_facing_but_retains_it_until_recovery() {
        for player in Player::ALL {
            verify_attack_motion(player);
        }
    }

    fn station_for(state: &Cabinet, player: Player) -> Station {
        let [marge, homer, bart, lisa] = state.stations();
        match player {
            Player::Marge => marge,
            Player::Homer => homer,
            Player::Bart => bart,
            Player::Lisa => lisa,
        }
    }

    fn verify_attack_motion(player: Player) {
        let mut state = four_playing();
        let before = station_for(&state, player);
        state.input_step(3, &[
            change(player, Control::Primary, true),
            change(player, Control::Left, true),
            change(player, Control::Up, true),
        ]);
        for tick in 4..11 {
            state.input_step(tick, &[]);
        }
        let left = station_for(&state, player);
        assert_eq!(left.facing, Facing::Left);
        assert_eq!(left.position.x, before.position.x.saturating_sub(2));
        assert_eq!(left.position.y, before.position.y);
        state.input_step(11, &[
            change(player, Control::Left, false),
            change(player, Control::Right, true),
        ]);
        for tick in 12..15 {
            state.input_step(tick, &[]);
        }
        let reversed = station_for(&state, player);
        assert_eq!(reversed.facing, Facing::Left);
        assert_eq!(reversed.position.x, before.position.x.saturating_sub(1));
        assert_eq!(reversed.position.y, before.position.y);
        assert_attack_recovery(state, player);
    }

    fn assert_attack_recovery(mut state: Cabinet, player: Player) {
        let end =
            3 + u64::from(Maneuver::Attack { tick: 0 }.pose_duration(player));
        for tick in 15..end {
            state.input_step(tick, &[]);
        }
        let last = station_for(&state, player);
        assert_eq!(last.facing, Facing::Left);
        assert_ne!(last.maneuver, Maneuver::Idle);
        for tick in end..23 {
            state.input_step(tick, &[]);
            assert_recovery_pose(station_for(&state, player), last.position.y);
        }
        let recovered = station_for(&state, player);
        state.input_step(23, &[]);
        let resumed = station_for(&state, player);
        assert_eq!(resumed.facing, Facing::Right);
        assert_eq!(resumed.maneuver, Maneuver::Idle);
        assert_eq!(resumed.position.x, recovered.position.x.saturating_add(2));
        assert_eq!(resumed.position.y, recovered.position.y.saturating_sub(2));
        assert_eq!(resumed.walking_ticks, Some(0));
    }

    fn assert_recovery_pose(recovering: Station, depth: i16) {
        assert_eq!(recovering.facing, Facing::Left);
        assert_eq!(recovering.maneuver, Maneuver::Idle);
        assert_eq!(recovering.position.y, depth);
        assert_eq!(recovering.walking_ticks, None);
    }

    #[test]
    fn ground_recovery_preserves_fractional_drift_until_offset_eighteen() {
        for player in Player::ALL {
            verify_recovery_direction(player, 21, -1, false);
            verify_recovery_direction(player, 22, 0, false);
            verify_recovery_direction(player, 23, -2, true);
        }
    }

    fn verify_recovery_direction(
        player: Player,
        target: u64,
        dx: i16,
        walking: bool,
    ) {
        let mut state = four_playing();
        let before = station_for(&state, player);
        state.input_step(3, &pulse(player, Control::Primary));
        for tick in 4..target {
            state.input_step(tick, &[]);
        }
        state.input_step(target, &[
            change(player, Control::Left, true),
            change(player, Control::Up, true),
        ]);
        let moved = station_for(&state, player);
        assert_eq!(moved.position.x, before.position.x.saturating_add(dx));
        let dy = if walking {
            -2
        } else {
            0
        };
        assert_eq!(moved.position.y, before.position.y.saturating_add(dy));
        let facing = if walking {
            Facing::Left
        } else {
            Facing::Right
        };
        assert_eq!(moved.facing, facing);
        assert_eq!(moved.walking_ticks, walking.then_some(0));
    }

    #[test]
    fn ground_attack_follow_up_requires_twenty_active_ticks() {
        for player in Player::ALL {
            for offset in 17..=20 {
                let mut state = four_playing();
                state.input_step(3, &pulse(player, Control::Primary));
                let timing = (3, 3 + offset, offset == 20);
                verify_follow_up(state, player, Control::Primary, timing);
                verify_follow_up(state, player, Control::Secondary, timing);
            }
        }
    }

    #[test]
    fn special_attack_follow_up_requires_thirty_one_active_ticks() {
        for player in Player::ALL {
            for offset in 29..=31 {
                let state = special_started(player);
                let timing = (3, 3 + offset, offset == 31);
                verify_follow_up(state, player, Control::Primary, timing);
                verify_follow_up(state, player, Control::Secondary, timing);
            }
        }
    }

    #[test]
    fn aerial_attack_follow_up_waits_until_after_landing() {
        for player in Player::ALL {
            for (takeoff, flight) in [(8u64, 33u64), (18, 21), (33, 24)] {
                verify_aerial_landing_edges(player, takeoff, flight);
            }
        }
    }

    fn verify_aerial_landing_edges(player: Player, takeoff: u64, flight: u64) {
        let mut state = four_playing();
        state.input_step(3, &[change(player, Control::Secondary, true)]);
        for tick in 4..takeoff {
            state.input_step(tick, &[]);
        }
        state.input_step(takeoff, &pulse(player, Control::Primary));
        assert!(matches!(station_for(&state, player).maneuver,
            Maneuver::AirAttack(motion) if motion.tick() == 0));
        let start = takeoff.saturating_add(1);
        state.input_step(start, &[change(player, Control::Secondary, false)]);
        let landing = takeoff.saturating_add(flight);
        for control in [Control::Primary, Control::Secondary] {
            verify_follow_up(state, player, control, (start, landing, false));
            verify_follow_up(
                state,
                player,
                control,
                (start, landing.saturating_add(1), true),
            );
        }
    }

    #[test]
    fn repeated_jump_waits_until_after_landing() {
        for player in Player::ALL {
            verify_jump_landing_edges(player, 3, 31);
            verify_jump_landing_edges(player, 30, 58);
        }
    }

    fn verify_jump_landing_edges(player: Player, hold: u64, flight: u64) {
        let mut state = four_playing();
        state.input_step(3, &[change(player, Control::Secondary, true)]);
        let release = 3u64.saturating_add(hold);
        for tick in 4..release {
            state.input_step(tick, &[]);
        }
        state.input_step(release, &[change(player, Control::Secondary, false)]);
        let landing = 3u64.saturating_add(flight);
        verify_follow_up(
            state,
            player,
            Control::Secondary,
            (release, landing, false),
        );
        verify_follow_up(
            state,
            player,
            Control::Secondary,
            (release, landing.saturating_add(1), true),
        );
    }

    #[test]
    fn landing_turn_does_not_start_walking() {
        for player in Player::ALL {
            let mut special = special_started(player);
            for tick in 4..33 {
                special.input_step(tick, &[]);
            }
            verify_landing_direction(special, player, 33, Facing::Left);
            let mut jump = four_playing();
            jump.input_step(3, &pulse(player, Control::Secondary));
            for tick in 4..34 {
                jump.input_step(tick, &[]);
            }
            verify_landing_direction(jump, player, 34, Facing::Left);
        }
    }

    fn verify_landing_direction(
        mut state: Cabinet,
        player: Player,
        target: u64,
        facing: Facing,
    ) {
        let before = station_for(&state, player);
        assert!(before.maneuver.airborne());
        state.input_step(target, &[
            change(player, Control::Left, true),
            change(player, Control::Up, true),
        ]);
        let landed = station_for(&state, player);
        assert_eq!(landed.maneuver, Maneuver::Idle);
        assert_eq!(landed.position, before.position);
        assert_eq!(landed.facing, facing);
        assert_eq!(landed.walking_ticks, None);
        state.input_step(target.saturating_add(1), &[]);
        let walking = station_for(&state, player);
        assert_eq!(walking.position.x, landed.position.x.saturating_sub(2));
        assert_eq!(walking.position.y, landed.position.y.saturating_sub(2));
        assert_eq!(walking.facing, Facing::Left);
        assert_eq!(walking.walking_ticks, Some(0));
    }

    fn special_started(player: Player) -> Cabinet {
        let mut state = four_playing();
        state.input_step(3, &[
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
            change(player, Control::Primary, false),
            change(player, Control::Secondary, false),
        ]);
        assert!(matches!(
            station_for(&state, player).maneuver,
            Maneuver::Special { tick: 0, .. }
        ));
        state
    }

    fn verify_follow_up(
        mut state: Cabinet,
        player: Player,
        control: Control,
        timing: (u64, u64, bool),
    ) {
        let (start, target, allowed) = timing;
        for tick in start.saturating_add(1)..target {
            state.input_step(tick, &[]);
        }
        let baseline = state;
        state.input_step(target, &pulse(player, control));
        if allowed {
            let maneuver = station_for(&state, player).maneuver;
            if control == Control::Primary {
                assert_eq!(maneuver, Maneuver::Attack { tick: 0 });
            } else {
                assert!(matches!(maneuver, Maneuver::Jump(j) if j.tick() == 0));
            }
        } else {
            assert_discarded_follow_up(state, baseline, target);
        }
    }

    fn assert_discarded_follow_up(
        mut state: Cabinet,
        mut baseline: Cabinet,
        target: u64,
    ) {
        baseline.input_step(target, &[]);
        assert_eq!(state.stations(), baseline.stations());
        for tick in target.saturating_add(1)..target.saturating_add(12) {
            state.input_step(tick, &[]);
            baseline.input_step(tick, &[]);
            assert_eq!(state.stations(), baseline.stations());
        }
    }

    #[test]
    fn action_recovery_does_not_buffer_held_input_or_lock_other_players() {
        let mut state = four_playing();
        state.input_step(3, &pulse(Player::Marge, Control::Primary));
        for tick in 4..22 {
            state.input_step(tick, &[]);
        }
        state.input_step(22, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Homer, Control::Primary, true),
        ]);
        let [marge, homer, ..] = state.stations();
        assert_eq!(marge.maneuver, Maneuver::Idle);
        assert_eq!(homer.maneuver, Maneuver::Attack { tick: 0 });
        for tick in 23..50 {
            state.input_step(tick, &[]);
        }
        let [recovered, other, ..] = state.stations();
        assert_eq!(recovered.attack_cues, 1);
        assert_eq!(other.attack_cues, 1);
        assert_eq!(recovered.maneuver, Maneuver::Idle);
        state.input_step(50, &[change(Player::Marge, Control::Primary, false)]);
        state.input_step(51, &pulse(Player::Marge, Control::Primary));
        assert_eq!(
            station_for(&state, Player::Marge).maneuver,
            Maneuver::Attack { tick: 0 }
        );
    }

    #[test]
    fn pause_does_not_advance_invisible_action_recovery() {
        let mut state = four_playing();
        state.input_step(3, &pulse(Player::Marge, Control::Primary));
        for tick in 4..21 {
            state.input_step(tick, &[]);
        }
        assert_eq!(station_for(&state, Player::Marge).maneuver, Maneuver::Idle);
        state.input_step(21, &pulse(Player::Marge, Control::Pause));
        for tick in 22..90 {
            state.input_step(tick, &[]);
        }
        state.input_step(90, &pulse(Player::Marge, Control::Pause));
        state.input_step(91, &pulse(Player::Marge, Control::Secondary));
        assert_eq!(station_for(&state, Player::Marge).maneuver, Maneuver::Idle);
        state.input_step(92, &pulse(Player::Marge, Control::Secondary));
        assert!(matches!(
            station_for(&state, Player::Marge).maneuver,
            Maneuver::Jump(j) if j.tick() == 0
        ));
    }

    #[test]
    fn jump_can_turn_and_move_after_takeoff() {
        let mut state = four_playing();
        state.input_step(3, &[
            change(Player::Marge, Control::Right, true),
            change(Player::Marge, Control::Secondary, true),
        ]);
        let [before, ..] = state.stations();
        assert_eq!(before.facing, Facing::Right);
        state.input_step(4, &[
            change(Player::Marge, Control::Right, false),
            change(Player::Marge, Control::Left, true),
            change(Player::Marge, Control::Up, true),
        ]);
        let [turned, ..] = state.stations();
        assert!(matches!(turned.maneuver, Maneuver::Jump(j) if j.tick() == 1));
        assert_eq!(turned.facing, Facing::Left);
        assert_eq!(turned.position.x, before.position.x.saturating_sub(2));
        assert_eq!(turned.position.y, before.position.y - 2);
    }

    #[test]
    fn early_airborne_attack_edges_are_not_buffered() {
        for player in Player::ALL {
            for offset in 1..4 {
                verify_early_airborne_attack(player, offset);
            }
        }
    }

    fn verify_early_airborne_attack(player: Player, offset: u64) {
        let mut state = four_playing();
        let before = station_for(&state, player);
        state.input_step(3, &[change(player, Control::Secondary, true)]);
        let input_tick = 3u64.saturating_add(offset);
        for tick in 4..input_tick {
            state.input_step(tick, &[]);
        }
        state.input_step(input_tick, &[change(player, Control::Primary, true)]);
        for tick in input_tick.saturating_add(1)..9 {
            state.input_step(tick, &[]);
        }
        let held = station_for(&state, player);
        assert!(matches!(held.maneuver, Maneuver::Jump(j) if j.tick() == 5));
        assert_eq!(held.position, before.position);
        state.input_step(9, &[
            change(player, Control::Primary, false),
            change(player, Control::Primary, true),
        ]);
        assert_air_attack_started(before, station_for(&state, player));
    }

    #[test]
    fn aerial_attack_is_accepted_at_four_tick_input_offset() {
        for player in Player::ALL {
            let mut state = four_playing();
            let before = station_for(&state, player);
            state.input_step(3, &[change(player, Control::Secondary, true)]);
            for tick in 4..7 {
                state.input_step(tick, &[]);
            }
            state.input_step(7, &pulse(player, Control::Primary));
            assert_air_attack_started(before, station_for(&state, player));
        }
    }

    #[test]
    fn short_jump_last_airborne_tick_accepts_an_aerial_attack() {
        let mut state = short_jump_after(31);
        let before = station_for(&state, Player::Marge);
        state.input_step(34, &pulse(Player::Marge, Control::Primary));
        assert_air_attack_started(before, station_for(&state, Player::Marge));
    }

    #[test]
    fn attack_after_short_jump_landing_is_grounded() {
        let mut state = short_jump_after(32);
        let before = station_for(&state, Player::Marge);
        assert_eq!(before.maneuver, Maneuver::Idle);
        state.input_step(35, &pulse(Player::Marge, Control::Primary));
        let attack = station_for(&state, Player::Marge);
        assert_eq!(attack.maneuver, Maneuver::Attack { tick: 0 });
        assert_eq!(attack.position, before.position);
    }

    fn short_jump_after(offset: u64) -> Cabinet {
        let mut state = four_playing();
        state.input_step(3, &[change(Player::Marge, Control::Secondary, true)]);
        for tick in 4..6 {
            state.input_step(tick, &[]);
        }
        state.input_step(6, &[change(
            Player::Marge,
            Control::Secondary,
            false,
        )]);
        for tick in 7..3u64.saturating_add(offset) {
            state.input_step(tick, &[]);
        }
        state
    }

    fn assert_air_attack_started(before: Station, attack: Station) {
        let maneuver = attack.maneuver;
        assert!(
            matches!(maneuver, Maneuver::AirAttack(air) if air.tick() == 0)
        );
        assert_eq!(attack.position.x.abs_diff(before.position.x), 2);
    }

    #[test]
    fn aerial_attack_locks_direction_and_lands_without_repetition() {
        let mut state = four_playing();
        let [before, other, ..] = state.stations();
        state.input_step(3, &[change(Player::Marge, Control::Secondary, true)]);
        for tick in 4..18 {
            state.input_step(tick, &[]);
        }
        state.input_step(18, &[change(Player::Marge, Control::Primary, true)]);
        let [attack, ..] = state.stations();
        assert_eq!(attack.maneuver.elevation(), 81);
        assert_eq!(attack.position.x, before.position.x + 4);
        state.input_step(19, &[change(Player::Marge, Control::Up, true)]);
        for tick in 20..39 {
            state.input_step(tick, &[]);
        }
        let [last, untouched, ..] = state.stations();
        assert!(last.maneuver.airborne());
        assert_eq!(last.position.x, before.position.x + 84);
        assert_eq!(last.position.y, before.position.y);
        assert_eq!(untouched, other);
        assert_air_attack_landing(&mut state, last);
    }

    fn assert_air_attack_landing(state: &mut Cabinet, last: Station) {
        verify_landing_direction(*state, Player::Marge, 39, Facing::Right);
        state.input_step(39, &[change(Player::Marge, Control::Up, false)]);
        let [landed, ..] = state.stations();
        assert_eq!(landed.maneuver, Maneuver::Idle);
        assert_eq!(landed.position, last.position);
        state.input_step(40, &[]);
        let [after, ..] = state.stations();
        assert_eq!(after, landed);
    }

    #[test]
    fn walking_ticks_track_actual_displacement_and_restart() {
        let mut state = four_playing();
        state.input_step(3, &[change(Player::Marge, Control::Right, true)]);
        let [started, _, _, _] = state.stations();
        assert_eq!(started.walking_ticks, Some(0));
        state.input_step(4, &[]);
        let [moving, _, _, _] = state.stations();
        assert_eq!(moving.walking_ticks, Some(1));
        state.input_step(5, &[change(Player::Marge, Control::Left, true)]);
        let [opposed, _, _, _] = state.stations();
        assert_eq!(opposed.walking_ticks, None);
        state.input_step(6, &[change(Player::Marge, Control::Right, false)]);
        let [restarted, _, _, _] = state.stations();
        assert_eq!(restarted.walking_ticks, Some(0));
        for tick in 7..300 {
            state.input_step(tick, &[]);
        }
        let [blocked, _, _, _] = state.stations();
        assert_eq!(blocked.position.x, StreetLayout::DOWNTOWN.min.x);
        assert_eq!(blocked.walking_ticks, None);
    }

    #[test]
    fn isolated_actions_respect_station_and_ground_state() {
        let mut state = four_playing();
        let [before, homer, ..] = state.stations();
        state.input_step(3, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Homer, Control::Secondary, true),
        ]);
        let [attacking, jumping, ..] = state.stations();
        assert_eq!(attacking.maneuver, Maneuver::Attack { tick: 0 });
        assert_eq!(attacking.position, before.position);
        assert_eq!(jumping.maneuver, Maneuver::jump());
        assert_eq!(jumping.position, homer.position);
        state.input_step(4, &pulse(Player::Marge, Control::Secondary));
        for tick in 5..20 {
            state.input_step(tick, &[]);
        }
        assert_maneuvers_at_end(&state, before.position);
        assert_walk_after_ground_recovery(state, before.position);
    }

    fn assert_walk_after_ground_recovery(mut state: Cabinet, position: Point) {
        state.input_step(20, &[change(Player::Marge, Control::Right, true)]);
        let [recovering, ..] = state.stations();
        assert_eq!(recovering.maneuver, Maneuver::Idle);
        assert_eq!(recovering.position, position);
        assert_eq!(recovering.walking_ticks, None);
        for tick in 21..=23 {
            state.input_step(tick, &[]);
        }
        let [walking, ..] = state.stations();
        assert_eq!(walking.maneuver, Maneuver::Idle);
        assert_eq!(walking.position.x, position.x.saturating_add(2));
        assert_eq!(walking.walking_ticks, Some(0));
    }

    fn assert_maneuvers_at_end(state: &Cabinet, position: Point) {
        let [attack_end, airborne, ..] = state.stations();
        assert_eq!(attack_end.position, position);
        assert_eq!(attack_end.maneuver, Maneuver::Attack { tick: 16 });
        assert!(
            matches!(airborne.maneuver, Maneuver::Jump(j) if j.tick() == 16)
        );
    }

    #[test]
    fn attack_cues_are_station_local_and_do_not_repeat_on_hold() {
        let mut state = four_playing();
        assert_eq!(state.stations().map(|station| station.attack_cues), [0; 4]);
        let attacks =
            Player::ALL.map(|player| change(player, Control::Primary, true));
        state.input_step(3, &attacks);
        for tick in 4..7 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [0; 4]);
        state.input_step(7, &[]);
        assert_eq!(state.stations().map(|station| station.attack_cues), [
            1, 0, 1, 0
        ]);
        state.input_step(8, &[]);
        assert_eq!(state.stations().map(|station| station.attack_cues), [1; 4]);
        for tick in 9..40 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [1; 4]);
        state.input_step(40, &[change(Player::Marge, Control::Primary, false)]);
        state.input_step(41, &pulse(Player::Marge, Control::Primary));
        for tick in 42..46 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [
            2, 1, 1, 1
        ]);
    }

    #[test]
    fn held_jump_lift_is_station_local_and_requires_unbroken_hold() {
        let mut state = four_playing();
        let ground = state.stations().map(|station| station.position);
        state.input_step(3, &[
            change(Player::Marge, Control::Secondary, true),
            change(Player::Homer, Control::Secondary, true),
            change(Player::Homer, Control::Secondary, false),
            change(Player::Homer, Control::Secondary, true),
        ]);
        for tick in 4..34 {
            state.input_step(tick, &[]);
        }
        assert_independent_lift(&mut state);
        for tick in 35..61 {
            state.input_step(tick, &[]);
        }
        assert_held_jump_lands(&mut state);
        assert_eq!(state.stations().map(|station| station.position), ground);
    }

    fn assert_independent_lift(state: &mut Cabinet) {
        let [marge, homer, bart, lisa] = state.stations();
        assert!(marge.maneuver.elevation() > homer.maneuver.elevation());
        assert_eq!(bart.maneuver, Maneuver::Idle);
        assert_eq!(lisa.maneuver, Maneuver::Idle);
        state.input_step(34, &[]);
        let [long, short, ..] = state.stations();
        assert!(matches!(long.maneuver, Maneuver::Jump(_)));
        assert_eq!(short.maneuver, Maneuver::Idle);
    }

    fn assert_held_jump_lands(state: &mut Cabinet) {
        assert!(matches!(
            state.stations().first().map(|station| station.maneuver),
            Some(Maneuver::Jump(_))
        ));
        state.input_step(61, &[]);
        assert!(
            state
                .stations()
                .iter()
                .all(|station| { station.maneuver == Maneuver::Idle })
        );
    }

    #[test]
    fn jump_release_edges_match_measured_hold_timing() {
        for (hold, duration) in [(5, 35), (10, 40), (15, 46), (20, 51)] {
            let mut state = four_playing();
            let [before, ..] = state.stations();
            state.input_step(3, &[change(
                Player::Marge,
                Control::Secondary,
                true,
            )]);
            advance_empty(&mut state, 4..3 + hold);
            state.input_step(3 + hold, &[
                change(Player::Marge, Control::Secondary, false),
                change(Player::Marge, Control::Secondary, true),
            ]);
            for elapsed in hold + 1..=duration {
                state.input_step(3 + elapsed, &[]);
                let [station, ..] = state.stations();
                assert_eq!(station.position, before.position);
                assert_eq!(
                    station.maneuver == Maneuver::Idle,
                    elapsed == duration
                );
            }
        }
    }

    #[test]
    fn jump_requires_new_press_after_landing() {
        let mut state = four_playing();
        let [_, homer, ..] = state.stations();
        state.input_step(3, &[change(Player::Homer, Control::Secondary, true)]);
        for tick in 4..62 {
            state.input_step(tick, &[]);
        }
        let [_, landed, ..] = state.stations();
        assert_eq!(landed.maneuver, Maneuver::Idle);
        assert_eq!(landed.position, homer.position);
        state.input_step(62, &[change(
            Player::Homer,
            Control::Secondary,
            true,
        )]);
        let [_, held, ..] = state.stations();
        assert_eq!(held.maneuver, Maneuver::Idle);
        state.input_step(63, &[change(
            Player::Homer,
            Control::Secondary,
            false,
        )]);
        state.input_step(64, &pulse(Player::Homer, Control::Secondary));
        let [_, restarted, ..] = state.stations();
        assert!(
            matches!(restarted.maneuver, Maneuver::Jump(j) if j.tick() == 0)
        );
    }

    #[test]
    fn replay_is_independent_of_render_frame_partition() {
        let mut script = InputScript::new();
        for action in pulse(Player::Marge, Control::Start) {
            script.record(2, action);
        }
        script.record(3, change(Player::Marge, Control::Right, true));
        script.record(8, change(Player::Marge, Control::Right, false));
        for action in pulse(Player::Marge, Control::Secondary) {
            script.record(9, action);
        }
        let clocks = StepClock::new(TICK_INTERVAL, NonZeroU32::MAX);
        assert!(clocks.is_ok());
        let Ok(mut slow_clock) = clocks else {
            return;
        };
        let mut fast_clock = slow_clock;
        let mut slow = Scripted::new(cabinet(), script.clone());
        let mut fast = Scripted::new(cabinet(), script);
        assert!(
            slow_clock
                .advance(TICK_INTERVAL.saturating_mul(12), &mut slow)
                .is_ok()
        );
        for _ in 0u8..12 {
            assert!(fast_clock.advance(TICK_INTERVAL, &mut fast).is_ok());
        }
        assert_eq!(slow.state(), fast.state());
    }

    fn advance_empty(state: &mut Cabinet, ticks: Range<u64>) {
        for tick in ticks {
            state.input_step(tick, &[]);
        }
    }

    #[test]
    fn scroll_preserves_cooperation_and_join() {
        let result = cabinet().with_scroll_limit(80);
        assert!(result.is_ok());
        let Ok(mut state) = result else {
            return;
        };
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.input_step(1, &pulse(Player::Homer, Control::Start));
        state.input_step(2, &[change(Player::Homer, Control::Left, true)]);
        advance_empty(&mut state, 3..50);
        state.input_step(50, &[
            change(Player::Homer, Control::Left, false),
            change(Player::Marge, Control::Right, true),
        ]);
        advance_empty(&mut state, 51..180);
        assert_eq!(state.camera_origin(), 0);
        let [leader, laggard, ..] = state.stations();
        assert_eq!(leader.position.x, StreetLayout::DOWNTOWN.max.x);
        assert_eq!(laggard.position.x, StreetLayout::DOWNTOWN.min.x);
        state.input_step(180, &[change(Player::Homer, Control::Right, true)]);
        advance_empty(&mut state, 181..250);
        assert_eq!(state.camera_origin(), 80);
        let [advanced, following, ..] = state.stations();
        assert_eq!(advanced.position.x, 368);
        assert!(following.position.x >= 112);
        state.input_step(250, &pulse(Player::Bart, Control::Start));
        let [_, _, joined, _] = state.stations();
        assert_eq!(joined.position.x, 212);
        assert_scroll_pause_reset(state);
    }

    fn assert_scroll_pause_reset(mut state: Cabinet) {
        state.input_step(251, &pulse(Player::Marge, Control::Pause));
        let paused = state;
        state.input_step(252, &[]);
        assert_eq!(state, paused);
        state.reset_world();
        assert_eq!(state.camera_origin(), 0);
    }

    #[test]
    fn excessive_scroll_is_rejected() {
        let state = cabinet();
        assert_eq!(
            state.with_scroll_limit(u16::MAX),
            Err(LayoutError::ScrollOverflow)
        );
    }

    #[test]
    fn invalid_spawn_is_rejected() {
        let layout = StreetLayout {
            max: Point { x: 1, y: 1 },
            ..StreetLayout::DOWNTOWN
        };
        assert_eq!(Cabinet::new(layout), Err(LayoutError::SpawnOutside));
    }
}
