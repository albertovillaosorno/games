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

use std::mem::take;
use std::num::NonZeroU64;
use std::time::Duration;

use games::input::InputEvent;
use games::input::physical::KeyboardKey;
use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::{FramePacing, rgba_row};
use games::replay::LiveSimulation;
use games::session::LocalSession;
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::measurement::{elapsed_between, now};
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::CanonicalBundle;

use super::{
    Art, Controls, Scene, cabinet_with_scroll, dimension, failure, fixed,
    ground_position, live, live_with_scroll, size,
};
use crate::{
    BONUS_MAX_INFLATION, Bonus, BonusPhase, CONTINUE_TICKS, Cabinet, Control,
    DiagnosticIntent, ENEMY_CAPACITY, ENEMY_DEFEAT_TICKS, Enemy, EnemyAction,
    FAR_STREET_ADMISSION, Facing, GROUND_ATTACK_RECOVERY_TICKS,
    LATER_COMBAT_GATE, MID_STREET_ADMISSION, MID_STREET_COMBAT_GATE,
    MID_STREET_ENEMY_COUNT, Maneuver, OPPOSING_ENEMY_COUNT, PLAYER_HEALTH,
    PLAYER_STOCKS, Participation, Phase, Player, Point, REFILL_TICKS,
    STOCK_REENTRY_TICKS, Station, TICK_INTERVAL, Wrestler, WrestlerAction,
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
        repelled_art: bool,
    ) -> Result<SpriteDraw, RenderError> {
        let fallback = if matches!(enemy.action, EnemyAction::Attack { .. }) {
            self.strike
        } else {
            self.approach
        };
        let pose = animated.unwrap_or(fallback);
        let flip = match enemy.action {
            EnemyAction::Restrain(_) => false,
            EnemyAction::Repelled { .. } if repelled_art => {
                enemy.facing == Facing::Left
            },
            EnemyAction::Approach { .. }
            | EnemyAction::Attack { .. }
            | EnemyAction::Defeat { .. }
            | EnemyAction::Repelled { .. } => enemy.facing == Facing::Right,
        };
        Ok(pose.draw(ground_position(pose, enemy.position, 0)?, flip))
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
        let rect = self.enemy_draw(enemy, animated, false)?.destination;
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
        if let Some(portion) = rect.left_columns(u32::from(meter.filled)) {
            draws.push(filled.draw(portion, false));
        } else if meter.filled != 0 {
            return Err(RenderError::InvalidDimensions);
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
    scancode: KeyboardKey,
    down: bool,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    controls.route(
        InputEvent::Key {
            code: scancode.code(),
            down,
        },
        TICK_INTERVAL.saturating_mul(tick),
        simulation,
    )
}

fn pulse(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    scancode: KeyboardKey,
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
    pulse(controls, simulation, KeyboardKey::ENTER)?;
    advance(simulation, 1)?;
    key(controls, simulation, KeyboardKey::LETTER_D, true)?;
    for _ in 0..60u8 {
        let [marge, ..] = simulation.state().stations();
        let enemy = simulation
            .state()
            .enemy()
            .ok_or(RenderError::InvalidSprite)?;
        if marge.position.x.abs_diff(enemy.position.x) <= 28 {
            key(controls, simulation, KeyboardKey::LETTER_D, false)?;
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
    smoke_join_meter(scene, art)?;
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
    smoke_reinforcements(scene, art, touch_enabled)?;
    smoke_restraint(scene, art, touch_enabled)?;
    smoke_wrestler(scene, art, touch_enabled)
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
    verify_defeat_score(scene, art, simulation, &hidden)
}

fn verify_defeat_score(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
    hidden: &[u8],
) -> Result<(), RenderError> {
    if simulation.state().stations().map(|station| station.score) != [0; 4] {
        return Err(failure("score awarded before disappearance"));
    }
    let without_score =
        readback_without_scores(scene, art, simulation.state())?;
    if hidden == without_score {
        return Err(failure("initial score was not rendered"));
    }
    advance(simulation, u16::from(ENEMY_DEFEAT_TICKS.saturating_sub(54)))?;
    if simulation.state().stations().map(|station| station.score)
        != [1, 0, 0, 0]
    {
        return Err(failure("defeat did not award the successful station"));
    }
    let _gone = art.present(scene, simulation.state())?;
    let gone = scene.readback()?;
    if hidden == gone || gone == without_score {
        return Err(failure("updated score was not rendered"));
    }
    if without_score != readback_without_scores(scene, art, simulation.state())?
    {
        return Err(failure(
            "defeated enemy changed the scene beyond its score",
        ));
    }
    let _restored = art.present(scene, simulation.state())?;
    Ok(())
}

fn readback_without_scores(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<Vec<u8>, RenderError> {
    let digits = art.score_digits.take().ok_or(RenderError::InvalidSprite)?;
    let presented = art.present(scene, cabinet);
    art.score_digits = Some(digits);
    let _presented = presented?;
    scene.readback()
}

fn smoke_join_meter(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
) -> Result<(), RenderError> {
    let mut simulation = live(true)?;
    let mut controls = Controls::new(false)?;
    pulse(&mut controls, &mut simulation, KeyboardKey::ENTER)?;
    advance(&mut simulation, 1)?;
    let scancode = controls.key(Player::Homer, Control::Start)?;
    pulse(&mut controls, &mut simulation, scancode)?;
    advance(&mut simulation, 1)?;
    for expected in 0..=1 {
        let [_, homer, ..] = simulation.state().stations();
        if !homer.can_fight() || homer.display_health != expected {
            return Err(failure("native late join meter cadence failed"));
        }
        let _drawn = art.present(scene, simulation.state())?;
        verify_station_health(
            &scene.readback().map_err(failure)?,
            Player::Homer,
            homer,
            scene.layout().width().get(),
        )?;
        advance(&mut simulation, 4)?;
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
        verify_enemy_health(scene, art, simulation.state())?;
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
    let gates = [50u16, 82, 154, 210, 274, 362];
    let mut verified = 0usize;
    for gate in gates.into_iter().filter(|gate| *gate <= limit) {
        verified = verified.saturating_add(1);
        smoke_reinforcement_gate(
            scene,
            art,
            touch_enabled,
            (limit, gate, verified),
        )?;
    }
    for (gate, expected) in [
        (LATER_COMBAT_GATE, OPPOSING_ENEMY_COUNT),
        (MID_STREET_ADMISSION, 10),
        (MID_STREET_COMBAT_GATE, MID_STREET_ENEMY_COUNT),
        (FAR_STREET_ADMISSION, ENEMY_CAPACITY),
    ] {
        if i16::try_from(limit).map_err(failure)? < gate {
            break;
        }
        smoke_later_reinforcements(
            scene,
            art,
            touch_enabled,
            (limit, gate, verified, expected),
        )?;
        verified = expected.saturating_sub(1);
    }
    write_status(format_args!("Reinforcements: {verified} meters verified"))
        .map_err(failure)
}

fn advance_diagnostic(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    held: &mut Option<Control>,
    (direction, requested): DiagnosticIntent,
) -> Result<(), RenderError> {
    if *held != direction {
        if let Some(previous) = *held {
            let scan = controls.key(Player::Marge, previous)?;
            key(controls, simulation, scan, false)?;
        }
        if let Some(next) = direction {
            let scan = controls.key(Player::Marge, next)?;
            key(controls, simulation, scan, true)?;
        }
        *held = direction;
    }
    if let Some(action) = requested {
        let scan = controls.key(Player::Marge, action)?;
        pulse(controls, simulation, scan)?;
    }
    advance(simulation, 1)
}

fn smoke_later_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
    (limit, gate, verified, expected): (u16, i16, usize, usize),
) -> Result<(), RenderError> {
    let mut simulation =
        live_with_scroll(true, LocalSession::default(), limit)?;
    let mut controls = Controls::new(touch_enabled)?;
    pulse(&mut controls, &mut simulation, KeyboardKey::ENTER)?;
    advance(&mut simulation, 1)?;
    let mut held = None;
    for _ in 0..3000u16 {
        if simulation.state().camera_origin() >= gate {
            break;
        }
        let intent = simulation.state().diagnostic_intent();
        advance_diagnostic(&mut controls, &mut simulation, &mut held, intent)?;
    }
    if let Some(previous) = held {
        let scan = controls.key(Player::Marge, previous)?;
        key(&mut controls, &mut simulation, scan, false)?;
    }
    let cabinet = simulation.state();
    if cabinet.camera_origin() != gate
        || cabinet.enemies().count() != expected
        || cabinet
            .enemies()
            .take(verified.saturating_add(1))
            .any(|enemy| enemy.health > 0)
    {
        return Err(failure("later encounter was not reached through combat"));
    }
    let mut seen = [false; ENEMY_CAPACITY - 1];
    for checked in seen.iter_mut().take(verified) {
        *checked = true;
    }
    verify_reinforcements(
        scene,
        art,
        &mut simulation,
        (expected.saturating_sub(1), &mut seen),
    )
}

pub(super) fn smoke_wrestler(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    if !art.poses.wrestler.enabled() {
        return Ok(());
    }
    let limit = u16::try_from(crate::LATE_BACKDROP_ORIGIN).map_err(failure)?;
    let initial = cabinet_with_scroll(true, LocalSession::default(), limit)?
        .with_late_wrestler(true)
        .with_bonus(art.poses.bonus.is_some());
    let mut simulation = fixed(art.poses.configure_stage_two(&initial)?)?;
    let mut controls = Controls::new(touch_enabled)?;
    pulse(&mut controls, &mut simulation, KeyboardKey::ENTER)?;
    advance(&mut simulation, 1)?;
    let mut held = None;
    let mut samples = 0u8;
    for _ in 0..8000u16 {
        let cabinet = simulation.state();
        if observe_wrestler(scene, art, cabinet, &mut samples)? {
            write_status(format_args!(
                "Wrestler: reached and cleared through routed input"
            ))
            .map_err(failure)?;
            return smoke_bonus(scene, art, &mut controls, &mut simulation);
        }
        let intent = wrestler_intent(cabinet);
        advance_diagnostic(&mut controls, &mut simulation, &mut held, intent)?;
    }
    Err(failure("wrestler was not cleared through play"))
}

fn smoke_bonus(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    if art.poses.bonus.is_none() {
        return Ok(());
    }
    await_bonus(simulation, BonusPhase::Instructions)?;
    let _instructions = art.present(scene, simulation.state())?;
    let initial = scene.readback()?;
    await_bonus(simulation, BonusPhase::Ready)?;
    let _ready = art.present(scene, simulation.state())?;
    if initial == scene.readback()? {
        return Err(failure("bonus instructions and ready appear identical"));
    }
    await_bonus(simulation, BonusPhase::Pumping)?;
    let _pumping = art.present(scene, simulation.state())?;
    let before = scene.readback()?;
    pump_bonus(controls, simulation, 12)?;
    let _inflated = art.present(scene, simulation.state())?;
    if before == scene.readback()? {
        return Err(failure("routed bonus inputs did not change readback"));
    }
    verify_bonus_pause(controls, simulation)?;
    pump_bonus(controls, simulation, 64)?;
    if !simulation.state().bonus().is_some_and(|bonus| {
        bonus.phase() == BonusPhase::Results
            && bonus.rank(Player::Marge) == Some(1)
            && bonus.inflation(Player::Marge) == Some(BONUS_MAX_INFLATION)
    }) {
        return Err(failure("bonus pumping did not produce a winning result"));
    }
    let _result = art.present(scene, simulation.state())?;
    let result = scene.readback()?;
    await_bonus(simulation, BonusPhase::Complete)?;
    let _complete = art.present(scene, simulation.state())?;
    if result == scene.readback()? {
        return Err(failure("bonus completion did not change readback"));
    }
    write_status(format_args!(
        "Bonus: routed pumping, pause, winning result and completion verified"
    ))
    .map_err(failure)?;
    smoke_stage_two(scene, art, controls, simulation)
}

fn smoke_stage_two(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let Some(stage) = art.poses.stage_two else {
        return Ok(());
    };
    let before = scene.readback()?;
    advance(simulation, 1)?;
    let entry = simulation.state();
    if !entry.stage_two()
        || entry.camera_origin() != 0
        || entry.bonus().is_some()
        || entry.wrestler().is_some()
    {
        return Err(failure("second stage was not admitted"));
    }
    let impacts = entry.impact_cues();
    let _entry = art.present(scene, entry)?;
    let entry_pixels = scene.readback()?;
    if before == entry_pixels {
        return Err(failure("second-stage entry did not change readback"));
    }
    let limit = i16::try_from(stage.world.scroll_limit()?).map_err(failure)?;
    let mut held = None;
    for _ in 0..5000u16 {
        let cabinet = simulation.state();
        if cabinet.stage_two()
            && cabinet.camera_origin() == limit
            && cabinet.impact_cues() > impacts
        {
            let _drawn = art.present(scene, cabinet)?;
            if entry_pixels == scene.readback()? {
                return Err(failure(
                    "second-stage travel did not change readback",
                ));
            }
            return write_status(format_args!(
                "Second stage: routed combat and {limit} columns verified"
            ))
            .map_err(failure);
        }
        let intent = cabinet.diagnostic_intent();
        advance_diagnostic(controls, simulation, &mut held, intent)?;
    }
    Err(failure("second-stage combat or travel was not reached"))
}

fn await_bonus(
    simulation: &mut LiveSimulation<Cabinet>,
    target: BonusPhase,
) -> Result<(), RenderError> {
    for _ in 0..1800u16 {
        if simulation.state().bonus().map(Bonus::phase) == Some(target) {
            return Ok(());
        }
        advance(simulation, 1)?;
    }
    Err(failure("bonus phase not reached"))
}

fn pump_bonus(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    ticks: u16,
) -> Result<(), RenderError> {
    let scan = controls.key(Player::Marge, Control::Primary)?;
    for _ in 0..ticks {
        if simulation.state().bonus().map(Bonus::phase)
            != Some(BonusPhase::Pumping)
        {
            break;
        }
        pulse(controls, simulation, scan)?;
        advance(simulation, 1)?;
    }
    Ok(())
}

fn verify_bonus_pause(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let scan = controls.key(Player::Marge, Control::Pause)?;
    pulse(controls, simulation, scan)?;
    advance(simulation, 1)?;
    let paused = simulation.state().bonus();
    advance(simulation, 30)?;
    if simulation.state().phase() != Phase::Paused
        || simulation.state().bonus() != paused
    {
        return Err(failure("bonus advanced during pause"));
    }
    pulse(controls, simulation, scan)?;
    advance(simulation, 1)
}

fn observe_wrestler(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    samples: &mut u8,
) -> Result<bool, RenderError> {
    let Some(boss) = cabinet.wrestler() else {
        return Ok(false);
    };
    let sample = wrestler_sample(boss);
    if sample & *samples != sample {
        verify_wrestler_art(scene, art, cabinet, boss.visible())?;
        *samples |= sample;
    }
    if boss.action != WrestlerAction::Cleared {
        return Ok(false);
    }
    if *samples != 63 || boss.health != 0 {
        return Err(failure("wrestler milestones were incomplete"));
    }
    Ok(true)
}

fn wrestler_sample(boss: Wrestler) -> u8 {
    if !boss.visible() && boss.action != WrestlerAction::Cleared {
        return 0;
    }
    match boss.action {
        WrestlerAction::Intro { .. } => 1,
        WrestlerAction::Approach => 2,
        WrestlerAction::Attack { .. } => 4,
        WrestlerAction::Stagger { .. } => 8,
        WrestlerAction::Defeat { .. } => 16,
        WrestlerAction::Cleared => 32,
    }
}

fn wrestler_intent(cabinet: &Cabinet) -> DiagnosticIntent {
    let Some(boss) = cabinet.wrestler() else {
        return cabinet.diagnostic_intent();
    };
    let [marge, ..] = cabinet.stations();
    if matches!(marge.participation, Participation::Continuing { .. }) {
        return (None, Some(Control::Start));
    }
    if !marge.can_fight() || matches!(boss.action, WrestlerAction::Intro { .. })
    {
        return (None, None);
    }
    let direction = crate::diagnostic_direction(marge, boss.position);
    let attack = (direction.is_none() && marge.maneuver == Maneuver::Idle)
        .then_some(Control::Primary);
    (direction, attack)
}

fn verify_wrestler_art(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    visible: bool,
) -> Result<(), RenderError> {
    let _presented = art.present(scene, cabinet)?;
    let pixels = scene.readback()?;
    let saved = take(&mut art.poses.wrestler);
    let omitted = art.present(scene, cabinet);
    art.poses.wrestler = saved;
    let _omitted = omitted?;
    if (pixels != scene.readback()?) != visible {
        return Err(failure("wrestler art visibility differs from state"));
    }
    let _restored = art.present(scene, cabinet)?;
    Ok(())
}

fn smoke_reinforcement_gate(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
    (limit, gate, expected): (u16, u16, usize),
) -> Result<(), RenderError> {
    let target_origin = i16::try_from(gate).map_err(failure)?;
    let mut simulation =
        live_with_scroll(true, LocalSession::default(), limit)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    pulse(&mut controls, &mut simulation, KeyboardKey::LETTER_Z)?;
    advance(&mut simulation, u16::from(GROUND_ATTACK_RECOVERY_TICKS))?;
    if simulation
        .state()
        .enemy()
        .is_none_or(|enemy| enemy.health > 0)
    {
        return Err(failure("opening enemy was not defeated"));
    }
    walk_reinforcements(
        scene,
        art,
        (&mut controls, &mut simulation),
        (target_origin, expected),
    )
}

fn walk_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    (controls, simulation): (&mut Controls, &mut LiveSimulation<Cabinet>),
    (target, expected): (i16, usize),
) -> Result<(), RenderError> {
    let mut seen = [false; ENEMY_CAPACITY - 1];
    for checked in seen.iter_mut().take(expected.saturating_sub(1)) {
        *checked = true;
    }
    key(controls, simulation, KeyboardKey::LETTER_D, true)?;
    for _ in 0..300u16 {
        advance(simulation, 1)?;
        if simulation.state().camera_origin() >= target {
            break;
        }
    }
    key(controls, simulation, KeyboardKey::LETTER_D, false)?;
    if simulation.state().camera_origin() != target
        || simulation.state().enemies().count() != expected.saturating_add(1)
    {
        return Err(failure(
            "street reinforcement admission did not match travel",
        ));
    }
    verify_reinforcements(scene, art, simulation, (expected, &mut seen))
}

fn smoke_restraint(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let limit = art
        .world_art
        .map_or(Ok(0), super::world::WorldArt::scroll_limit)?;
    if !art.poses.restraint_enabled() || limit < 154 {
        return Ok(());
    }
    let cabinet = cabinet_with_scroll(true, LocalSession::default(), limit)?
        .with_marge_restraint(true);
    let mut simulation = fixed(cabinet)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    pulse(&mut controls, &mut simulation, KeyboardKey::LETTER_Z)?;
    advance(&mut simulation, u16::from(GROUND_ATTACK_RECOVERY_TICKS))?;
    key(&mut controls, &mut simulation, KeyboardKey::LETTER_D, true)?;
    for _ in 0..450u16 {
        advance(&mut simulation, 1)?;
        if restrained(simulation.state()) {
            break;
        }
    }
    key(&mut controls, &mut simulation, KeyboardKey::LETTER_D, false)?;
    verify_restraint_art(scene, art, simulation.state())?;
    for edge in 0..12u8 {
        let button = if edge.is_multiple_of(2) {
            KeyboardKey::LETTER_Z
        } else {
            KeyboardKey::SPACE
        };
        pulse(&mut controls, &mut simulation, button)?;
        advance(&mut simulation, 1)?;
        if !restrained(simulation.state()) {
            return Err(failure("restraint released before twelve edges"));
        }
    }
    advance(&mut simulation, 3)?;
    if !restrained(simulation.state()) {
        return Err(failure("restraint release delay was shortened"));
    }
    advance(&mut simulation, 1)?;
    if restrained(simulation.state())
        || !simulation.state().enemies().any(|enemy| {
            enemy.health > 0
                && matches!(enemy.action, EnemyAction::Repelled { .. })
        })
    {
        return Err(failure(
            "restraint did not release into nonlethal recovery",
        ));
    }
    verify_repelled_art(scene, art, simulation.state())?;
    write_status(format_args!("Restraint: combined art and escape verified"))
        .map_err(failure)
}

fn restrained(cabinet: &Cabinet) -> bool {
    let [marge, ..] = cabinet.stations();
    marge.maneuver == Maneuver::Restrained
}

fn verify_repelled_art(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let _presented = art.present(scene, cabinet)?;
    let pixels = scene.readback()?;
    if !art.poses.repelled_enabled() {
        return Ok(());
    }
    let enemy = cabinet
        .enemies()
        .find(|enemy| matches!(enemy.action, EnemyAction::Repelled { .. }))
        .ok_or_else(|| failure("missing repelled enemy"))?;
    let pose = art
        .poses
        .enemy(enemy.action)
        .ok_or(RenderError::InvalidSprite)?;
    if !art.world.iter().any(|(_, draw)| {
        draw.atlas == pose.atlas
            && draw.flip_horizontal == (enemy.facing == Facing::Left)
    }) {
        return Err(failure("dedicated recovery pose was not drawn"));
    }
    let saved = art.poses.take_enemy();
    let fallback = art.present(scene, cabinet);
    art.poses.restore_enemy(saved);
    let _fallback_presented = fallback?;
    if pixels == scene.readback()? {
        return Err(failure("dedicated recovery art was not visible"));
    }
    write_status(format_args!("Restraint: dedicated recovery art verified"))
        .map_err(failure)
}

fn verify_restraint_art(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let captor = cabinet
        .enemies()
        .find(|enemy| matches!(enemy.action, EnemyAction::Restrain(_)))
        .ok_or_else(|| failure("restraint was not reached through play"))?;
    let pose = art
        .poses
        .enemy(captor.action)
        .ok_or(RenderError::InvalidSprite)?;
    let _presented = art.present(scene, cabinet)?;
    let pixels = scene.readback()?;
    if art.world.len()
        != cabinet.enemies().filter(|enemy| enemy.visible()).count()
        || art
            .world
            .iter()
            .filter(|(_, draw)| draw.atlas == pose.atlas)
            .count()
            != 1
    {
        return Err(failure("restraint did not replace both actor sprites"));
    }
    let saved = art.poses.take_enemy();
    let fallback = art.present(scene, cabinet);
    art.poses.restore_enemy(saved);
    let _fallback_presented = fallback?;
    if pixels == scene.readback()? {
        return Err(failure("combined restraint art was not visible"));
    }
    Ok(())
}

fn verify_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
    (expected, seen): (usize, &mut [bool; ENEMY_CAPACITY - 1]),
) -> Result<(), RenderError> {
    // The widest supported view determines offscreen arrival coordinates.
    // In a narrow view, a lone stationary player can lose a stock before
    // the last enemy arrives. Allow a complete recovery and meter drain
    // in addition to the original bounded approach time.
    let ticks = 320u16
        .saturating_add(STOCK_REENTRY_TICKS)
        .saturating_add(REFILL_TICKS)
        .saturating_add(u16::from(PLAYER_HEALTH).saturating_mul(2));
    for _ in 0..ticks.div_ceil(4) {
        observe_reinforcements(scene, art, simulation.state(), seen)?;
        if seen.iter().filter(|checked| **checked).count() == expected {
            return Ok(());
        }
        advance(simulation, 4)?;
    }
    let cabinet = simulation.state();
    let enemies = cabinet.enemies().collect::<Vec<_>>();
    Err(failure(format!(
        "expected {expected} visible reinforcement meters: {seen:?}; \
         camera {}; stations {:?}; enemies {enemies:?}",
        cabinet.camera_origin(),
        cabinet.stations(),
    )))
}

fn observe_reinforcements(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &Cabinet,
    seen: &mut [bool; ENEMY_CAPACITY - 1],
) -> Result<(), RenderError> {
    let _presented = art.present(scene, cabinet)?;
    let pixels = scene.readback()?;
    let view = art.presentation_view(scene, cabinet)?;
    let combat = art.combat.as_ref().ok_or(RenderError::InvalidSprite)?;
    for (mut enemy, checked) in cabinet.enemies().skip(1).zip(seen) {
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
    Ok(())
}

fn smoke_strikes(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let _drawn = art.present(scene, simulation.state())?;
    let initial = scene.readback().map_err(failure)?;
    verify_enemy_health(scene, art, simulation.state())?;
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
    let injected = now().map_err(failure)?;
    pulse(controls, simulation, KeyboardKey::LETTER_Z)?;
    measure_strike(scene, art, simulation, (&mut collector, injected))?;
    if simulation.state().enemy().map(|enemy| enemy.health) != Some(0) {
        return Err(failure("native strike did not defeat enemy"));
    }
    let pixels = scene.readback().map_err(failure)?;
    if initial == pixels {
        return Err(failure("enemy defeat was not presented"));
    }
    verify_enemy_health(scene, art, simulation.state())?;
    verify_enemy_art(scene, art, simulation.state(), &pixels)?;
    report_strikes(&collector)
}

fn measure_strike(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    simulation: &mut LiveSimulation<Cabinet>,
    measurement: (&mut PerformanceCollector, Duration),
) -> Result<(), RenderError> {
    let (collector, injected) = measurement;
    for tick in 0..GROUND_ATTACK_RECOVERY_TICKS {
        let started = now().map_err(failure)?;
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
        let completed = now().map_err(failure)?;
        collector
            .observe(FrameObservation {
                frame_time: elapsed_between(started, completed)
                    .map_err(failure)?,
                input_latency: (tick == 0)
                    .then(|| elapsed_between(injected, completed))
                    .transpose()
                    .map_err(failure)?,
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
    write_status(format_args!(
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
    ))
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
    pulse(&mut controls, &mut simulation, KeyboardKey::ENTER)?;
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
    pulse(controls, simulation, KeyboardKey::LETTER_Z)?;
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
    pulse(controls, simulation, KeyboardKey::LETTER_Z)?;
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
    scene: &mut SceneSession<'_, '_>,
    art: &Scene,
    cabinet: &Cabinet,
) -> Result<(), RenderError> {
    let pixels = scene.readback()?;
    let view = art.presentation_view(scene, cabinet)?;
    let width = view.width;
    let mut enemy = cabinet.enemy().ok_or(RenderError::InvalidSprite)?;
    enemy.position = view.project(enemy.position);
    let combat = art.combat.as_ref().ok_or(RenderError::InvalidSprite)?;
    let position =
        combat.enemy_health_position(enemy, art.poses.enemy(enemy.action))?;
    if enemy.health == 0 {
        let row = health_row(&pixels, position, width, 24)?;
        if row.as_chunks::<4>().0.iter().all(|pixel| *pixel == MISSING) {
            return Err(failure("defeated enemy still has a health bar"));
        }
    } else {
        verify_bar(&pixels, (position, width), Meter::enemy(enemy.health), [
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
    let horizontal = u32::try_from(point.x).map_err(failure)?;
    let vertical = u32::try_from(point.y).map_err(failure)?;
    if horizontal.saturating_add(u32::from(columns)) > width {
        return Err(RenderError::InvalidDimensions);
    }
    rgba_row(pixels, width, (horizontal, vertical), u32::from(columns))
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
    pulse(controls, simulation, KeyboardKey::ENTER)?;
    advance(simulation, 1)?;
    let [continued, ..] = simulation.state().stations();
    if continued.health != PLAYER_HEALTH
        || continued.stocks != PLAYER_STOCKS
        || continued.participation != Participation::Playing
        || continued.display_health != 0
    {
        return Err(failure("native free start did not continue player"));
    }
    let _continued_drawn = art.present(scene, simulation.state())?;
    verify_station_health(
        &scene.readback().map_err(failure)?,
        Player::Marge,
        continued,
        scene.layout().width().get(),
    )?;
    advance(simulation, 4)?;
    let [refilling, ..] = simulation.state().stations();
    if refilling.display_health != 1 {
        return Err(failure("native free rejoin meter did not begin refill"));
    }
    let _refill_drawn = art.present(scene, simulation.state())?;
    verify_station_health(
        &scene.readback().map_err(failure)?,
        Player::Marge,
        refilling,
        scene.layout().width().get(),
    )
}

fn verify_prompt(scene: &mut SceneSession<'_, '_>) -> Result<(), RenderError> {
    let pixels = scene.readback().map_err(failure)?;
    let width = scene.layout().width().get();
    let top = health_row(&pixels, Point { x: 8, y: 3 }, width, 64)?;
    if !top.as_chunks::<4>().0.contains(&[248, 228, 168, 255]) {
        return Err(failure("missing native free rejoin countdown"));
    }
    for y in 14..26 {
        let row = health_row(&pixels, Point { x: 8, y }, width, 64)?;
        if !row
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [24, 22, 30, 255])
        {
            return Err(failure("unexpected native gameplay join binding"));
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
    pulse(controls, simulation, KeyboardKey::ENTER)?;
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
    verify_enemy_health(scene, art, simulation.state())
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
