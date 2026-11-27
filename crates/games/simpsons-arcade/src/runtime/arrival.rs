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
//   - Registered body, tether and shared shadow art for stage two arrival.
// - Must-Not:
//   - Advance simulation, interpret source objects or choose combat rules.
// - Allows:
//   - Inputs: Canonical isolated art, scripted samples and world viewport.
//   - Outputs: Depth-ordered, world-projected sprite draws.
//   - Side effects: Bounded atlas uploads at scene startup.
// - Split-When:
//   - A separate landing sequence needs independent art and timing.
// - Merge-When:
//   - Shared registered animation supports these independent layers.
// - Summary:
//   - Present the scripted descent and released tether withdrawal.
// - Description:
//   - Reuse body and tether payloads and the existing ground-shadow mask.
// - Usage:
//   - Select the optional arrival catalogue record with second-stage art.
// - Defaults:
//   - Fall back to the existing entrance pose without a selected bundle.

//! Registered second-stage arrival body and tether presentation.

use std::num::NonZeroI64;

use games_assets::{AssetFormat, ContentHash};
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};

use super::shadow::ShadowArt;
use super::world::View;
use super::{Art, Scene, failure, ground_position, open_bundle, scenery};
use crate::{Cabinet, Player, Point, StageEntrance};

const BODY_KEYS: [&str; 4] = [
    "actor/arrival-marge",
    "actor/arrival-homer",
    "actor/arrival-bart",
    "actor/arrival-lisa",
];
const TETHER_KEYS: [&str; 4] = [
    "actor/rope-marge",
    "actor/rope-homer",
    "actor/rope-bart",
    "actor/rope-lisa",
];
const MARKER_KEYS: [&str; 4] = [
    "actor/landing-release-marge",
    "actor/landing-release-homer",
    "actor/landing-release-bart",
    "actor/landing-release-lisa",
];
const MARKER_EXTENTS: [(u32, u32); 4] = [(32, 31); 4];
const BODY_EXTENTS: [(u32, u32); 4] = [(31, 76), (30, 64), (28, 48), (33, 104)];
const TETHER_EXTENTS: [(u32, u32); 4] =
    [(54, 63), (64, 71), (57, 55), (79, 79)];

pub(super) const ATLAS_COUNT: usize = 8;
pub(super) const ATLAS_BYTES: usize = 105_496;
pub(super) const EXTRA_SPRITES: usize = 4;
pub(super) const MARKER_ATLAS_COUNT: usize = 4;
pub(super) const MARKER_ATLAS_BYTES: usize = 15_872;
pub(super) const MARKER_EXTRA_SPRITES: usize = 4;

#[derive(Debug)]
pub(super) struct ArrivalArt {
    bodies: [Art; 4],
    markers: Option<[Art; 4]>,
    tethers: [Art; 4],
}

impl ArrivalArt {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        (digest, markers): (ContentHash, Option<ContentHash>),
    ) -> Result<Self, RenderError> {
        let bundle = open_bundle(store, digest)?;
        Ok(Self {
            bodies: load_layers(scene, &bundle, BODY_KEYS, BODY_EXTENTS)?,
            markers: markers
                .map(|hash| {
                    let marker_bundle = open_bundle(store, hash)?;
                    load_layers(
                        scene,
                        &marker_bundle,
                        MARKER_KEYS,
                        MARKER_EXTENTS,
                    )
                })
                .transpose()?,
            tethers: load_layers(scene, &bundle, TETHER_KEYS, TETHER_EXTENTS)?,
        })
    }

    pub(super) fn queue(
        &self,
        (cabinet, entrance): (&Cabinet, StageEntrance),
        (view, shadow): (View, &ShadowArt),
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        if !entrance.scripted_art() {
            return self.queue_released(entrance, view, draws);
        }
        // The shadows are below the airborne layers. Every tether precedes
        // every body, independently of the current character selection.
        let mut tethers = [None; 4];
        let mut bodies = [None; 4];
        for (player, station) in Player::ALL.into_iter().zip(cabinet.stations())
        {
            if !station.can_fight() {
                continue;
            }
            let Some((point, elevation)) =
                entrance.presentation_sample(player, 1)
            else {
                continue;
            };
            let registration = Registration::for_player(player);
            let index = player.index();
            let body =
                *self.bodies.get(index).ok_or(RenderError::InvalidSprite)?;
            let destination = ground_position(
                body,
                view.project(registration.body(point)),
                elevation,
            )?;
            *bodies.get_mut(index).ok_or(RenderError::InvalidSprite)? =
                Some(body.draw(destination, false));
            if let Some((tether_point, tether_elevation)) =
                entrance.presentation_sample(player, 2)
            {
                draws.push(
                    shadow.draw(view.project(shadow_position(tether_point))),
                );
                *tethers.get_mut(index).ok_or(RenderError::InvalidSprite)? =
                    Some(self.tether_draw(
                        player,
                        (tether_point, tether_elevation),
                        view,
                    )?);
            }
        }
        draws.extend(tethers.into_iter().flatten());
        draws.extend(bodies.into_iter().flatten());
        Ok(())
    }

    pub(super) fn queue_markers(
        &self,
        cabinet: &Cabinet,
        view: View,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let (Some(markers), Some(entrance)) =
            (&self.markers, cabinet.stage_arrival())
        else {
            return Ok(());
        };
        for (player, station) in Player::ALL.into_iter().zip(cabinet.stations())
        {
            if !station.can_fight() {
                continue;
            }
            let Some(elevation) = entrance.marker_elevation(player) else {
                continue;
            };
            let marker = *markers
                .get(player.index())
                .ok_or(RenderError::InvalidSprite)?;
            let origin = view.project(marker_position(
                player,
                station.position,
                elevation,
            ));
            draws.push(marker.draw(
                SpriteRect {
                    x: i32::from(origin.x),
                    y: i32::from(origin.y),
                    ..marker.rect
                },
                false,
            ));
        }
        Ok(())
    }

    fn queue_released(
        &self,
        entrance: StageEntrance,
        view: View,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        for player in Player::ALL {
            let Some(sample) = entrance.released_tether(player) else {
                continue;
            };
            let draw = self.tether_draw(player, sample, view)?;
            let height = NonZeroI64::from(draw.destination.height);
            let bottom =
                i64::from(draw.destination.y).saturating_add(height.get());
            if bottom > 0 {
                draws.push(draw);
            }
        }
        Ok(())
    }

    fn tether_draw(
        &self,
        player: Player,
        (point, elevation): (Point, i16),
        view: View,
    ) -> Result<SpriteDraw, RenderError> {
        let tether = *self
            .tethers
            .get(player.index())
            .ok_or(RenderError::InvalidSprite)?;
        let origin = view
            .project(Registration::for_player(player).tether(point, elevation));
        Ok(tether.draw(
            SpriteRect {
                x: i32::from(origin.x),
                y: i32::from(origin.y),
                ..tether.rect
            },
            false,
        ))
    }
}

#[derive(Clone, Copy, Debug)]
struct Registration {
    body_x: i16,
    tether_height: i16,
    tether_x: i16,
}

impl Registration {
    const fn body(self, point: Point) -> Point {
        Point {
            x: point.x.saturating_sub(self.body_x),
            y: point.y.saturating_sub(8),
        }
    }

    const fn for_player(player: Player) -> Self {
        let (body_x, tether_x, tether_height) = match player {
            Player::Marge => (-7, 14, 58),
            Player::Homer => (-8, 24, 50),
            Player::Bart => (-9, 24, 66),
            Player::Lisa => (-3, 32, 42),
        };
        Self {
            body_x,
            tether_height,
            tether_x,
        }
    }

    const fn tether(self, point: Point, elevation: i16) -> Point {
        Point {
            x: point.x.saturating_sub(self.tether_x),
            y: self.tether_height.saturating_sub(elevation),
        }
    }
}

fn load_layers(
    scene: &mut SceneSession<'_, '_>,
    bundle: &CanonicalBundle,
    keys: [&str; 4],
    extents: [(u32, u32); 4],
) -> Result<[Art; 4], RenderError> {
    let mut entries = Vec::new();
    for (key, extent) in keys.into_iter().zip(extents) {
        let record = bundle
            .manifest()
            .record(key)
            .ok_or(RenderError::InvalidSprite)?;
        let AssetFormat::Rgba8 { width, height } = record.format else {
            return Err(RenderError::InvalidSprite);
        };
        if (width.get(), height.get()) != extent {
            return Err(failure("unexpected arrival art extent"));
        }
        entries.push(Art::load(scene, bundle, key)?);
    }
    <[Art; 4]>::try_from(entries).map_err(|_error| RenderError::InvalidSprite)
}

const fn marker_position(
    player: Player,
    point: Point,
    elevation: i16,
) -> Point {
    let offset = match player {
        Player::Marge => 123,
        Player::Homer => 105,
        Player::Bart | Player::Lisa => 92,
    };
    Point {
        x: point.x.saturating_sub(16),
        y: point.y.saturating_sub(offset).saturating_sub(elevation),
    }
}

const fn shadow_position(point: Point) -> Point {
    Point {
        x: point.x.saturating_add(8),
        y: point.y.saturating_sub(7),
    }
}

pub(super) fn verify(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    let Some(arrival) = art.poses.arrival.as_ref() else {
        return Ok(());
    };
    let view = art.presentation_view(scene, cabinet)?;
    let shadow = art.shadow.as_ref().ok_or(RenderError::InvalidSprite)?;
    let mut expected = Vec::new();
    let entrance = cabinet.stage_arrival();
    if let Some(motion) = entrance {
        arrival.queue((cabinet, motion), (view, shadow), &mut expected)?;
    }
    if visible {
        verify_draws(scene, art, cabinet, (view, &expected))?;
    } else if !expected.is_empty()
        || art.draws.iter().any(|draw| {
            arrival
                .bodies
                .iter()
                .chain(&arrival.tethers)
                .any(|layer| layer.atlas == draw.atlas)
        })
    {
        return Err(failure("arrival art outside its interval"));
    }
    let state = if visible {
        "visible"
    } else {
        "absent"
    };
    write_status(format_args!(
        "Second stage arrival layers: {state} at tick {:?}",
        entrance.map(StageEntrance::tick)
    ))
    .map_err(failure)
}

fn verify_draws(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    (view, expected): (View, &[SpriteDraw]),
) -> Result<(), RenderError> {
    if expected.is_empty()
        || !art
            .draws
            .windows(expected.len())
            .any(|draws| draws == expected)
    {
        return Err(failure("arrival layer order differs"));
    }
    let actual = scene.readback()?;
    let baseline_draws: Vec<_> = art
        .draws
        .iter()
        .copied()
        .filter(|draw| !expected.contains(draw))
        .collect();
    let bounds: Vec<_> = expected.iter().map(|draw| draw.destination).collect();
    let _baseline = scene.present([24, 22, 30], &baseline_draws)?;
    let baseline = scene.readback()?;
    let _restored = art.present(scene, cabinet)?;
    scenery::verify_pixels(&actual, &baseline, view.width, &bounds)
}

pub(super) fn verify_ordinary_bodies(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    if art.poses.arrival.is_none() {
        return Ok(());
    }
    let view = art.presentation_view(scene, cabinet)?;
    for (player, station) in Player::ALL.into_iter().zip(cabinet.stations()) {
        if !station.can_fight() {
            continue;
        }
        let elevation = cabinet.stage_entrance().map_or_else(
            || station.maneuver.elevation(),
            |motion| motion.elevation(player).unwrap_or_default(),
        );
        let pose = if elevation > 0 && cabinet.stage_entrance().is_some() {
            art.poses.entrance(player)
        } else {
            art.poses.select(player, station)
        }
        .or_else(|| art.actors.get(player.index()).copied())
        .ok_or(RenderError::InvalidSprite)?;
        let draw = pose.draw(
            ground_position(pose, view.project(station.position), elevation)?,
            station.facing == crate::Facing::Left,
        );
        let present = art.world.iter().any(|(_, actual)| *actual == draw);
        if present != visible {
            return Err(failure("ordinary arrival body admission mismatch"));
        }
        if visible {
            verify_draws(scene, art, cabinet, (view, &[draw]))?;
        }
    }
    write_status(format_args!(
        "Second stage ordinary bodies: {visible} at tick {:?}",
        cabinet.stage_arrival().map(StageEntrance::tick)
    ))
    .map_err(failure)
}

pub(super) fn verify_markers(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    let Some(arrival) = art.poses.arrival.as_ref() else {
        return Ok(());
    };
    let Some(markers) = arrival.markers.as_ref() else {
        return Ok(());
    };
    let view = art.presentation_view(scene, cabinet)?;
    let mut expected = Vec::new();
    arrival.queue_markers(cabinet, view, &mut expected)?;
    if visible {
        verify_draws(scene, art, cabinet, (view, &expected))?;
    } else if !expected.is_empty()
        || art
            .draws
            .iter()
            .any(|draw| markers.iter().any(|layer| layer.atlas == draw.atlas))
    {
        return Err(failure("landing markers outside their interval"));
    }
    write_status(format_args!(
        "Second stage landing markers: {visible} at tick {:?}",
        cabinet.stage_arrival().map(StageEntrance::tick)
    ))
    .map_err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_budget_matches_the_admitted_extents() {
        let bytes: u64 = BODY_EXTENTS
            .into_iter()
            .chain(TETHER_EXTENTS)
            .map(|(width, height)| {
                u64::from(width)
                    .saturating_mul(u64::from(height))
                    .saturating_mul(4)
            })
            .sum();
        assert_eq!(usize::try_from(bytes).ok(), Some(ATLAS_BYTES));
    }

    #[test]
    fn markers_register_at_the_observed_grounded_origins() {
        let points = Player::ALL.map(|player| {
            let index = i16::try_from(player.index()).unwrap_or_default();
            marker_position(
                player,
                Point {
                    x: 68i16.saturating_add(32i16.saturating_mul(index)),
                    y: 192,
                },
                0,
            )
        });
        assert_eq!(points, [
            Point { x: 52, y: 69 },
            Point { x: 84, y: 87 },
            Point { x: 116, y: 100 },
            Point { x: 148, y: 100 },
        ]);
        assert_eq!(MARKER_ATLAS_BYTES, 4 * 32 * 31 * 4);
    }

    #[test]
    fn final_registration_matches_all_four_observed_origins() {
        let body_points = Player::ALL.map(|player| {
            let index = i16::try_from(player.index()).unwrap_or_default();
            Registration::for_player(player).body(Point {
                x: 68i16.saturating_add(32i16.saturating_mul(index)),
                y: 192,
            })
        });
        assert_eq!(body_points, [
            Point { x: 75, y: 184 },
            Point { x: 108, y: 184 },
            Point { x: 141, y: 184 },
            Point { x: 167, y: 184 },
        ]);
        let tethers = Player::ALL.map(|player| {
            let index = i16::try_from(player.index()).unwrap_or_default();
            let registration = Registration::for_player(player);
            registration.tether(
                Point {
                    x: 67i16.saturating_add(32i16.saturating_mul(index)),
                    y: 192,
                },
                registration.tether_height,
            )
        });
        assert_eq!(tethers, [
            Point { x: 53, y: 0 },
            Point { x: 75, y: 0 },
            Point { x: 107, y: 0 },
            Point { x: 131, y: 0 },
        ]);
        assert_eq!(shadow_position(Point { x: 67, y: 192 }), Point {
            x: 75,
            y: 185
        });
    }
}
