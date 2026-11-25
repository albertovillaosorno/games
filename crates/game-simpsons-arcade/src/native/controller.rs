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
//   - Inputs: Approved TLS connection and native keyboard, controller or touch
//     events.
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

use std::ffi::OsString;
use std::io::ErrorKind;
use std::time::Duration;
use std::{mem, thread};

use games::input::InputEvent;
use games::input::physical::ControllerAxis;
use games_platform_graphics::frame::{FrameLayout, RenderError};
use games_platform_graphics::linux::NativeWindow;
use games_platform_graphics::linux::input::{
    HostEvent, InputStartup, NativeInput,
};
use games_platform_graphics::linux::sprites::SceneSession;
use games_platform_graphics::linux::ui::{PlateLayout, TextAtlas, TextLine};
use games_platform_graphics::scene::{EventBatch, SceneLimits, SpriteDraw};

use super::launch::ControllerLaunch;
use super::{
    CONTROLS, Controls, EVENT_BUDGET, LocalActions, Profiles, dimension,
    failure, key_label, lan, project_contact, size, touch,
};
use crate::Control;
use crate::lan::ClientLan;
use crate::remote::RemoteControl;

const WIDTH: u16 = 320;
const HEIGHT: u16 = touch::PANEL_TOP;
const MAX_BYTES: usize = 389_120;

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

const fn remote_label(control: Control) -> Option<&'static str> {
    match control {
        Control::Pause => None,
        Control::Start => Some("JOIN"),
        Control::Primary => Some("ATTACK"),
        Control::Secondary => Some("JUMP"),
        Control::Left => Some("LEFT"),
        Control::Right => Some("RIGHT"),
        Control::Up => Some("UP"),
        Control::Down => Some("DOWN"),
    }
}

fn binding_hints(profiles: &Profiles) -> Vec<String> {
    let [keys, ..] = profiles.keys;
    let [buttons, ..] = profiles.buttons;
    let [mouse_bindings, ..] = profiles.mouse;
    CONTROLS
        .into_iter()
        .zip(keys.into_iter().zip(buttons).zip(mouse_bindings))
        .filter_map(|(control, ((key, button), mouse))| {
            let label = remote_label(control)?;
            let key_name = key.map_or("NONE", key_label);
            let mouse_name = mouse.map_or_else(String::new, |binding| {
                format!(" / MOUSE {}", super::mouse_label(binding))
            });
            Some(format!(
                "{label}: {key_name}{mouse_name} / {}",
                button.profile_name()
            ))
        })
        .collect()
}

fn axis_label(slot: Option<usize>) -> Option<&'static str> {
    slot.and_then(|index| CONTROLS.get(index).copied())
        .and_then(remote_label)
}

fn axis_hints(profiles: &Profiles) -> Vec<String> {
    let [axes, ..] = profiles.axes;
    let mut hints: Vec<_> = ControllerAxis::ALL
        .into_iter()
        .zip(axes)
        .filter_map(|(axis, profile)| {
            let negative = axis_label(profile.negative);
            let positive = axis_label(profile.positive);
            if negative.is_none() && positive.is_none() {
                return None;
            }
            Some(format!(
                "{}: -{} +{}",
                axis.profile_name(),
                negative.unwrap_or("NONE"),
                positive.unwrap_or("NONE"),
            ))
        })
        .collect();
    if hints.is_empty() {
        hints.push(String::from("NO REMOTE AXIS BINDINGS"));
    }
    hints
}

fn panel(
    scene: &mut SceneSession<'_, '_>,
    profiles: &Profiles,
    touch_enabled: bool,
) -> Result<SpriteDraw, RenderError> {
    let bindings = binding_hints(profiles);
    let axes = axis_hints(profiles);
    let lines = [
        ("REMOTE CONTROLLER", 8, 3),
        ("WATCH THE HOST SCREEN", 25, 2),
    ]
    .into_iter()
    .chain(
        [42, 54, 66, 78, 90, 102, 114]
            .into_iter()
            .zip(&bindings)
            .map(|(top, hint)| (hint.as_str(), top, 2)),
    )
    .chain(
        [128, 140, 152, 164, 176, 188]
            .into_iter()
            .zip(&axes)
            .map(|(top, hint)| (hint.as_str(), top, 2)),
    )
    .chain([("CLOSE WINDOW TO LEAVE", 201, 2)])
    .chain(touch_enabled.then_some((
        "TOUCH BELOW - DRAG OFF TO RELEASE",
        213,
        2,
    )))
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
            max_bytes: size(MAX_BYTES)?,
            width: WIDTH,
        },
        &[lines],
    )?;
    atlas.draw(0, 0, 0)
}

fn controller_controls(
    profiles: &Profiles,
    touch_enabled: bool,
) -> Result<Controls, RenderError> {
    Controls::with_panel(
        touch_enabled.then_some(touch::PanelRole::Controller),
        profiles,
        [false, true, true, true],
    )
}

fn controller_draws(
    scene: &mut SceneSession<'_, '_>,
    profiles: &Profiles,
    touch_enabled: bool,
) -> Result<Vec<SpriteDraw>, RenderError> {
    let mut draws = vec![panel(scene, profiles, touch_enabled)?];
    if touch_enabled {
        draws.push(touch::load(scene, profiles, touch::PanelRole::Controller)?);
    }
    Ok(draws)
}

fn controller_layout(
    profiles: &Profiles,
    touch_enabled: bool,
) -> Result<FrameLayout, RenderError> {
    let height = if touch_enabled {
        touch::height(profiles)?
    } else {
        HEIGHT
    };
    FrameLayout::new(
        dimension(u32::from(WIDTH))?,
        dimension(u32::from(height))?,
        size(MAX_BYTES)?,
    )
}

fn controller_limits(
    layout: FrameLayout,
    touch_enabled: bool,
) -> Result<SceneLimits, RenderError> {
    let count = size(if touch_enabled {
        2
    } else {
        1
    })?;
    Ok(SceneLimits {
        atlas_bytes: size(layout.bytes())?,
        atlases: count,
        sprites: count,
    })
}

fn project_event(
    scene: &SceneSession<'_, '_>,
    event: Option<HostEvent>,
) -> Result<Option<HostEvent>, RenderError> {
    match event {
        Some(HostEvent::Input(change)) => {
            Ok(project_contact(scene, change)?.map(HostEvent::Input))
        },
        other => Ok(other),
    }
}

fn route_event(
    event: Option<HostEvent>,
    controls: &mut Controls,
    controller: &mut ClientLan,
    deferred: &mut Vec<RemoteControl>,
) -> Result<bool, RenderError> {
    let change = match event {
        Some(HostEvent::Quit) => return Ok(false),
        Some(HostEvent::Input(change)) => change,
        None => return Ok(true),
    };
    *deferred = enqueue(controller, controls.actions(change)?)
        .err()
        .unwrap_or_default();
    Ok(deferred.is_empty())
}

fn discover_controllers(
    input: &mut NativeInput,
    controls: &mut Controls,
    controller: &mut ClientLan,
) -> Result<(), RenderError> {
    for attached in input.discover_after_drain()? {
        enqueue(controller, controls.actions(attached)?)
            .map_err(|_rejected| connection_failure(ErrorKind::WouldBlock))?;
    }
    Ok(())
}

fn interactive(
    scene: &mut SceneSession<'_, '_>,
    startup: InputStartup,
    connection: lan::ControllerConnection,
) -> Result<(), RenderError> {
    let lan::ControllerConnection {
        controls: resolved,
        stream,
        touch_enabled,
    } = connection;
    let mut controller = ClientLan::new(stream);
    let mut input = startup.input;
    let mut controls = controller_controls(&resolved.profiles, touch_enabled)?;
    if touch_enabled {
        input.enable_touch();
    }
    for event in startup.attached {
        enqueue(&mut controller, controls.actions(event)?)
            .map_err(|_rejected| connection_failure(ErrorKind::WouldBlock))?;
    }
    let draws = controller_draws(scene, &resolved.profiles, touch_enabled)?;
    let _initial_drawn = scene.present([24, 22, 30], &draws)?;
    resolved.persist().map_err(failure)?;
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
        if input.synchronize_touch(scene)?.is_some()
            && !route_event(
                Some(HostEvent::Input(InputEvent::TouchCancelled)),
                &mut controls,
                &mut controller,
                &mut deferred,
            )?
        {
            continue;
        }
        let batch =
            scene.dispatch_events(size(EVENT_BUDGET)?, 8, |view, native| {
                route_event(
                    project_event(view, input.translate(native))?,
                    &mut controls,
                    &mut controller,
                    &mut deferred,
                )
            })?;
        if batch == EventBatch::Stopped && deferred.is_empty() {
            return Ok(());
        }
        if matches!(batch, EventBatch::Empty | EventBatch::Drained) {
            discover_controllers(&mut input, &mut controls, &mut controller)?;
        }
        if batch != EventBatch::Empty || !scene.presentation_is_current()? {
            let _drawn = scene.present([24, 22, 30], &draws)?;
        }
    }
}

pub(super) fn run(args: Vec<OsString>) -> Result<(), String> {
    let connection = lan::open_client(ControllerLaunch::parse(args)?)?;
    let touch_enabled = connection.touch_enabled;
    let result = (|| {
        let layout =
            controller_layout(&connection.controls.profiles, touch_enabled)?;
        let mut window = NativeWindow::open(
            "The Simpsons Arcade - remote controller",
            dimension(layout.width().get().saturating_mul(2))?,
            dimension(layout.height().get().saturating_mul(2))?,
            false,
        )?;
        let startup = window.input(size(1)?)?;
        let limits = controller_limits(layout, touch_enabled)?;
        window.with_scene(layout, limits, |scene| {
            interactive(scene, startup, connection)
        })
    })();
    result.map_err(|error: RenderError| format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use std::env;

    use games::input::physical::{ControllerButton, KeyboardKey};
    use games::input::touch::{ContactId, ContactPhase};

    use super::super::{DEFAULT_PROFILES, configured_profiles};
    use super::*;
    use crate::Player;

    fn remapped_profiles() -> Result<Profiles, RenderError> {
        configured_profiles(concat!(
            "version=1\nkey 1 primary=V\n",
            "button 1 primary=x\nkey 1 right=H\n",
        ))
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
        if !hints
            .iter()
            .any(|line| line == "ATTACK: V / MOUSE LEFT / x")
            || !hints.iter().any(|line| line == "RIGHT: H / dpright")
        {
            return Err(failure("controller hints ignored profile"));
        }
        let mut controls = Controls::with_reserved(false, &profiles, [
            false, true, true, true,
        ])?;
        for key in [
            KeyboardKey::LETTER_Z,
            KeyboardKey::LETTER_D,
            KeyboardKey::ESCAPE,
        ] {
            check_edge(
                &mut controls,
                InputEvent::Key {
                    code: key.code(),
                    down: true,
                },
                None,
            )?;
        }
        check_edge(
            &mut controls,
            InputEvent::Key {
                code: KeyboardKey::LETTER_H.code(),
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
                code: ControllerButton::West.code(),
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
    fn axis_profiles_drive_remote_control_edges() -> Result<(), RenderError> {
        let profiles = configured_profiles(concat!(
            "version=1\naxis 1 leftx=none/none\n",
            "axis 1 rightx=left/right\n",
            "axis-threshold 1 rightx=20000/5000\n",
        ))
        .map_err(failure)?;
        if axis_hints(&profiles) != ["lefty: -UP +DOWN", "rightx: -LEFT +RIGHT"]
        {
            return Err(failure("controller hint ignored custom axes"));
        }
        let mut controls = Controls::with_reserved(false, &profiles, [
            false, true, true, true,
        ])?;
        check_edge(&mut controls, InputEvent::Attached { device: 1 }, None)?;
        for (axis, value, expected) in [
            (ControllerAxis::LeftX, 32_767, None),
            (ControllerAxis::RightX, 19_999, None),
            (
                ControllerAxis::RightX,
                20_000,
                RemoteControl::new(Control::Right, true),
            ),
            (ControllerAxis::RightX, 5_001, None),
            (
                ControllerAxis::RightX,
                5_000,
                RemoteControl::new(Control::Right, false),
            ),
        ] {
            check_edge(
                &mut controls,
                InputEvent::Axis {
                    device: 1,
                    code: axis.code(),
                    value,
                },
                expected,
            )?;
        }
        Ok(())
    }

    fn all_axis_profiles() -> Result<Profiles, RenderError> {
        configured_profiles(concat!(
            "version=1\n",
            "axis 1 leftx=primary/pause\n",
            "axis 1 lefty=pause/secondary\n",
            "axis 1 rightx=up/down\n",
            "axis 1 righty=start/left\n",
            "axis 1 lefttrigger=none/primary\n",
            "axis 1 righttrigger=none/secondary\n",
        ))
        .map_err(failure)
    }

    #[test]
    fn axis_hints_cover_remote_assignments() -> Result<(), RenderError> {
        if axis_hints(&DEFAULT_PROFILES)
            != ["leftx: -LEFT +RIGHT", "lefty: -UP +DOWN"]
        {
            return Err(failure("default axis hints changed"));
        }
        let profiles = all_axis_profiles()?;
        let hints = axis_hints(&profiles);
        if hints
            != [
                "leftx: -ATTACK +NONE",
                "lefty: -NONE +JUMP",
                "rightx: -UP +DOWN",
                "righty: -JOIN +LEFT",
                "lefttrigger: -NONE +ATTACK",
                "righttrigger: -NONE +JUMP",
            ]
        {
            return Err(failure("incorrect effective axis hints"));
        }
        if !hints.iter().all(|hint| {
            TextLine {
                color: [245, 240, 220, 255],
                scale: 2,
                text: hint,
                top: 0,
            }
            .fit_width(WIDTH)
            .scale
                == 2
        }) {
            return Err(failure("axis hints require reduced text scale"));
        }
        let disabled = configured_profiles(concat!(
            "version=1\n",
            "axis 1 leftx=pause/pause\n",
            "axis 1 lefty=none/none\n",
            "axis 1 rightx=none/none\n",
            "axis 1 righty=none/none\n",
            "axis 1 lefttrigger=none/none\n",
            "axis 1 righttrigger=none/pause\n",
        ))
        .map_err(failure)?;
        if axis_hints(&disabled) != ["NO REMOTE AXIS BINDINGS"] {
            return Err(failure("inactive axis hint advertises a control"));
        }
        Ok(())
    }

    #[test]
    fn touch_controller_routes_and_releases() -> Result<(), RenderError> {
        let profiles = configured_profiles(
            "version=1\ntouch 1 primary=secondary\ntouch 1 secondary=primary\n",
        )
        .map_err(failure)?;
        let mut controls = controller_controls(&profiles, true)?;
        let right = ContactId { device: 1, finger: 1 };
        let attack = ContactId { device: 1, finger: 2 };
        for (id, control) in
            [(right, Control::Right), (attack, Control::Primary)]
        {
            check_edge(
                &mut controls,
                InputEvent::Contact {
                    id,
                    phase: ContactPhase::Down(touch::center(
                        Player::Marge,
                        control,
                        &profiles,
                    )?),
                },
                RemoteControl::new(control, true),
            )?;
        }
        check_edge(
            &mut controls,
            InputEvent::Contact {
                id: right,
                phase: ContactPhase::Motion(None),
            },
            RemoteControl::new(Control::Right, false),
        )?;
        check_edge(
            &mut controls,
            InputEvent::FocusLost,
            RemoteControl::new(Control::Primary, false),
        )
    }

    #[test]
    fn touch_controller_omits_pause_and_requires_opt_in()
    -> Result<(), RenderError> {
        let id = ContactId { device: 1, finger: 1 };
        for (enabled, control) in
            [(true, Control::Pause), (false, Control::Start)]
        {
            let mut controls = controller_controls(&DEFAULT_PROFILES, enabled)?;
            let phase = ContactPhase::Down(touch::center(
                Player::Marge,
                control,
                &DEFAULT_PROFILES,
            )?);
            if !controls
                .actions(InputEvent::Contact { id, phase })?
                .is_empty()
            {
                return Err(failure("unexpected touch action"));
            }
        }
        Ok(())
    }

    fn verify_axis_rows(pixels: &[u8]) -> Result<(), RenderError> {
        for top in [128usize, 140, 152, 164, 176, 188] {
            let width = usize::from(WIDTH).saturating_mul(4);
            let visible =
                pixels.chunks_exact(width).skip(top).take(10).any(|row| {
                    row.as_chunks::<4>().0.contains(&[245, 240, 220, 255])
                });
            if !visible {
                return Err(failure("controller axis row missing"));
            }
        }
        Ok(())
    }

    fn verify_touch_controller() -> Result<(), RenderError> {
        let profiles = all_axis_profiles()?;
        let layout = controller_layout(&profiles, true)?;
        let mut window = NativeWindow::open(
            "Touch controller smoke",
            layout.width(),
            layout.height(),
            true,
        )?;
        window.with_scene(layout, controller_limits(layout, true)?, |scene| {
            let draws = controller_draws(scene, &profiles, true)?;
            if !scene.present([24, 22, 30], &draws)? {
                return Err(failure("touch controller not drawn"));
            }
            let pixels = scene.readback()?;
            verify_axis_rows(&pixels)?;
            if !pixels.as_chunks::<4>().0.contains(&[242, 245, 250, 255]) {
                return Err(failure("touch controller symbols missing"));
            }
            let logical =
                touch::center(Player::Marge, Control::Start, &profiles)?;
            let point = scene
                .surface_touch(logical)?
                .ok_or(RenderError::InvalidDimensions)?;
            let id = ContactId { device: 1, finger: 1 };
            let event = project_contact(scene, InputEvent::Contact {
                id,
                phase: ContactPhase::Down(point),
            })?
            .ok_or(RenderError::InvalidDimensions)?;
            let mut controls = controller_controls(&profiles, true)?;
            check_edge(
                &mut controls,
                event,
                RemoteControl::new(Control::Start, true),
            )
        })
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
            size(MAX_BYTES)?,
        )?;
        window.with_scene(
            layout,
            SceneLimits {
                atlas_bytes: size(MAX_BYTES)?,
                atlases: size(1)?,
                sprites: size(1)?,
            },
            |scene| {
                let sprite = panel(scene, &all_axis_profiles()?, false)?;
                if !scene.present([24, 22, 30], &[sprite])? {
                    return Err(failure("controller panel not drawn"));
                }
                verify_axis_rows(&scene.readback()?)
            },
        )?;
        drop(window);
        verify_touch_controller()?;
        super::super::station_hud::tests::remote_station_indicators_smoke()
    }
}
