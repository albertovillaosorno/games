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
//   - Verified automatic competitor art and scene registration.
// - Must-Not:
//   - Advance gameplay, choose competition scores or decode source media.
// - Allows:
//   - Inputs: Canonical bundles, station and read-only bonus state.
//   - Outputs: Bounded scene draws.
//   - Side effects: Scene-local atlas allocation during admission.
// - Split-When:
//   - An independent competitor presentation needs separate resources.
// - Merge-When:
//   - A shared bonus presentation contract can own these roles.
// - Summary:
//   - Show automated competitors that affect the playable balloon bonus.
// - Description:
//   - Sampled artwork and scaling are provisional between observations.
// - Usage:
//   - Select all nine locally imported bundles together.
// - Defaults:
//   - No automatic competitor art without explicit selection.

//! Optional automatic competitor presentation.

use games_assets::ContentHash;
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::BundleSource;

use super::{Art, SpriteSpec, failure, open_bundle};
use crate::{AutomaticDeparture, Bonus, Player};

pub(in crate::runtime) const BUNDLE_COUNT: usize = 9;
pub(in crate::runtime) const ATLAS_COUNT: usize = 14;
pub(in crate::runtime) const ATLAS_BYTES: usize = 210_836;

// Captured positions use the second station as the registration base.
// The third station's hanging crop is translated by its 64-pixel offset.
const NEUTRAL: SpriteSpec = SpriteSpec {
    height: 30,
    key: "bonus/opponent-neutral",
    width: 27,
    x: 109,
    y: 132,
};
const PUMP: SpriteSpec = SpriteSpec {
    height: 48,
    key: "bonus/opponent-pump",
    width: 5,
    x: 123,
    y: 152,
};
const BODY: SpriteSpec = SpriteSpec {
    height: 64,
    key: "bonus/opponent-body",
    width: 32,
    x: 128,
    y: 136,
};
const PRESSED_PUMP: SpriteSpec = SpriteSpec {
    height: 48,
    key: "bonus/opponent-pressed-pump",
    width: 6,
    x: 122,
    y: 152,
};
const PRESSED_BODY: SpriteSpec = SpriteSpec {
    height: 58,
    key: "bonus/opponent-pressed-body",
    width: 32,
    x: 128,
    y: 142,
};
const INFLATED: SpriteSpec = SpriteSpec {
    height: 176,
    key: "bonus/opponent-inflated",
    width: 80,
    x: 76,
    y: -16,
};
const HANGING: SpriteSpec = SpriteSpec {
    height: 96,
    key: "bonus/opponent-hanging",
    width: 32,
    x: 130,
    y: 102,
};
const BURST_START: SpriteSpec = SpriteSpec {
    height: 22,
    key: "bonus/opponent-burst-start",
    width: 11,
    x: 119,
    y: -21,
};
const FALL_START: SpriteSpec = SpriteSpec {
    height: 55,
    key: "bonus/opponent-fall-start",
    width: 54,
    x: 120,
    y: 145,
};
const BURST_MIDDLE: SpriteSpec = SpriteSpec {
    height: 124,
    key: "bonus/opponent-burst-middle",
    width: 72,
    x: 84,
    y: -91,
};
const FALL_TURN: SpriteSpec = SpriteSpec {
    key: "bonus/opponent-fall-turn",
    ..FALL_START
};
const BURST_LATE: SpriteSpec = SpriteSpec {
    height: 134,
    key: "bonus/opponent-burst-late",
    width: 70,
    x: 84,
    y: -81,
};
const LANDING: SpriteSpec = SpriteSpec {
    height: 63,
    key: "bonus/opponent-landing",
    width: 63,
    x: 99,
    y: 137,
};
const LANDED: SpriteSpec = SpriteSpec {
    height: 29,
    key: "bonus/opponent-landed",
    width: 64,
    x: 90,
    y: 170,
};

pub(in crate::runtime) type Digests = [ContentHash; BUNDLE_COUNT];

#[derive(Debug)]
pub(super) struct OpponentArt {
    body: Art,
    burst_late: Art,
    burst_middle: Art,
    burst_start: Art,
    fall_start: Art,
    fall_turn: Art,
    hanging: Art,
    inflated: Art,
    landed: Art,
    landing: Art,
    neutral: Art,
    pressed_body: Art,
    pressed_pump: Art,
    pump: Art,
}

impl OpponentArt {
    fn balloon(
        &self,
        bonus: Bonus,
        (player, rise): (Player, u32),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let axes = bonus
            .visual_axes(player)
            .ok_or(RenderError::InvalidSprite)?;
        let (art, destination) = if axes == (0xb0, 0xb0) {
            (self.neutral, NEUTRAL.rising_destination(0, rise)?)
        } else {
            (
                self.inflated,
                INFLATED.inverse_destination_at((64, 32), axes, rise)?,
            )
        };
        draws.push(art.draw(translate(destination, player, true), false));
        Ok(())
    }

    fn burst(
        &self,
        age: u16,
        player: Player,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        // Only three separately verified fragment groups are admitted.
        // Their sampling intervals are a provisional presentation policy.
        let (art, spec) = match age {
            0..=5 => (self.burst_start, BURST_START),
            6..=18 => (self.burst_middle, BURST_MIDDLE),
            19..=33 => (self.burst_late, BURST_LATE),
            _ => return Ok(()),
        };
        push(draws, (art, spec), player, (0, true))
    }

    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        hashes: &Digests,
    ) -> Result<Self, RenderError> {
        let [
            neutral,
            pressed,
            inflated,
            hanging,
            release,
            middle,
            late,
            landing,
            landed,
        ] = *hashes;
        let neutral_bundle = open_bundle(store, neutral)?;
        let pressed_bundle = open_bundle(store, pressed)?;
        let release_bundle = open_bundle(store, release)?;
        let middle_bundle = open_bundle(store, middle)?;
        Ok(Self {
            body: BODY.load(scene, &neutral_bundle)?,
            burst_late: BURST_LATE.load(scene, &open_bundle(store, late)?)?,
            burst_middle: BURST_MIDDLE.load(scene, &middle_bundle)?,
            burst_start: BURST_START.load(scene, &release_bundle)?,
            fall_start: FALL_START.load(scene, &release_bundle)?,
            fall_turn: FALL_TURN.load(scene, &middle_bundle)?,
            hanging: HANGING.load(scene, &open_bundle(store, hanging)?)?,
            inflated: INFLATED.load(scene, &open_bundle(store, inflated)?)?,
            landed: LANDED.load(scene, &open_bundle(store, landed)?)?,
            landing: LANDING.load(scene, &open_bundle(store, landing)?)?,
            neutral: NEUTRAL.load(scene, &neutral_bundle)?,
            pressed_body: PRESSED_BODY.load(scene, &pressed_bundle)?,
            pressed_pump: PRESSED_PUMP.load(scene, &pressed_bundle)?,
            pump: PUMP.load(scene, &neutral_bundle)?,
        })
    }

    pub(super) fn queue(
        &self,
        bonus: Bonus,
        player: Player,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        match bonus.automatic_departure(player) {
            Some(AutomaticDeparture::Rising { balloon, body }) => {
                self.balloon(bonus, (player, balloon), draws)?;
                push(draws, (self.hanging, HANGING), player, (body, false))?;
            },
            Some(AutomaticDeparture::Falling { age, elevation }) => {
                self.burst(age, player, draws)?;
                let (art, spec) = if age < 10 {
                    (self.fall_start, FALL_START)
                } else {
                    (self.fall_turn, FALL_TURN)
                };
                push(draws, (art, spec), player, (elevation, false))?;
            },
            Some(AutomaticDeparture::Landed { age }) => {
                self.burst(age, player, draws)?;
                let (art, spec) = if age < 48 {
                    (self.landing, LANDING)
                } else {
                    (self.landed, LANDED)
                };
                push(draws, (art, spec), player, (0, false))?;
            },
            None => {
                self.balloon(bonus, (player, 0), draws)?;
                let (pump, pump_spec, body, body_spec) =
                    if bonus.automatic_pressed(player) {
                        (
                            self.pressed_pump,
                            PRESSED_PUMP,
                            self.pressed_body,
                            PRESSED_BODY,
                        )
                    } else {
                        (self.pump, PUMP, self.body, BODY)
                    };
                push(draws, (pump, pump_spec), player, (0, false))?;
                push(draws, (body, body_spec), player, (0, false))?;
            },
        }
        Ok(())
    }
}

fn push(
    draws: &mut Vec<SpriteDraw>,
    (art, spec): (Art, SpriteSpec),
    player: Player,
    (rise, balloon): (u32, bool),
) -> Result<(), RenderError> {
    let destination = spec.rising_destination(0, rise)?;
    draws.push(art.draw(translate(destination, player, balloon), false));
    Ok(())
}

const fn translate(
    mut destination: SpriteRect,
    player: Player,
    balloon: bool,
) -> SpriteRect {
    let offset: i32 = match player {
        Player::Marge if balloon => -64,
        Player::Marge => -62,
        Player::Homer => 0,
        Player::Bart => 64,
        Player::Lisa if balloon => 128,
        Player::Lisa => 126,
    };
    destination.x = destination.x.saturating_add(offset);
    destination
}

pub(in crate::runtime) fn parse(
    line: &str,
    selected: bool,
) -> Result<Digests, RenderError> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("bonus-opponents") || selected {
        return Err(failure("unexpected or duplicate bonus-opponents record"));
    }
    let mut read = || {
        fields
            .next()
            .ok_or_else(|| failure("missing opponent pose bundle"))?
            .parse::<ContentHash>()
            .map_err(failure)
    };
    let digests = [
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
        read()?,
    ];
    if fields.next().is_some() {
        return Err(failure("extra opponent pose bundle"));
    }
    Ok(digests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_atlas_budget_covers_every_admitted_pose() {
        let bytes = [
            NEUTRAL,
            PUMP,
            BODY,
            PRESSED_PUMP,
            PRESSED_BODY,
            INFLATED,
            HANGING,
            BURST_START,
            FALL_START,
            BURST_MIDDLE,
            FALL_TURN,
            BURST_LATE,
            LANDING,
            LANDED,
        ]
        .into_iter()
        .map(|spec| spec.width * spec.height * 4)
        .sum::<u32>();
        assert_eq!(usize::try_from(bytes).ok(), Some(ATLAS_BYTES));
    }

    #[test]
    fn verified_station_origins_are_independent_of_scaling() {
        let neutral_result = NEUTRAL.destination(0);
        assert!(neutral_result.is_ok());
        let Ok(neutral) = neutral_result else {
            return;
        };
        let body_result = BODY.destination(0);
        assert!(body_result.is_ok());
        let Ok(body) = body_result else {
            return;
        };
        for (player, x, body_x) in [
            (Player::Homer, 109i32, 128i32),
            (Player::Bart, 173i32, 192i32),
            (Player::Lisa, 237i32, 254i32),
        ] {
            assert_eq!(translate(neutral, player, true).x, x);
            assert_eq!(translate(body, player, false).x, body_x);
        }
        let inflated_result =
            INFLATED.inverse_destination_at((64, 32), (64, 32), 0);
        assert!(inflated_result.is_ok());
        let Ok(inflated) = inflated_result else {
            return;
        };
        assert_eq!(inflated, INFLATED.destination(0).unwrap_or(inflated));
    }
}
