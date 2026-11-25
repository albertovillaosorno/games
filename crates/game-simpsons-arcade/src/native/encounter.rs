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

use games::input::InputEvent;
use games::replay::LiveSimulation;
use games_platform_graphics::linux::input::{Scancode, key_code};
use games_platform_graphics::linux::sprites::{
    SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, RenderError};
use games_platform_storage::bundle::CanonicalBundle;

use super::continue_hud::ContinueArt;
use super::{
    Art, Controls, Scene, dimension, failure, ground_position, indicator, live,
    size,
};
use crate::{
    CONTINUE_TICKS, Cabinet, Enemy, EnemyAction, Facing, Participation, Phase,
    Player, Point, Station, TICK_INTERVAL,
};

const FILLED: [u8; 4] = [86, 196, 111, 255];
const MISSING: [u8; 4] = [128, 45, 53, 255];

#[derive(Debug)]
pub(super) struct CombatArt {
    approach: Art,
    continue_hud: ContinueArt,
    filled: Art,
    missing: Art,
    strike: Art,
}

impl CombatArt {
    pub(super) fn continue_draw(
        &self,
        player: Player,
        paid: bool,
        ticks_left: u16,
    ) -> Result<SpriteDraw, RenderError> {
        self.continue_hud.draw(player, paid, ticks_left)
    }

    pub(super) fn enemy_draw(
        &self,
        enemy: Enemy,
    ) -> Result<SpriteDraw, RenderError> {
        let pose = if matches!(enemy.action, EnemyAction::Attack { .. }) {
            self.strike
        } else {
            self.approach
        };
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
    ) -> Result<(), RenderError> {
        self.health(draws, self.enemy_health_position(enemy)?, enemy.health)
    }

    fn enemy_health_position(
        &self,
        enemy: Enemy,
    ) -> Result<Point, RenderError> {
        let rect = self.enemy_draw(enemy)?.destination;
        Ok(Point {
            x: enemy.position.x.saturating_sub(12),
            y: i16::try_from(rect.y).map_err(failure)?.saturating_sub(5),
        })
    }

    fn health(
        &self,
        draws: &mut Vec<SpriteDraw>,
        position: Point,
        health: u8,
    ) -> Result<(), RenderError> {
        let rect = SpriteRect {
            height: dimension(3)?,
            width: dimension(24)?,
            x: i32::from(position.x),
            y: i32::from(position.y),
        };
        draws.push(self.missing.draw(rect, false));
        if health > 0 {
            draws.push(self.filled.draw(
                SpriteRect {
                    width: dimension(u32::from(health).saturating_mul(8))?,
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
        let layout = FrameLayout::new(dimension(2)?, dimension(1)?, size(8)?)?;
        let atlas =
            scene.load_atlas(layout, [FILLED, MISSING].as_flattened())?;
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
        Ok(Self {
            approach,
            continue_hud: ContinueArt::load(scene)?,
            filled,
            missing,
            strike,
        })
    }

    pub(super) fn station_health(
        &self,
        draws: &mut Vec<SpriteDraw>,
        player: Player,
        station: Station,
    ) -> Result<(), RenderError> {
        let rect = indicator(player)?;
        self.health(
            draws,
            Point {
                x: i16::try_from(rect.x).map_err(failure)?,
                y: 25,
            },
            station.health,
        )
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
    for scancode in [Scancode::Num1, Scancode::Num5, Scancode::Z] {
        pulse(controls, simulation, scancode)?;
    }
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
    let mut simulation = live(true)?;
    let mut controls = Controls::new(touch_enabled)?;
    approach(&mut controls, &mut simulation)?;
    let _drawn = art.present(scene, simulation.state())?;
    let initial = scene.readback().map_err(failure)?;
    verify_enemy_health(art, simulation.state(), &initial)?;
    let mut previous = initial;
    for health in [2, 1, 0] {
        pulse(&mut controls, &mut simulation, Scancode::Z)?;
        advance(&mut simulation, 17)?;
        if simulation.state().enemy().map(|enemy| enemy.health) != Some(health)
        {
            return Err(failure("native strike did not change enemy health"));
        }
        let _draw = art.present(scene, simulation.state())?;
        let pixels = scene.readback().map_err(failure)?;
        if previous == pixels {
            return Err(failure("enemy damage was not presented"));
        }
        verify_enemy_health(art, simulation.state(), &pixels)?;
        previous = pixels;
    }
    let _settle = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let _defeated = art.present(scene, simulation.state())?;
    let final_pixels = scene.readback().map_err(failure)?;
    advance(&mut simulation, 60)?;
    let _stable = art.present(scene, simulation.state())?;
    if final_pixels != scene.readback().map_err(failure)? {
        return Err(failure("defeated enemy continued to affect the scene"));
    }
    smoke_counterstrike(scene, art, touch_enabled)?;
    smoke_expiry(scene, art, touch_enabled)
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
        let [marge, ..] = simulation.state().stations();
        verify_bar(&pixels, Point { x: 28, y: 25 }, marge.health)?;
        if marge.health == 2 {
            return smoke_continue(scene, art, &mut controls, &mut simulation);
        }
        advance(&mut simulation, 1)?;
    }
    Err(failure("native enemy counterstrike did not damage Marge"))
}

fn verify_enemy_health(
    art: &Scene,
    cabinet: &Cabinet,
    pixels: &[u8],
) -> Result<(), RenderError> {
    let enemy = cabinet.enemy().ok_or(RenderError::InvalidSprite)?;
    let combat = art.combat.as_ref().ok_or(RenderError::InvalidSprite)?;
    let position = combat.enemy_health_position(enemy)?;
    if enemy.health == 0 {
        let row = health_row(pixels, position)?;
        if row.as_chunks::<4>().0.iter().all(|pixel| *pixel == MISSING) {
            return Err(failure("defeated enemy still has a health bar"));
        }
    } else {
        verify_bar(pixels, position, enemy.health)?;
    }
    Ok(())
}

fn health_row(pixels: &[u8], point: Point) -> Result<&[u8], RenderError> {
    let horizontal = usize::try_from(point.x).map_err(failure)?;
    let vertical = usize::try_from(point.y).map_err(failure)?;
    if horizontal.saturating_add(24) > 320 {
        return Err(RenderError::InvalidDimensions);
    }
    let offset = vertical
        .saturating_mul(320)
        .saturating_add(horizontal)
        .saturating_mul(4);
    pixels
        .get(offset..offset.saturating_add(96))
        .ok_or(RenderError::InvalidLength)
}

fn verify_bar(
    pixels: &[u8],
    position: Point,
    health: u8,
) -> Result<(), RenderError> {
    let row = health_row(pixels, position)?;
    for (column, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
        let expected = if column < usize::from(health).saturating_mul(8) {
            FILLED
        } else {
            MISSING
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
    wait_for_continue(simulation)?;
    let _drawn = art.present(scene, simulation.state())?;
    verify_prompt(scene, false)?;
    pulse(controls, simulation, Scancode::Num5)?;
    advance(simulation, 1)?;
    let [unpaid, ..] = simulation.state().stations();
    if unpaid.health != 0 {
        return Err(failure("unpaid start restored native player"));
    }
    pulse(controls, simulation, Scancode::Num1)?;
    advance(simulation, 1)?;
    let [paid, ..] = simulation.state().stations();
    if paid.credits != 1 || paid.health != 0 {
        return Err(failure("native coin incorrectly restored player"));
    }
    let _paid_drawn = art.present(scene, simulation.state())?;
    verify_prompt(scene, true)?;
    pulse(controls, simulation, Scancode::Num5)?;
    advance(simulation, 1)?;
    let [continued, ..] = simulation.state().stations();
    if continued.health != 3
        || continued.credits != 0
        || continued.participation != Participation::Playing
    {
        return Err(failure("native paid start did not continue player"));
    }
    let _continued_drawn = art.present(scene, simulation.state())?;
    verify_bar(
        &scene.readback().map_err(failure)?,
        Point { x: 28, y: 25 },
        3,
    )
}

fn verify_prompt(
    scene: &mut SceneSession<'_, '_>,
    paid: bool,
) -> Result<(), RenderError> {
    let pixels = scene.readback().map_err(failure)?;
    let row = health_row(&pixels, Point { x: 9, y: 3 })?;
    let start = row.get(..4).ok_or(RenderError::InvalidLength)?;
    let paid_start = row.get(48..52).ok_or(RenderError::InvalidLength)?;
    let ink = [248, 228, 168, 255];
    let background = [24, 22, 30, 255];
    if (paid && (start != background || paid_start != ink))
        || (!paid && start != ink)
    {
        return Err(failure("unexpected native continue prompt pixels"));
    }
    Ok(())
}

fn wait_for_continue(
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    for _ in 0..100u8 {
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
    verify_prompt(scene, false)?;
    let before = scene.readback().map_err(failure)?;
    advance(&mut simulation, 60)?;
    let _counted = art.present(scene, simulation.state())?;
    verify_prompt(scene, false)?;
    if before == scene.readback().map_err(failure)? {
        return Err(failure("native continue countdown did not change"));
    }
    pulse(&mut controls, &mut simulation, Scancode::Num1)?;
    advance(&mut simulation, CONTINUE_TICKS.saturating_sub(60))?;
    let [expired, ..] = simulation.state().stations();
    if expired.participation != Participation::Absent
        || expired.credits != 1
        || simulation.state().phase() != Phase::Selection
    {
        return Err(failure("native paid timeout did not retain credit"));
    }
    let _expired_drawn = art.present(scene, simulation.state())?;
    verify_prompt(scene, true)?;
    restart_after_expiry(scene, art, &mut controls, &mut simulation)
}

fn restart_after_expiry(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    pulse(controls, simulation, Scancode::Num5)?;
    advance(simulation, 1)?;
    let [selected, ..] = simulation.state().stations();
    if selected.participation != Participation::Selected
        || selected.credits != 0
    {
        return Err(failure("native expired credit did not select station"));
    }
    pulse(controls, simulation, Scancode::Z)?;
    advance(simulation, 1)?;
    if simulation.state().phase() != Phase::Street {
        return Err(failure("native restart did not enter street"));
    }
    let _restarted = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    verify_bar(&pixels, Point { x: 28, y: 25 }, 3)?;
    verify_enemy_health(art, simulation.state(), &pixels)
}
