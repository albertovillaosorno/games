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
//   - Verified protected-room airborne attack pose registration.
// - Must-Not:
//   - Advance maneuvers or decode original media.
// - Allows:
//   - Inputs: Station and delayed airborne attack phase.
//   - Outputs: Existing street or newly registered room role.
//   - Side effects: None.
// - Split-When:
//   - Independent airborne tracks need different admission policies.
// - Merge-When:
//   - A shared registration expresses the observed pose tracks.
// - Summary:
//   - Reuse exact street crops and the two additional Lisa phases.
// - Description:
//   - Select the bounded 39-tick observed sequence for each station.
// - Usage:
//   - Resolve the role against the preceding projected ground position.
// - Defaults:
//   - No unobserved pose.

use super::Role;
use crate::Player;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Source {
    Room(usize),
    Street(usize),
}

pub(super) const fn keys(player: Player) -> [Option<u8>; 5] {
    match player {
        Player::Marge => [Some(0), Some(4), Some(7), Some(10), Some(13)],
        Player::Homer => [Some(0), Some(6), None, None, None],
        Player::Bart => [Some(0), Some(8), None, None, None],
        Player::Lisa => [Some(0), Some(5), Some(10), Some(15), None],
    }
}

pub(super) const fn source(player: Player, tick: u8) -> Option<Source> {
    if tick >= 39 {
        return None;
    }
    let index = match player {
        Player::Marge => match tick {
            0..=3 => 0,
            4..=6 => 1,
            7..=9 => 2,
            10..=12 => 3,
            _ => 4,
        },
        Player::Homer => {
            if tick < 6 {
                0
            } else {
                1
            }
        },
        Player::Bart => {
            if tick < 8 {
                0
            } else {
                1
            }
        },
        Player::Lisa => match tick {
            0..=4 | 30..=34 => 0,
            5..=9 | 35..=38 => 1,
            10..=14 => 2,
            15..=19 => 3,
            20..=24 => return Some(Source::Room(0)),
            _ => return Some(Source::Room(1)),
        },
    };
    Some(Source::Street(index))
}

pub(super) const fn street_role(player: Player, index: usize) -> Option<Role> {
    let role = match (player, index) {
        (Player::Marge, 0) => {
            Role::new("actor/marge", (62, 72), (-112i32, -48i32))
        },
        (Player::Marge, 1) => {
            Role::new("actor/marge", (40, 64), (-88i32, -40i32))
        },
        (Player::Marge, 2) => {
            Role::new("actor/marge", (38, 62), (-88i32, -38i32))
        },
        (Player::Marge, 3) => {
            Role::new("actor/marge", (65, 62), (-86i32, -38i32))
        },
        (Player::Marge, 4) => {
            Role::new("actor/marge", (81, 63), (-91i32, -39i32))
        },
        (Player::Homer, 0) => {
            Role::new("actor/homer", (63, 39), (-96i32, -15i32))
        },
        (Player::Homer, 1) => {
            Role::new("actor/homer", (54, 45), (-89i32, -8i32))
        },
        (Player::Bart, 0) => {
            Role::new("actor/bart", (48, 38), (-91i32, -14i32))
        },
        (Player::Bart, 1) => Role::new("actor/bart", (46, 36), (-85i32, -6i32)),
        (Player::Lisa, 0) => {
            Role::new("actor/lisa", (32, 53), (-80i32, -35i32))
        },
        (Player::Lisa, 1) => {
            Role::new("actor/lisa", (38, 54), (-75i32, -37i32))
        },
        (Player::Lisa, 2) => {
            Role::new("actor/lisa", (35, 49), (-75i32, -33i32))
        },
        (Player::Lisa, 3) => {
            Role::new("actor/lisa", (29, 53), (-80i32, -34i32))
        },
        _ => return None,
    };
    Some(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Track = &'static [(u8, Source)];

    #[test]
    fn every_observed_air_tick_selects_the_exact_registered_family() {
        let tracks: [Track; 4] = [
            &[
                (0, Source::Street(0)),
                (4, Source::Street(1)),
                (7, Source::Street(2)),
                (10, Source::Street(3)),
                (13, Source::Street(4)),
            ],
            &[(0, Source::Street(0)), (6, Source::Street(1))],
            &[(0, Source::Street(0)), (8, Source::Street(1))],
            &[
                (0, Source::Street(0)),
                (5, Source::Street(1)),
                (10, Source::Street(2)),
                (15, Source::Street(3)),
                (20, Source::Room(0)),
                (25, Source::Room(1)),
                (30, Source::Street(0)),
                (35, Source::Street(1)),
            ],
        ];
        for (player, track) in Player::ALL.into_iter().zip(tracks) {
            assert_track(player, track);
        }
    }

    fn assert_track(player: Player, track: &[(u8, Source)]) {
        for tick in 0u8..39 {
            let expected = track
                .iter()
                .rev()
                .find(|(start, _)| *start <= tick)
                .map(|(_, role)| *role);
            assert_eq!(source(player, tick), expected);
            if let Some(Source::Street(index)) = expected {
                assert!(street_role(player, index).is_some());
                assert!(keys(player).get(index).is_some_and(Option::is_some));
            }
        }
        assert_eq!(source(player, 39), None);
    }
}
