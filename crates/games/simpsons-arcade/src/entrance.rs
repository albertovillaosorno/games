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
//   - Title-specific fixed-step second-stage arrival and landing motion.
//   - Released tether motion independent of the gameplay input gate.
// - Must-Not:
//   - Decode original media, admit stations or render sprites.
// - Allows:
//   - Inputs: Elapsed ticks, initial admission and character identity.
//   - Outputs: Visibility, scripted positions, height and steering timing.
//   - Side effects: Deterministic in-memory progression.
// - Split-When:
//   - Another stage needs an independent arrival policy.
// - Merge-When:
//   - A shared scene mechanism replaces the title's arrival timing.
// - Summary:
//   - Bridge the stage card and playable second-stage encounter.
// - Description:
//   - Model the observed descent and subsequent airborne landing.
// - Usage:
//   - Advance once per unpaused playing tick.
// - Defaults:
//   - No entrance outside second-stage admission.

//! Deterministic second-stage arrival and landing.

use super::{Player, Point};

const FIRST_ACTOR: u16 = 128;
const DESCENT_STEPS: u16 = 100;
const SCRIPT_END: u16 = 229;
const FIRST_BOUNCE: u16 = 230;
const LAST_LANDING: u16 = 295;
#[cfg(any(test, feature = "native-preview"))]
const RELEASE: u16 = SCRIPT_END + 3;
const MARKER_END: u16 = 383;
#[cfg(any(test, feature = "native-preview"))]
const MARKER_HEIGHTS: [i16; 32] = [
    59, 63, 67, 70, 72, 74, 75, 75, 74, 74, 73, 72, 71, 69, 68, 66, 63, 61, 58,
    55, 52, 48, 45, 41, 36, 32, 27, 22, 17, 11, 6, 0,
];
#[cfg(any(test, feature = "native-preview"))]
const TALLEST_TETHER: i16 = 79;
const UNITS_PER_PIXEL: i32 = 256;
const INITIAL_HEIGHT: i32 = 192;
const GROUND: i16 = 192;

/// Elapsed fixed-step second-stage arrival.
///
/// During the scripted interval, players cannot move the actors. The
/// following landing permits the sampled alternating directional ticks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StageEntrance {
    admitted: [bool; 4],
    tick: u16,
}

impl StageEntrance {
    pub(super) const fn advance(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    const fn at(self, tick: u16) -> Self {
        Self { tick, ..self }
    }

    fn bounce_height(self, player: Player) -> i32 {
        let steps = i32::from(
            self.tick
                .saturating_sub(FIRST_BOUNCE)
                .checked_div(2)
                .unwrap_or_default()
                .saturating_add(1),
        );
        let initial = self.at(SCRIPT_END).descent_height(player);
        let rising = steps.min(8i32);
        let falling = steps.saturating_sub(rising);
        initial
            .saturating_add(1344i32.saturating_mul(rising))
            .saturating_sub(
                96i32
                    .saturating_mul(rising)
                    .saturating_mul(rising.saturating_sub(1)),
            )
            .saturating_sub(
                32i32
                    .saturating_mul(falling)
                    .saturating_mul(falling.saturating_add(1)),
            )
    }

    pub(super) const fn complete(self) -> bool {
        self.tick >= LAST_LANDING
    }

    fn descent_height(self, player: Player) -> i32 {
        let steps =
            i32::from(self.tick.saturating_sub(FIRST_ACTOR).min(DESCENT_STEPS));
        initial_height(player)
            .saturating_sub(640i32.saturating_mul(steps))
            .saturating_add(
                3i32.saturating_mul(steps)
                    .saturating_mul(steps.saturating_sub(1)),
            )
    }

    /// Visible height above the ground, in logical pixels.
    ///
    /// Returns no height before the character's scripted admission.
    #[must_use]
    pub fn elevation(self, player: Player) -> Option<i16> {
        if !self.visible(player) {
            return None;
        }
        let height: i32 = if self.complete() {
            0
        } else if self.tick <= SCRIPT_END {
            self.descent_height(player)
        } else {
            self.bounce_height(player)
        };
        i16::try_from(height.max(0).checked_div(UNITS_PER_PIXEL)?).ok()
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn marker_elevation(self, player: Player) -> Option<i16> {
        if self.scripted_art()
            || self.presentation_complete()
            || !self.admitted.get(player.index()).copied()?
        {
            return None;
        }
        if self.complete() {
            return Some(0);
        }
        let step =
            usize::from(self.tick.saturating_sub(RELEASE).checked_div(2)?);
        let offset: i16 = match player {
            Player::Marge => 0,
            Player::Homer => -6,
            Player::Bart => 7,
            Player::Lisa => -17,
        };
        Some(MARKER_HEIGHTS.get(step)?.saturating_add(offset).max(0))
    }

    pub(super) const fn new(admitted: [bool; 4]) -> Self {
        Self { admitted, tick: 0 }
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn ordinary_body_visible(self, player: Player) -> bool {
        if !self
            .admitted
            .get(player.index())
            .copied()
            .unwrap_or_default()
            || self.scripted_art()
            || self.presentation_complete()
        {
            return true;
        }
        // The neutral replay changes cadence at landing and holds tick 311.
        // Keep this presentation independent of input and collision.
        if !self.complete() {
            self.tick & 2 != 0
        } else if self.tick < 311 {
            self.tick.is_multiple_of(2)
        } else {
            !self.tick.is_multiple_of(2)
        }
    }

    pub(super) const fn presentation_complete(self) -> bool {
        self.tick >= MARKER_END
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn presentation_sample(
        self,
        player: Player,
        delay: u16,
    ) -> Option<(Point, i16)> {
        if !self.scripted_art() {
            return None;
        }
        let tick = self.tick.saturating_sub(delay).min(SCRIPT_END);
        let source = self.at(tick);
        source
            .scripted_position(player)
            .zip(source.elevation(player))
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) const fn release_shadows_active(self) -> bool {
        !self.scripted_art() && self.tick < 324
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn released_shadow(self, player: Player) -> Option<Point> {
        if !self.release_shadows_active()
            || !self.admitted.get(player.index()).copied()?
        {
            return None;
        }
        self.at(SCRIPT_END).scripted_position(player)
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) fn released_tether(
        self,
        player: Player,
    ) -> Option<(Point, i16)> {
        if self.scripted_art()
            || self.tether_withdrawal() >= TALLEST_TETHER
            || !self.admitted.get(player.index()).copied()?
        {
            return None;
        }
        let release = self.at(SCRIPT_END);
        Some((
            release.scripted_position(player)?,
            release
                .elevation(player)?
                .saturating_add(self.tether_withdrawal()),
        ))
    }

    #[cfg(any(test, feature = "native-preview"))]
    pub(super) const fn scripted_art(self) -> bool {
        self.tick < RELEASE
    }

    pub(super) fn scripted_position(self, player: Player) -> Option<Point> {
        if !self.visible(player) || self.tick > SCRIPT_END {
            return None;
        }
        let start = match player {
            Player::Marge => -32i16,
            Player::Homer => 0,
            Player::Bart => 32,
            Player::Lisa => 64,
        };
        let steps = i16::try_from(
            self.tick.saturating_sub(FIRST_ACTOR).min(DESCENT_STEPS),
        )
        .ok()?;
        Some(Point {
            x: start.saturating_add(steps),
            y: GROUND,
        })
    }

    pub(super) const fn steering_tick(self) -> bool {
        self.tick > FIRST_BOUNCE && self.tick.is_multiple_of(2)
    }

    #[cfg(any(test, feature = "native-preview"))]
    fn tether_withdrawal(self) -> i16 {
        // The sampled ropes accelerate by 1/16 pixel per movement step.
        // Their cadence changes from alternate ticks to every tick after
        // the last landing; both captures hold one step at tick 311.
        // This is a presentation curve, not a second gameplay gate.
        let alternate = self
            .tick
            .min(LAST_LANDING - 1)
            .saturating_sub(RELEASE)
            .checked_div(2)
            .unwrap_or_default();
        let every = self.tick.saturating_sub(LAST_LANDING - 1);
        let steps = i32::from(alternate)
            .saturating_add(i32::from(every))
            .saturating_sub(i32::from(self.tick >= 311));
        let displacement = steps
            .saturating_mul(steps.saturating_add(1))
            .checked_div(32)
            .unwrap_or_default();
        i16::try_from(displacement).unwrap_or(i16::MAX)
    }

    /// Elapsed arrival ticks, including the pre-admission interval.
    #[must_use]
    pub const fn tick(self) -> u16 {
        self.tick
    }

    const fn visible(self, player: Player) -> bool {
        self.tick > FIRST_ACTOR
            || (self.tick == FIRST_ACTOR && matches!(player, Player::Marge))
    }
}

const fn initial_height(player: Player) -> i32 {
    let offset: i32 = match player {
        Player::Marge => 0,
        Player::Homer => -8,
        Player::Bart => 8,
        Player::Lisa => -16,
    };
    INITIAL_HEIGHT
        .saturating_add(offset)
        .saturating_mul(UNITS_PER_PIXEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u16) -> StageEntrance {
        StageEntrance::new([true; 4]).at(tick)
    }

    #[test]
    fn descent_admits_four_stations_and_preserves_relative_offsets() {
        let before = sample(127);
        assert_eq!(
            Player::ALL.map(|player| before.elevation(player)),
            [None; 4]
        );
        let first = sample(128);
        assert_eq!(Player::ALL.map(|player| first.elevation(player)), [
            Some(192),
            None,
            None,
            None
        ]);
        let visible = sample(129);
        assert_eq!(Player::ALL.map(|player| visible.elevation(player)), [
            Some(189),
            Some(181),
            Some(197),
            Some(173)
        ]);
        let end = sample(229);
        assert_eq!(Player::ALL.map(|player| end.elevation(player)), [
            Some(58),
            Some(50),
            Some(66),
            Some(42)
        ]);
        assert_eq!(Player::ALL.map(|player| end.scripted_position(player)), [
            Some(Point { x: 68, y: 192 }),
            Some(Point { x: 100, y: 192 }),
            Some(Point { x: 132, y: 192 }),
            Some(Point { x: 164, y: 192 }),
        ]);
        assert!(!end.steering_tick());
    }

    #[test]
    fn scripted_art_tracks_display_admission() {
        let before = sample(128);
        assert_eq!(
            Player::ALL.map(|player| before.presentation_sample(player, 1)),
            [None; 4]
        );
        let first = sample(129);
        assert_eq!(
            first.presentation_sample(Player::Marge, 1),
            Some((Point { x: -32, y: 192 }, 192))
        );
        assert_eq!(first.presentation_sample(Player::Marge, 2), None);
        assert_eq!(first.presentation_sample(Player::Homer, 1), None);
    }

    #[test]
    fn scripted_art_holds_layers_until_release() {
        let end = sample(229);
        let next = sample(230);
        let held = sample(231);
        assert_eq!(
            end.presentation_sample(Player::Marge, 1),
            Some((Point { x: 68, y: 192 }, 58))
        );
        assert_eq!(
            end.presentation_sample(Player::Marge, 2),
            Some((Point { x: 67, y: 192 }, 58))
        );
        assert_eq!(
            next.presentation_sample(Player::Marge, 1),
            end.presentation_sample(Player::Marge, 1)
        );
        assert_eq!(
            next.presentation_sample(Player::Marge, 2),
            next.presentation_sample(Player::Marge, 1)
        );
        assert_eq!(
            next.presentation_sample(Player::Marge, 2),
            held.presentation_sample(Player::Marge, 2)
        );
        let released = sample(232);
        assert!(!released.scripted_art());
        assert_eq!(
            Player::ALL.map(|player| released.presentation_sample(player, 1)),
            [None; 4]
        );
    }

    #[test]
    fn released_tethers_retain_their_origins_and_accelerate() {
        let release = sample(232);
        for (tick, displacement) in [
            (232, 0),
            (243, 0),
            (244, 1),
            (254, 4),
            (280, 18),
            (294, 31),
            (295, 33),
            (310, 70),
            (311, 70),
            (312, 73),
            (313, 76),
        ] {
            let state = sample(tick);
            for player in Player::ALL {
                let origin = release.released_tether(player);
                assert_eq!(
                    state.released_tether(player),
                    origin.map(|(point, height)| {
                        (point, height.saturating_add(displacement))
                    })
                );
            }
            assert!(!state.presentation_complete());
        }
        assert!(sample(295).complete());
        assert!(!sample(295).presentation_complete());
        assert!(!sample(314).presentation_complete());
        assert!(sample(383).presentation_complete());
        assert_eq!(sample(314).released_tether(Player::Lisa), None);
    }

    #[test]
    fn markers_follow_the_sampled_bounce_and_expire_independently() {
        assert_eq!(sample(231).marker_elevation(Player::Marge), None);
        assert_eq!(
            Player::ALL.map(|player| sample(232).marker_elevation(player)),
            [Some(59), Some(53), Some(66), Some(42)]
        );
        assert_eq!(sample(233).marker_elevation(Player::Marge), Some(59));
        assert_eq!(sample(244).marker_elevation(Player::Marge), Some(75));
        assert_eq!(sample(294).marker_elevation(Player::Bart), Some(7));
        assert_eq!(sample(295).marker_elevation(Player::Bart), Some(0));
        assert_eq!(sample(382).marker_elevation(Player::Marge), Some(0));
        assert_eq!(sample(383).marker_elevation(Player::Marge), None);
    }

    #[test]
    fn markers_belong_to_initially_admitted_characters() {
        let selective = StageEntrance::new([false, true, false, true]).at(300);
        assert_eq!(selective.marker_elevation(Player::Marge), None);
        assert_eq!(selective.marker_elevation(Player::Homer), Some(0));
    }

    #[test]
    fn ordinary_body_cadence_changes_at_landing_without_hiding_late_joins() {
        let visible = |tick| sample(tick).ordinary_body_visible(Player::Marge);
        assert_eq!((232..240).map(visible).collect::<Vec<_>>(), [
            false, false, true, true, false, false, true, true,
        ]);
        assert_eq!((292..300).map(visible).collect::<Vec<_>>(), [
            false, false, true, false, true, false, true, false,
        ]);
        assert_eq!((308..316).map(visible).collect::<Vec<_>>(), [
            true, false, true, true, false, true, false, true,
        ]);
        assert!(!visible(382));
        assert!(visible(383));
        let selective = StageEntrance::new([false, true, false, true]);
        for tick in 295..383 {
            assert!(selective.at(tick).ordinary_body_visible(Player::Marge));
            assert!(selective.at(tick).ordinary_body_visible(Player::Bart));
        }
    }

    #[test]
    fn released_shadow_lifetime_is_independent_of_clipped_tethers() {
        let selective = StageEntrance::new([true, false, true, false]);
        for player in Player::ALL {
            assert_eq!(selective.at(231).released_shadow(player), None);
            assert_eq!(selective.at(324).released_shadow(player), None);
        }
        let marge = Some(Point { x: 68, y: 192 });
        let bart = Some(Point { x: 132, y: 192 });
        for tick in 232..324 {
            let arrival = selective.at(tick);
            assert_eq!(arrival.released_shadow(Player::Marge), marge);
            assert_eq!(arrival.released_shadow(Player::Homer), None);
            assert_eq!(arrival.released_shadow(Player::Bart), bart);
            assert_eq!(arrival.released_shadow(Player::Lisa), None);
        }
        assert!(selective.at(323).released_tether(Player::Marge).is_none());
    }

    #[test]
    fn released_tethers_only_belong_to_initially_admitted_characters() {
        let released = StageEntrance::new([false, true, false, true]).at(232);
        assert_eq!(released.released_tether(Player::Marge), None);
        assert!(released.released_tether(Player::Homer).is_some());
        assert_eq!(released.released_tether(Player::Bart), None);
        assert!(released.released_tether(Player::Lisa).is_some());
    }

    #[test]
    fn bounce_holds_alternate_frames_and_reaches_apex() {
        let first = sample(230);
        assert_eq!(first.elevation(Player::Marge), Some(63));
        assert!(!first.steering_tick());
        let held = sample(231);
        assert_eq!(held.elevation(Player::Marge), Some(63));
        assert!(!held.steering_tick());
        let steer = sample(232);
        assert_eq!(steer.elevation(Player::Marge), Some(67));
        assert!(steer.steering_tick());
        let apex = sample(244);
        assert_eq!(apex.elevation(Player::Marge), Some(79));
    }

    #[test]
    fn players_land_in_character_order() {
        let lisa = sample(288);
        assert_eq!(lisa.elevation(Player::Lisa), Some(0));
        assert_eq!(lisa.elevation(Player::Marge), Some(15));
        let homer = sample(292);
        assert_eq!(homer.elevation(Player::Homer), Some(0));
        assert_eq!(homer.elevation(Player::Marge), Some(4));
        let marge = sample(294);
        assert_eq!(marge.elevation(Player::Marge), Some(0));
        assert_eq!(marge.elevation(Player::Bart), Some(5));
        let bart = sample(295);
        assert!(bart.complete());
        assert_eq!(
            Player::ALL.map(|player| bart.elevation(player)),
            [Some(0); 4]
        );
    }
}
