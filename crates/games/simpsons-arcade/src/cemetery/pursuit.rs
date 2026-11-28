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
//   - Bounded first-room approach, alignment, strikes and leap precursor.
// - Must-Not:
//   - Admit contact damage, decode source media or render sprites.
// - Allows:
//   - Inputs: Fixed-step station state and shared room projection.
//   - Outputs: Two persistent enemy positions, selected targets and body poses.
//   - Side effects: Deterministic in-memory progression only.
// - Split-When:
//   - Unprotected combat or terrain policies need separate ownership.
// - Merge-When:
//   - Another encounter implements equivalent target reservation rules.
// - Summary:
//   - Model observed pursuit and bounded enemy attack poses.
// - Description:
//   - Track fractional motion, staggered steering and periodic target
//     balancing, stopping at the first unsupported transition.
// - Usage:
//   - Drive once per admitted room tick, before station movement.
// - Defaults:
//   - No normal campaign admission or unverified enemy combat.

//! Bounded first-room enemy pursuit.

use games::animation::cycle_tick;

use crate::{Facing, Maneuver, Player, Point, Slot, nearest_fighting_station};

const UNIT: i32 = 256;
const APPROACH_SPEED: i32 = 320;
const ALIGNMENT_SPEED: i32 = 280;
const APPROACH_DISTANCE: u32 = 64;
const ATTACK_DISTANCE: u32 = 32;
const LANE_DISTANCE: u32 = 3;
const TARGET_INTERVAL: u8 = 16;
const STEERING_INTERVAL: u8 = 8;
const ANIMATION_INTERVAL: u8 = 6;
const ANIMATION_DURATION: u8 = 48;
const LEVEL_FACING_DISTANCE: u32 = 16;

/// Depth component of the sampled enemy body orientation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CemeteryEnemyDepth {
    /// Body faces toward decreasing world depth.
    Decreasing,
    /// Body faces toward increasing world depth.
    Increasing,
    /// Body faces along the horizontal axis.
    Level,
}

/// Latched enemy body identity, independent of simulation motion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CemeteryEnemyPose {
    /// One of eight six-tick pursuit or alignment keyframes.
    Approach {
        /// Depth orientation at the keyframe boundary.
        depth: CemeteryEnemyDepth,
        /// Horizontal orientation at the same boundary.
        facing: Facing,
        /// Zero-based approach frame.
        phase: u8,
    },
    /// Left-facing first airborne body.
    Leap,
    /// Left-facing strike, with no contact damage admitted.
    Strike {
        /// Horizontal orientation at the start of the strike.
        facing: Facing,
        /// Zero-based strike frame.
        phase: u8,
    },
    /// Left-facing leap wind-up body.
    Windup,
}

impl CemeteryEnemyPose {
    const fn directed(dx: i32, dy: i32, phase: u8) -> Self {
        let depth = if dy.unsigned_abs() < LEVEL_FACING_DISTANCE {
            CemeteryEnemyDepth::Level
        } else if dy.is_negative() {
            CemeteryEnemyDepth::Decreasing
        } else {
            CemeteryEnemyDepth::Increasing
        };
        Self::Approach {
            depth,
            facing: if dx < 0 {
                Facing::Left
            } else {
                Facing::Right
            },
            phase,
        }
    }
}

/// One enemy's camera-relative position and currently selected station.
///
/// The bounded first-room diagnostic does not include contact response,
/// terrain changes or the subsequent encounter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CemeteryEnemyApproach {
    /// Whole-pixel elevation above the ground depth.
    pub elevation: i16,
    /// Latched body art identity, absent before both bodies initialize.
    pub pose: Option<CemeteryEnemyPose>,
    /// Floored logical position at the sampled camera offset.
    pub position: Point,
    /// Linked station in cabinet order.
    pub target: Player,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PursuitMotion {
    AlignDepth,
    AlignHorizontal,
    Leap { age: u8 },
    LongRange,
    Retreat { primed: bool },
    Strike { tick: u8 },
    Windup { tick: u8 },
}

// Both actors share the stage choice byte and the once-per-step word.
// The caller must supply a verified protected-room checkpoint word;
// this is not a campaign-wide seed or an independent per-actor RNG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChoiceStream {
    choice: u8,
    word: u16,
}

impl ChoiceStream {
    const fn advance(&mut self) {
        self.word = self.word.wrapping_mul(5).wrapping_add(0x3711);
    }

    const fn draw(&mut self) -> u8 {
        let [high, _] = self.word.to_be_bytes();
        self.choice = self.choice.wrapping_add(high);
        self.choice
    }

    // The verified first profile accepts only the near, level,
    // left-facing strike. Other admitted weighted outcomes terminate
    // the diagnostic instead of substituting this strike.
    fn first_strike(&mut self, dx: i32, dy: i32) -> Option<bool> {
        if dx >= 0i32 || dy.unsigned_abs() > 4 {
            return Some(false);
        }
        let distance = dx.unsigned_abs();
        let threshold = match distance {
            1..=31 => 154,
            32..=40 => 77,
            48..=56 | 64..=72 => 26,
            _ => return Some(false),
        };
        if self.draw() >= threshold {
            return Some(false);
        }
        let weighted = self.draw();
        (distance < ATTACK_DISTANCE && weighted < 179).then_some(true)
    }

    const fn new(word: u16) -> Self {
        Self { choice: 0, word }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pursuer {
    animation_tick: u8,
    motion: PursuitMotion,
    pose: Option<CemeteryEnemyPose>,
    position: (i32, i32),
    steering: u8,
    target: Player,
    velocity: (i32, i32),
}

impl Pursuer {
    fn advance_pose(&mut self, dx: i32, dy: i32) -> Option<()> {
        self.animation_tick = cycle_tick(
            u64::from(self.animation_tick).saturating_add(1),
            ANIMATION_DURATION,
        )?;
        if self.animation_tick.checked_rem(ANIMATION_INTERVAL) == Some(0) {
            self.pose = Some(CemeteryEnemyPose::directed(
                dx,
                dy,
                self.animation_tick.checked_div(ANIMATION_INTERVAL)?,
            ));
        }
        Some(())
    }

    const fn align(&mut self, dx: i32, dy: i32) -> Option<()> {
        // The next selector may choose an attack. Do not invent its
        // selection, or extrapolate right-facing/other depth families.
        if dx >= 0i32
            || dx.unsigned_abs() < ATTACK_DISTANCE
            || dy.unsigned_abs() >= LEVEL_FACING_DISTANCE
        {
            return None;
        }
        match self.motion {
            PursuitMotion::AlignDepth => {
                self.velocity =
                    (0i32, ALIGNMENT_SPEED.saturating_mul(dy.signum()));
                if dy.unsigned_abs() < LANE_DISTANCE {
                    self.motion = PursuitMotion::AlignHorizontal;
                }
            },
            PursuitMotion::AlignHorizontal => {
                self.velocity =
                    (ALIGNMENT_SPEED.saturating_mul(dx.signum()), 0i32);
                if dy.unsigned_abs() >= LANE_DISTANCE {
                    self.motion = PursuitMotion::AlignDepth;
                }
            },
            PursuitMotion::LongRange
            | PursuitMotion::Retreat { .. }
            | PursuitMotion::Strike { .. }
            | PursuitMotion::Windup { .. }
            | PursuitMotion::Leap { .. } => {
                return None;
            },
        }
        Some(())
    }

    const fn begin_alignment(
        &mut self,
        dx: i32,
        dy: i32,
        enabled: bool,
    ) -> Option<()> {
        // Only the observed leftward, decreasing-depth admission is
        // registered. Preserve the preceding stride on entry.
        if !enabled
            || dx >= 0i32
            || dy >= 0i32
            || dy.unsigned_abs() >= LEVEL_FACING_DISTANCE
        {
            return None;
        }
        self.motion = PursuitMotion::AlignDepth;
        self.animation_tick = 0;
        self.pose = Some(CemeteryEnemyPose::Approach {
            depth: CemeteryEnemyDepth::Decreasing,
            facing: Facing::Left,
            phase: 0,
        });
        Some(())
    }

    fn begin_strike(
        &mut self,
        dx: i32,
        dy: i32,
        motion: PursuitMotion,
    ) -> Option<()> {
        self.advance_pose(dx, dy)?;
        self.motion = motion;
        self.velocity = (0i32, 0i32);
        Some(())
    }

    fn long_range(&mut self, dx: i32, dy: i32) -> Option<()> {
        self.advance_pose(dx, dy)?;
        self.steering = self.steering.saturating_sub(1);
        if self.steering == 0 {
            self.velocity = approach_velocity(
                dx,
                dy,
                matches!(self.motion, PursuitMotion::Retreat { .. }),
            )?;
            self.steering = STEERING_INTERVAL;
        }
        Some(())
    }

    const fn new(position: (i32, i32), steering: u8) -> Self {
        Self {
            animation_tick: 0,
            motion: PursuitMotion::LongRange,
            pose: None,
            position,
            steering,
            target: Player::Marge,
            velocity: (0, 0),
        }
    }

    fn point(self) -> Point {
        let (horizontal, depth) = self.position;
        Point {
            x: i16::try_from(horizontal.div_euclid(UNIT)).unwrap_or_default(),
            y: i16::try_from(depth.div_euclid(UNIT)).unwrap_or_default(),
        }
    }

    // After the strike, only the observed near, leftward alignment
    // decision has an admitted follow-up. Retain the stopped velocity.
    fn recover_strike(
        &mut self,
        dx: i32,
        dy: i32,
        stream: &mut ChoiceStream,
    ) -> Option<()> {
        if dx < 0i32
            && dx.unsigned_abs() >= APPROACH_DISTANCE
            && dy.unsigned_abs() >= LEVEL_FACING_DISTANCE
            && dy.is_positive()
        {
            if stream.draw() == u8::MAX {
                return None;
            }
            self.motion = PursuitMotion::Retreat { primed: false };
            self.animation_tick = 0;
            self.steering = 1;
            self.pose = Some(CemeteryEnemyPose::directed(dx, dy, 0));
            return Some(());
        }
        if dx >= 0i32
            || dx.unsigned_abs() >= ATTACK_DISTANCE
            || !(-4i32..0i32).contains(&dy)
            || stream.draw() == u8::MAX
        {
            return None;
        }
        self.begin_alignment(dx, dy, true)
    }

    fn retreat_step(
        &mut self,
        (dx, dy): (i32, i32),
        primed: bool,
        choice: Option<&mut ChoiceStream>,
    ) -> Option<()> {
        if !primed {
            if choice?.draw() >= 230 {
                return None;
            }
            self.motion = PursuitMotion::Retreat { primed: true };
            return Some(());
        }
        if dx >= 0i32
            || !dy.is_positive()
            || dy.unsigned_abs() < LEVEL_FACING_DISTANCE
        {
            return None;
        }
        self.long_range(dx, dy)
    }

    fn sample(self, depth_offset: i16) -> CemeteryEnemyApproach {
        let point = self.point();
        CemeteryEnemyApproach {
            elevation: match self.motion {
                PursuitMotion::Leap { age } => leap_elevation(age),
                PursuitMotion::AlignDepth
                | PursuitMotion::AlignHorizontal
                | PursuitMotion::LongRange
                | PursuitMotion::Retreat { .. }
                | PursuitMotion::Strike { .. }
                | PursuitMotion::Windup { .. } => 0,
            },
            pose: self.pose,
            position: Point {
                y: point.y.saturating_add(depth_offset),
                ..point
            },
            target: self.target,
        }
    }

    fn steer(
        &mut self,
        slots: &[Slot; 4],
        alignment: bool,
        choice: Option<&mut ChoiceStream>,
    ) -> Option<()> {
        let (dx, dy) = self.target_delta(slots)?;
        match self.motion {
            PursuitMotion::LongRange
                if dx.unsigned_abs() < APPROACH_DISTANCE =>
            {
                self.begin_alignment(dx, dy, alignment)?;
                if let Some(stream) = choice {
                    // All weighted outcomes in the admitted first
                    // profile's medium-range row choose alignment.
                    let _draw = stream.draw();
                }
            },
            PursuitMotion::LongRange => self.long_range(dx, dy)?,
            PursuitMotion::AlignDepth | PursuitMotion::AlignHorizontal => {
                if let Some(stream) = choice
                    && stream.first_strike(dx, dy)?
                {
                    return self.begin_strike(dx, dy, PursuitMotion::Strike {
                        tick: 0,
                    });
                }
                self.align(dx, dy)?;
                self.advance_pose(dx, dy)?;
            },
            PursuitMotion::Retreat { primed } => {
                self.retreat_step((dx, dy), primed, choice)?;
            },
            PursuitMotion::Strike { tick } => {
                self.strike_step(tick, (dx, dy), choice)?;
            },
            PursuitMotion::Windup { tick } => self.windup_step(tick),
            PursuitMotion::Leap { age } => {
                if age >= 3 {
                    return None;
                }
                self.motion = PursuitMotion::Leap {
                    age: age.saturating_add(1),
                };
            },
        }
        let (horizontal, depth) = self.position;
        let (vx, vy) = self.velocity;
        self.position =
            (horizontal.saturating_add(vx), depth.saturating_add(vy));
        Some(())
    }

    fn strike_step(
        &mut self,
        tick: u8,
        (dx, dy): (i32, i32),
        choice: Option<&mut ChoiceStream>,
    ) -> Option<()> {
        if let Some(phase) = strike_phase(tick) {
            self.pose = Some(CemeteryEnemyPose::Strike {
                facing: Facing::Left,
                phase,
            });
            self.motion = PursuitMotion::Strike {
                tick: tick.saturating_add(1),
            };
            Some(())
        } else {
            self.recover_strike(dx, dy, choice?)
        }
    }

    fn target_delta(self, slots: &[Slot; 4]) -> Option<(i32, i32)> {
        let current = self.point();
        let target = slots.get(self.target.index())?.station.position;
        Some((
            i32::from(target.x).saturating_sub(i32::from(current.x)),
            i32::from(target.y).saturating_sub(i32::from(current.y)),
        ))
    }

    // The distant left-facing profile evaluates its previous target
    // before the periodic target refresh. The verified weighted row
    // selects a wind-up; other accepted outcomes remain unsupported.
    fn try_windup(
        &mut self,
        slots: &[Slot; 4],
        stream: &mut ChoiceStream,
    ) -> Option<bool> {
        if self.motion != PursuitMotion::LongRange {
            return Some(false);
        }
        let (dx, dy) = self.target_delta(slots)?;
        if dx >= 0i32 || dy.unsigned_abs() > 4 {
            return Some(false);
        }
        match dx.unsigned_abs() {
            78..=100 => {
                if stream.draw() >= 26 {
                    return Some(false);
                }
                if !(77..179).contains(&stream.draw()) {
                    return None;
                }
                self.begin_strike(dx, dy, PursuitMotion::Windup { tick: 0 })?;
                Some(true)
            },
            0..=77 => None,
            _ => Some(false),
        }
    }

    const fn windup_step(&mut self, tick: u8) {
        if tick >= 11 {
            self.pose = Some(CemeteryEnemyPose::Leap);
            self.velocity = (-512i32, 0i32);
            self.motion = PursuitMotion::Leap { age: 0 };
        } else {
            self.pose = Some(if tick < 4 {
                CemeteryEnemyPose::Strike {
                    facing: Facing::Left,
                    phase: 7,
                }
            } else {
                CemeteryEnemyPose::Windup
            });
            self.motion = PursuitMotion::Windup {
                tick: tick.saturating_add(1),
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PursuitPair {
    choice: Option<ChoiceStream>,
    enemies: [Pursuer; 2],
    reservations: [u8; 4],
    target_timer: u8,
}

impl PursuitPair {
    // Until the first refresh both links refer to the first station,
    // without an assigned reservation. The refresh is sequential.
    pub(super) fn advance(
        &mut self,
        slots: &[Slot; 4],
        alignment: bool,
    ) -> Option<()> {
        if slots.iter().any(|slot| {
            !slot.station.can_fight() || slot.station.maneuver != Maneuver::Idle
        }) {
            return None;
        }
        self.target_timer = self.target_timer.saturating_sub(1);
        let refresh = self.target_timer == 0;
        if refresh {
            self.target_timer = TARGET_INTERVAL;
        }
        for (index, enemy) in self.enemies.iter_mut().enumerate() {
            if index == 1
                && let Some(stream) = &mut self.choice
                && enemy.try_windup(slots, stream)?
            {
                continue;
            }
            if refresh
                && !matches!(
                    enemy.motion,
                    PursuitMotion::Strike { .. }
                        | PursuitMotion::Windup { .. }
                        | PursuitMotion::Leap { .. }
                )
            {
                Self::retarget(enemy, &mut self.reservations, slots)?;
            }
            // The second profile has its own distant selector and
            // does not share the first profile's close-range rules.
            let enabled = alignment && (index == 0 || self.choice.is_none());
            let choice = if index == 0 {
                self.choice.as_mut()
            } else {
                None
            };
            enemy.steer(slots, enabled, choice)?;
        }
        if let Some(stream) = &mut self.choice {
            stream.advance();
        }
        Some(())
    }

    pub(super) fn begin_animation(&mut self) -> Option<()> {
        if let Some(stream) = &mut self.choice {
            // At this checkpoint the two initial decisions precede
            // the next word advance. Both must admit long-range motion.
            if stream.draw() >= 230 || stream.draw() >= 230 {
                return None;
            }
            stream.advance();
        }
        for enemy in &mut self.enemies {
            enemy.pose = Some(CemeteryEnemyPose::Approach {
                depth: CemeteryEnemyDepth::Increasing,
                facing: Facing::Left,
                phase: 0,
            });
        }
        Some(())
    }

    pub(super) const fn new(seed: Option<u16>) -> Self {
        Self {
            choice: match seed {
                Some(word) => Some(ChoiceStream::new(word)),
                None => None,
            },
            enemies: [
                Pursuer::new((98_304, 32_768), 1),
                Pursuer::new((100_352, 45_056), 2),
            ],
            reservations: [0; 4],
            target_timer: TARGET_INTERVAL.saturating_sub(1),
        }
    }

    fn retarget(
        enemy: &mut Pursuer,
        reservations: &mut [u8; 4],
        slots: &[Slot; 4],
    ) -> Option<()> {
        let previous_target = enemy.target;
        let previous = reservations.get_mut(enemy.target.index())?;
        *previous = previous.saturating_sub(1);
        let (nearest, _) = nearest_fighting_station(enemy.point(), slots)?;
        enemy.target = if *reservations.get(nearest.index())? == 0 {
            nearest
        } else {
            Player::ALL
                .into_iter()
                .zip(*reservations)
                .min_by_key(|&(_, count)| count)?
                .0
        };
        if !matches!(
            enemy.motion,
            PursuitMotion::LongRange | PursuitMotion::Retreat { .. }
        ) && enemy.target != previous_target
        {
            return None;
        }
        let selected = reservations.get_mut(enemy.target.index())?;
        *selected = selected.saturating_add(1);
        Some(())
    }

    pub(super) fn sample(
        self,
        depth_offset: i16,
    ) -> [CemeteryEnemyApproach; 2] {
        self.enemies.map(|enemy| enemy.sample(depth_offset))
    }
}

// For the verified horizontal band, a six-bit minor/major ratio chooses
// a quantized depth component. The main axis remains at full speed.
// Reduce a wide denominator before computing the ratio to retain the
// observed truncation near bin boundaries.
fn approach_velocity(dx: i32, dy: i32, retreat: bool) -> Option<(i32, i32)> {
    let mut major = dx.unsigned_abs();
    let mut minor = dy.unsigned_abs().saturating_mul(64);
    while major > 255 {
        major >>= 1u32;
        minor >>= 1u32;
    }
    let ratio = minor.checked_div(major)?;
    if retreat {
        return retreat_velocity(ratio);
    }
    let depth: i32 = match ratio {
        0 => 0,
        1..=5 => 16,
        6 => 32,
        7..=11 => 48,
        12..=14 => 64,
        _ => return None,
    };
    Some((
        APPROACH_SPEED.saturating_mul(dx.signum()),
        depth.saturating_mul(dy.signum()),
    ))
}

// Integrate the launch velocity and quarter-pixel acceleration in Q8.
// The age counts complete steps after the first airborne sample.
fn leap_elevation(age: u8) -> i16 {
    let elapsed = i32::from(age).saturating_add(1);
    let elevation = elapsed.saturating_mul(1024).saturating_sub(
        elapsed
            .saturating_mul(elapsed.saturating_sub(1))
            .saturating_mul(32),
    );
    i16::try_from(elevation.div_euclid(UNIT)).unwrap_or_default()
}

// Diagonal bins independently observed at the four steering updates.
// Other bins remain unsupported rather than extrapolating a speed table.
const fn retreat_velocity(ratio: u32) -> Option<(i32, i32)> {
    match ratio {
        40 => Some((-272, 176)),
        33 => Some((-288, 144)),
        27 | 28 => Some((-288, 128)),
        _ => None,
    }
}

const fn strike_phase(tick: u8) -> Option<u8> {
    match tick {
        0..=2 => Some(0),
        3..=5 => Some(1),
        6..=8 => Some(2),
        9..=11 => Some(3),
        12..=15 => Some(4),
        16..=20 => Some(5),
        21..=26 => Some(6),
        27..=31 => Some(7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ChoiceStream, PursuitPair};

    #[test]
    fn choice_word_recurrence_matches_every_word() {
        for word in 0..=u16::MAX {
            let mut stream = ChoiceStream { choice: 0x5a, word };
            let expected =
                u32::from(word).saturating_mul(5).saturating_add(0x3711)
                    & 0xffff;
            stream.advance();
            assert_eq!(u32::from(stream.word), expected);
            assert_eq!(stream.choice, 0x5a);
        }
    }

    #[test]
    fn choice_byte_recurrence_matches_every_pair() {
        for pair in 0..=u16::MAX {
            let [choice, high] = pair.to_be_bytes();
            let expected =
                u16::from(choice).saturating_add(u16::from(high)) & 0xff;
            for low in [0, u8::MAX] {
                let word = u16::from_be_bytes([high, low]);
                let mut stream = ChoiceStream { choice, word };
                assert_eq!(u16::from(stream.draw()), expected);
                assert_eq!(u16::from(stream.choice), expected);
                assert_eq!(stream.word, word);
            }
        }
    }

    #[test]
    fn room_admission_shares_a_word_across_both_choices() {
        for (word, first, second, next) in
            [(0x1207, 0x12, 0x24, 0x9134), (0x4a25, 0x4a, 0x94, 0xa9ca)]
        {
            let mut stream = ChoiceStream::new(word);
            assert_eq!(stream.draw(), first);
            assert_eq!(stream.draw(), second);
            stream.advance();
            assert_eq!(stream.word, next);
            assert_eq!(stream.choice, second);
        }
    }

    #[test]
    fn unsupported_initial_weighted_choice_does_not_admit_pursuit() {
        let mut pair = PursuitPair::new(Some(0xe600));
        assert_eq!(pair.begin_animation(), None);
        let mut second_rejected = PursuitPair::new(Some(0x7300));
        assert_eq!(second_rejected.begin_animation(), None);
    }

    #[test]
    fn near_strike_requires_both_observed_decisions() {
        for (word, choice, dx, dy, result, final_choice) in [
            (0x3700, 0xe6, -50i32, -4i32, Some(false), 0x1d),
            (0x4a00, 0x1d, -48i32, -3i32, Some(false), 0x67),
            (0x3700, 0x67, -40i32, -1i32, Some(false), 0x9e),
            (0x4a00, 0x9e, -37i32, -1i32, Some(false), 0xe8),
            (0xa900, 0xe8, -34i32, -1i32, Some(false), 0x91),
            (0x8800, 0x91, -31i32, -1i32, Some(true), 0xa1),
            (0x4000, 0x00, -31i32, 5i32, Some(false), 0x00),
            (0x4000, 0x00, 31i32, 0i32, Some(false), 0x00),
            (0x6000, 0x00, -31i32, 0i32, None, 0xc0),
        ] {
            let mut stream = ChoiceStream { choice, word };
            assert_eq!(stream.first_strike(dx, dy), result);
            assert_eq!(stream.choice, final_choice);
        }
    }
}
