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

mod bonus;
mod cooperative;
mod encounter;
mod entrance;
#[cfg(feature = "native-preview")]
/// Native authenticated host input for the local arcade screen.
pub mod lan;
mod maneuver;
/// Host-authoritative remote input for the free-play cabinet.
pub mod remote;

#[cfg(feature = "native-preview")]
/// Title runtime composition using shared platform capabilities.
pub mod runtime;
mod wrestler;
use std::mem;
use std::time::Duration;

pub use bonus::{BONUS_MAX_INFLATION, Bonus, BonusPhase};
pub use cooperative::{BART_LISA_JUMP_TICKS, Pair, PairMotion, PairPose};
pub use encounter::{
    ENEMY_ATTACK_TICKS, ENEMY_DEFEAT_TICKS, ENEMY_WALK_TICKS, Enemy,
    EnemyAction, RestraintMotion,
};
pub use entrance::StageEntrance;
use games::animation::cycle_tick;
use games::input::LogicalAction;
use games::presentation::StripViewport;
use games::replay::InputStep;
pub use games::session::SessionPhase as Phase;
use games::session::{LocalSession, MenuInput, SessionEffect};
pub use maneuver::{AirAttackMotion, JumpMotion, Maneuver};
pub use wrestler::{Wrestler, WrestlerAction};

/// Fixed reference screen period: 384 by 264 clocks at 6 MHz.
pub const TICK_INTERVAL: Duration = Duration::from_micros(16_896);

/// Approximate twenty-second original continue opportunity in ticks.
pub const CONTINUE_TICKS: u16 = 1200;
const BASE_WIDTH: u32 = 320;
const MAX_VIEW_WIDTH: u32 = 512;
const SIDE: u32 = 16;
const STRIP_VIEWPORT: StripViewport = StripViewport {
    design_width: BASE_WIDTH,
    maximum_width: MAX_VIEW_WIDTH,
    side_inset: SIDE,
};

const ENEMY_CAPACITY: usize = 12;
const PLAYER_STRIKE: u8 = 10;
const WRESTLER_ATTACK_STRIKE: u8 = 24;
const OPENING_ENEMY_COUNT: usize = 7;
const LATER_COMBAT_GATE: i16 = 458;
const OPPOSING_ENEMY_COUNT: usize = 9;
const STATIONARY_SCENE_ORIGIN: i16 = 482;
const STATIONARY_SCENE_CYCLE: u16 = 100;
const STATIONARY_SCENE_END: u16 = 65;
const MID_STREET_ADMISSION: i16 = 562;
const LATE_BACKDROP_ORIGIN: i16 = 1360;
const LATE_BACKDROP_SETTLED_ORIGIN: i16 = 1359;
const LATE_BACKDROP_HOLD_TICK: u16 = 320;
const MID_STREET_SCRIPT_ORIGIN: i16 = 674;
const MID_STREET_SCRIPT_TICKS: u16 = 336;
const MID_STREET_COMBAT_GATE: i16 = 770;
const MID_STREET_ENEMY_COUNT: usize = 11;
const FAR_STREET_ADMISSION: i16 = 834;
const PLAYER_HEALTH: u8 = 32;
const PLAYER_STOCKS: u8 = 2;
const STOCK_REENTRY_TICKS: u16 = 150;
const FINAL_KNOCKOUT_TICKS: u16 = 168;
// Control returns 59 frames after the first displayed refill column.
const REFILL_TICKS: u16 = 60;
// The first visible column follows an in-game Start edge by four ticks.
const JOIN_METER_DELAY: u8 = 4;
// Original ground-attack follow-up edges first take effect at offset 20.
const GROUND_ATTACK_RECOVERY_TICKS: u8 = 20;

#[cfg(any(test, feature = "native-preview"))]
type DiagnosticIntent = (Option<Control>, Option<Control>);

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
    /// Waiting for the next stock or the final continue prompt.
    Recovering {
        /// Remaining delay after the meter reaches its target.
        ticks_left: u16,
    },
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

/// Consecutive admitted walking input, including blocked movement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalkMotion {
    /// Whether the station is walking away from the viewer.
    pub away: bool,
    /// Zero-based animation phase, independent of displacement.
    pub tick: u64,
}

/// A read-only view of an individual station.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Station {
    /// Cumulative attack cues at the observed provisional onset tick.
    pub attack_cues: u64,
    /// Visible health columns, which animate toward logical health.
    pub display_health: u8,
    /// Current sprite facing.
    pub facing: Facing,
    /// Experimental encounter health; zero means the station is knocked out.
    pub health: u8,
    /// Cumulative regular ground-hit reaction audio cues.
    pub hurt_cues: u64,
    /// Current action or hit reaction, independent of rendering.
    pub maneuver: Maneuver,
    /// Whether the station has joined.
    pub participation: Participation,
    /// Current foot position.
    pub position: Point,
    /// Score from completed enemy and wrestler defeats in this campaign.
    pub score: u16,
    /// Remaining health stocks, including the active stock.
    pub stocks: u8,
    /// Walking phase and direction, absent outside admitted walking.
    pub walking: Option<WalkMotion>,
}

impl Station {
    const fn award_defeat_point(&mut self) {
        // The original station counter uses four packed decimal digits.
        self.score = if self.score >= 9_999 {
            0
        } else {
            self.score.saturating_add(1)
        };
    }

    const fn can_fight(self) -> bool {
        matches!(self.participation, Participation::Playing) && self.health > 0
    }

    /// Zero-based walking phase, absent outside admitted walking.
    #[must_use]
    pub fn walking_ticks(self) -> Option<u64> {
        self.walking.map(|motion| motion.tick)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingManeuver {
    button_edges: u8,
    candidate: Option<Maneuver>,
    chord: bool,
    direction_edges: u8,
}

impl PendingManeuver {
    fn capture_interlude(&mut self, active: bool, control: Control) -> bool {
        if !active || control == Control::Pause {
            return false;
        }
        if matches!(control, Control::Primary | Control::Secondary) {
            self.button_edges |= control.mask();
        }
        true
    }

    // Resolve after all input edges for this tick have arrived.
    // A fresh directional edge selects the ordinary jump, even if opposed.
    const fn resolve(self, held: u8) -> Option<Maneuver> {
        if self.chord && self.direction_edges & held == 0 {
            let horizontal = if held & Control::Left.mask() != 0 {
                -1
            } else if held & Control::Right.mask() != 0 {
                1
            } else {
                0
            };
            Some(Maneuver::special(horizontal))
        } else {
            self.candidate
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Slot {
    action_recovery: Recovery,
    held: u8,
    horizontal_quarters: i16,
    meter_wait: u8,
    spawn: Point,
    station: Station,
}

impl Slot {
    fn advance_maneuver(&mut self, player: Player, pending: PendingManeuver) {
        let queued = pending.resolve(self.held);

        self.action_recovery = self.action_recovery.advance();
        if self.station.can_fight()
            && matches!(queued, Some(Maneuver::Attack { .. }))
            && let Maneuver::Jump(jump) = self.station.maneuver
            && jump.accepts_attack()
        {
            self.station.maneuver = Maneuver::air_attack(jump);
            return;
        }
        let mut previous = self.station.maneuver;
        if let Maneuver::Attack { moving, tick: 0 } = &mut previous {
            *moving = self.held(Control::Left) || self.held(Control::Right);
        }
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

    const fn advance_meter(&mut self) {
        if self.station.display_health == self.station.health {
            self.meter_wait = 0;
        } else if self.meter_wait > 0 {
            self.meter_wait = self.meter_wait.saturating_sub(1);
        } else {
            self.meter_wait = 1;
            self.station.display_health =
                if self.station.display_health < self.station.health {
                    self.station.display_health.saturating_add(1)
                } else {
                    self.station.display_health.saturating_sub(1)
                };
        }
    }

    const fn advance_participation(&mut self, camera: i16) {
        self.advance_meter();
        if let Participation::Recovering { ticks_left } =
            self.station.participation
        {
            if self.station.health == 0 && self.station.display_health > 0 {
                return;
            }
            self.station.participation = if ticks_left > 1 {
                Participation::Recovering {
                    ticks_left: ticks_left.saturating_sub(1),
                }
            } else if self.station.health > 0 {
                Participation::Playing
            } else if self.station.stocks > 0 {
                self.restore_position(camera);
                self.station.health = PLAYER_HEALTH;
                Participation::Recovering { ticks_left: REFILL_TICKS }
            } else {
                Participation::Continuing {
                    ticks_left: CONTINUE_TICKS,
                }
            };
            return;
        }
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

    fn begin_maneuver(&mut self, maneuver: Maneuver) {
        if self.station.can_fight()
            && self.station.maneuver == Maneuver::Idle
            && self.action_recovery == Recovery::Ready
        {
            if matches!(maneuver, Maneuver::Attack { .. }) {
                self.action_recovery =
                    Recovery::Ground(GROUND_ATTACK_RECOVERY_TICKS);
            }
            self.station.maneuver = maneuver;
            self.station.walking = None;
        }
    }

    const fn damage(&mut self, amount: u8) {
        self.horizontal_quarters = 0;
        self.action_recovery = Recovery::Ready;
        self.station.health = self.station.health.saturating_sub(amount);
        self.station.maneuver = if self.station.health == 0 {
            Maneuver::Idle
        } else {
            Maneuver::Hurt { tick: 0 }
        };
        self.station.walking = None;
        if self.station.health == 0 {
            self.station.stocks = self.station.stocks.saturating_sub(1);
            self.station.participation = Participation::Recovering {
                ticks_left: if self.station.stocks > 0 {
                    STOCK_REENTRY_TICKS
                } else {
                    FINAL_KNOCKOUT_TICKS
                },
            };
        }
    }

    const fn direction(self) -> (i16, i16) {
        (
            axis(self.held(Control::Left), self.held(Control::Right)),
            axis(self.held(Control::Up), self.held(Control::Down)),
        )
    }

    fn edge(
        &mut self,
        control: Control,
        down: bool,
        pending: &mut PendingManeuver,
    ) -> bool {
        if control == Control::Secondary && !down {
            self.station.maneuver.release_jump();
            if let Some(maneuver) = &mut pending.candidate {
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
        let pressed = down && !was_held;
        if pressed
            && matches!(
                control,
                Control::Left | Control::Right | Control::Up | Control::Down
            )
        {
            pending.direction_edges |= mask;
        }
        pressed
    }

    // Retain a quarter-pixel remainder and round the position downward.
    const fn fractional_step(&mut self, quarters: i16) -> i16 {
        let next = self.horizontal_quarters.saturating_add(quarters);
        if next < 0 {
            self.horizontal_quarters = next.saturating_add(4);
            -1
        } else if next >= 4 {
            self.horizontal_quarters = next.saturating_sub(4);
            1
        } else {
            self.horizontal_quarters = next;
            0
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
            Maneuver::Attack { tick: 2.., .. } => {
                (self.fractional_step(dx.signum()), 0)
            },
            Maneuver::Special { drift_quarters, .. } => {
                (self.fractional_step(drift_quarters), 0)
            },
            Maneuver::Attack { .. }
            | Maneuver::Restrained
            | Maneuver::Hurt { .. } => (0, 0),
            // Original drift continues through offset 18, then stops.
            Maneuver::Idle
                if matches!(self.action_recovery, Recovery::Ground(2..)) =>
            {
                (self.fractional_step(dx.signum()), 0)
            },
            Maneuver::Idle if self.action_recovery != Recovery::Ready => (0, 0),
            Maneuver::Jump(ref mut motion) => (motion.horizontal_step(dx), dy),
            Maneuver::Idle => (dx, dy),
        }
    }

    fn move_tick(&mut self, layout: StreetLayout) {
        if !self.station.can_fight() {
            return;
        }
        let (dx, dy) = self.direction();
        let up_held = self.held(Control::Up);
        let walking_held = up_held
            || self.held(Control::Down)
            || self.held(Control::Left)
            || self.held(Control::Right);
        let facing_locked = match self.station.maneuver {
            Maneuver::Attack { tick, .. } => tick > 0,
            Maneuver::AirAttack(air) => air.tick() > 0,
            Maneuver::Hurt { .. } | Maneuver::Restrained => true,
            Maneuver::Idle => !self.action_recovery.allows_turn(),
            Maneuver::Jump(_) | Maneuver::Special { .. } => false,
        };
        if !facing_locked && dx != 0 {
            self.station.facing = if dx < 0 {
                Facing::Left
            } else {
                Facing::Right
            };
        }
        let (step_x, step_y) = self.maneuver_step(dx, dy);
        let position = &mut self.station.position;
        let next_x = position.x.saturating_add(step_x);
        // A remainder at the right edge would extend beyond the layout.
        // At the left edge only an actual outward step is discarded.
        if position.x.checked_add(step_x).is_none()
            || next_x < layout.min.x
            || next_x >= layout.max.x
        {
            self.horizontal_quarters = 0;
        }
        position.x = next_x.clamp(layout.min.x, layout.max.x);
        position.y = position
            .y
            .saturating_add(step_y)
            .clamp(layout.min.y, layout.max.y);
        self.station.walking = if !walking_held
            || self.station.maneuver != Maneuver::Idle
            || self.action_recovery != Recovery::Ready
        {
            None
        } else {
            Some(WalkMotion {
                away: up_held,
                tick: self
                    .station
                    .walking
                    .map_or(0, |motion| motion.tick.saturating_add(1)),
            })
        };
    }

    const fn new(position: Point) -> Self {
        Self {
            action_recovery: Recovery::Ready,
            held: 0,
            horizontal_quarters: 0,
            meter_wait: 0,
            spawn: position,
            station: Station {
                attack_cues: 0,
                display_health: PLAYER_HEALTH,
                facing: Facing::Right,
                health: PLAYER_HEALTH,
                hurt_cues: 0,
                maneuver: Maneuver::Idle,
                participation: Participation::Absent,
                position,
                score: 0,
                stocks: PLAYER_STOCKS,
                walking: None,
            },
        }
    }

    const fn restore_position(&mut self, camera: i16) {
        self.horizontal_quarters = 0;
        self.action_recovery = Recovery::Ready;
        self.station.position = Point {
            x: self.spawn.x.saturating_add(camera),
            ..self.spawn
        };
        self.station.facing = Facing::Right;
        self.station.maneuver = Maneuver::Idle;
        self.station.walking = None;
    }

    const fn start(&mut self, camera: i16, fill_meter: bool) {
        if matches!(
            self.station.participation,
            Participation::Playing | Participation::Recovering { .. }
        ) {
            return;
        }
        if matches!(self.station.participation, Participation::Absent) {
            self.station.score = 0;
        }
        self.station.health = PLAYER_HEALTH;
        // In-game Start admits control immediately. Only the HUD refills.
        self.station.display_health = if fill_meter {
            0
        } else {
            PLAYER_HEALTH
        };
        self.station.stocks = PLAYER_STOCKS;
        self.meter_wait = if fill_meter {
            JOIN_METER_DELAY
        } else {
            0
        };
        self.restore_position(camera);
        self.station.participation = Participation::Playing;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PairCommand {
    Jump,
    None,
    Primary { first: bool, presses: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PairDriver {
    First,
    Second,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PairState {
    driver: Option<PairDriver>,
    idle_ticks: u8,
    motion: Option<PairMotion>,
    support: PairSupport,
    walking: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Cooperation {
    bart_lisa: PairState,
    marge_homer: PairState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PairSupport {
    #[default]
    Disabled,
    Jump,
    Locomotion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Encounter {
    art_travel: i16,
    enemies: [Option<Enemy>; ENEMY_CAPACITY],
    marge_restraint: bool,
    opening_spawn: Point,
    progressive: bool,
    stage_two: bool,
}

/// Deterministic arcade title state, independent of the host platform.
///
/// The prototype omits the original intro, full stage progression, complete
/// collision and scoring rules. Optional combat, continuing and timings are
/// provisional. It is not a gameplay parity claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cabinet {
    bonus: Option<Bonus>,
    bonus_enabled: bool,
    camera: i16,
    choices: [Player; 4],
    cooperation: Cooperation,
    encounter: Option<Encounter>,
    impact_cues: u64,
    late_backdrop_tick: Option<u16>,
    layout: StreetLayout,
    scripted_tick: Option<u16>,
    scroll_limit: i16,
    session: LocalSession,
    slots: [Slot; 4],
    stage_entrance: Option<StageEntrance>,
    stage_two_limit: Option<i16>,
    stationary_tick: Option<u16>,
    wrestler: Option<Wrestler>,
    wrestler_enabled: bool,
}

impl Cabinet {
    fn active_scroll_limit(&self) -> i16 {
        if self.stage_two() {
            self.stage_two_limit.unwrap_or_default()
        } else {
            self.scroll_limit
        }
    }

    fn admit_bonus(&mut self) {
        if self.bonus_enabled
            && self
                .wrestler
                .is_some_and(|boss| boss.action == WrestlerAction::Cleared)
        {
            self.bonus = Some(Bonus::new(
                self.slots.map(|slot| slot.station.can_fight()),
            ));
        }
    }

    fn advance_bonus(&mut self, presses: [bool; 4]) -> bool {
        let Some(bonus) = &mut self.bonus else {
            return false;
        };
        if let Some(limit) = self.stage_two_limit
            && matches!(
                bonus.phase(),
                BonusPhase::Complete | BonusPhase::StageCard
            )
        {
            if bonus.advance_stage_card(presses) {
                self.enter_stage_two(limit);
            }
        } else {
            bonus.advance(presses);
        }
        true
    }

    fn advance_camera(&mut self) {
        if let Some(tick) = self.late_backdrop_tick {
            self.late_backdrop_tick =
                Some(tick.saturating_add(1).min(LATE_BACKDROP_HOLD_TICK));
            return;
        }
        // The verified long street first reaches its full extent. On the
        // next playing tick the encounter settles one column to the left.
        if !self.stage_two()
            && self.scroll_limit == LATE_BACKDROP_ORIGIN
            && self.camera == LATE_BACKDROP_ORIGIN
        {
            self.camera = LATE_BACKDROP_SETTLED_ORIGIN;
            self.late_backdrop_tick = Some(0);
            return;
        }
        let travel = self.active_scroll_limit();
        let limit = self
            .encounter
            .as_ref()
            .map_or(travel, |encounter| encounter.limit_travel(travel));
        if self.camera >= limit {
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
        if let Some(enemy) = self.enemy().filter(|enemy| enemy.health > 0) {
            left = left.min(enemy.position.x);
        }
        // Provisional rightward anchor, not an original collision rule.
        let anchor =
            self.layout.max.x.saturating_sub(80).max(self.layout.min.x);
        let target = right
            .saturating_sub(anchor)
            .min(left.saturating_sub(self.layout.min.x))
            .min(limit);
        self.camera =
            self.camera.max(target.min(self.camera.saturating_add(2)));
    }

    fn advance_continue(&mut self) {
        if self.phase() != Phase::Playing || self.encounter.is_none() {
            return;
        }
        for slot in &mut self.slots {
            slot.advance_participation(self.camera);
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

    fn advance_scripted_scene(&mut self) {
        if self.stage_two() {
            return;
        }
        if self.camera >= MID_STREET_SCRIPT_ORIGIN {
            self.scripted_tick = Some(self.scripted_tick.map_or(0, |tick| {
                tick.saturating_add(1).min(MID_STREET_SCRIPT_TICKS)
            }));
        }
        self.stationary_tick = if self.camera < STATIONARY_SCENE_ORIGIN
            || self
                .scripted_tick
                .is_some_and(|tick| tick >= STATIONARY_SCENE_END)
        {
            None
        } else {
            cycle_tick(
                u64::from(
                    self.stationary_tick
                        .map_or(0, |tick| tick.saturating_add(1)),
                ),
                STATIONARY_SCENE_CYCLE,
            )
        };
    }

    fn advance_stage_entrance(&mut self) -> bool {
        let Some(mut entrance) = self.stage_entrance else {
            return false;
        };
        entrance.advance();
        let layout = self.movement_layout();
        for (player, slot) in Player::ALL.into_iter().zip(&mut self.slots) {
            if !slot.station.can_fight() {
                continue;
            }
            if let Some(point) = entrance.scripted_position(player) {
                slot.station.position = point;
            } else if entrance.steering_tick() {
                slot.move_tick(layout);
            }
        }
        self.stage_entrance = (!entrance.complete()).then_some(entrance);
        true
    }

    fn apply(
        &mut self,
        change: &LogicalAction<Action>,
        pending: &mut [PendingManeuver; 4],
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
        let playing = self.session.phase() == Phase::Playing;
        if !slot.edge(control, change.down, queued)
            || queued.capture_interlude(
                playing
                    && (self.bonus.is_some() || self.stage_entrance.is_some()),
                control,
            )
        {
            return;
        }
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
                if !slot.station.can_fight() {
                    return;
                }
                queued.button_edges |= control.mask();
                if slot.station.maneuver != Maneuver::Restrained {
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
            SessionEffect::Join => {
                slot.start(self.camera, playing && self.encounter.is_some());
            },
            SessionEffect::Reset => self.reset_world(),
            SessionEffect::None | SessionEffect::Exit => {},
        }
        if self.phase() != Phase::Playing {
            *pending = [PendingManeuver::default(); 4];
        }
    }

    /// Optional first-stage victory and balloon bonus.
    #[must_use]
    pub const fn bonus(&self) -> Option<Bonus> {
        self.bonus
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

    // Shared input policy for portable traversal and native readback tests.
    // It is not a simulation or production input rule.
    #[cfg(any(test, feature = "native-preview"))]
    fn diagnostic_intent(&self) -> DiagnosticIntent {
        if self.stage_entrance.is_some() {
            return (None, None);
        }
        let [marge, ..] = self.stations();
        let target = self
            .enemies()
            .filter(|enemy| enemy.health > 0)
            .min_by_key(|enemy| distance(marge.position, enemy.position));
        let direction = target.map_or(Some(Control::Right), |enemy| {
            diagnostic_direction(marge, enemy.position)
        });
        let action = if matches!(
            marge.participation,
            Participation::Continuing { .. }
        ) {
            Some(Control::Start)
        } else if target.is_some()
            && direction.is_none()
            && marge.maneuver == Maneuver::Idle
        {
            Some(Control::Primary)
        } else {
            None
        };
        (direction, action)
    }

    /// Inspect admitted enemies in stable arrival order, including defeat.
    pub fn enemies(&self) -> impl Iterator<Item = Enemy> + '_ {
        self.encounter.iter().flat_map(Encounter::enemies)
    }

    /// Inspect the initial experimental enemy, including after defeat.
    #[must_use]
    pub const fn enemy(&self) -> Option<Enemy> {
        match self.encounter {
            Some(encounter) => encounter.initial(),
            None => None,
        }
    }

    fn enter_stage_two(&mut self, travel: i16) {
        if let Some(encounter) = &mut self.encounter {
            encounter.enter_stage_two(travel);
        }
        self.bonus = None;
        self.camera = 0;
        self.cooperation.reset();
        self.late_backdrop_tick = None;
        self.scripted_tick = None;
        self.stage_entrance = Some(StageEntrance::default());
        self.stationary_tick = None;
        self.wrestler = None;
        for slot in &mut self.slots {
            slot.restore_position(0);
        }
    }

    fn finish_tick(&mut self, mut pending: [PendingManeuver; 4]) {
        if self.phase() != Phase::Playing {
            return;
        }
        if self.advance_bonus(pending.map(|queued| queued.button_edges != 0))
            || self.advance_stage_entrance()
        {
            return;
        }
        let [marge, ..] = pending;
        let restraint_edge = marge.button_edges.is_power_of_two();
        let pair_command = self.cooperation.filter_actions(&mut pending);
        for ((player, slot), queued) in
            Player::ALL.into_iter().zip(&mut self.slots).zip(pending)
        {
            slot.advance_maneuver(player, queued);
        }
        self.advance_continue();
        if self.phase() == Phase::Playing {
            let layout = self.movement_layout();
            let travel = self.active_scroll_limit();
            self.cooperation
                .advance(&mut self.slots, layout, pair_command);
            if let Some(encounter) = &mut self.encounter {
                let defeated = encounter.advance(
                    layout,
                    (self.layout, travel),
                    (&mut self.slots, restraint_edge),
                );
                self.impact_cues = self.impact_cues.saturating_add(defeated);
            }
            if let Some(boss) = &mut self.wrestler {
                let defeated = boss.advance(layout, &mut self.slots);
                self.impact_cues =
                    self.impact_cues.saturating_add(u64::from(defeated));
            }
            self.cooperation.invalidate(&self.slots);
            self.advance_camera();
            if self.wrestler_enabled && self.late_backdrop_tick == Some(0) {
                self.wrestler = Some(Wrestler::new(self.camera));
            }
            self.advance_scripted_scene();
            if let Some(encounter) = &mut self.encounter {
                encounter.admit(self.camera);
            }
            self.record_attack_cues();
            self.admit_bonus();
        }
    }

    /// Cumulative resolved enemy defeats for optional host audio.
    #[must_use]
    pub const fn impact_cues(&self) -> u64 {
        self.impact_cues
    }

    /// Fixed playing ticks since the late backdrop camera admission.
    #[must_use]
    pub const fn late_backdrop_tick(&self) -> Option<u16> {
        self.late_backdrop_tick
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
            bonus: None,
            bonus_enabled: false,
            camera: 0,
            choices: Player::ALL,
            cooperation: Cooperation::default(),
            encounter: None,
            impact_cues: 0,
            late_backdrop_tick: None,
            layout,
            scripted_tick: None,
            scroll_limit: 0,
            session: LocalSession::default(),
            slots: layout.spawn.map(Slot::new),
            stage_entrance: None,
            stage_two_limit: None,
            stationary_tick: None,
            wrestler: None,
            wrestler_enabled: false,
        })
    }

    /// Inspect the shared pose of an admitted character pair.
    #[must_use]
    pub const fn pair_pose(&self, pair: Pair) -> Option<PairPose> {
        self.cooperation.pose(pair, &self.slots)
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
        self.bonus = None;
        self.cooperation.reset();
        self.camera = 0;
        self.late_backdrop_tick = None;
        self.scripted_tick = None;
        self.stage_entrance = None;
        self.stationary_tick = None;
        self.wrestler = None;
        if let Some(encounter) = &mut self.encounter {
            encounter.restart();
            encounter.art_travel = self.scroll_limit;
        }
        for slot in &mut self.slots {
            let held = slot.held;
            let attack_cues = slot.station.attack_cues;
            let hurt_cues = slot.station.hurt_cues;
            *slot = Slot::new(slot.spawn);
            slot.held = held;
            slot.station.attack_cues = attack_cues;
            slot.station.hurt_cues = hurt_cues;
        }
    }

    /// Elapsed fixed ticks in the one-shot mid-street scripted scene.
    #[must_use]
    pub fn scripted_scene_tick(&self) -> Option<u16> {
        self.scripted_tick
            .filter(|tick| *tick < MID_STREET_SCRIPT_TICKS)
    }

    /// Shared menu state used by native and future host presentations.
    #[must_use]
    pub const fn session(&self) -> LocalSession {
        self.session
    }

    /// Active second-stage arrival and landing sequence, if any.
    #[must_use]
    pub const fn stage_entrance(&self) -> Option<StageEntrance> {
        self.stage_entrance
    }

    /// Whether the optional second stage has been admitted.
    #[must_use]
    pub const fn stage_two(&self) -> bool {
        match self.encounter {
            Some(encounter) => encounter.stage_two,
            None => false,
        }
    }

    /// Current fixed-step phase of the stationary scenic actor.
    #[must_use]
    pub const fn stationary_scene_tick(&self) -> Option<u16> {
        self.stationary_tick
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

    /// Enable the observed Bart/Lisa paired button responses.
    ///
    /// Requires Bart/Lisa locomotion to be enabled. Ground points and
    /// combat eligibility remain provisional during these actions.
    #[must_use]
    pub const fn with_bart_lisa_jump(self, enabled: bool) -> Self {
        self.with_pair_jump(Pair::BartLisa, enabled)
    }

    /// Opt into the provisional balloon bonus after the wrestler fight.
    #[must_use]
    pub const fn with_bonus(mut self, enabled: bool) -> Self {
        self.bonus_enabled = enabled && self.wrestler_enabled;
        if !self.bonus_enabled {
            self.bonus = None;
            self.stage_two_limit = None;
        }
        self
    }

    /// Enable optional pair locomotion in cabinet pair order.
    ///
    /// Solo actions and damage separate the pair until neutral admission.
    /// This remains provisional rather than a cooperative combat claim.
    #[must_use]
    pub const fn with_cooperative_pairs(mut self, enabled: [bool; 2]) -> Self {
        self.cooperation = Cooperation::configure(enabled);
        self
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
        cabinet.encounter = Some(Encounter::new(spawn, false));
        Ok(cabinet)
    }

    /// Enable the provisional wrestler fight at the complete street end.
    /// Requires an opening-street encounter and the verified travel limit.
    #[must_use]
    pub const fn with_late_wrestler(mut self, enabled: bool) -> Self {
        let progressive = match self.encounter {
            Some(encounter) => encounter.progressive,
            None => false,
        };
        self.wrestler_enabled =
            enabled && self.scroll_limit == LATE_BACKDROP_ORIGIN && progressive;
        if !self.wrestler_enabled {
            self.wrestler = None;
            self.bonus = None;
            self.bonus_enabled = false;
            self.stage_two_limit = None;
        }
        self
    }

    /// Opt into the provisional Marge restraint for the opening street.
    ///
    /// The single-player, right-facing contact rule is experimental.
    /// A host should enable it only with the complete combined pose set.
    #[must_use]
    pub const fn with_marge_restraint(mut self, enabled: bool) -> Self {
        if let Some(encounter) = &mut self.encounter {
            encounter.marge_restraint = enabled && encounter.progressive;
        }
        self
    }

    /// Construct the provisional opening-street encounter and arrivals.
    ///
    /// Additional enemies are admitted only as verified world travel permits.
    /// # Errors
    /// Rejects an invalid starting layout.
    pub fn with_opening_street() -> Result<Self, LayoutError> {
        let mut cabinet = Self::new(StreetLayout::DOWNTOWN)?;
        cabinet.encounter =
            Some(Encounter::new(Point { x: 246, y: 190 }, true));
        Ok(cabinet)
    }

    /// Enable the observed combined vertical action for one pair.
    ///
    /// Requires pair locomotion. Ground points and hit rules remain
    /// provisional.
    #[must_use]
    pub const fn with_pair_jump(mut self, pair: Pair, enabled: bool) -> Self {
        let state = match pair {
            Pair::BartLisa => &mut self.cooperation.bart_lisa,
            Pair::MargeHomer => &mut self.cooperation.marge_homer,
        };
        state.support = match (state.support, enabled) {
            (PairSupport::Disabled, _) => PairSupport::Disabled,
            (_, true) => PairSupport::Jump,
            (_, false) => PairSupport::Locomotion,
        };
        self
    }

    /// Enable bounded rightward opening-street travel.
    ///
    /// The host supplies the extent of verified additional world art.
    /// Progressive combat gates the remaining art until the admitted
    /// encounters are cleared.
    /// Rendering dimensions are not simulation inputs.
    /// # Errors
    /// Rejects travel outside the representable world coordinates.
    pub fn with_scroll_limit(
        mut self,
        travel: u16,
    ) -> Result<Self, LayoutError> {
        let requested = i16::try_from(travel)
            .map_err(|_error| LayoutError::ScrollOverflow)?;
        if self.stage_two()
            || self.layout.max.x.checked_add(requested).is_none()
            || requested < self.camera
        {
            return Err(LayoutError::ScrollOverflow);
        }
        if let Some(encounter) = &mut self.encounter {
            encounter.art_travel = requested;
        }
        self.scroll_limit = requested;
        Ok(self)
    }

    /// Opt into the verified extent of the provisional second stage.
    /// Requires the optional balloon bonus and a positive art extent.
    /// # Errors
    /// Rejects travel outside the representable world coordinates.
    pub fn with_stage_two(mut self, travel: u16) -> Result<Self, LayoutError> {
        let limit = i16::try_from(travel)
            .map_err(|_error| LayoutError::ScrollOverflow)?;
        if self.stage_two() || self.layout.max.x.checked_add(limit).is_none() {
            return Err(LayoutError::ScrollOverflow);
        }
        self.stage_two_limit =
            (self.bonus_enabled && limit > 0).then_some(limit);
        Ok(self)
    }

    /// Current optional wrestler encounter, including its cleared state.
    #[must_use]
    pub const fn wrestler(&self) -> Option<Wrestler> {
        self.wrestler
    }
}

impl InputStep for Cabinet {
    type Input = LogicalAction<Action>;

    fn input_step(&mut self, _tick: u64, actions: &[Self::Input]) {
        let mut pending = [PendingManeuver::default(); 4];
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

#[cfg(any(test, feature = "native-preview"))]
fn diagnostic_direction(station: Station, to: Point) -> Option<Control> {
    let from = station.position;
    let facing_right = station.facing == Facing::Right;
    if from.x != to.x
        && (from.x.abs_diff(to.x) > 24 || (from.x < to.x) != facing_right)
    {
        Some(if from.x < to.x {
            Control::Right
        } else {
            Control::Left
        })
    } else if from.y.abs_diff(to.y) > 6 {
        Some(if from.y < to.y {
            Control::Down
        } else {
            Control::Up
        })
    } else {
        None
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

fn player_strike(
    station: &mut Station,
    target: Point,
    extent: (u16, u16),
) -> bool {
    station.can_fight()
        && in_range(station.position, station.facing, target, extent)
        && (matches!(station.maneuver, Maneuver::Attack {
            tick: PLAYER_STRIKE,
            ..
        }) || station.maneuver.consume_air_strike())
}

const fn hurt_slot(slot: &mut Slot, amount: u8) {
    slot.damage(amount);
    slot.station.hurt_cues = slot.station.hurt_cues.saturating_add(1);
}

fn queue_maneuver(queued: &mut PendingManeuver, slot: &Slot, control: Control) {
    let (maneuver, other) = if control == Control::Primary {
        (
            Maneuver::Attack { moving: false, tick: 0 },
            Control::Secondary,
        )
    } else {
        (Maneuver::jump(), Control::Primary)
    };
    if slot.station.maneuver == Maneuver::Idle
        && !queued.chord
        && slot.held(other)
    {
        match (queued.candidate, maneuver) {
            (Some(jump @ Maneuver::Jump(_)), Maneuver::Attack { .. })
            | (Some(Maneuver::Attack { .. }), jump @ Maneuver::Jump(_)) => {
                queued.candidate = Some(jump);
                queued.chord = true;
                return;
            },
            _ => {},
        }
    }
    let _first = queued.candidate.get_or_insert(maneuver);
}

/// Clamp the logical viewport to the available strip.
fn viewport_width(strip_width: u32) -> u32 {
    STRIP_VIEWPORT.width(strip_width).unwrap_or(BASE_WIDTH)
}

/// World origin of a supported logical view.
fn view_origin(strip_width: u32, width: u32, camera: i16) -> Option<i16> {
    let origin = STRIP_VIEWPORT.origin(
        strip_width,
        width,
        u32::try_from(camera).ok()?,
    )?;
    i16::try_from(origin).ok()
}

/// Verified strip interior width for additional camera travel.
fn strip_width(travel: i16) -> u32 {
    BASE_WIDTH
        .saturating_sub(SIDE * 2)
        .saturating_add(u32::try_from(travel).unwrap_or_default())
}

#[cfg(test)]
fn cabinet() -> Cabinet {
    let result = Cabinet::new(StreetLayout::DOWNTOWN);
    assert!(result.is_ok());
    // A valid constant layout cannot fail; keep fixture construction
    // independent from any mutable global or host input.
    result.unwrap_or_else(|_error| Cabinet {
        bonus: None,
        bonus_enabled: false,
        camera: 0,
        choices: Player::ALL,
        cooperation: Cooperation::default(),
        encounter: None,
        impact_cues: 0,
        late_backdrop_tick: None,
        layout: StreetLayout::DOWNTOWN,
        scripted_tick: None,
        scroll_limit: 0,
        session: LocalSession::default(),
        slots: StreetLayout::DOWNTOWN.spawn.map(Slot::new),
        stage_entrance: None,
        stage_two_limit: None,
        stationary_tick: None,
        wrestler: None,
        wrestler_enabled: false,
    })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::ops::Range;

    use games::replay::{InputScript, Scripted};
    use games::timing::StepClock;

    use super::*;

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
    fn light_hit_interrupts_actions_and_releases_held_movement() {
        for (player, duration) in light_hit_durations() {
            check_hurt_movement(player, duration);
        }
    }

    fn check_hurt_movement(player: Player, duration: u8) {
        let mut state = four_playing();
        state.input_step(3, &pulse(player, Control::Primary));
        let original = state.stations();
        hurt_station(&mut state, player);
        let before = station(&state, player);
        assert_eq!(before.maneuver, Maneuver::Hurt { tick: 0 });
        assert_eq!(before.health, PLAYER_HEALTH - 3);
        assert_eq!(before.attack_cues, 0);
        check_hurt_lock(&mut state, player, duration, before);
        state.input_step(3 + u64::from(duration), &[]);
        let after = station(&state, player);
        assert_eq!(after.maneuver, Maneuver::Idle);
        assert_eq!(after.position.x, before.position.x.saturating_sub(2));
        assert_eq!(after.facing, Facing::Left);
        assert_eq!(after.walking_ticks(), Some(0));
        let unchanged = Player::ALL
            .into_iter()
            .zip(original)
            .filter(|(other, _)| *other != player)
            .all(|(other, previous)| station(&state, other) == previous);
        assert!(unchanged);
    }

    fn check_hurt_lock(
        state: &mut Cabinet,
        player: Player,
        duration: u8,
        before: Station,
    ) {
        for offset in 1..duration {
            let movement = [change(player, Control::Left, true)];
            let actions = if offset == 1 {
                movement.as_slice()
            } else {
                &[]
            };
            state.input_step(3 + u64::from(offset), actions);
            let current = station(state, player);
            assert_eq!(current.maneuver, Maneuver::Hurt { tick: offset });
            assert_eq!(current.position, before.position);
            assert_eq!(current.facing, before.facing);
            assert_eq!(current.walking, None);
            assert_eq!(current.attack_cues, before.attack_cues);
        }
    }

    #[test]
    fn light_hit_discards_early_buttons_and_admits_fresh_recovery_edges() {
        for (player, duration) in light_hit_durations() {
            check_hurt_button(player, duration, Control::Primary);
            check_hurt_button(player, duration, Control::Secondary);
        }
    }

    fn check_hurt_button(player: Player, duration: u8, control: Control) {
        let mut state = four_playing();
        hurt_station(&mut state, player);
        for offset in 1..duration {
            state.input_step(2 + u64::from(offset), &pulse(player, control));
            assert_eq!(station(&state, player).maneuver, Maneuver::Hurt {
                tick: offset
            });
        }
        let mut neutral = state;
        neutral.input_step(2 + u64::from(duration), &[]);
        assert_eq!(station(&neutral, player).maneuver, Maneuver::Idle);
        state.input_step(2 + u64::from(duration), &pulse(player, control));
        let resumed = station(&state, player).maneuver;
        if control == Control::Primary {
            assert_eq!(resumed, Maneuver::Attack { moving: false, tick: 0 });
        } else {
            assert!(matches!(resumed, Maneuver::Jump(_)));
        }
    }

    const fn light_hit_durations() -> [(Player, u8); 4] {
        [
            (Player::Marge, 14),
            (Player::Homer, 14),
            (Player::Bart, 17),
            (Player::Lisa, 14),
        ]
    }

    fn hurt_station(state: &mut Cabinet, player: Player) {
        if let Some(slot) = state.slots.get_mut(player.index()) {
            slot.damage(3);
        }
    }

    fn station(state: &Cabinet, player: Player) -> Station {
        let [marge, homer, bart, lisa] = state.stations();
        match player {
            Player::Marge => marge,
            Player::Homer => homer,
            Player::Bart => bart,
            Player::Lisa => lisa,
        }
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
        assert_eq!(marge.maneuver, Maneuver::special(0));
        assert_eq!(homer.maneuver, marge.maneuver);
        assert_eq!(bart.maneuver, Maneuver::Idle);
        assert_eq!(lisa.maneuver, Maneuver::Idle);
        for tick in 4..34 {
            state.input_step(tick, &[]);
        }
        let [
            mut expected_marge,
            mut expected_homer,
            expected_bart,
            expected_lisa,
        ] = original;
        expected_marge.attack_cues = 1;
        expected_homer.attack_cues = 1;
        assert_eq!(state.stations(), [
            expected_marge,
            expected_homer,
            expected_bart,
            expected_lisa
        ]);
    }

    #[test]
    fn directional_chords_share_the_jump_trace() {
        for player in Player::ALL {
            for direction in
                [Control::Left, Control::Right, Control::Up, Control::Down]
            {
                compare_chord_orders(player, direction);
            }
        }
    }

    fn compare_chord_orders(player: Player, direction: Control) {
        for (buttons, hold) in [
            ([Control::Primary, Control::Secondary], 3),
            ([Control::Secondary, Control::Primary], 3),
            ([Control::Primary, Control::Secondary], 30),
            ([Control::Secondary, Control::Primary], 30),
        ] {
            compare_directional_chord(player, direction, buttons, hold);
        }
    }

    fn compare_directional_chord(
        player: Player,
        direction: Control,
        buttons: [Control; 2],
        hold: u64,
    ) {
        let mut ordinary = four_playing();
        let mut chord = four_playing();
        ordinary.input_step(3, &[
            change(player, direction, true),
            change(player, Control::Secondary, true),
        ]);
        let [first, second] = buttons;
        chord.input_step(3, &[
            change(player, first, true),
            change(player, second, true),
            change(player, direction, true),
        ]);
        assert!(matches!(
            station_for(&chord, player).maneuver,
            Maneuver::Jump(j) if j.tick() == 0
        ));
        assert_eq!(chord.stations(), ordinary.stations());
        ordinary.input_step(4, &[change(player, direction, false)]);
        chord.input_step(4, &[
            change(player, direction, false),
            change(player, Control::Primary, false),
        ]);
        assert_eq!(chord.stations(), ordinary.stations());
        for tick in 5..64 {
            let release = [change(player, Control::Secondary, false)];
            let edges = if tick == 3u64.saturating_add(hold) {
                release.as_slice()
            } else {
                &[]
            };
            ordinary.input_step(tick, edges);
            chord.input_step(tick, edges);
            assert_eq!(chord.stations(), ordinary.stations());
        }
        assert_eq!(station_for(&chord, player).maneuver, Maneuver::Idle);
    }

    #[test]
    fn fresh_opposed_directions_select_the_ordinary_jump() {
        let all = [Control::Left, Control::Right, Control::Up, Control::Down];
        for player in Player::ALL {
            for directions in [
                &[Control::Left, Control::Right][..],
                &[Control::Up, Control::Down][..],
                &all[..],
            ] {
                compare_opposed_chord(player, directions);
            }
        }
    }

    fn compare_opposed_chord(player: Player, directions: &[Control]) {
        let mut ordinary = four_playing();
        let mut chord = ordinary;
        let position = station_for(&ordinary, player).position;
        let mut edges = direction_changes(player, directions, true);
        edges.push(change(player, Control::Secondary, true));
        ordinary.input_step(3, &edges);
        edges.push(change(player, Control::Primary, true));
        chord.input_step(3, &edges);
        assert_eq!(chord.stations(), ordinary.stations());
        chord.input_step(4, &[change(player, Control::Primary, false)]);
        ordinary.input_step(4, &[]);
        assert_eq!(chord.stations(), ordinary.stations());
        let mut release = direction_changes(player, directions, false);
        release.push(change(player, Control::Secondary, false));
        for tick in 5..63 {
            let changes = if tick == 13 {
                release.as_slice()
            } else {
                &[]
            };
            chord.input_step(tick, changes);
            ordinary.input_step(tick, changes);
            assert_eq!(chord.stations(), ordinary.stations());
            assert_eq!(station_for(&chord, player).position, position);
        }
        assert_eq!(station_for(&chord, player).maneuver, Maneuver::Idle);
    }

    fn direction_changes(
        player: Player,
        directions: &[Control],
        down: bool,
    ) -> Vec<LogicalAction<Action>> {
        directions
            .iter()
            .map(|control| change(player, *control, down))
            .collect()
    }

    #[test]
    fn preheld_directions_select_special_with_retained_drift() {
        for player in Player::ALL {
            for (directions, stride) in [
                (&[Control::Right][..], 3i16),
                (&[Control::Left][..], -3),
                (&[Control::Left, Control::Right][..], -3),
                (&[Control::Up][..], 0),
                (&[Control::Up, Control::Down][..], 0),
            ] {
                check_special_drift(player, directions, stride);
            }
        }
    }

    fn check_special_drift(
        player: Player,
        directions: &[Control],
        stride: i16,
    ) {
        let mut state = four_playing();
        state.input_step(3, &direction_changes(player, directions, true));
        state.input_step(4, &[]);
        let position = station_for(&state, player).position;
        state.input_step(5, &[
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
        ]);
        let mut release = direction_changes(player, directions, false);
        release.push(change(player, Control::Primary, false));
        release.push(change(player, Control::Secondary, false));
        assert_special_drift(station_for(&state, player), position, stride, 0);
        for offset in 1..30u8 {
            let edges = if offset == 10 {
                release.as_slice()
            } else {
                &[]
            };
            state.input_step(5 + u64::from(offset), edges);
            assert_special_drift(
                station_for(&state, player),
                position,
                stride,
                offset,
            );
        }
        let previous = station_for(&state, player).position;
        state.input_step(35, &[]);
        let landed = station_for(&state, player);
        assert_eq!(landed.maneuver, Maneuver::Idle);
        assert_eq!(landed.position, previous);
    }

    fn assert_special_drift(
        station: Station,
        position: Point,
        stride: i16,
        offset: u8,
    ) {
        let elapsed = i16::from(offset).saturating_add(1);
        let expected = stride.saturating_mul(elapsed).div_euclid(4);
        assert_eq!(station.position.x, position.x.saturating_add(expected));
        assert_eq!(station.position.y, position.y);
        let maneuver = station.maneuver;
        assert!(matches!(maneuver, Maneuver::Special { tick, .. }
                if tick == offset));
    }

    #[test]
    fn directional_chord_preserves_a_same_tick_jump_release() {
        let mut state = four_playing();
        let player = Player::Marge;
        state.input_step(3, &[
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
            change(player, Control::Secondary, false),
            change(player, Control::Secondary, true),
            change(player, Control::Right, true),
        ]);
        state.input_step(4, &[change(player, Control::Right, false)]);
        for tick in 5..34 {
            state.input_step(tick, &[]);
        }
        assert!(matches!(
            station_for(&state, player).maneuver,
            Maneuver::Jump(j) if j.tick() == 30
        ));
        state.input_step(34, &[]);
        assert_eq!(station_for(&state, player).maneuver, Maneuver::Idle);
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
        assert_eq!(marge.maneuver, Maneuver::Attack { moving: false, tick: 0 });
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
        assert_eq!(marge.health, PLAYER_HEALTH);
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
        let end = 3 + u64::from(
            Maneuver::Attack { moving: false, tick: 0 }.pose_duration(player),
        );
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
        assert_eq!(resumed.walking_ticks(), Some(0));
    }

    fn assert_recovery_pose(recovering: Station, depth: i16) {
        assert_eq!(recovering.facing, Facing::Left);
        assert_eq!(recovering.maneuver, Maneuver::Idle);
        assert_eq!(recovering.position.y, depth);
        assert_eq!(recovering.walking_ticks(), None);
    }

    #[test]
    fn ground_recovery_preserves_fractional_drift_until_offset_eighteen() {
        for player in Player::ALL {
            verify_recovery_direction(player, 21, -1, false);
            verify_recovery_direction(player, 22, 0, false);
            verify_recovery_direction(player, 23, -2, true);
        }
    }

    #[test]
    fn attack_drift_begins_after_the_moving_pose_sample() {
        for player in Player::ALL {
            for (direction, offset, length, delta) in [
                (Control::Left, 0, 1, 0),
                (Control::Left, 1, 1, 0),
                (Control::Left, 1, 2, -1),
                (Control::Left, 2, 1, -1),
                (Control::Right, 1, 4, 0),
                (Control::Right, 1, 5, 1),
                (Control::Right, 1, 8, 1),
            ] {
                let mut state = four_playing();
                let before = station_for(&state, player);
                ground_pulse(
                    &mut state,
                    player,
                    direction,
                    (3, offset, length),
                );
                let after = station_for(&state, player);
                assert_eq!(
                    after.position.x,
                    before.position.x.saturating_add(delta)
                );
                assert_eq!(after.position.y, before.position.y);
            }
        }
    }

    fn ground_pulse(
        state: &mut Cabinet,
        player: Player,
        direction: Control,
        (start, offset, length): (u64, u64, u64),
    ) {
        let down = start.saturating_add(offset);
        let up = down.saturating_add(length);
        for tick in start..start.saturating_add(20) {
            let mut edges = Vec::new();
            if tick == start {
                edges.extend(pulse(player, Control::Primary));
            }
            if tick == down {
                edges.push(change(player, direction, true));
            }
            if tick == up {
                edges.push(change(player, direction, false));
            }
            state.input_step(tick, &edges);
        }
    }

    #[test]
    fn two_attacks_share_the_visible_fractional_carry() {
        for player in Player::ALL {
            for (direction, delta) in [(Control::Left, -1), (Control::Right, 1)]
            {
                let mut state = four_playing();
                let before = station_for(&state, player);
                ground_pulse(&mut state, player, direction, (3, 1, 3));
                ground_pulse(&mut state, player, direction, (23, 1, 3));
                let after = station_for(&state, player);
                assert_eq!(
                    after.position.x,
                    before.position.x.saturating_add(delta)
                );
                assert_eq!(after.position.y, before.position.y);
            }
        }
    }

    #[test]
    fn carry_survives_walking_neutral_jump_and_special() {
        let mut state = four_playing();
        let player = Player::Marge;
        let before = station_for(&state, player);
        ground_pulse(&mut state, player, Control::Right, (3, 1, 3));
        state.input_step(23, &[change(player, Control::Right, true)]);
        state.input_step(24, &[]);
        state.input_step(25, &[change(player, Control::Right, false)]);
        state.input_step(26, &pulse(player, Control::Secondary));
        for tick in 27..65 {
            state.input_step(tick, &[]);
        }
        state.input_step(65, &[
            change(player, Control::Primary, true),
            change(player, Control::Secondary, true),
            change(player, Control::Primary, false),
            change(player, Control::Secondary, false),
        ]);
        assert!(matches!(
            station_for(&state, player).maneuver,
            Maneuver::Special { .. }
        ));
        for tick in 66..110 {
            state.input_step(tick, &[]);
        }
        ground_pulse(&mut state, player, Control::Right, (110, 1, 3));
        let after = station_for(&state, player);
        assert_eq!(after.position.x, before.position.x.saturating_add(5));
        assert_eq!(after.position.y, before.position.y);
    }

    #[test]
    fn blocked_outward_fraction_cannot_delay_inward_motion() {
        let player = Player::Marge;
        let mut state = four_playing();
        state.input_step(3, &[change(player, Control::Left, true)]);
        for tick in 4..70 {
            state.input_step(tick, &[]);
        }
        state.input_step(70, &[change(player, Control::Left, false)]);
        let edge = StreetLayout::DOWNTOWN.min.x;
        assert_eq!(station_for(&state, player).position.x, edge);
        ground_pulse(&mut state, player, Control::Left, (71, 2, 1));
        assert_eq!(station_for(&state, player).position.x, edge);
        ground_pulse(&mut state, player, Control::Right, (91, 1, 3));
        assert_eq!(station_for(&state, player).position.x, edge);
        ground_pulse(&mut state, player, Control::Right, (111, 1, 3));
        assert_eq!(
            station_for(&state, player).position.x,
            edge.saturating_add(1)
        );
    }

    #[test]
    fn right_boundary_discards_outward_fraction() {
        let player = Player::Marge;
        let mut state = four_playing();
        state.input_step(3, &[change(player, Control::Right, true)]);
        for tick in 4..130 {
            state.input_step(tick, &[]);
        }
        state.input_step(130, &[change(player, Control::Right, false)]);
        let edge = StreetLayout::DOWNTOWN.max.x;
        assert_eq!(station_for(&state, player).position.x, edge);
        ground_pulse(&mut state, player, Control::Right, (131, 2, 1));
        assert_eq!(station_for(&state, player).position.x, edge);
        ground_pulse(&mut state, player, Control::Left, (151, 2, 1));
        assert_eq!(
            station_for(&state, player).position.x,
            edge.saturating_sub(1)
        );
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
        assert_eq!(moved.walking_ticks(), walking.then_some(0));
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
        assert_eq!(landed.walking_ticks(), None);
        state.input_step(target.saturating_add(1), &[]);
        let walking = station_for(&state, player);
        assert_eq!(walking.position.x, landed.position.x.saturating_sub(2));
        assert_eq!(walking.position.y, landed.position.y.saturating_sub(2));
        assert_eq!(walking.facing, Facing::Left);
        assert_eq!(walking.walking_ticks(), Some(0));
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
                assert_eq!(maneuver, Maneuver::Attack {
                    moving: false,
                    tick: 0
                });
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
        assert_eq!(homer.maneuver, Maneuver::Attack { moving: false, tick: 0 });
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
            Maneuver::Attack { moving: false, tick: 0 }
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
    fn airborne_stride_survives_release_and_can_reverse() {
        for player in Player::ALL {
            verify_airborne_stride(player);
        }
    }

    fn verify_airborne_stride(player: Player) {
        let mut state = four_playing();
        let start = station_for(&state, player).position;
        state.input_step(3, &[
            change(player, Control::Right, true),
            change(player, Control::Secondary, true),
        ]);
        state.input_step(4, &[
            change(player, Control::Right, false),
            change(player, Control::Secondary, false),
        ]);
        for tick in 5..13 {
            state.input_step(tick, &[]);
        }
        assert_eq!(
            station_for(&state, player).position.x,
            start.x.saturating_add(20)
        );
        state.input_step(13, &[
            change(player, Control::Left, true),
            change(player, Control::Up, true),
        ]);
        state.input_step(14, &[
            change(player, Control::Left, false),
            change(player, Control::Up, false),
        ]);
        for tick in 15..34 {
            state.input_step(tick, &[]);
        }
        verify_stride_landing(&mut state, player, start);
    }

    fn verify_stride_landing(
        state: &mut Cabinet,
        player: Player,
        start: Point,
    ) {
        let last = station_for(state, player);
        assert!(matches!(last.maneuver, Maneuver::Jump(j) if j.tick() == 30));
        assert_eq!(last.position.x, start.x.saturating_sub(22));
        assert_eq!(last.position.y, start.y.saturating_sub(2));
        assert_eq!(last.facing, Facing::Left);
        assert_eq!(last.walking_ticks(), None);
        verify_stride_stops(state, player, last.position);
    }

    fn verify_stride_stops(
        state: &mut Cabinet,
        player: Player,
        position: Point,
    ) {
        state.input_step(34, &[]);
        let landed = station_for(state, player);
        assert_eq!(landed.position, position);
        assert_eq!(landed.maneuver, Maneuver::Idle);
        state.input_step(35, &[]);
        assert_eq!(station_for(state, player).position, landed.position);
    }

    #[test]
    fn airborne_stride_can_start_after_a_stationary_takeoff() {
        let mut state = four_playing();
        let player = Player::Marge;
        let start = station_for(&state, player).position;
        state.input_step(3, &pulse(player, Control::Secondary));
        for tick in 4..13 {
            state.input_step(tick, &[]);
        }
        assert_eq!(station_for(&state, player).position, start);
        state.input_step(13, &[change(player, Control::Right, true)]);
        state.input_step(14, &[change(player, Control::Right, false)]);
        for tick in 15..35 {
            state.input_step(tick, &[]);
        }
        let landed = station_for(&state, player);
        assert_eq!(landed.position.x, start.x.saturating_add(42));
        assert_eq!(landed.position.y, start.y);
        assert_eq!(landed.maneuver, Maneuver::Idle);
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
        assert_eq!(attack.maneuver, Maneuver::Attack {
            moving: false,
            tick: 0
        });
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
    fn aerial_attack_admits_takeoff_turn_then_locks_facing() {
        for player in Player::ALL {
            let mut state = four_playing();
            let before = station_for(&state, player);
            state.input_step(3, &[change(player, Control::Secondary, true)]);
            for tick in 4..18 {
                state.input_step(tick, &[]);
            }
            state.input_step(18, &[
                change(player, Control::Primary, true),
                change(player, Control::Left, true),
            ]);
            let attack = station_for(&state, player);
            assert_eq!(attack.facing, Facing::Left);
            assert_eq!(attack.position.x, before.position.x.saturating_sub(4));
            assert!(matches!(attack.maneuver, Maneuver::AirAttack(air)
                if air.tick() == 0));
            state.input_step(19, &[
                change(player, Control::Left, false),
                change(player, Control::Right, true),
            ]);
            let next = station_for(&state, player);
            assert_eq!(next.facing, Facing::Left);
            assert_eq!(next.position.x, attack.position.x.saturating_sub(4));
        }
    }

    #[test]
    fn stationary_special_turns_without_displacement_during_flight() {
        for player in Player::ALL {
            let mut state = special_started(player);
            let before = station_for(&state, player);
            for tick in 4..13 {
                state.input_step(tick, &[]);
            }
            state.input_step(13, &[change(player, Control::Left, true)]);
            let turned = station_for(&state, player);
            assert_eq!(turned.facing, Facing::Left);
            assert_eq!(turned.position, before.position);
            assert!(matches!(turned.maneuver, Maneuver::Special {
                tick: 10,
                ..
            }));
            state.input_step(14, &[change(player, Control::Left, false)]);
            let next = station_for(&state, player);
            assert_eq!(next.facing, Facing::Left);
            assert_eq!(next.position, before.position);
        }
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
    fn opposed_walk_preserves_pose_intent_without_translation() {
        for player in Player::ALL {
            let mut state = four_playing();
            let position = station_for(&state, player).position;
            state.input_step(3, &[
                change(player, Control::Left, true),
                change(player, Control::Right, true),
            ]);
            let forward = station_for(&state, player);
            assert_eq!(forward.position, position);
            assert_eq!(
                forward.walking,
                Some(WalkMotion { away: false, tick: 0 })
            );
            state.input_step(4, &[
                change(player, Control::Up, true),
                change(player, Control::Down, true),
            ]);
            let rear = station_for(&state, player);
            assert_eq!(rear.position, position);
            assert_eq!(rear.walking, Some(WalkMotion { away: true, tick: 1 }));
            state.input_step(5, &[change(player, Control::Left, false)]);
            let lateral = station_for(&state, player);
            assert_eq!(lateral.position.y, position.y);
            assert_eq!(lateral.position.x, position.x.saturating_add(2));
            assert_eq!(
                lateral.walking,
                Some(WalkMotion { away: true, tick: 2 })
            );
            state.input_step(
                6,
                &direction_changes(
                    player,
                    &[Control::Right, Control::Up, Control::Down],
                    false,
                ),
            );
            assert_eq!(station_for(&state, player).walking, None);
        }
    }

    #[test]
    // The walk cycle continues while input is held against a boundary.
    fn walking_ticks_follow_held_input_and_restart() {
        let mut state = four_playing();
        state.input_step(3, &[change(Player::Marge, Control::Right, true)]);
        let [started, _, _, _] = state.stations();
        assert_eq!(started.walking_ticks(), Some(0));
        state.input_step(4, &[]);
        let [moving, _, _, _] = state.stations();
        assert_eq!(moving.walking_ticks(), Some(1));
        state.input_step(5, &[change(Player::Marge, Control::Left, true)]);
        let [opposed, _, _, _] = state.stations();
        assert_eq!(opposed.walking_ticks(), Some(2));
        assert_eq!(opposed.position, moving.position);
        state.input_step(6, &[change(Player::Marge, Control::Right, false)]);
        let [restarted, _, _, _] = state.stations();
        assert_eq!(restarted.walking_ticks(), Some(3));
        for tick in 7..300 {
            state.input_step(tick, &[]);
        }
        let [blocked, _, _, _] = state.stations();
        assert_eq!(blocked.position.x, StreetLayout::DOWNTOWN.min.x);
        assert_eq!(blocked.walking_ticks(), Some(296));
        state.input_step(300, &[change(Player::Marge, Control::Left, false)]);
        assert_eq!(station_for(&state, Player::Marge).walking, None);
    }

    #[test]
    fn moving_attack_samples_only_the_second_input_tick() {
        for player in Player::ALL {
            for offset in 0..3 {
                assert_moving_attack_offset(
                    player,
                    offset,
                    &[Control::Right],
                    true,
                );
                assert_moving_attack_offset(
                    player,
                    offset,
                    &[Control::Left, Control::Right],
                    true,
                );
                assert_moving_attack_offset(
                    player,
                    offset,
                    &[Control::Up, Control::Down],
                    false,
                );
            }
        }
    }

    fn assert_moving_attack_offset(
        player: Player,
        offset: u64,
        directions: &[Control],
        horizontal: bool,
    ) {
        let mut state = four_playing();
        let position = station_for(&state, player).position;
        for tick in 3..8 {
            let mut edges = Vec::new();
            if tick == 3 {
                edges.extend(pulse(player, Control::Primary));
            }
            if tick == 3u64.saturating_add(offset) {
                edges.extend(direction_changes(player, directions, true));
            }
            if tick == 4u64.saturating_add(offset) {
                edges.extend(direction_changes(player, directions, false));
            }
            state.input_step(tick, &edges);
            if directions.len() == 2 {
                assert_eq!(station_for(&state, player).position, position);
            }
        }
        assert_eq!(station_for(&state, player).maneuver, Maneuver::Attack {
            moving: horizontal && offset == 1,
            tick: 4
        });
    }

    #[test]
    fn walking_phase_survives_direction_changes() {
        for player in Player::ALL {
            assert_walking_direction_changes(player);
        }
    }

    fn assert_walking_direction_changes(player: Player) {
        let mut state = four_playing();
        state.input_step(3, &[change(player, Control::Right, true)]);
        let started = station_for(&state, player);
        assert_eq!(started.walking, Some(WalkMotion { away: false, tick: 0 }));
        state.input_step(4, &[change(player, Control::Up, true)]);
        let diagonal = station_for(&state, player);
        assert_eq!(diagonal.walking, Some(WalkMotion { away: true, tick: 1 }));
        assert_eq!(diagonal.position.x, started.position.x.saturating_add(2));
        assert_eq!(diagonal.position.y, started.position.y.saturating_sub(2));
        state.input_step(5, &[change(player, Control::Up, false)]);
        assert_eq!(
            station_for(&state, player).walking,
            Some(WalkMotion { away: false, tick: 2 })
        );
        state.input_step(6, &[change(player, Control::Right, false)]);
        assert_eq!(station_for(&state, player).walking, None);
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
        assert_eq!(attacking.maneuver, Maneuver::Attack {
            moving: false,
            tick: 0
        });
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
        assert_eq!(recovering.walking_ticks(), None);
        for tick in 21..=23 {
            state.input_step(tick, &[]);
        }
        let [walking, ..] = state.stations();
        assert_eq!(walking.maneuver, Maneuver::Idle);
        assert_eq!(walking.position.x, position.x.saturating_add(2));
        assert_eq!(walking.walking_ticks(), Some(0));
    }

    fn assert_maneuvers_at_end(state: &Cabinet, position: Point) {
        let [attack_end, airborne, ..] = state.stations();
        assert_eq!(attack_end.position, position);
        assert_eq!(attack_end.maneuver, Maneuver::Attack {
            moving: false,
            tick: 16
        });
        assert!(
            matches!(airborne.maneuver, Maneuver::Jump(j) if j.tick() == 16)
        );
    }

    #[test]
    fn air_attack_cues_are_station_local_and_plain_jumps_are_silent() {
        let mut state = four_playing();
        let jumps =
            Player::ALL.map(|player| change(player, Control::Secondary, true));
        state.input_step(3, &jumps);
        for tick in 4..18 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [0; 4]);
        let attacks = [Player::Marge, Player::Bart]
            .map(|player| change(player, Control::Primary, true));
        state.input_step(18, &attacks);
        for tick in 19..22 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [0; 4]);
        state.input_step(22, &[]);
        assert_eq!(state.stations().map(|station| station.attack_cues), [
            1, 0, 1, 0
        ]);
        for tick in 23..83 {
            state.input_step(tick, &[]);
        }
        assert_eq!(state.stations().map(|station| station.attack_cues), [
            1, 0, 1, 0
        ]);
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

    #[test]
    fn late_backdrop_clock_pauses_holds_and_resets() {
        let Ok(mut state) = cabinet().with_scroll_limit(1360) else {
            assert!(cabinet().with_scroll_limit(1360).is_ok());
            return;
        };
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.input_step(1, &[change(Player::Marge, Control::Right, true)]);
        for tick in 2..1000 {
            state.input_step(tick, &[]);
            if state.camera_origin() == LATE_BACKDROP_ORIGIN {
                break;
            }
            assert_eq!(state.late_backdrop_tick(), None);
        }
        assert_late_backdrop_admission(state);
    }

    #[test]
    fn wrestler_admission_is_opt_in_and_pauses_with_the_arena() {
        let result = Cabinet::with_opening_street()
            .and_then(|state| state.with_scroll_limit(1360));
        assert!(result.is_ok());
        let Ok(initial) = result else {
            return;
        };
        let mut state = initial.with_late_wrestler(true);
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.camera = LATE_BACKDROP_ORIGIN;
        let [first, ..] = &mut state.slots;
        first.station.position.x = LATE_BACKDROP_ORIGIN.saturating_add(208);
        state.input_step(1, &[]);
        assert_eq!(
            state.wrestler().map(|boss| boss.action),
            Some(WrestlerAction::Intro { tick: 0 })
        );
        state.input_step(2, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 3..20);
        assert_eq!(
            state.wrestler().map(|boss| boss.action),
            Some(WrestlerAction::Intro { tick: 0 })
        );
        state.input_step(20, &pulse(Player::Marge, Control::Pause));
        assert_eq!(
            state.wrestler().map(|boss| boss.action),
            Some(WrestlerAction::Intro { tick: 1 })
        );
        state.reset_world();
        assert_eq!(state.wrestler(), None);
        assert_eq!(state.camera_origin(), 0);
    }

    #[test]
    fn bonus_is_opt_in_and_freezes_street_during_pumping() {
        let result = Cabinet::with_opening_street()
            .and_then(|state| state.with_scroll_limit(1360));
        assert!(result.is_ok());
        let Ok(initial) = result else {
            return;
        };
        let mut state = initial.with_late_wrestler(true);
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.wrestler = Some(Wrestler {
            action: WrestlerAction::Cleared,
            health: 0,
            ..Wrestler::new(0)
        });
        state.admit_bonus();
        assert_eq!(state.bonus(), None);
        state = state.with_bonus(true);
        state.admit_bonus();
        assert_eq!(state.bonus().map(Bonus::phase), Some(BonusPhase::Victory));
        let stations = state.stations();
        for tick in 1..2000 {
            if state.bonus().map(Bonus::phase) == Some(BonusPhase::Pumping) {
                break;
            }
            state.input_step(tick, &[]);
        }
        assert_eq!(state.bonus().map(Bonus::phase), Some(BonusPhase::Pumping));
        assert_eq!(state.stations(), stations);
        assert_bonus_inputs(state);
        assert_bonus_chord(state);
    }

    #[test]
    fn second_stage_preserves_stations_and_resets_campaign() {
        let initial = Cabinet::with_opening_street()
            .and_then(|state| state.with_scroll_limit(1360))
            .map(|state| state.with_late_wrestler(true).with_bonus(true))
            .and_then(|state| state.with_stage_two(270));
        assert!(initial.is_ok());
        let Ok(mut state) = initial else {
            return;
        };
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.input_step(1, &pulse(Player::Homer, Control::Start));
        state.camera = LATE_BACKDROP_SETTLED_ORIGIN;
        let [marge, ..] = &mut state.slots;
        marge.station.health = PLAYER_HEALTH.saturating_sub(1);
        marge.station.stocks = 1;
        marge.station.score = 11;
        state.wrestler = Some(Wrestler {
            action: WrestlerAction::Cleared,
            health: 0,
            ..Wrestler::new(state.camera)
        });
        state.admit_bonus();
        for tick in 2..4000 {
            if state.bonus().map(Bonus::phase) == Some(BonusPhase::Complete) {
                break;
            }
            state.input_step(tick, &[]);
        }
        assert_eq!(state.bonus().map(Bonus::phase), Some(BonusPhase::Complete));
        state.input_step(4000, &pulse(Player::Marge, Control::Pause));
        state.input_step(4001, &[]);
        assert!(!state.stage_two());
        assert_stage_two_handoff(state);
    }

    fn assert_stage_two_handoff(mut state: Cabinet) {
        let before = state.stations();
        state.input_step(4002, &pulse(Player::Marge, Control::Pause));
        assert_eq!(
            state.bonus().map(Bonus::phase),
            Some(BonusPhase::StageCard)
        );
        assert_card_input_gate(state);
        assert_card_boundary(state);
        assert_card_pause(state);
        advance_empty(&mut state, 4003..4194);
        assert!(!state.stage_two());
        state.input_step(4194, &[]);
        assert!(state.stage_two());
        assert_eq!(state.camera_origin(), 0);
        assert_eq!(state.bonus(), None);
        assert_eq!(state.wrestler(), None);
        assert_eq!(state.enemies().count(), 2);
        assert_stage_two_stations(&before, &state.stations());
        assert_stage_entrance(state);
        assert_stage_two_restart(state);
    }

    fn assert_card_input_gate(mut state: Cabinet) {
        state.input_step(4003, &[change(
            Player::Marge,
            Control::Primary,
            true,
        )]);
        advance_empty(&mut state, 4004..4068);
        assert_eq!(
            state.bonus().map(Bonus::phase),
            Some(BonusPhase::StageCard)
        );
        state.input_step(4068, &[change(
            Player::Marge,
            Control::Primary,
            false,
        )]);
        state.input_step(4069, &pulse(Player::Bart, Control::Primary));
        assert!(!state.stage_two());
        state.input_step(4070, &pulse(Player::Marge, Control::Secondary));
        assert!(state.stage_two());
    }

    fn assert_card_boundary(mut state: Cabinet) {
        advance_empty(&mut state, 4003..4066);
        state.input_step(4066, &pulse(Player::Marge, Control::Primary));
        assert!(!state.stage_two());
        state.input_step(4067, &pulse(Player::Marge, Control::Secondary));
        assert!(state.stage_two());
    }

    fn assert_card_pause(mut state: Cabinet) {
        advance_empty(&mut state, 4003..4033);
        let before = state.bonus().map(Bonus::tick);
        state.input_step(4033, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 4034..4234);
        assert_eq!(state.bonus().map(Bonus::tick), before);
        assert!(!state.stage_two());
        state.input_step(4234, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.bonus().map(Bonus::tick), Some(31));
    }

    fn assert_stage_two_stations(before: &[Station; 4], after: &[Station; 4]) {
        for (old, new) in before.iter().zip(after) {
            assert_eq!(old.health, new.health);
            assert_eq!(old.stocks, new.stocks);
            assert_eq!(old.participation, new.participation);
            assert_eq!(old.display_health, new.display_health);
            assert_eq!(old.score, new.score);
        }
    }

    fn assert_stage_entrance(mut state: Cabinet) {
        assert_eq!(state.stage_entrance().map(StageEntrance::tick), Some(0));
        let mut held = state;
        let initial = state.stations();
        let enemies = state.encounter;
        held.input_step(4195, &[change(Player::Marge, Control::Right, true)]);
        state.input_step(4195, &[]);
        for tick in 4196..4426 {
            held.input_step(tick, &pulse(Player::Marge, Control::Primary));
            state.input_step(tick, &[]);
            assert_eq!(state.stations(), held.stations());
            assert_eq!(state.stage_entrance(), held.stage_entrance());
        }
        assert_eq!(state.stage_entrance().map(StageEntrance::tick), Some(231));
        assert_stage_formation(&state);
        assert_stage_two_stations(&initial, &state.stations());
        assert_eq!(state.encounter, enemies);
        assert_stage_steering_and_pause(state, held);
    }

    fn assert_stage_formation(state: &Cabinet) {
        let [marge, homer, bart, lisa] = state.stations();
        assert_eq!(marge.position, Point { x: 132, y: 192 });
        assert_eq!(homer.position, Point { x: 164, y: 192 });
        assert_eq!(bart.participation, Participation::Absent);
        assert_eq!(lisa.participation, Participation::Absent);
    }

    fn assert_stage_steering_and_pause(mut state: Cabinet, mut held: Cabinet) {
        held.input_step(4426, &[]);
        state.input_step(4426, &[]);
        assert_eq!(
            held.stations().first().map(|station| station.position.x),
            Some(134)
        );
        assert_eq!(
            state.stations().first().map(|station| station.position.x),
            Some(132)
        );
        held.input_step(4427, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut held, 4428..4528);
        assert_eq!(held.stage_entrance().map(StageEntrance::tick), Some(232));
        held.reset_world();
        assert_eq!(held.stage_entrance(), None);
    }

    fn assert_stage_two_restart(mut state: Cabinet) {
        advance_empty(&mut state, 4195..4490);
        assert_eq!(state.stage_entrance(), None);
        state.input_step(4490, &pulse(Player::Marge, Control::Primary));
        assert_ne!(
            state.stations().first().map(|station| station.maneuver),
            Some(Maneuver::Idle)
        );
        state.reset_world();
        assert!(!state.stage_two());
        assert_eq!(state.active_scroll_limit(), 1360);
        assert_eq!(state.stations().map(|station| station.score), [0; 4]);
        assert_eq!(state.enemies().count(), 1);
    }

    fn assert_bonus_chord(mut state: Cabinet) {
        let mut single = state;
        single.input_step(3000, &pulse(Player::Marge, Control::Primary));
        state.input_step(3000, &[
            change(Player::Marge, Control::Primary, true),
            change(Player::Marge, Control::Secondary, true),
            change(Player::Marge, Control::Primary, false),
            change(Player::Marge, Control::Secondary, false),
        ]);
        assert_eq!(state.bonus(), single.bonus());
        let mut secondary = single;
        state.input_step(3001, &pulse(Player::Marge, Control::Primary));
        secondary.input_step(3001, &pulse(Player::Marge, Control::Secondary));
        assert_eq!(state.bonus(), secondary.bonus());
    }

    fn assert_bonus_inputs(mut state: Cabinet) {
        let stations = state.stations();
        state.input_step(2000, &pulse(Player::Marge, Control::Pause));
        let paused = state.bonus();
        advance_empty(&mut state, 2001..2020);
        assert_eq!(state.bonus(), paused);
        state.input_step(2020, &pulse(Player::Marge, Control::Pause));
        state.input_step(2021, &[change(
            Player::Marge,
            Control::Primary,
            true,
        )]);
        assert_eq!(
            state
                .bonus()
                .and_then(|bonus| bonus.inflation(Player::Marge)),
            Some(3)
        );
        state.input_step(2022, &[change(
            Player::Marge,
            Control::Primary,
            true,
        )]);
        state.input_step(2023, &pulse(Player::Homer, Control::Start));
        state.input_step(2024, &pulse(Player::Homer, Control::Secondary));
        assert_eq!(
            state
                .bonus()
                .and_then(|bonus| bonus.inflation(Player::Marge)),
            Some(3)
        );
        assert_eq!(
            state
                .bonus()
                .and_then(|bonus| bonus.inflation(Player::Homer)),
            None
        );
        assert_eq!(state.stations(), stations);
        state.reset_world();
        assert_eq!(state.bonus(), None);
    }

    fn assert_late_backdrop_admission(mut state: Cabinet) {
        assert_eq!(state.camera_origin(), LATE_BACKDROP_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), None);
        state.input_step(1000, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 1001..1020);
        assert_eq!(state.camera_origin(), LATE_BACKDROP_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), None);
        state.input_step(1020, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.camera_origin(), LATE_BACKDROP_SETTLED_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), Some(0));
        assert_late_backdrop_pause_and_hold(state);
    }

    fn assert_late_backdrop_pause_and_hold(mut state: Cabinet) {
        state.input_step(1021, &[]);
        assert_eq!(state.camera_origin(), LATE_BACKDROP_SETTLED_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), Some(1));
        state.input_step(1022, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 1023..1040);
        assert_eq!(state.camera_origin(), LATE_BACKDROP_SETTLED_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), Some(1));
        state.input_step(1040, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.late_backdrop_tick(), Some(2));
        advance_empty(&mut state, 1041..1420);
        assert_eq!(state.camera_origin(), LATE_BACKDROP_SETTLED_ORIGIN);
        assert_eq!(state.late_backdrop_tick(), Some(LATE_BACKDROP_HOLD_TICK));
        state.reset_world();
        assert_eq!(state.late_backdrop_tick(), None);
        assert_eq!(state.camera_origin(), 0);
    }

    fn advance_empty(state: &mut Cabinet, ticks: Range<u64>) {
        for tick in ticks {
            state.input_step(tick, &[]);
        }
    }

    #[test]
    fn scripted_scene_uses_camera_admission_and_fixed_playing_ticks() {
        let result = cabinet().with_scroll_limit(1360);
        assert!(result.is_ok());
        let Ok(mut state) = result else {
            return;
        };
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.input_step(1, &[change(Player::Marge, Control::Right, true)]);
        assert_eq!(state.scripted_scene_tick(), None);
        for tick in 2..1000 {
            state.input_step(tick, &[]);
            if state.camera_origin() >= MID_STREET_SCRIPT_ORIGIN {
                break;
            }
            assert_eq!(state.scripted_scene_tick(), None);
        }
        assert_eq!(state.camera_origin(), MID_STREET_SCRIPT_ORIGIN);
        assert_eq!(state.scripted_scene_tick(), Some(0));
        assert_stationary_departure(state);
        assert_scripted_pause_and_completion(state);
    }

    fn assert_scripted_pause_and_completion(mut state: Cabinet) {
        state.input_step(1000, &[change(Player::Marge, Control::Right, false)]);
        assert_eq!(state.scripted_scene_tick(), Some(1));
        state.input_step(1001, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 1002..1050);
        assert_eq!(state.scripted_scene_tick(), Some(1));
        state.input_step(1050, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.scripted_scene_tick(), Some(2));
        advance_empty(&mut state, 1051..1385);
        assert_eq!(state.scripted_scene_tick(), None);
        advance_empty(&mut state, 1385..1400);
        assert_eq!(state.scripted_scene_tick(), None);
        assert_eq!(state.impact_cues(), 0);
        assert_eq!(state.enemies().count(), 0);
        state.reset_world();
        assert_eq!(state.scripted_scene_tick(), None);
        assert_eq!(state.camera_origin(), 0);
    }

    fn assert_stationary_departure(mut state: Cabinet) {
        assert!(state.stationary_scene_tick().is_some());
        advance_empty(&mut state, 1000..1064);
        assert_eq!(state.scripted_scene_tick(), Some(64));
        assert!(state.stationary_scene_tick().is_some());
        state.input_step(1064, &[]);
        assert_eq!(state.scripted_scene_tick(), Some(65));
        assert_eq!(state.stationary_scene_tick(), None);
        state.reset_world();
        assert_eq!(state.stationary_scene_tick(), None);
    }

    #[test]
    fn stationary_scene_loops_at_the_admission_camera_limit() {
        let Ok(mut state) = cabinet().with_scroll_limit(482) else {
            return;
        };
        state.input_step(0, &pulse(Player::Marge, Control::Start));
        state.input_step(1, &[change(Player::Marge, Control::Right, true)]);
        for tick in 2..1000 {
            state.input_step(tick, &[]);
            if state.camera_origin() == STATIONARY_SCENE_ORIGIN {
                break;
            }
            assert_eq!(state.stationary_scene_tick(), None);
        }
        assert_eq!(state.stationary_scene_tick(), Some(0));
        state.input_step(1000, &[change(Player::Marge, Control::Right, false)]);
        advance_empty(&mut state, 1001..1100);
        assert_eq!(state.stationary_scene_tick(), Some(0));
        state.input_step(1100, &pulse(Player::Marge, Control::Pause));
        advance_empty(&mut state, 1101..1110);
        assert_eq!(state.stationary_scene_tick(), Some(0));
        state.input_step(1110, &pulse(Player::Marge, Control::Pause));
        assert_eq!(state.stationary_scene_tick(), Some(1));
        assert_eq!(state.camera_origin(), STATIONARY_SCENE_ORIGIN);
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
