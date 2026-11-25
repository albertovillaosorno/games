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

use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;
use std::{mem, thread};

use games_platform_graphics::linux::input::{HostEvent, InputStartup};
use games_platform_graphics::linux::sprites::{
    EventBatch, SceneLimits, SceneSession, SpriteDraw,
};
use games_platform_graphics::linux::ui::{PlateLayout, TextAtlas, TextLine};
use games_platform_graphics::linux::{FrameLayout, NativeWindow, RenderError};

use super::{
    Controls, EVENT_BUDGET, LocalActions, Profiles, dimension, key_label, lan,
    size,
};
use crate::lan::ClientLan;
use crate::remote::RemoteControl;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 176;
const BYTES: usize = 225_280;

fn connection_failure(kind: ErrorKind) -> RenderError {
    let context = if kind == ErrorKind::TimedOut {
        "LAN connection timed out"
    } else if kind == ErrorKind::InvalidData {
        "Invalid LAN frame; use matching host and controller builds"
    } else if kind == ErrorKind::WouldBlock {
        "Remote input queue full"
    } else if matches!(
        kind,
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof
    ) {
        "LAN connection lost"
    } else {
        "LAN controller transport failed"
    };
    RenderError::Backend(format!("{context} ({kind:?}); re-launch to rejoin"))
}

fn enqueue(
    controller: &mut ClientLan,
    actions: LocalActions,
) -> Result<(), Vec<RemoteControl>> {
    controller.enqueue_batch(remote_controls(actions).collect())
}

fn remote_controls(
    actions: LocalActions,
) -> impl Iterator<Item = RemoteControl> {
    actions.into_iter().filter_map(|change| {
        RemoteControl::new(change.action.control, change.down)
    })
}

fn binding_hints(profiles: &Profiles) -> Vec<String> {
    let [keys, ..] = profiles.keys;
    let [buttons, ..] = profiles.buttons;
    ["JOIN", "ATTACK", "JUMP", "LEFT", "RIGHT", "UP", "DOWN"]
        .into_iter()
        .zip(keys.into_iter().zip(buttons).skip(1))
        .map(|(label, (key, button))| {
            format!("{label}: {} / {}", key_label(key), button.string())
        })
        .collect()
}

fn panel(
    scene: &mut SceneSession<'_, '_>,
    profiles: &Profiles,
) -> Result<SpriteDraw, RenderError> {
    let bindings = binding_hints(profiles);
    let lines = [
        ("REMOTE CONTROLLER", 8, 3),
        ("WATCH THE HOST SCREEN", 28, 2),
    ]
    .into_iter()
    .chain(
        [44, 58, 72, 86, 100, 114, 128]
            .into_iter()
            .zip(&bindings)
            .map(|(top, hint)| (hint.as_str(), top, 2)),
    )
    .chain([
        ("LEFT STICK ALSO MOVES", 144, 2),
        ("CLOSE WINDOW TO LEAVE", 160, 2),
    ])
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
    profiles: &Profiles,
) -> Result<(), RenderError> {
    let mut input = startup.input;
    let mut controls =
        Controls::with_reserved(false, profiles, [false, true, true, true])?;
    for event in startup.attached {
        enqueue(&mut controller, controls.actions(event)?)
            .map_err(|_rejected| connection_failure(ErrorKind::WouldBlock))?;
    }
    let sprite = panel(scene, profiles)?;
    let _initial = scene.present([24, 22, 30], &[sprite])?;
    let mut deferred = Vec::new();
    loop {
        controller
            .poll()
            .map_err(|error| connection_failure(error.kind()))?;
        if let Err(rejected) =
            controller.enqueue_batch(mem::take(&mut deferred))
        {
            deferred = rejected;
            // Transport must progress before consuming more native input.
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let batch = scene.dispatch_events(
            size(EVENT_BUDGET)?,
            8,
            |_scene, native| {
                let change = match input.translate(native)? {
                    Some(HostEvent::Quit) => return Ok(false),
                    Some(HostEvent::Input(change)) => change,
                    None => return Ok(true),
                };
                deferred = enqueue(&mut controller, controls.actions(change)?)
                    .err()
                    .unwrap_or_default();
                Ok(deferred.is_empty())
            },
        )?;
        if batch == EventBatch::Stopped && deferred.is_empty() {
            return Ok(());
        }
        if matches!(batch, EventBatch::Empty | EventBatch::Drained) {
            for attached in input.discover_after_drain()? {
                enqueue(&mut controller, controls.actions(attached)?).map_err(
                    |_rejected| connection_failure(ErrorKind::WouldBlock),
                )?;
            }
        }
        if batch != EventBatch::Empty {
            let _drawn = scene.present([24, 22, 30], &[sprite])?;
        }
    }
}

pub(super) fn run(path: &Path) -> Result<(), String> {
    let (stream, profiles) = lan::open_client(path)?;
    let controller = ClientLan::new(stream);
    let result = (|| {
        let mut window = NativeWindow::open(
            "The Simpsons Arcade - remote controller",
            dimension(640)?,
            dimension(352)?,
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
            interactive(scene, startup, controller, &profiles)
        })
    })();
    result.map_err(|error: RenderError| format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use std::env;

    use games::input::InputEvent;
    use games_platform_graphics::linux::input::{
        Button, Scancode, button_code, key_code,
    };

    use super::super::{ACTION_NAMES, DEFAULT_PROFILES, failure};
    use super::*;
    use crate::Control;

    fn remapped_profiles() -> Result<Profiles, RenderError> {
        DEFAULT_PROFILES
            .configured(
                ACTION_NAMES,
                concat!(
                    "version=1\nkey 1 primary=Space\n",
                    "button 1 primary=x\nkey 1 right=H\n",
                ),
            )
            .map_err(failure)
    }

    fn check_edge(
        controls: &mut Controls,
        event: InputEvent,
        expected: Option<RemoteControl>,
    ) -> Result<(), RenderError> {
        if !remote_controls(controls.actions(event)?).eq(expected) {
            return Err(failure("unexpected controller wire transition"));
        }
        Ok(())
    }

    #[test]
    fn runtime_transport_failures_explain_recovery() -> Result<(), RenderError>
    {
        for (kind, expected) in [
            (ErrorKind::TimedOut, "LAN connection timed out"),
            (
                ErrorKind::InvalidData,
                "use matching host and controller builds",
            ),
            (ErrorKind::WouldBlock, "Remote input queue full"),
            (ErrorKind::ConnectionReset, "LAN connection lost"),
            (ErrorKind::Other, "LAN controller transport failed"),
        ] {
            let RenderError::Backend(message) = connection_failure(kind) else {
                return Err(failure("expected a native backend diagnostic"));
            };
            if !message.contains(expected)
                || !message.ends_with("re-launch to rejoin")
            {
                return Err(failure("missing transport recovery guidance"));
            }
        }
        Ok(())
    }

    #[test]
    fn remapped_bindings_match_hints() -> Result<(), RenderError> {
        let profiles = remapped_profiles()?;
        let hints = binding_hints(&profiles);
        if !hints.iter().any(|line| line == "ATTACK: Space / x")
            || !hints.iter().any(|line| line == "RIGHT: H / dpright")
        {
            return Err(failure("controller hints ignored profile"));
        }
        let mut controls = Controls::with_reserved(false, &profiles, [
            false, true, true, true,
        ])?;
        for key in [Scancode::Z, Scancode::Right, Scancode::Escape] {
            check_edge(
                &mut controls,
                InputEvent::Key {
                    code: key_code(key),
                    down: true,
                },
                None,
            )?;
        }
        check_edge(
            &mut controls,
            InputEvent::Key {
                code: key_code(Scancode::H),
                down: true,
            },
            RemoteControl::new(Control::Right, true),
        )?;
        // Losing focus must release the mapped direction.
        check_edge(
            &mut controls,
            InputEvent::FocusLost,
            RemoteControl::new(Control::Right, false),
        )?;
        check_edge(&mut controls, InputEvent::Attached { device: 1 }, None)?;
        check_edge(
            &mut controls,
            InputEvent::Button {
                device: 1,
                code: button_code(Button::X),
                down: true,
            },
            RemoteControl::new(Control::Primary, true),
        )?;
        check_edge(
            &mut controls,
            InputEvent::Detached { device: 1 },
            RemoteControl::new(Control::Primary, false),
        )
    }

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
                let sprite = panel(scene, &remapped_profiles()?)?;
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
