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
//   - Native art and health indicators for the experimental encounter.
// - Must-Not:
//   - Derive combat damage from graphics or interpret original media.
// - Allows:
//   - Inputs: Verified enemy bundles and read-only encounter state.
//   - Outputs: Depth-ordered enemy draw and bounded health indicators.
//   - Side effects: Local GPU atlas allocation and sprite submission data.
// - Split-When:
//   - A separate title-local HUD or combat presentation becomes necessary.
// - Merge-When:
//   - Another native module duplicates these encounter visuals.
// - Summary:
//   - Present the optional initial enemy and the provisional health rules.
// - Description:
//   - Uses isolated original enemy art and independently authored indicators.
// - Usage:
//   - Supply two verified canonical enemy bundles.
// - Defaults:
//   - No encounter art or indicators without explicit local opt-in.

//! Native presentation of the optional first-street encounter.

use std::io::{Write as _, stdout};
use std::mem::take;
use std::num::NonZeroU64;
use std::time::Instant;

use games::input::InputEvent;
use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::FramePacing;
use games::replay::LiveSimulation;
use games::session::LocalSession;
use games_platform_graphics::linux::input::{Scancode, key_code};
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};
use games_platform_storage::bundle::CanonicalBundle;

use super::{
    Art, Controls, Scene, dimension, failure, ground_position, live,
    live_with_scroll, size,
};
use crate::{
    CONTINUE_TICKS, Cabinet, Control, ENEMY_CAPACITY, ENEMY_DEFEAT_TICKS,
    Enemy, EnemyAction, Facing, GROUND_ATTACK_RECOVERY_TICKS, Maneuver,
    PLAYER_HEALTH, PLAYER_STOCKS, Participation, Phase, Player, Point, Station,
    TICK_INTERVAL,
};

const FILLED: [u8; 4] = [86, 196, 111, 255];
const MISSING: [u8; 4] = [128, 45, 53, 255];
const PLAYER_FILLED: [u8; 4] = [255, 0, 255, 255];
const PLAYER_MISSING: [u8; 4] = [33, 82, 189, 255];

// Three draws per enemy, plus four station meters and stock markers.
// Reserve the complete bounded encounter and active-player case.
pub(super) const EXTRA_SPRITES: usize = 16 + ENEMY_CAPACITY * 3;

#[derive(Debug)]
pub(super) struct CombatArt {
    approach: Art,
    filled: Art,
    missing: Art,
    player_filled: Art,
    player_missing: Art,
    strike: Art,
}

impl CombatArt {
    pub(super) fn enemy_draw(
        &self,
        enemy: Enemy,
        animated: Option<Art>,
    ) -> Result<SpriteDraw, RenderError> {
        let fallback = if matches!(enemy.action, EnemyAction::Attack { .. }) {
            self.strike
        } else {
            self.approach
        };
        let pose = animated.unwrap_or(fallback);
        // The selected reference observations face left.
        Ok(pose.draw(
            ground_position(pose, enemy.position, 0)?,
            enemy.facing == Facing::Right,
        ))
    }

    pub(super) fn enemy_health(
        &self,
        draws: &mut Vec<SpriteDraw>,
        enemy: Enemy,
        animated: Option<Art>,
    ) -> Result<(), RenderError> {
        Self::health(
            draws,
            self.enemy_health_position(enemy, animated)?,
            Meter::enemy(enemy.health),
            [self.filled, self.missing],
        )
    }

    fn enemy_health_position(
        &self,
        enemy: Enemy,
        animated: Option<Art>,
    ) -> Result<Point, RenderError> {
        let rect = self.enemy_draw(enemy, animated)?.destination;
        Ok(Point {
            x: enemy.position.x.saturating_sub(12),
            y: i16::try_from(rect.y).map_err(failure)?.saturating_sub(5),
        })
    }

    fn health(
        draws: &mut Vec<SpriteDraw>,
        position: Point,
        meter: Meter,
        [filled, missing]: [Art; 2],
    ) -> Result<(), RenderError> {
        let rect = SpriteRect {
            height: dimension(3)?,
            width: dimension(u32::from(meter.columns))?,
            x: i32::from(position.x),
            y: i32::from(position.y),
        };
        draws.push(missing.draw(rect, false));
        if meter.filled > 0 {
            draws.push(filled.draw(
                SpriteRect {
                    width: dimension(u32::from(meter.filled))?,
                    ..rect
                },
                false,
            ));
        }
        Ok(())
    }

    pub(super) fn load(
        scene: &mut SceneSession<'_, '_>,
        bundles: &[CanonicalBundle; 2],
    ) -> Result<Self, RenderError> {
        let [approach_bundle, strike_bundle] = bundles;
        let approach = Art::load(scene, approach_bundle, "enemy/approach")?;
        let strike = Art::load(scene, strike_bundle, "enemy/strike")?;
        if [approach, strike].into_iter().any(|art| {
            art.rect.width.get() > 128 || art.rect.height.get() > 128
        }) {
            return Err(failure("unexpected enemy extent"));
        }
        let layout = FrameLayout::new(dimension(4)?, dimension(1)?, size(16)?)?;
        let atlas = scene.load_atlas(
            layout,
            [FILLED, MISSING, PLAYER_FILLED, PLAYER_MISSING].as_flattened(),
        )?;
        let filled = Art {
            atlas,
            rect: SpriteRect {
                height: dimension(1)?,
                width: dimension(1)?,
                x: 0,
                y: 0,
            },
        };
        let missing = Art {
            rect: SpriteRect { x: 1, ..filled.rect },
            ..filled
        };
        let player_filled = Art {
            rect: SpriteRect { x: 2, ..filled.rect },
            ..filled
        };
        let player_missing = Art {
            rect: SpriteRect { x: 3, ..filled.rect },
            ..filled
        };
        Ok(Self {
            approach,
            filled,
            missing,
            player_filled,
            player_missing,
            strike,
        })
    }

    pub(super) fn station_health(
        &self,
        draws: &mut Vec<SpriteDraw>,
        x: i32,
        station: Station,
    ) -> Result<(), RenderError> {
        Self::health(
            draws,
            Point {
                x: i16::try_from(x).map_err(failure)?,
                y: 25,
            },
            Meter::player(station.display_health),
            [self.player_filled, self.player_missing],
        )?;
        for stock in 0..PLAYER_STOCKS {
            let marker = if stock < station.stocks {
                self.player_filled
            } else {
                self.player_missing
            };
            draws.push(
                marker.draw(
                    SpriteRect {
                        x: x.saturating_add(i32::from(PLAYER_HEALTH))
                            .saturating_add(2)
                            .saturating_add(i32::from(stock).saturating_mul(5)),
                        y: 25,
                        width: dimension(3)?,
                        height: dimension(2)?,
                    },
                    false,
                ),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Meter {
    columns: u8,
    filled: u8,
}

impl Meter {
    const fn enemy(health: u8) -> Self {
        Self {
            columns: 24,
            filled: if health > 0 {
                24
            } else {
                0
            },
        }
    }

    const fn player(health: u8) -> Self {
        Self {
            columns: PLAYER_HEALTH,
            filled: health,
        }
    }
}

fn key(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    scancode: Scancode,
    down: bool,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    controls.route(
        InputEvent::Key {
            code: key_code(scancode),
            down,
        },
        TICK_INTERVAL.saturating_mul(tick),
        simulation,
    )
}

fn pulse(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    scancode: Scancode,
) -> Result<(), RenderError> {
    for down in [true, false] {
        key(controls, simulation, scancode, down)?;
    }
    Ok(())
}

fn advance(
    simulation: &mut LiveSimulation<Cabinet>,
    ticks: u16,
) -> Result<(), RenderError> {
    for _ in 0..ticks {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    }
    Ok(())
}

fn approach(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    pulse(controls, simulation, Scancode::Return)?;
    advance(simulation, 1)?;
    key(controls, simulation, Scancode::Right, true)?;
    for _ in 0..60u8 {
        let [marge, ..] = simulation.state().stations();
        let enemy = simulation
            .state()
            .enemy()
            .ok_or(RenderError::InvalidSprite)?;
        if marge.position.x.abs_diff(enemy.position.x) <= 28 {
            key(controls, simulation, Scancode::Right, false)?;
            return Ok(());
        }
        advance(simulation, 1)?;
    }
    Err(failure("enemy did not approach within striking range"))
}

pub(super) fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    smoke_enemy_walk(scene, art, touch_enabled)?;
    smoke_four_player_encounter(scene, art)?;
    let mut simulation = live(true)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    smoke_strikes(scene, art, &mut controls, &mut simulation)?;
    smoke_defeat(scene, art, &mut simulation)?;
    let _defeated = art.present(scene, simulation.state())?;
    let final_pixels = scene.readback().map_err(failure)?;
    advance(&mut simulation, 60)?;
    let _stable = art.present(scene, simulation.state())?;
    if final_pixels != scene.readback().map_err(failure)? {
        return Err(failure("defeated enemy continued to affect the scene"));
    }
    smoke_counterstrike(scene, art, touch_enabled)?;
    smoke_expiry(scene, art, touch_enabled)?;
    smoke_reinforcements(scene, art, touch_enabled)
}

fn smoke_defeat(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let enemy = simulation
        .state()
        .enemy()
        .ok_or(RenderError::InvalidSprite)?;
    let EnemyAction::Defeat { tick } = enemy.action else {
        return Err(failure("expected active defeat timeline"));
    };
    advance(simulation, u16::from(52u8.saturating_sub(tick)))?;
    let _hidden = art.present(scene, simulation.state())?;
    let hidden = scene.readback()?;
    advance(simulation, 1)?;
    let _blink = art.present(scene, simulation.state())?;
    let blink = scene.readback()?;
    if blink == hidden {
        return Err(failure("defeated enemy blink was not visible"));
    }
    verify_enemy_art(scene, art, simulation.state(), &blink)?;
    advance(simulation, 1)?;
    let _hidden_again = art.present(scene, simulation.state())?;
    if hidden != scene.readback()? {
        return Err(failure("defeated enemy did not blink off"));
    }
    advance(simulation, u16::from(ENEMY_DEFEAT_TICKS.saturating_sub(54)))?;
    let _gone = art.present(scene, simulation.state())?;
    if hidden != scene.readback()? {
        return Err(failure("defeated enemy did not disappear"));
    }
    Ok(())
}

fn smoke_four_player_encounter(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
) -> Result<(), RenderError> {
    let mut simulation = live(true)?;
    let mut controls = Controls::new(false)?;
    for control in [Control::Start, Control::Secondary] {
        for player in Player::ALL {
            let scancode = controls.key(player, control)?;
            pulse(&mut controls, &mut simulation, scancode)?;
        }
        advance(&mut simulation, 1)?;
        if !art.present(scene, simulation.state())? {
            return Err(failure("four-player encounter was not submitted"));
        }
        verify_active_stations(scene, simulation.state())?;
        verify_enemy_health(
            art,
            simulation.state(),
            &scene.readback()?,
            scene.layout().width().get(),
        )?;
    }
    if simulation
        .state()
        .stations()
        .iter()
        .any(|station| !station.maneuver.airborne())
    {
        return Err(failure("simultaneous four-player jump failed"));
    }
    Ok(())
}

fn smoke_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let limit = art
        .world_art
        .map_or(Ok(0), super::world::WorldArt::scroll_limit)?;
    let target = limit.min(154);
    let expected: usize = match target {
        0..=49 => return Ok(()),
        50..=81 => 1,
        82..=153 => 2,
        _ => 3,
    };
    let mut simulation =
        live_with_scroll(true, LocalSession::default(), limit)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    pulse(&mut controls, &mut simulation, Scancode::Z)?;
    advance(&mut simulation, u16::from(GROUND_ATTACK_RECOVERY_TICKS))?;
    if simulation
        .state()
        .enemy()
        .is_none_or(|enemy| enemy.health > 0)
    {
        return Err(failure("opening enemy was not defeated"));
    }
    key(&mut controls, &mut simulation, Scancode::Right, true)?;
    for _ in 0..300u16 {
        advance(&mut simulation, 1)?;
        if simulation.state().camera_origin()
            >= i16::try_from(target).map_err(failure)?
        {
            break;
        }
    }
    key(&mut controls, &mut simulation, Scancode::Right, false)?;
    if simulation.state().enemies().count() != expected.saturating_add(1usize) {
        return Err(failure(
            "street reinforcement admission did not match travel",
        ));
    }
    verify_reinforcements(scene, art, &mut simulation, expected)
}

fn verify_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
    expected: usize,
) -> Result<(), RenderError> {
    let mut seen = [false; ENEMY_CAPACITY - 1];
    for _ in 0..40u8 {
        let _presented = art.present(scene, simulation.state())?;
        let pixels = scene.readback()?;
        let view = art.presentation_view(scene, simulation.state())?;
        let combat = art.combat.as_ref().ok_or(RenderError::InvalidSprite)?;
        for (mut enemy, checked) in
            simulation.state().enemies().skip(1).zip(&mut seen)
        {
            if *checked || enemy.health == 0 {
                continue;
            }
            enemy.position = view.project(enemy.position);
            let position = combat
                .enemy_health_position(enemy, art.poses.enemy(enemy.action))?;
            let right = i32::from(position.x).saturating_add(24);
            if position.x >= 0
                && right <= i32::try_from(view.width).map_err(failure)?
            {
                verify_bar(
                    &pixels,
                    (position, view.width),
                    Meter::enemy(enemy.health),
                    [FILLED, MISSING],
                )?;
                *checked = true;
            }
        }
        if seen.iter().filter(|checked| **checked).count() == expected {
            writeln!(
                stdout().lock(),
                "Reinforcements: {expected} meters verified"
            )
            .map_err(failure)?;
            return Ok(());
        }
        advance(simulation, 4)?;
    }
    Err(failure("street reinforcement meters were not all visible"))
}

fn smoke_strikes(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let _drawn = art.present(scene, simulation.state())?;
    let initial = scene.readback().map_err(failure)?;
    verify_enemy_health(
        art,
        simulation.state(),
        &initial,
        scene.layout().width().get(),
    )?;
    let layout = scene.layout();
    let mut collector = PerformanceCollector::with_percentiles(
        QualityProfile {
            frame_pacing: FramePacing::Uncapped,
            height: layout.height().get(),
            optional_effects: false,
            width: layout.width().get(),
        },
        NonZeroU64::new(16_666_667),
    );
    let injected = Instant::now();
    pulse(controls, simulation, Scancode::Z)?;
    measure_strike(scene, art, simulation, (&mut collector, injected))?;
    if simulation.state().enemy().map(|enemy| enemy.health) != Some(0) {
        return Err(failure("native strike did not defeat enemy"));
    }
    let pixels = scene.readback().map_err(failure)?;
    if initial == pixels {
        return Err(failure("enemy defeat was not presented"));
    }
    verify_enemy_health(
        art,
        simulation.state(),
        &pixels,
        layout.width().get(),
    )?;
    verify_enemy_art(scene, art, simulation.state(), &pixels)?;
    report_strikes(&collector)
}

fn measure_strike(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
    measurement: (&mut PerformanceCollector, Instant),
) -> Result<(), RenderError> {
    let (collector, injected) = measurement;
    for tick in 0..GROUND_ATTACK_RECOVERY_TICKS {
        let started = Instant::now();
        advance(simulation, 1)?;
        let [marge, ..] = simulation.state().stations();
        let attack = Maneuver::Attack { moving: false, tick };
        let expected = if tick < attack.pose_duration(Player::Marge) {
            attack
        } else {
            Maneuver::Idle
        };
        if marge.maneuver != expected {
            return Err(failure("strike timeline did not advance"));
        }
        if !art.present(scene, simulation.state())? {
            return Err(failure("combat frame was not submitted"));
        }
        let completed = Instant::now();
        collector
            .observe(FrameObservation {
                frame_time: completed.duration_since(started),
                input_latency: (tick == 0)
                    .then(|| completed.duration_since(injected)),
                resident_bytes: None,
            })
            .map_err(failure)?;
    }
    Ok(())
}

fn report_strikes(collector: &PerformanceCollector) -> Result<(), RenderError> {
    let report = collector.report();
    let frames = report.frame_time;
    let input = report.input_latency;
    writeln!(
        stdout().lock(),
        concat!(
            "Combat strike submission: {} frames; mean {:?}; ",
            "max {:?}; p95 {:?}; budget {:?}\n",
            "Synthetic input-to-submission: {} samples; mean {:?}; max {:?}"
        ),
        frames.samples(),
        frames.mean(),
        frames.maximum(),
        collector.frame_percentile(95),
        report.frame_budget,
        input.samples(),
        input.mean(),
        input.maximum(),
    )
    .map_err(failure)
}

fn smoke_counterstrike(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let mut simulation = live(true)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    for _ in 0..40u8 {
        let _drawn = art.present(scene, simulation.state())?;
        let pixels = scene.readback().map_err(failure)?;
        verify_enemy_art(scene, art, simulation.state(), &pixels)?;
        let [marge, ..] = simulation.state().stations();
        verify_station_health(
            &pixels,
            Player::Marge,
            marge,
            scene.layout().width().get(),
        )?;
        if marge.health < PLAYER_HEALTH {
            smoke_hurt(scene, art, &mut controls, &mut simulation)?;
            // Continue admission uses a separate, undefeated encounter.
            simulation = live(true)?;
            controls = Controls::new(touch_enabled)?;
            approach(&mut controls, &mut simulation)?;
            return smoke_continue(scene, art, &mut controls, &mut simulation);
        }
        advance(&mut simulation, 1)?;
    }
    Err(failure("native enemy counterstrike did not damage Marge"))
}

fn smoke_enemy_walk(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let mut simulation = live(true)?;
    let mut controls = Controls::new(touch_enabled)?;
    pulse(&mut controls, &mut simulation, Scancode::Return)?;
    advance(&mut simulation, 6)?;
    let enemy = simulation
        .state()
        .enemy()
        .ok_or(RenderError::InvalidSprite)?;
    if enemy.action != (EnemyAction::Approach { tick: 6 }) {
        return Err(failure("native enemy approach did not advance its phase"));
    }
    let _drawn = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    verify_enemy_art(scene, art, simulation.state(), &pixels)
}

fn verify_enemy_art(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    pixels: &[u8],
) -> Result<(), RenderError> {
    let enemy = cabinet.enemy().ok_or(RenderError::InvalidSprite)?;
    if !matches!(
        enemy.action,
        EnemyAction::Attack { tick: 12 }
            | EnemyAction::Approach { tick: 6 }
            | EnemyAction::Defeat { .. }
    ) || art.poses.enemy(enemy.action).is_none()
    {
        return Ok(());
    }
    let saved = art.poses.take_enemy();
    let fallback = art.present(scene, cabinet);
    art.poses.restore_enemy(saved);
    let _fallback_presented = fallback?;
    if pixels == scene.readback().map_err(failure)? {
        return Err(failure("optional enemy pose was not visibly presented"));
    }
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

fn smoke_hurt(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, ..] = simulation.state().stations();
    if before.maneuver != (Maneuver::Hurt { tick: 0 }) {
        return Err(failure("native damage did not start a hit reaction"));
    }
    verify_hurt_art(scene, art, simulation.state())?;
    pulse(controls, simulation, Scancode::Z)?;
    advance(simulation, 1)?;
    let [first, ..] = simulation.state().stations();
    if first.maneuver != (Maneuver::Hurt { tick: 1 })
        || first.position != before.position
        || first.attack_cues != before.attack_cues
    {
        return Err(failure("native hit reaction admitted an early attack"));
    }
    advance(simulation, 5)?;
    verify_hurt_art(scene, art, simulation.state())?;
    advance(simulation, 7)?;
    pulse(controls, simulation, Scancode::Z)?;
    advance(simulation, 1)?;
    let [after, ..] = simulation.state().stations();
    if after.maneuver != (Maneuver::Attack { moving: false, tick: 0 }) {
        return Err(failure("native hit recovery did not admit a new attack"));
    }
    Ok(())
}

fn verify_hurt_art(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let [marge, ..] = cabinet.stations();
    if art.poses.select(Player::Marge, marge).is_none() {
        return Ok(());
    }
    let _presented = art.present(scene, cabinet)?;
    let animated = scene.readback().map_err(failure)?;
    let mut saved = take(&mut art.poses);
    art.poses = saved.take_enemy();
    let fallback = art.present(scene, cabinet);
    saved.restore_enemy(take(&mut art.poses));
    art.poses = saved;
    let _fallback_presented = fallback?;
    if animated == scene.readback().map_err(failure)? {
        return Err(failure("optional hit pose was not visibly presented"));
    }
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

fn verify_enemy_health(
    art: &Scene,
    cabinet: &Cabinet,
    pixels: &[u8],
    width: u32,
) -> Result<(), RenderError> {
    let enemy = cabinet.enemy().ok_or(RenderError::InvalidSprite)?;
    let combat = art.combat.as_ref().ok_or(RenderError::InvalidSprite)?;
    let position =
        combat.enemy_health_position(enemy, art.poses.enemy(enemy.action))?;
    if enemy.health == 0 {
        let row = health_row(pixels, position, width, 24)?;
        if row.as_chunks::<4>().0.iter().all(|pixel| *pixel == MISSING) {
            return Err(failure("defeated enemy still has a health bar"));
        }
    } else {
        verify_bar(pixels, (position, width), Meter::enemy(enemy.health), [
            FILLED, MISSING,
        ])?;
    }
    Ok(())
}

fn health_row(
    pixels: &[u8],
    point: Point,
    width: u32,
    columns: u8,
) -> Result<&[u8], RenderError> {
    let horizontal = usize::try_from(point.x).map_err(failure)?;
    let vertical = usize::try_from(point.y).map_err(failure)?;
    let stride = usize::try_from(width).map_err(failure)?;
    if horizontal.saturating_add(usize::from(columns)) > stride {
        return Err(RenderError::InvalidDimensions);
    }
    let offset = vertical
        .saturating_mul(stride)
        .saturating_add(horizontal)
        .saturating_mul(4);
    pixels
        .get(
            offset
                ..offset.saturating_add(usize::from(columns).saturating_mul(4)),
        )
        .ok_or(RenderError::InvalidLength)
}

fn verify_bar(
    pixels: &[u8],
    (position, width): (Point, u32),
    meter: Meter,
    [filled, missing]: [[u8; 4]; 2],
) -> Result<(), RenderError> {
    let row = health_row(pixels, position, width, meter.columns)?;
    for (column, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
        let expected = if column < usize::from(meter.filled) {
            filled
        } else {
            missing
        };
        if *pixel != expected {
            return Err(failure("unexpected native health indicator pixels"));
        }
    }
    Ok(())
}

fn smoke_continue(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    smoke_stock(scene, art, simulation)?;
    wait_for_continue(simulation)?;
    let _drawn = art.present(scene, simulation.state())?;
    verify_prompt(scene)?;
    pulse(controls, simulation, Scancode::Return)?;
    advance(simulation, 1)?;
    let [continued, ..] = simulation.state().stations();
    if continued.health != PLAYER_HEALTH
        || continued.stocks != PLAYER_STOCKS
        || continued.participation != Participation::Playing
    {
        return Err(failure("native free start did not continue player"));
    }
    let _continued_drawn = art.present(scene, simulation.state())?;
    verify_station_health(
        &scene.readback().map_err(failure)?,
        Player::Marge,
        continued,
        scene.layout().width().get(),
    )
}

fn verify_prompt(scene: &mut SceneSession<'_, '_>) -> Result<(), RenderError> {
    let pixels = scene.readback().map_err(failure)?;
    for y in [3, 14, 20] {
        let row = health_row(
            &pixels,
            Point { x: 8, y },
            scene.layout().width().get(),
            64,
        )?;
        if !row.as_chunks::<4>().0.contains(&[248, 228, 168, 255]) {
            return Err(failure("missing native free rejoin prompt"));
        }
    }
    Ok(())
}

fn wait_for_continue(
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    for _ in 0..1600u16 {
        let [marge, ..] = simulation.state().stations();
        if matches!(marge.participation, Participation::Continuing { .. }) {
            return Ok(());
        }
        advance(simulation, 1)?;
    }
    Err(failure("native encounter did not reach continue state"))
}

fn smoke_expiry(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let mut simulation = live(true)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    wait_for_continue(&mut simulation)?;
    let _initial = art.present(scene, simulation.state())?;
    verify_prompt(scene)?;
    let before = scene.readback().map_err(failure)?;
    advance(&mut simulation, 60)?;
    let _counted = art.present(scene, simulation.state())?;
    verify_prompt(scene)?;
    if before == scene.readback().map_err(failure)? {
        return Err(failure("native continue countdown did not change"));
    }
    advance(&mut simulation, CONTINUE_TICKS.saturating_sub(60))?;
    let [expired, ..] = simulation.state().stations();
    if expired.participation != Participation::Absent
        || simulation.state().phase() != Phase::MainMenu
    {
        return Err(failure(
            "native free continue timeout did not return to menu",
        ));
    }
    let _expired_drawn = art.present(scene, simulation.state())?;
    restart_after_expiry(scene, art, &mut controls, &mut simulation)
}

fn restart_after_expiry(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    pulse(controls, simulation, Scancode::Return)?;
    advance(simulation, 1)?;
    if simulation.state().phase() != Phase::Playing {
        return Err(failure("native restart did not enter street"));
    }
    let _restarted = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    let [restarted, ..] = simulation.state().stations();
    verify_station_health(
        &pixels,
        Player::Marge,
        restarted,
        scene.layout().width().get(),
    )?;
    verify_enemy_health(
        art,
        simulation.state(),
        &pixels,
        scene.layout().width().get(),
    )
}

pub(super) fn verify_active_stations(
    scene: &mut SceneSession<'_, '_>,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let pixels = scene.readback().map_err(failure)?;
    let width = scene.layout().width().get();
    for (player, station) in Player::ALL.into_iter().zip(cabinet.stations()) {
        if station.participation != Participation::Playing {
            return Err(failure("expected four active health meters"));
        }
        verify_station_health(&pixels, player, station, width)?;
    }
    Ok(())
}

fn verify_station_health(
    pixels: &[u8],
    player: Player,
    station: Station,
    width: u32,
) -> Result<(), RenderError> {
    let x = i16::try_from(
        super::indicator(player)?
            .x
            .saturating_add(super::world::station_offset(width, player)?),
    )
    .map_err(failure)?;
    verify_bar(
        pixels,
        (Point { x, y: 25 }, width),
        Meter::player(station.display_health),
        [PLAYER_FILLED, PLAYER_MISSING],
    )?;
    for stock in 0..PLAYER_STOCKS {
        let point = Point {
            x: x.saturating_add(34)
                .saturating_add(i16::from(stock).saturating_mul(5)),
            y: 25,
        };
        let row = health_row(pixels, point, width, 3)?;
        let expected = if stock < station.stocks {
            PLAYER_FILLED
        } else {
            PLAYER_MISSING
        };
        if !row
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == expected)
        {
            return Err(failure("unexpected native stock indicator pixels"));
        }
    }
    Ok(())
}

fn smoke_stock(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let mut saw_zero = false;
    for _ in 0..800u16 {
        let [station, ..] = simulation.state().stations();
        if station.stocks == 1
            && station.health == 0
            && station.display_health == 0
            && !saw_zero
        {
            let _drawn = art.present(scene, simulation.state())?;
            verify_station_health(
                &scene.readback().map_err(failure)?,
                Player::Marge,
                station,
                scene.layout().width().get(),
            )?;
            saw_zero = true;
        }
        if saw_zero
            && station.stocks == 1
            && station.health == PLAYER_HEALTH
            && station.display_health == PLAYER_HEALTH.div_ceil(2)
        {
            let _drawn = art.present(scene, simulation.state())?;
            return verify_station_health(
                &scene.readback().map_err(failure)?,
                Player::Marge,
                station,
                scene.layout().width().get(),
            );
        }
        advance(simulation, 1)?;
    }
    Err(failure("native automatic stock refill was not observed"))
}
