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
//   - Registered ground-action art tracks for the protected room.
// - Must-Not:
//   - Decode original media, allocate textures or advance gameplay.
// - Allows:
//   - Inputs: Player and delayed ground-action sample.
//   - Outputs: Registered source identity.
//   - Side effects: None.
// - Split-When:
//   - Other room action tracks require distinct policies.
// - Merge-When:
//   - Shared registration expresses all title action tracks.
// - Summary:
//   - Select observed pre-action, active and terminal art.
// - Description:
//   - Reuse the room idle roles and Bart's exact descending jump crop.
// - Usage:
//   - Resolve the source before drawing against the projected anchor.
// - Defaults:
//   - No source outside the observed interval.

//! Exact isolated ground-action pose tracks.

use crate::{CemeteryGroundAttack, Player};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Source {
    BartDescent,
    Idle,
    Room(usize),
}

pub(super) const fn source(
    player: Player,
    sample: CemeteryGroundAttack,
) -> Option<Source> {
    match sample.tick {
        18 if matches!(player, Player::Bart) && !sample.moving => {
            return Some(Source::BartDescent);
        },
        0 | 18 => return Some(Source::Idle),
        1..=17 => (),
        _ => return None,
    }
    let tick = sample.tick;
    Some(Source::Room(if sample.moving {
        moving(player, tick)
    } else {
        neutral(player, tick)
    }))
}

const fn moving(player: Player, tick: u8) -> usize {
    match player {
        Player::Marge => match tick {
            ..=4 => 20,
            8..=12 => 22,
            _ => 21,
        },
        Player::Homer => match tick {
            ..=3 => 23,
            4..=7 => 24,
            8..=12 => 25,
            _ => 26,
        },
        Player::Bart => match tick {
            ..=4 => 27,
            5..=7 => 28,
            8..=12 => 29,
            _ => 30,
        },
        Player::Lisa => match tick {
            ..=4 => 31,
            5..=7 => 32,
            _ => 33,
        },
    }
}

const fn neutral(player: Player, tick: u8) -> usize {
    match player {
        Player::Marge => match tick {
            ..=3 => 0,
            4..=5 => 1,
            6..=7 => 2,
            8..=12 => 3,
            _ => 4,
        },
        Player::Homer => match tick {
            ..=2 => 5,
            3..=4 => 6,
            5..=7 => 7,
            8..=9 => 8,
            10..=11 => 9,
            12..=13 => 10,
            14..=15 => 11,
            _ => 12,
        },
        Player::Bart => match tick {
            ..=3 => 13,
            4..=7 => 14,
            _ => 15,
        },
        Player::Lisa => match tick {
            ..=4 => 16,
            5..=7 => 17,
            8..=12 => 18,
            _ => 19,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_neutral_and_moving_samples_select_registered_roles() {
        let neutral = [
            [0, 0, 0, 1, 1, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4],
            [5, 5, 6, 6, 7, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12],
            [
                13, 13, 13, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15,
                15,
            ],
            [
                16, 16, 16, 16, 17, 17, 17, 18, 18, 18, 18, 18, 19, 19, 19, 19,
                19,
            ],
        ];
        let moving = [
            [
                20, 20, 20, 20, 21, 21, 21, 22, 22, 22, 22, 22, 21, 21, 21, 21,
                21,
            ],
            [
                23, 23, 23, 24, 24, 24, 24, 25, 25, 25, 25, 25, 26, 26, 26, 26,
                26,
            ],
            [
                27, 27, 27, 27, 28, 28, 28, 29, 29, 29, 29, 29, 30, 30, 30, 30,
                30,
            ],
            [
                31, 31, 31, 31, 32, 32, 32, 33, 33, 33, 33, 33, 33, 33, 33, 33,
                33,
            ],
        ];
        for (player, (neutral_track, moving_track)) in
            Player::ALL.into_iter().zip(neutral.into_iter().zip(moving))
        {
            assert_track(player, false, &neutral_track);
            assert_track(player, true, &moving_track);
        }
    }

    fn assert_track(player: Player, moving: bool, track: &[usize; 17]) {
        let at = |tick| source(player, CemeteryGroundAttack { moving, tick });
        assert_eq!(at(0), Some(Source::Idle));
        for (tick, role) in (1u8..=17).zip(track.iter().copied()) {
            assert_eq!(at(tick), Some(Source::Room(role)));
        }
        let terminal = if player == Player::Bart && !moving {
            Source::BartDescent
        } else {
            Source::Idle
        };
        assert_eq!(at(18), Some(terminal));
        assert_eq!(at(19), None);
    }
}
