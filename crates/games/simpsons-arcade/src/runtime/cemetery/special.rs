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
//   - Registered special-maneuver pose tracks for the protected room.
// - Must-Not:
//   - Decode original media, allocate textures or advance gameplay.
// - Allows:
//   - Inputs: Player and zero-based maneuver tick.
//   - Outputs: Registered source identity and crop geometry.
//   - Side effects: None.
// - Split-When:
//   - Other title-specific action tracks require independent policies.
// - Merge-When:
//   - A shared registration mechanism expresses all title tracks.
// - Summary:
//   - Select the observed thirty-tick special presentation families.
// - Description:
//   - Reuse exact street sprites and select twelve additional room roles.
// - Usage:
//   - Resolve the source and draw using the projected station anchor.
// - Defaults:
//   - No source outside the observed interval.

//! Exact isolated special-maneuver pose tracks.

use super::Role;
use crate::Player;

const STREET_ROLES: [[Role; 3]; 4] = [
    [
        Role::new("actor/marge", (40, 75), (-88i32, -51i32)),
        Role::new("actor/marge", (59, 60), (-99i32, -36i32)),
        Role::new("actor/marge", (62, 42), (-86i32, -19i32)),
    ],
    [
        Role::new("actor/homer", (56, 56), (-88i32, -38i32)),
        Role::new("actor/homer", (36, 61), (-75i32, -38i32)),
        Role::new("actor/homer", (59, 52), (-88i32, -39i32)),
    ],
    [
        Role::new("actor/bart", (31, 53), (-88i32, -37i32)),
        Role::new("actor/bart", (27, 52), (-79i32, -37i32)),
        Role::new("actor/bart", (62, 52), (-71i32, -37i32)),
    ],
    [
        Role::new("actor/lisa", (39, 57), (-87i32, -33i32)),
        Role::new("actor/lisa", (54, 52), (-97i32, -28i32)),
        Role::new("actor/lisa", (55, 52), (-73i32, -28i32)),
    ],
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Source {
    Room(usize),
    Street(usize),
}

pub(super) fn source(player: Player, tick: u8) -> Option<Source> {
    if tick >= 30 {
        return None;
    }
    Some(match player {
        Player::Marge => marge(tick),
        Player::Homer => homer(tick),
        Player::Bart => bart(tick),
        Player::Lisa => lisa(tick.checked_rem(10)?),
    })
}

const fn bart(tick: u8) -> Source {
    match tick {
        ..=4 => Source::Street(0),
        5..=8 => Source::Street(1),
        9..=10 => Source::Room(4),
        11..=13 => Source::Street(2),
        14..=15 => Source::Room(5),
        16..=17 => Source::Room(6),
        _ => Source::Room(7),
    }
}

const fn homer(tick: u8) -> Source {
    match tick {
        0..=1 | 22..=23 => Source::Street(0),
        2..=3 | 20..=21 | 24..=25 => Source::Room(0),
        4..=5 | 18..=19 | 26..=27 => Source::Room(1),
        6..=7 | 16..=17 | 28..=29 => Source::Street(1),
        8..=9 | 14..=15 => Source::Room(2),
        10..=11 => Source::Room(3),
        _ => Source::Street(2),
    }
}

const fn lisa(tick: u8) -> Source {
    match tick {
        0 => Source::Street(0),
        1 => Source::Room(8),
        2..=3 => Source::Street(1),
        4..=5 => Source::Room(9),
        6..=7 => Source::Street(2),
        8 => Source::Room(10),
        _ => Source::Room(11),
    }
}

const fn marge(tick: u8) -> Source {
    match tick {
        ..=2 => Source::Street(0),
        3..=4 => Source::Street(1),
        _ => Source::Street(2),
    }
}

pub(super) const fn street_keys(player: Player) -> [u8; 3] {
    match player {
        Player::Marge => [0, 3, 5],
        Player::Homer => [0, 6, 12],
        Player::Bart => [0, 5, 11],
        Player::Lisa => [0, 2, 6],
    }
}

pub(super) fn street_role(player: Player, index: usize) -> Option<Role> {
    STREET_ROLES.get(player.index())?.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_index(value: Source) -> usize {
        match value {
            Source::Street(index) => index,
            Source::Room(index) => index.saturating_add(10),
        }
    }

    #[test]
    fn all_four_tracks_preserve_the_independent_tick_samples() {
        let expected: [[usize; 30]; 4] = [
            [
                0, 0, 0, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
                2, 2, 2, 2, 2, 2, 2, 2, 2,
            ],
            [
                0, 0, 10, 10, 11, 11, 1, 1, 12, 12, 13, 13, 2, 2, 12, 12, 1, 1,
                11, 11, 10, 10, 0, 0, 10, 10, 11, 11, 1, 1,
            ],
            [
                0, 0, 0, 0, 0, 1, 1, 1, 1, 14, 14, 2, 2, 2, 15, 15, 16, 16, 17,
                17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
            ],
            [
                0, 18, 1, 1, 19, 19, 2, 2, 20, 21, 0, 18, 1, 1, 19, 19, 2, 2,
                20, 21, 0, 18, 1, 1, 19, 19, 2, 2, 20, 21,
            ],
        ];
        for (player, track) in Player::ALL.into_iter().zip(&expected) {
            for (tick, &phase) in (0u8..30).zip(track) {
                assert_eq!(source(player, tick).map(source_index), Some(phase));
            }
            assert_eq!(source(player, 30), None);
        }
    }
}
