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
//   - Bounded browser frame scheduler and title presentation smoke.
// - Must-Not:
//   - Implement movement rules or call browser APIs directly.
// - Allows:
//   - Inputs: Host frame timestamps and synthetic title control actions.
//   - Outputs: Verified fixed ticks and rendered procedural stations.
//   - Side effects: Open a scoped window and report terminal completion.
// - Split-When:
//   - Production gameplay exercises the same scheduler and window.
// - Merge-When:
//   - Shared diagnostics can preserve both terminal outcomes.
// - Summary:
//   - Drive four-player simulation across independent browser callbacks.
// - Description:
//   - Keep a scene resident and advance the shared fixed clock.
// - Usage:
//   - Run in a visible browser document.
// - Defaults:
//   - Finite procedural exercise, no original media or persistent changes.

//! Browser fixed-step simulation across host animation frames.

use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

use games::input::LogicalAction;
use games::replay::{InputScript, Scripted};
use games::timing::StepClock;
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::graphics::{NativeWindow, frame_loop};
use games_platform::ir::process::{write_diagnostic, write_status};
use games_platform::ir::scene::{AtlasId, SceneLimits, SpriteDraw, SpriteRect};
use games_simpsons_arcade::{
    Action, Cabinet, Control, Participation, Player, Station, StreetLayout,
    TICK_INTERVAL,
};

type ResidentScene = SceneSession<'static, 'static>;
type SceneSetup = (ResidentScene, [AtlasId; 4]);

const fn change(
    player: Player,
    control: Control,
    down: bool,
) -> LogicalAction<Action> {
    LogicalAction {
        action: Action { control, player },
        down,
    }
}

fn scenario() -> InputScript<LogicalAction<Action>> {
    let mut script = InputScript::new();
    for (tick, player) in (0u64..).zip(Player::ALL) {
        script.record(tick, change(player, Control::Start, true));
        script.record(tick, change(player, Control::Start, false));
        script.record(4, change(player, Control::Right, true));
        script.record(12, change(player, Control::Right, false));
    }
    script
}

fn complete(result: Result<(), RenderError>) {
    match result {
        Ok(()) => {
            let _reported =
                write_status(format_args!("browser-frame-smoke: passed"));
        },
        Err(error) => {
            let _reported = write_diagnostic(format_args!(
                "browser-frame-smoke: failed: {error:?}"
            ));
        },
    }
}

fn scene() -> Result<SceneSetup, RenderError> {
    let width = NonZeroU32::new(320).ok_or(RenderError::InvalidDimensions)?;
    let height = NonZeroU32::new(224).ok_or(RenderError::InvalidDimensions)?;
    let capacity =
        NonZeroUsize::new(320 * 224 * 4).ok_or(RenderError::InvalidLength)?;
    let layout = FrameLayout::new(width, height, capacity)?;
    let four = NonZeroUsize::new(4).ok_or(RenderError::InvalidLength)?;
    let mut scene = NativeWindow::open("Frame smoke", width, height, false)?
        .into_scene(layout, SceneLimits {
            atlas_bytes: NonZeroUsize::new(16)
                .ok_or(RenderError::InvalidLength)?,
            atlases: four,
            sprites: four,
        })?;
    let pixel = FrameLayout::new(NonZeroU32::MIN, NonZeroU32::MIN, four)?;
    let atlases = [
        scene.load_atlas(pixel, &[255, 0, 0, 255])?,
        scene.load_atlas(pixel, &[0, 255, 0, 255])?,
        scene.load_atlas(pixel, &[0, 0, 255, 255])?,
        scene.load_atlas(pixel, &[255, 255, 0, 255])?,
    ];
    Ok((scene, atlases))
}

fn draw(
    stations: &[Station; 4],
    atlases: [AtlasId; 4],
    scene: &mut SceneSession<'_, '_>,
) -> Result<(), RenderError> {
    let eight = NonZeroU32::new(8).ok_or(RenderError::InvalidDimensions)?;
    let source = SpriteRect {
        x: 0,
        y: 0,
        width: NonZeroU32::MIN,
        height: NonZeroU32::MIN,
    };
    let sprites: Vec<_> = stations
        .iter()
        .zip(atlases)
        .filter(|(station, _atlas)| {
            station.participation == Participation::Playing
        })
        .map(|(station, atlas)| SpriteDraw {
            atlas,
            source,
            destination: SpriteRect {
                x: i32::from(station.position.x),
                y: i32::from(station.position.y),
                width: eight,
                height: eight,
            },
            flip_horizontal: false,
            flip_vertical: false,
        })
        .collect();
    if !scene.present([0; 3], &sprites)? {
        return Err(RenderError::InvalidDimensions);
    }
    Ok(())
}

fn verify(stations: &[Station; 4]) -> Result<(), RenderError> {
    for (station, spawn) in stations.iter().zip(StreetLayout::DOWNTOWN.spawn) {
        if station.participation != Participation::Playing
            || station.position.x != spawn.x.saturating_add(16)
            || station.position.y != spawn.y
        {
            return Err(RenderError::InvalidDimensions);
        }
    }
    Ok(())
}

fn main() -> Result<(), RenderError> {
    let cabinet = Cabinet::new(StreetLayout::DOWNTOWN)
        .map_err(|_error| RenderError::InvalidDimensions)?;
    let mut simulation = Scripted::new(cabinet, scenario());
    let mut clock = StepClock::new(TICK_INTERVAL, NonZeroU32::MAX)
        .map_err(|_error| RenderError::InvalidTimestamp)?;
    let (mut scene, atlases) = scene()?;
    let mut last = None::<Duration>;
    let mut remaining = TICK_INTERVAL.saturating_mul(16);
    frame_loop::start(
        move |now| {
            let elapsed = now.saturating_sub(last.replace(now).unwrap_or(now));
            let delta = elapsed.min(remaining);
            remaining = remaining.saturating_sub(delta);
            let report = clock
                .advance(delta, &mut simulation)
                .map_err(|_error| RenderError::InvalidTimestamp)?;
            let stations = simulation.state().stations();
            draw(&stations, atlases, &mut scene)?;
            if report.next_tick < 16 {
                return Ok(true);
            }
            verify(&stations)?;
            Ok(false)
        },
        complete,
    )
}
