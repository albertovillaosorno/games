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
//   - Title art admission, station control mapping and native composition.
// - Must-Not:
//   - Read original media or derive simulation timing from render rate.
// - Allows:
//   - Inputs: Pinned canonical RGBA bundles and native input events.
//   - Outputs: Fixed-step movement and GPU-composed title preview.
//   - Side effects: Native presentation and bounded local bundle reads.
// - Split-When:
//   - Additional platform adapters require independent composition.
// - Merge-When:
//   - Host code duplicates shared platform or simulation mechanisms.
// - Summary:
//   - Interactive initial street preview with four independent stations.
// - Description:
//   - Uses isolated original poses; no combat or animation parity claim.
// - Usage:
//   - Provide four local actor bundle hashes in cabinet order.
// - Defaults:
//   - Paired keyboard or controller controls and uncapped presentation.

//! Native initial street preview using only verified canonical art.

mod touch;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Debug;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::Path;
use std::time::Duration;

use games::input::touch::{ContactId, ContactPhase};
use games::input::{AxisBinding, InputEvent, InputMapping, InputRouter};
use games::replay::{LiveInputLimits, LiveSimulation, QueueRejection};
use games::timing::StepClock;
use games_assets::{AssetFormat, ContentHash, ManifestLimits};
use games_platform_graphics::linux::input::{
    Axis, Button, HostEvent, NativeInput, Scancode, axis_code, button_code,
    key_code,
};
use games_platform_graphics::linux::sprites::{
    AtlasId, SceneLimits, SceneSession, SpriteDraw, SpriteRect,
};
use games_platform_graphics::linux::{FrameLayout, NativeWindow, RenderError};
use games_platform_storage::bundle::CanonicalBundle;

use super::{
    Action, Cabinet, Control, Facing, Maneuver, Participation, Phase, Player,
    Station, StreetLayout, TICK_INTERVAL,
};

const SOURCE_SHA: &str =
    "c88e549ab8f039ec3079b34bc39af6738379d306ca06cf81123f4d3501cf8dfa";
const KEYS: [&str; 4] =
    ["actor/marge", "actor/homer", "actor/bart", "actor/lisa"];
const CONTROLS: [Control; 8] = [
    Control::Coin,
    Control::Start,
    Control::Primary,
    Control::Secondary,
    Control::Left,
    Control::Right,
    Control::Up,
    Control::Down,
];
const KEYBOARD: [[Scancode; 8]; 4] = [
    [
        Scancode::Num1,
        Scancode::Num5,
        Scancode::Z,
        Scancode::X,
        Scancode::Left,
        Scancode::Right,
        Scancode::Up,
        Scancode::Down,
    ],
    [
        Scancode::Num2,
        Scancode::Num6,
        Scancode::F,
        Scancode::G,
        Scancode::A,
        Scancode::D,
        Scancode::W,
        Scancode::S,
    ],
    [
        Scancode::Num3,
        Scancode::Num7,
        Scancode::O,
        Scancode::P,
        Scancode::J,
        Scancode::L,
        Scancode::I,
        Scancode::K,
    ],
    [
        Scancode::Num4,
        Scancode::Num8,
        Scancode::Kp1,
        Scancode::Kp2,
        Scancode::Kp4,
        Scancode::Kp6,
        Scancode::Kp8,
        Scancode::Kp5,
    ],
];
const BUTTONS: [Button; 8] = [
    Button::Back,
    Button::Start,
    Button::A,
    Button::B,
    Button::DPadLeft,
    Button::DPadRight,
    Button::DPadUp,
    Button::DPadDown,
];

#[derive(Debug)]
struct Controls {
    devices: [Option<u64>; 4],
    router: InputRouter<Action>,
}

impl Controls {
    fn attach(&mut self, device: u64) -> Result<(), RenderError> {
        if self.devices.contains(&Some(device)) {
            return Ok(());
        }
        let Some(index) = self.devices.iter().position(Option::is_none) else {
            return Ok(());
        };
        let player =
            *Player::ALL.get(index).ok_or(RenderError::InvalidSprite)?;
        let buttons = BUTTONS
            .into_iter()
            .zip(CONTROLS)
            .map(|(button, control)| {
                (button_code(button), Action { control, player })
            })
            .collect();
        let axes = controller_axes(player)?;
        let releases = self
            .router
            .set_controller_controls(device, buttons, axes)
            .ok_or_else(|| failure("controller attachment not routed"))?;
        if !releases.is_empty() {
            return Err(failure("new controller had unexpected holds"));
        }
        *self
            .devices
            .get_mut(index)
            .ok_or(RenderError::InvalidSprite)? = Some(device);
        Ok(())
    }

    fn detach(&mut self, device: u64) {
        if let Some(slot) =
            self.devices.iter_mut().find(|slot| **slot == Some(device))
        {
            *slot = None;
        }
    }

    fn new(touch_enabled: bool) -> Result<Self, RenderError> {
        let mut mapping = InputMapping::new();
        for (player, keys) in Player::ALL.into_iter().zip(KEYBOARD) {
            for (key, control) in keys.into_iter().zip(CONTROLS) {
                mapping.bind_key(key_code(key), Action { control, player });
            }
        }
        if touch_enabled {
            touch::install(&mut mapping)?;
        }
        Ok(Self {
            devices: [None; 4],
            router: InputRouter::new(mapping),
        })
    }

    fn route(
        &mut self,
        event: InputEvent,
        elapsed: Duration,
        live: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
        let actions = self.router.handle(event);
        if let InputEvent::Attached { device } = event {
            self.attach(device)?;
        } else if let InputEvent::Detached { device } = event {
            self.detach(device);
        }
        if actions.is_empty() {
            return Ok(());
        }
        if let Err(rejected) = live.enqueue_at(elapsed, actions) {
            if rejected.reason != QueueRejection::Late {
                return Err(failure(rejected.reason));
            }
            live.enqueue(live.next_tick(), rejected.actions)
                .map_err(|error| failure(error.reason))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Art {
    atlas: AtlasId,
    rect: SpriteRect,
}

impl Art {
    const fn draw(self, destination: SpriteRect, flip: bool) -> SpriteDraw {
        SpriteDraw {
            atlas: self.atlas,
            destination,
            flip_horizontal: flip,
            flip_vertical: false,
            source: self.rect,
        }
    }

    fn load(
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
        key: &str,
    ) -> Result<Self, RenderError> {
        let record = bundle
            .manifest()
            .record(key)
            .ok_or_else(|| failure("missing art record"))?;
        let AssetFormat::Rgba8 { height, width } = record.format else {
            return Err(failure("art must be RGBA8"));
        };
        let layout = FrameLayout::new(width, height, size(1_048_576)?)?;
        let pixels = bundle.read_payload(key).map_err(failure)?;
        let atlas = scene
            .load_atlas(layout, &pixels)
            .map_err(|error| failure(format!("load {key}: {error:?}")))?;
        Ok(Self {
            atlas,
            rect: SpriteRect {
                height,
                width,
                x: 0,
                y: 0,
            },
        })
    }
}

#[derive(Debug)]
struct Scene {
    actors: [Art; 4],
    background: Art,
    draws: Vec<SpriteDraw>,
    panel: Option<SpriteDraw>,
}

impl Scene {
    fn load(
        scene: &mut SceneSession<'_, '_>,
        bundles: &[CanonicalBundle; 4],
        touch_enabled: bool,
    ) -> Result<Self, RenderError> {
        let [first, _, _, _] = bundles;
        let mut background = Art::load(scene, first, "scene/background")?;
        if (background.rect.width.get(), background.rect.height.get())
            != (320, 224)
        {
            return Err(failure("unexpected background dimensions"));
        }
        // The original HUD reflects the captured player, not live state.
        background.rect.y = 28i32;
        background.rect.height = dimension(196)?;
        let mut actors = Vec::new();
        for (bundle, key) in bundles.iter().zip(KEYS) {
            let actor = Art::load(scene, bundle, key)?;
            if actor.rect.width.get() > 128 || actor.rect.height.get() > 128 {
                return Err(failure("unexpected actor extent"));
            }
            actors.push(actor);
        }
        Ok(Self {
            actors: actors
                .try_into()
                .map_err(|_error| failure("actor count"))?,
            background,
            draws: Vec::with_capacity(10),
            panel: if touch_enabled {
                Some(touch::load(scene)?)
            } else {
                None
            },
        })
    }

    fn present(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        cabinet: &Cabinet,
    ) -> Result<bool, RenderError> {
        self.draws.clear();
        self.draws
            .push(self.background.draw(self.background.rect, false));
        let [marge, homer, bart, lisa] = cabinet.stations();
        let mut order = [
            (Player::Marge, marge),
            (Player::Homer, homer),
            (Player::Bart, bart),
            (Player::Lisa, lisa),
        ];
        order.sort_by_key(|(_, station)| station.position.y);
        for (player, station) in order {
            let actor = *self
                .actors
                .get(player.index())
                .ok_or(RenderError::InvalidSprite)?;
            if station.credits > 0
                || station.participation != Participation::Absent
            {
                self.draws.push(actor.draw(indicator(player)?, false));
            }
            if station.participation == Participation::Playing
                && cabinet.phase() == Phase::Street
            {
                self.draws.push(actor.draw(
                    actor_position(actor, station)?,
                    station.facing == Facing::Left,
                ));
            }
        }
        if let Some(panel) = self.panel {
            self.draws.push(panel);
        }
        scene.present([24, 22, 30], &self.draws)
    }
}

type ControllerAxes = BTreeMap<u16, AxisBinding<Action>>;

fn actor_position(
    actor: Art,
    station: Station,
) -> Result<SpriteRect, RenderError> {
    let half = actor
        .rect
        .width
        .get()
        .checked_div(2)
        .ok_or(RenderError::InvalidDimensions)?;
    let horizontal = i32::try_from(half).map_err(failure)?;
    let vertical = i32::try_from(actor.rect.height.get()).map_err(failure)?;
    Ok(SpriteRect {
        x: i32::from(station.position.x).saturating_sub(horizontal),
        y: i32::from(station.position.y)
            .saturating_sub(vertical)
            .saturating_sub(i32::from(station.maneuver.elevation())),
        ..actor.rect
    })
}

fn controller_axes(player: Player) -> Result<ControllerAxes, RenderError> {
    let binding = |negative, positive| {
        AxisBinding::new(
            Some(Action {
                control: negative,
                player,
            }),
            Some(Action {
                control: positive,
                player,
            }),
            16_000,
            10_000,
        )
        .ok_or_else(|| failure("invalid controller thresholds"))
    };
    Ok(BTreeMap::from([
        (
            axis_code(Axis::LeftX),
            binding(Control::Left, Control::Right)?,
        ),
        (axis_code(Axis::LeftY), binding(Control::Up, Control::Down)?),
    ]))
}

fn dimension(value: u32) -> Result<NonZeroU32, RenderError> {
    NonZeroU32::new(value).ok_or(RenderError::InvalidDimensions)
}

fn failure<Error>(error: Error) -> RenderError
where
    Error: Debug,
{
    RenderError::Backend(format!("{error:?}"))
}

fn indicator(player: Player) -> Result<SpriteRect, RenderError> {
    let offset = i32::try_from(player.index()).map_err(failure)?;
    Ok(SpriteRect {
        height: dimension(24)?,
        width: dimension(24)?,
        x: offset.saturating_mul(80).saturating_add(28),
        y: 2,
    })
}

fn live() -> Result<LiveSimulation<Cabinet>, RenderError> {
    let cabinet = Cabinet::new(StreetLayout::DOWNTOWN).map_err(failure)?;
    let clock =
        StepClock::new(TICK_INTERVAL, dimension(8)?).map_err(failure)?;
    Ok(LiveSimulation::new(cabinet, clock, LiveInputLimits {
        future_ticks: 120,
        pending_actions: size(2048)?,
    }))
}

fn open_bundle(
    parent: &Path,
    digest: &OsString,
) -> Result<CanonicalBundle, RenderError> {
    let hash: ContentHash = digest
        .to_str()
        .ok_or_else(|| failure("invalid hash"))?
        .parse()
        .map_err(failure)?;
    let limits = ManifestLimits {
        manifest_bytes: size(32_768)?,
        payload_bytes: NonZeroU64::new(1_048_576)
            .ok_or(RenderError::TooLarge)?,
        records: size(33)?,
        total_bytes: NonZeroU64::new(33_554_432)
            .ok_or(RenderError::TooLarge)?,
    };
    let bundle =
        CanonicalBundle::open(parent, hash, limits).map_err(failure)?;
    let source = bundle.manifest().source();
    if source.title != "simpsons-arcade"
        || source.revision != "world-4p-set-1"
        || source.digest != SOURCE_SHA.parse().map_err(failure)?
    {
        return Err(failure("unexpected source edition"));
    }
    Ok(bundle)
}

fn poll(
    scene: &mut SceneSession<'_, '_>,
    input: &mut NativeInput,
    controls: &mut Controls,
    live: &mut LiveSimulation<Cabinet>,
) -> Result<bool, RenderError> {
    while let Some(event) = scene.poll_event() {
        match input.translate(&event)? {
            Some(HostEvent::Quit) => return Ok(false),
            Some(HostEvent::Input(logical)) => {
                let elapsed = input
                    .event_elapsed(&event)
                    .ok_or_else(|| failure("ambiguous event timestamp"))?;
                if let Some(projected) = project_contact(scene, logical)? {
                    controls.route(projected, elapsed, live)?;
                }
            },
            None => {},
        }
    }
    Ok(true)
}

fn project_contact(
    scene: &SceneSession<'_, '_>,
    event: InputEvent,
) -> Result<Option<InputEvent>, RenderError> {
    let InputEvent::Contact { id, phase } = event else {
        return Ok(Some(event));
    };
    let projected = match phase {
        ContactPhase::Down(point) => {
            let Some(logical) = scene.logical_touch(point)? else {
                return Ok(None);
            };
            ContactPhase::Down(logical)
        },
        ContactPhase::Motion(Some(point)) => {
            ContactPhase::Motion(scene.logical_touch(point)?)
        },
        ContactPhase::Motion(None) | ContactPhase::Up => phase,
    };
    Ok(Some(InputEvent::Contact { id, phase: projected }))
}

/// Launch interactive preview or a hidden finite native smoke.
///
/// # Errors
/// Rejects invalid inputs, missing art, unsupported native device or input.
pub fn run(mut args: Vec<OsString>) -> Result<(), String> {
    let smoke = args.first().is_some_and(|arg| arg == "--smoke");
    if smoke {
        let _flag = args.remove(0);
    }
    let touch_enabled = args.first().is_some_and(|arg| arg == "--touch");
    if touch_enabled {
        let _flag = args.remove(0);
    }
    let [directory, marge, homer, bart, lisa] = <[OsString; 5]>::try_from(args)
        .map_err(|_error| {
            String::from(concat!(
                "usage: simpsons-arcade [--smoke] [--touch] PARENT ",
                "MARGE_SHA HOMER_SHA BART_SHA LISA_SHA"
            ))
        })?;
    let parent = Path::new(&directory);
    let result = [marge, homer, bart, lisa]
        .iter()
        .map(|hash| open_bundle(parent, hash))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|bundles| {
            bundles.try_into().map_err(|_error| failure("bundle count"))
        })
        .and_then(|bundles| window(&bundles, smoke, touch_enabled));
    result.map_err(|error| format!("{error:?}"))
}

fn size(value: usize) -> Result<NonZeroUsize, RenderError> {
    NonZeroUsize::new(value).ok_or(RenderError::TooLarge)
}

fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let mut simulation = live()?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene
        .readback()
        .map_err(|error| failure(format!("initial readback: {error:?}")))?;
    let mut controls = Controls::new(touch_enabled)?;
    if touch_enabled {
        smoke_touch(scene, &mut controls, &mut simulation)?;
    }
    for (player, keys) in Player::ALL.into_iter().zip(KEYBOARD) {
        if touch_enabled && player == Player::Marge {
            continue;
        }
        for key in keys.into_iter().take(3) {
            for down in [true, false] {
                controls.route(
                    InputEvent::Key {
                        code: key_code(key),
                        down,
                    },
                    Duration::ZERO,
                    &mut simulation,
                )?;
            }
        }
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    if simulation
        .state()
        .stations()
        .iter()
        .any(|station| station.participation != Participation::Playing)
    {
        return Err(failure("four-player admission failed"));
    }
    let _after_drawn = art.present(scene, simulation.state())?;
    if before
        == scene
            .readback()
            .map_err(|error| failure(format!("active readback: {error:?}")))?
    {
        return Err(failure("admitted players were not rendered"));
    }
    if touch_enabled {
        smoke_touch_movement(scene, &mut controls, &mut simulation)?;
    }
    smoke_jump(scene, art, &mut controls, &mut simulation)
}

fn smoke_jump(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, ..] = simulation.state().stations();
    let _drawn = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    for down in [true, false] {
        controls.route(
            InputEvent::Key {
                code: key_code(Scancode::X),
                down,
            },
            TICK_INTERVAL.saturating_mul(tick),
            simulation,
        )?;
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [airborne, ..] = simulation.state().stations();
    if airborne.maneuver != (Maneuver::Jump { tick: 0 })
        || airborne.position != before.position
    {
        return Err(failure("jump did not preserve ground position"));
    }
    let _redrawn = art.present(scene, simulation.state())?;
    if pixels == scene.readback().map_err(failure)? {
        return Err(failure("jump not visibly presented"));
    }
    for _ in 0..31u8 {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    }
    let [landed, ..] = simulation.state().stations();
    if landed.maneuver != Maneuver::Idle || landed.position != before.position {
        return Err(failure("jump did not return to ground"));
    }
    Ok(())
}

fn smoke_touch(
    scene: &SceneSession<'_, '_>,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let id = ContactId { device: 1, finger: 1 };
    for control in [Control::Coin, Control::Start, Control::Primary] {
        let point = touch::center(control)?;
        for phase in [ContactPhase::Down(point), ContactPhase::Up] {
            let event =
                project_contact(scene, InputEvent::Contact { id, phase })?
                    .ok_or_else(|| failure("touch smoke outside scene"))?;
            controls.route(event, Duration::ZERO, simulation)?;
        }
    }
    Ok(())
}

fn smoke_touch_movement(
    scene: &SceneSession<'_, '_>,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, _, _, _] = simulation.state().stations();
    let horizontal = ContactId { device: 1, finger: 2 };
    let vertical = ContactId { device: 1, finger: 3 };
    for (id, control) in [(horizontal, Control::Right), (vertical, Control::Up)]
    {
        let event = project_contact(scene, InputEvent::Contact {
            id,
            phase: ContactPhase::Down(touch::center(control)?),
        })?
        .ok_or_else(|| failure("movement touch outside scene"))?;
        controls.route(event, TICK_INTERVAL, simulation)?;
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [moving, _, _, _] = simulation.state().stations();
    if moving.position.x != before.position.x.saturating_add(2)
        || moving.position.y != before.position.y.saturating_sub(2)
    {
        return Err(failure("simultaneous touch movement failed"));
    }
    for id in [horizontal, vertical] {
        controls.route(
            InputEvent::Contact {
                id,
                phase: ContactPhase::Up,
            },
            TICK_INTERVAL.saturating_mul(2),
            simulation,
        )?;
    }
    let _release_step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [stopped, _, _, _] = simulation.state().stations();
    if stopped.position != moving.position {
        return Err(failure("touch release failed"));
    }
    Ok(())
}

fn window(
    bundles: &[CanonicalBundle; 4],
    hidden: bool,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let mut window = NativeWindow::open(
        "The Simpsons Arcade - movement prototype",
        dimension(960)?,
        dimension(if touch_enabled {
            912
        } else {
            672
        })?,
        hidden,
    )?;
    let height = if touch_enabled {
        u32::from(touch::SCENE_HEIGHT)
    } else {
        224
    };
    let layout =
        FrameLayout::new(dimension(320)?, dimension(height)?, size(389_120)?)?;
    let limits = SceneLimits {
        atlas_bytes: size(2_097_152)?,
        atlases: size(if touch_enabled {
            6
        } else {
            5
        })?,
        sprites: size(if touch_enabled {
            10
        } else {
            9
        })?,
    };
    let startup = window.input(size(4)?)?;
    window.with_scene(layout, limits, |scene| {
        let mut art = Scene::load(scene, bundles, touch_enabled)?;
        if hidden {
            return smoke(scene, &mut art, touch_enabled);
        }
        let mut simulation = live()?;
        let mut controls = Controls::new(touch_enabled)?;
        for event in startup.attached {
            controls.route(event, Duration::ZERO, &mut simulation)?;
        }
        let mut input = startup.input;
        if touch_enabled {
            input.enable_touch();
        }
        let mut previous = Duration::ZERO;
        while poll(scene, &mut input, &mut controls, &mut simulation)? {
            let elapsed = input.elapsed();
            let delta = elapsed.saturating_sub(previous);
            previous = elapsed;
            let _advanced = simulation.advance(delta).map_err(failure)?;
            let _drawn = art.present(scene, simulation.state())?;
        }
        Ok(())
    })
}
