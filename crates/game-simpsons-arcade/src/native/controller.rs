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
//   - Native shared-screen controller window and bounded outgoing input.
// - Must-Not:
//   - Select a host seat, simulate gameplay or replicate original art.
// - Allows:
//   - Inputs: Approved TLS connection and native keyboard/controller events.
//   - Outputs: Ordered seat-independent digital control transitions.
//   - Side effects: Small native window and bounded nonblocking LAN polling.
// - Split-When:
//   - A client displays independently replicated gameplay state.
// - Merge-When:
//   - A reusable shared-screen controller owns equivalent input composition.
// - Summary:
//   - Control one host-assigned seat without loading any game assets.
// - Description:
//   - Reuse the local input router, discard only the host-owned Pause action.
// - Usage:
//   - Explicitly select a controller catalogue and watch the host screen.
// - Defaults:
//   - No discovery, automatic connections, account service or world display.

use std::path::Path;

use games::input::InputEvent;
use games_platform_graphics::linux::input::{HostEvent, InputStartup};
use games_platform_graphics::linux::sprites::{
    SceneLimits, SceneSession, SpriteDraw,
};
use games_platform_graphics::linux::ui::{PlateLayout, TextAtlas, TextLine};
use games_platform_graphics::linux::{FrameLayout, NativeWindow, RenderError};

use super::{
    Controls, DEFAULT_PROFILES, LocalActions, dimension, failure, lan, size,
};
use crate::lan::ClientLan;
use crate::remote::RemoteControl;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 120;
const BYTES: usize = 153_600;

fn enqueue(
    controller: &mut ClientLan,
    actions: LocalActions,
) -> Result<(), RenderError> {
    for value in actions.into_iter().filter_map(|change| {
        RemoteControl::new(change.action.control, change.down)
    }) {
        controller.enqueue(value).map_err(failure)?;
    }
    Ok(())
}

fn panel(scene: &mut SceneSession<'_, '_>) -> Result<SpriteDraw, RenderError> {
    let lines = [
        ("REMOTE CONTROLLER", 8u16, 3u16),
        ("WATCH THE HOST SCREEN", 30, 2),
        ("ENTER OR START TO JOIN", 48, 2),
        ("Z / A ATTACK    X / B JUMP", 64, 2),
        ("ARROW KEYS OR GAMEPAD", 80, 2),
        ("CLOSE WINDOW TO LEAVE", 100, 2),
    ]
    .into_iter()
    .map(|(text, top, scale)| {
        TextLine {
            color: [245, 240, 220, 255],
            scale,
            text,
            top,
        }
        .fit_width(WIDTH)
    })
    .collect();
    let atlas = TextAtlas::load(
        scene,
        PlateLayout {
            background: [24, 22, 30, 255],
            height: HEIGHT,
            max_bytes: size(BYTES)?,
            width: WIDTH,
        },
        &[lines],
    )?;
    atlas.draw(0, 0, 0)
}

fn interactive(
    scene: &mut SceneSession<'_, '_>,
    startup: InputStartup,
    mut controller: ClientLan,
) -> Result<(), RenderError> {
    let mut input = startup.input;
    let mut controls = Controls::with_reserved(false, &DEFAULT_PROFILES, [
        false, true, true, true,
    ])?;
    for event in startup.attached {
        enqueue(&mut controller, controls.actions(event)?)?;
    }
    let sprite = panel(scene)?;
    let _initial = scene.present([24, 22, 30], &[sprite])?;
    loop {
        controller.poll().map_err(failure)?;
        let mut event = scene.wait_event_timeout(8);
        let redraw = event.is_some();
        let mut removed = false;
        while let Some(native) = event {
            match input.translate(&native)? {
                Some(HostEvent::Quit) => return Ok(()),
                Some(HostEvent::Input(change)) => {
                    removed |= matches!(change, InputEvent::Detached { .. });
                    enqueue(&mut controller, controls.actions(change)?)?;
                },
                None => {},
            }
            event = scene.poll_event();
        }
        if removed {
            for attached in input.discover_available()? {
                enqueue(&mut controller, controls.actions(attached)?)?;
            }
        }
        if redraw {
            let _drawn = scene.present([24, 22, 30], &[sprite])?;
        }
    }
}

pub(super) fn run(path: &Path) -> Result<(), String> {
    let stream = lan::open_client(path)?;
    let controller = ClientLan::new(stream);
    let result = (|| {
        let mut window = NativeWindow::open(
            "The Simpsons Arcade - remote controller",
            dimension(640)?,
            dimension(240)?,
            false,
        )?;
        let startup = window.input(size(1)?)?;
        let layout = FrameLayout::new(
            dimension(u32::from(WIDTH))?,
            dimension(u32::from(HEIGHT))?,
            size(BYTES)?,
        )?;
        let limits = SceneLimits {
            atlas_bytes: size(BYTES)?,
            atlases: size(1)?,
            sprites: size(1)?,
        };
        window.with_scene(layout, limits, |scene| {
            interactive(scene, startup, controller)
        })
    })();
    result.map_err(|error: RenderError| format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn opt_in_native_panels_smoke() -> Result<(), RenderError> {
        if env::var_os("GAMES_NATIVE_RENDER_SMOKE").is_none() {
            return Ok(());
        }
        let mut window = NativeWindow::open(
            "Controller panel smoke",
            dimension(u32::from(WIDTH))?,
            dimension(u32::from(HEIGHT))?,
            true,
        )?;
        let layout = FrameLayout::new(
            dimension(u32::from(WIDTH))?,
            dimension(u32::from(HEIGHT))?,
            size(BYTES)?,
        )?;
        window.with_scene(
            layout,
            SceneLimits {
                atlas_bytes: size(BYTES)?,
                atlases: size(1)?,
                sprites: size(1)?,
            },
            |scene| {
                let sprite = panel(scene)?;
                if !scene.present([24, 22, 30], &[sprite])? {
                    return Err(failure("controller panel not drawn"));
                }
                let pixels = scene.readback()?;
                if !pixels.as_chunks::<4>().0.contains(&[245, 240, 220, 255]) {
                    return Err(failure("controller panel text missing"));
                }
                Ok(())
            },
        )?;
        drop(window);
        super::super::station_hud::tests::remote_station_indicators_smoke()
    }
}
