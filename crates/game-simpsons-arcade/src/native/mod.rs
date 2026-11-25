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

mod audio;
mod continue_hud;
mod encounter;
mod glyph;
mod poses;
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
    Point, Station, StreetLayout, TICK_INTERVAL,
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
    combat: Option<encounter::CombatArt>,
    draws: Vec<SpriteDraw>,
    panel: Option<SpriteDraw>,
    poses: poses::PoseArt,
    world: Vec<(i16, SpriteDraw)>,
}

impl Scene {
    fn load(
        scene: &mut SceneSession<'_, '_>,
        source: &Sources<'_>,
        touch_enabled: bool,
    ) -> Result<Self, RenderError> {
        let [first, _, _, _] = &source.actors;
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
        for (bundle, key) in source.actors.iter().zip(KEYS) {
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
            combat: source
                .enemy
                .as_ref()
                .map(|bundles| encounter::CombatArt::load(scene, bundles))
                .transpose()?,
            draws: Vec::with_capacity(if source.enemy.is_some() {
                21
            } else {
                10
            }),
            panel: if touch_enabled {
                Some(touch::load(scene)?)
            } else {
                None
            },
            poses: source.poses.load(source.parent, scene)?,
            world: Vec::with_capacity(5),
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
        self.world.clear();
        for (player, station) in Player::ALL.into_iter().zip(cabinet.stations())
        {
            self.queue_station(player, station, cabinet.phase())?;
        }
        if let (Some(combat), Some(enemy)) = (&self.combat, cabinet.enemy())
            && cabinet.phase() == Phase::Street
            && enemy.health > 0
        {
            self.world
                .push((enemy.position.y, combat.enemy_draw(enemy)?));
        }
        self.world.sort_by_key(|(depth, _)| *depth);
        self.draws.extend(self.world.iter().map(|(_, draw)| *draw));
        if let (Some(combat), Some(enemy)) = (&self.combat, cabinet.enemy())
            && cabinet.phase() == Phase::Street
            && enemy.health > 0
        {
            combat.enemy_health(&mut self.draws, enemy)?;
        }
        if let Some(panel) = self.panel {
            self.draws.push(panel);
        }
        scene.present([24, 22, 30], &self.draws)
    }

    fn queue_station(
        &mut self,
        player: Player,
        station: Station,
        phase: Phase,
    ) -> Result<(), RenderError> {
        if let (Some(combat), Participation::Continuing { ticks_left }) =
            (&self.combat, station.participation)
        {
            self.draws.push(combat.continue_draw(
                player,
                station.credits > 0,
                ticks_left,
            )?);
            return Ok(());
        }
        if let Some(combat) = &self.combat
            && station.participation == Participation::Absent
            && station.credits > 0
        {
            self.draws.push(combat.continue_draw(player, true, 0)?);
            return Ok(());
        }
        let actor = *self
            .actors
            .get(player.index())
            .ok_or(RenderError::InvalidSprite)?;
        if station.credits > 0 || station.participation != Participation::Absent
        {
            self.draws.push(actor.draw(indicator(player)?, false));
        }
        if station.participation == Participation::Playing
            && phase == Phase::Street
        {
            if let Some(combat) = &self.combat {
                combat.station_health(&mut self.draws, player, station)?;
            }
            if station.health > 0 {
                let pose = self.poses.select(player, station).unwrap_or(actor);
                self.world.push((
                    station.position.y,
                    pose.draw(
                        ground_position(
                            pose,
                            station.position,
                            station.maneuver.elevation(),
                        )?,
                        station.facing == Facing::Left,
                    ),
                ));
            }
        }
        Ok(())
    }
}

struct OptionalSources {
    audio: Option<OsString>,
    enemy: Option<[OsString; 2]>,
    poses: poses::Catalogue,
}

struct Sources<'source> {
    actors: [CanonicalBundle; 4],
    audio: Option<audio::Samples>,
    enemy: Option<[CanonicalBundle; 2]>,
    parent: &'source Path,
    poses: poses::Catalogue,
}

impl<'source> Sources<'source> {
    fn load(
        parent: &'source Path,
        actors: &[OsString; 4],
        options: OptionalSources,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            actors: open_bundles(parent, actors)?,
            audio: options
                .audio
                .as_ref()
                .map(|hash| audio::Samples::load(parent, hash))
                .transpose()?,
            enemy: options
                .enemy
                .as_ref()
                .map(|hashes| open_bundles(parent, hashes))
                .transpose()?,
            parent,
            poses: options.poses,
        })
    }
}

type ControllerAxes = BTreeMap<u16, AxisBinding<Action>>;

fn ground_position(
    actor: Art,
    position: Point,
    elevation: i16,
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
        x: i32::from(position.x).saturating_sub(horizontal),
        y: i32::from(position.y)
            .saturating_sub(vertical)
            .saturating_sub(i32::from(elevation)),
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

fn live(combat: bool) -> Result<LiveSimulation<Cabinet>, RenderError> {
    let cabinet = if combat {
        Cabinet::with_encounter(StreetLayout::DOWNTOWN, Point {
            x: 246,
            y: 190,
        })
    } else {
        Cabinet::new(StreetLayout::DOWNTOWN)
    }
    .map_err(failure)?;
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
    let limits = ManifestLimits {
        manifest_bytes: size(32_768)?,
        payload_bytes: NonZeroU64::new(1_048_576)
            .ok_or(RenderError::TooLarge)?,
        records: size(33)?,
        total_bytes: NonZeroU64::new(33_554_432)
            .ok_or(RenderError::TooLarge)?,
    };
    open_selected_bundle(parent, digest, limits)
}

fn open_selected_bundle(
    parent: &Path,
    digest: &OsString,
    limits: ManifestLimits,
) -> Result<CanonicalBundle, RenderError> {
    let hash: ContentHash = digest
        .to_str()
        .ok_or_else(|| failure("invalid hash"))?
        .parse()
        .map_err(failure)?;
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

fn open_bundles<const COUNT: usize>(
    parent: &Path,
    hashes: &[OsString; COUNT],
) -> Result<[CanonicalBundle; COUNT], RenderError> {
    hashes
        .iter()
        .map(|hash| open_bundle(parent, hash))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_error| failure("bundle count"))
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
    let catalogue = if args.first().is_some_and(|arg| arg == "--poses") {
        if args.len() < 2 {
            return Err(String::from("missing pose catalogue path"));
        }
        let _flag = args.remove(0);
        poses::Catalogue::open(Path::new(&args.remove(0)))
            .map_err(|error| format!("{error:?}"))?
    } else {
        poses::Catalogue::default()
    };
    let enemy = if args.first().is_some_and(|arg| arg == "--enemy") {
        if args.len() < 3 {
            return Err(String::from("missing enemy bundle hashes"));
        }
        let _flag = args.remove(0);
        Some([args.remove(0), args.remove(0)])
    } else {
        None
    };
    let audio = if args.first().is_some_and(|arg| arg == "--audio") {
        if args.len() < 2 {
            return Err(String::from("missing audio bundle hash"));
        }
        let _flag = args.remove(0);
        Some(args.remove(0))
    } else {
        None
    };
    let [directory, marge, homer, bart, lisa] = <[OsString; 5]>::try_from(args)
        .map_err(|_error| {
            String::from(concat!(
                "usage: simpsons-arcade [--smoke] [--touch] ",
                "[--poses FILE] [--enemy APPROACH_SHA STRIKE_SHA] ",
                "[--audio SHA] PARENT ",
                "MARGE_SHA HOMER_SHA BART_SHA LISA_SHA"
            ))
        })?;
    let parent = Path::new(&directory);
    let result =
        Sources::load(parent, &[marge, homer, bart, lisa], OptionalSources {
            audio,
            enemy,
            poses: catalogue,
        })
        .and_then(|source| window(&source, smoke, touch_enabled));
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
    let mut simulation = live(false)?;
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
    smoke_jump(scene, art, &mut controls, &mut simulation)?;
    if !art.poses.is_empty() {
        smoke_poses(scene, art, &mut controls, &mut simulation)?;
    }
    Ok(())
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

fn smoke_poses(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let _drawn = art.present(scene, simulation.state())?;
    let idle = scene.readback().map_err(failure)?;
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    for [_, _, primary, ..] in KEYBOARD {
        for down in [true, false] {
            controls.route(
                InputEvent::Key {
                    code: key_code(primary),
                    down,
                },
                TICK_INTERVAL.saturating_mul(tick),
                simulation,
            )?;
        }
    }
    let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    if simulation
        .state()
        .stations()
        .iter()
        .any(|station| station.maneuver != (Maneuver::Attack { tick: 0 }))
    {
        return Err(failure("four-player attack admission failed"));
    }
    let _attack = art.present(scene, simulation.state())?;
    if idle == scene.readback().map_err(failure)? {
        return Err(failure("attack poses not visibly presented"));
    }
    for _ in 0..18u8 {
        let _cycle_step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        let _draw = art.present(scene, simulation.state())?;
    }
    if idle != scene.readback().map_err(failure)? {
        return Err(failure("attack pose did not return to idle"));
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
    source: &Sources<'_>,
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
    let layout = preview_layout(touch_enabled)?;
    let limits = scene_limits(source, touch_enabled)?;
    let startup = window.input(size(4)?)?;
    let mut playback = source
        .audio
        .as_ref()
        .map(audio::Playback::open)
        .transpose()?;
    window.with_scene(layout, limits, |scene| {
        let mut art = Scene::load(scene, source, touch_enabled)?;
        if hidden {
            smoke(scene, &mut art, touch_enabled)?;
            if source.enemy.is_some() {
                encounter::smoke(scene, &mut art, touch_enabled)?;
            }
            if let Some(audio) = &mut playback {
                audio.smoke()?;
            }
            return Ok(());
        }
        if let Some(audio) = &playback {
            audio.resume();
        }
        let mut simulation = live(source.enemy.is_some())?;
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
            if let Some(audio) = &mut playback {
                audio.update(simulation.state())?;
            }
            let _drawn = art.present(scene, simulation.state())?;
        }
        Ok(())
    })
}

fn scene_limits(
    source: &Sources<'_>,
    touch_enabled: bool,
) -> Result<SceneLimits, RenderError> {
    let enemy_atlases = if source.enemy.is_some() {
        4
    } else {
        0
    };
    let enemy_sprites: usize = if source.enemy.is_some() {
        11
    } else {
        0
    };
    let count = source.poses.count().saturating_add(enemy_atlases);
    Ok(SceneLimits {
        atlas_bytes: size(
            2_097_152usize.saturating_add(count.saturating_mul(65_536)),
        )?,
        atlases: size(count.saturating_add(if touch_enabled {
            6
        } else {
            5
        }))?,
        sprites: size(enemy_sprites.saturating_add(if touch_enabled {
            10
        } else {
            9
        }))?,
    })
}

fn preview_layout(touch_enabled: bool) -> Result<FrameLayout, RenderError> {
    let height = if touch_enabled {
        u32::from(touch::SCENE_HEIGHT)
    } else {
        224
    };
    FrameLayout::new(dimension(320)?, dimension(height)?, size(389_120)?)
}
