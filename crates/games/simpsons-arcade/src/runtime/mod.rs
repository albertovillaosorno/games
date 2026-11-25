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

//! Title runtime composition for the initial street and session loop.

mod audio;
mod controller;
mod encounter;
mod lan;
mod launch;
mod measurement;
mod poses;
mod shadow;
mod station_hud;
mod touch;
mod world;

use std::env;
use std::ffi::OsString;
use std::fmt::Debug;
use std::io::{Write as _, stdout};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::Path;
use std::time::{Duration, Instant};

use games::audio::MasterVolume;
use games::input::physical::{ControllerButton, KeyboardKey, PointerButton};
use games::input::profile::{AxisProfile, DigitalProfiles, MAX_PROFILE_BYTES};
use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::{
    DisplaySettings, FramePacing, PresentationLayout, WindowMode,
};
use games::replay::{
    InputStep, LiveInputLimits, LiveSimulation, QueueRejection, RejectedInput,
};
use games::session::LocalSession;
use games::timing::StepClock;
use games_assets::{AssetFormat, ContentHash, ManifestLimits};
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::NativeWindow;
use games_platform::ir::graphics::scene_renderer::{SceneSession, SceneView};
use games_platform::ir::input::{
    HostEvent, InputStartup, NativeInput, key_from_name, key_name,
};
use games_platform::ir::menu::{MenuArt, MenuContent};
use games_platform::ir::performance::{
    FrameMeter, peak_resident_bytes, resident_bytes,
};
use games_platform::ir::scene::{
    AtlasId, EventBatch, SceneLimits, SpriteDraw, SpriteRect,
};
use games_platform::ir::storage::bundle::CanonicalBundle;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::preferences::{DisplayStore, VolumeStore};
use games_platform::ir::ui::PlateLayout;

use super::{
    Action, Cabinet, Control, Facing, Maneuver, Pair, Participation, Phase,
    Player, Point, Station, StreetLayout, TICK_INTERVAL,
};
use crate::lan::HostLan;
use crate::remote::{HostedCabinet, HostedInput};

// Leave transport, fixed updates and presentation time between input bursts.
const EVENT_BUDGET: usize = 32;
// Keep fixed updates and LAN polling responsive on static screens.
const IDLE_EVENT_WAIT_MS: u32 = 8;

const KEYS: [&str; 4] =
    ["actor/marge", "actor/homer", "actor/bart", "actor/lisa"];
const CONTROLS: [Control; 8] = [
    Control::Pause,
    Control::Start,
    Control::Primary,
    Control::Secondary,
    Control::Left,
    Control::Right,
    Control::Up,
    Control::Down,
];
const KEYBOARD: [[Option<KeyboardKey>; 8]; 4] = [
    [
        Some(KeyboardKey::ESCAPE),
        Some(KeyboardKey::ENTER),
        Some(KeyboardKey::LETTER_Z),
        Some(KeyboardKey::SPACE),
        Some(KeyboardKey::LETTER_A),
        Some(KeyboardKey::LETTER_D),
        Some(KeyboardKey::LETTER_W),
        Some(KeyboardKey::LETTER_S),
    ],
    [
        None,
        Some(KeyboardKey::NUM_6),
        Some(KeyboardKey::LETTER_F),
        Some(KeyboardKey::LETTER_G),
        Some(KeyboardKey::LEFT),
        Some(KeyboardKey::RIGHT),
        Some(KeyboardKey::UP),
        Some(KeyboardKey::DOWN),
    ],
    [
        None,
        Some(KeyboardKey::NUM_7),
        Some(KeyboardKey::LETTER_O),
        Some(KeyboardKey::LETTER_P),
        Some(KeyboardKey::LETTER_J),
        Some(KeyboardKey::LETTER_L),
        Some(KeyboardKey::LETTER_I),
        Some(KeyboardKey::LETTER_K),
    ],
    [
        None,
        Some(KeyboardKey::NUM_8),
        Some(KeyboardKey::KP_1),
        Some(KeyboardKey::KP_2),
        Some(KeyboardKey::KP_4),
        Some(KeyboardKey::KP_6),
        Some(KeyboardKey::KP_8),
        Some(KeyboardKey::KP_5),
    ],
];
const BUTTONS: [ControllerButton; 8] = [
    ControllerButton::Back,
    ControllerButton::Start,
    ControllerButton::South,
    ControllerButton::East,
    ControllerButton::DPadLeft,
    ControllerButton::DPadRight,
    ControllerButton::DPadUp,
    ControllerButton::DPadDown,
];

const ACTION_NAMES: [&str; 8] = [
    "pause",
    "start",
    "primary",
    "secondary",
    "left",
    "right",
    "up",
    "down",
];
const DEFAULT_PROFILES: Profiles = Profiles {
    axes: [[
        AxisProfile {
            negative: Some(4),
            positive: Some(5),
            ..AxisProfile::UNBOUND
        },
        AxisProfile {
            negative: Some(6),
            positive: Some(7),
            ..AxisProfile::UNBOUND
        },
        AxisProfile::UNBOUND,
        AxisProfile::UNBOUND,
        AxisProfile::UNBOUND,
        AxisProfile::UNBOUND,
    ]; 4],
    buttons: [BUTTONS; 4],
    keys: KEYBOARD,
    mouse: [
        [
            None,
            None,
            Some(PointerButton::Left),
            None,
            None,
            None,
            None,
            None,
        ],
        [None; 8],
        [None; 8],
        [None; 8],
    ],
    touch_players: [true, false, false, false],
    touch_slots: [[0, 1, 2, 3, 4, 5, 6, 7]; 4],
};
type LocalActions = Vec<LogicalAction<Action>>;
type Profiles = DigitalProfiles<4, 8>;

#[derive(Debug)]
struct Controls {
    devices: [Option<u64>; 4],
    profiles: Profiles,
    reserved: [bool; 4],
    router: InputRouter<Action>,
}

impl Controls {
    fn actions(
        &mut self,
        event: InputEvent,
    ) -> Result<LocalActions, RenderError> {
        let actions = self.router.handle(event);
        if let InputEvent::Attached { device } = event {
            self.attach(device)?;
        } else if let InputEvent::Detached { device } = event {
            self.detach(device);
        }
        Ok(actions)
    }

    fn attach(&mut self, device: u64) -> Result<(), RenderError> {
        if self.devices.contains(&Some(device)) {
            return Ok(());
        }
        let Some(index) = self
            .devices
            .iter()
            .zip(self.reserved)
            .position(|(assigned, remote)| assigned.is_none() && !remote)
        else {
            return Ok(());
        };
        let player =
            *Player::ALL.get(index).ok_or(RenderError::InvalidSprite)?;
        let buttons = self
            .profiles
            .buttons
            .get(index)
            .ok_or(RenderError::InvalidSprite)?
            .iter()
            .copied()
            .zip(CONTROLS)
            .map(|(button, control)| {
                (button.code(), Action { control, player })
            })
            .collect();
        let axes = self
            .profiles
            .axis_bindings(
                index,
                CONTROLS.map(|control| Action { control, player }),
            )
            .map_err(failure)?;
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

    fn key(
        &self,
        player: Player,
        control: Control,
    ) -> Result<KeyboardKey, RenderError> {
        let index = CONTROLS
            .iter()
            .position(|item| *item == control)
            .ok_or(RenderError::InvalidSprite)?;
        self.profiles
            .keys
            .get(player.index())
            .and_then(|keys| keys.get(index))
            .copied()
            .flatten()
            .ok_or(RenderError::InvalidSprite)
    }

    fn new(touch_enabled: bool) -> Result<Self, RenderError> {
        Self::with_profiles(touch_enabled, &DEFAULT_PROFILES)
    }

    fn route<Simulation>(
        &mut self,
        event: InputEvent,
        elapsed: Duration,
        live: &mut LiveSimulation<Simulation>,
    ) -> Result<(), RenderError>
    where
        Simulation: InputStep,
        Simulation::Input: From<LogicalAction<Action>>,
    {
        let actions: Vec<_> =
            self.actions(event)?.into_iter().map(Into::into).collect();
        enqueue_local(live, elapsed, actions)
            .map(|_tick| ())
            .map_err(|rejected| failure(rejected.reason))
    }

    fn with_panel(
        panel: Option<touch::PanelRole>,
        profiles: &Profiles,
        reserved: [bool; 4],
    ) -> Result<Self, RenderError> {
        profiles.validate().map_err(failure)?;
        let mut mapping = desktop_mapping(profiles, reserved);
        if let Some(role) = panel {
            if touch::reserved(profiles, reserved) {
                return Err(failure("touch seat reserved for remote input"));
            }
            touch::install(&mut mapping, profiles, role)?;
        }
        Ok(Self {
            devices: [None; 4],
            profiles: *profiles,
            reserved,
            router: InputRouter::new(mapping),
        })
    }

    fn with_profiles(
        touch_enabled: bool,
        profiles: &Profiles,
    ) -> Result<Self, RenderError> {
        Self::with_reserved(touch_enabled, profiles, [false; 4])
    }

    fn with_reserved(
        touch_enabled: bool,
        profiles: &Profiles,
        reserved: [bool; 4],
    ) -> Result<Self, RenderError> {
        Self::with_panel(
            touch_enabled.then_some(touch::PanelRole::Cabinet),
            profiles,
            reserved,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct Art {
    atlas: AtlasId,
    rect: SpriteRect,
}

type LoadedArt = (Art, Vec<u8>);

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
        Self::load_with_pixels(scene, bundle, key).map(|(art, _pixels)| art)
    }

    fn load_with_pixels(
        scene: &mut SceneSession<'_, '_>,
        bundle: &CanonicalBundle,
        key: &str,
    ) -> Result<LoadedArt, RenderError> {
        let record = bundle
            .manifest()
            .record(key)
            .ok_or_else(|| failure("missing art record"))?;
        let AssetFormat::Rgba8 { height, width } = record.format else {
            return Err(failure("art must be RGBA8"));
        };
        let pixels = bundle.read_payload(key).map_err(failure)?;
        let art = Self::upload(scene, (width, height), &pixels)
            .map_err(|error| failure(format!("load {key}: {error:?}")))?;
        Ok((art, pixels))
    }

    fn upload(
        scene: &mut SceneSession<'_, '_>,
        (width, height): (NonZeroU32, NonZeroU32),
        pixels: &[u8],
    ) -> Result<Self, RenderError> {
        let layout = FrameLayout::new(width, height, size(1_048_576)?)?;
        let atlas = scene.load_atlas(layout, pixels)?;
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
    hud: station_hud::StationHud,
    menu: MenuArt,
    panel: Option<SpriteDraw>,
    poses: poses::PoseArt,
    shadow: Option<shadow::ShadowArt>,
    world: Vec<(i16, SpriteDraw)>,
    world_art: Option<world::WorldArt>,
}

impl Scene {
    fn load(
        scene: &mut SceneSession<'_, '_>,
        source: &Sources<'_>,
        options: PreviewOptions<'_>,
        reserved: [bool; 4],
    ) -> Result<Self, RenderError> {
        let touch_enabled = options.touch_enabled;
        let profiles = &options.controls.profiles;
        let [first, _, _, _] = &source.actors;
        let (mut background, original) =
            Art::load_with_pixels(scene, first, "scene/background")?;
        if (background.rect.width.get(), background.rect.height.get())
            != (world::BASE_WIDTH, world::BASE_HEIGHT)
        {
            return Err(failure("unexpected background dimensions"));
        }
        // The original HUD reflects the captured player, not live state.
        background.rect.y = 28i32;
        background.rect.height = dimension(196)?;
        let world_art = source
            .world
            .as_ref()
            .map(|bundle| world::WorldArt::load(scene, &original, bundle))
            .transpose()?;
        drop(original);
        let actors = load_actors(scene, source)?;
        Ok(Self {
            actors,
            background,
            combat: source
                .enemy
                .as_ref()
                .map(|bundles| encounter::CombatArt::load(scene, bundles))
                .transpose()?,
            draws: Vec::with_capacity(
                (if source.enemy.is_some() {
                    21usize.saturating_add(encounter::EXTRA_SPRITES)
                } else {
                    21usize
                })
                .saturating_add(
                    usize::from(source.shadow.is_some()).saturating_mul(4),
                ),
            ),
            hud: station_hud::StationHud::load(scene, profiles, reserved)?,
            menu: load_menu(scene, world_art.is_some(), profiles, reserved)?,
            panel: if touch_enabled {
                Some(touch::load(scene, profiles, touch::PanelRole::Cabinet)?)
            } else {
                None
            },
            poses: source.poses.load(source.parent, scene)?,
            shadow: source
                .shadow
                .as_ref()
                .map(|bundle| shadow::ShadowArt::load(scene, bundle))
                .transpose()?,
            world: Vec::with_capacity(world_capacity(source.enemy.is_some())),
            world_art,
        })
    }

    fn present(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        cabinet: &Cabinet,
    ) -> Result<bool, RenderError> {
        let view = self.presentation_view(scene, cabinet)?;
        let width = view.width;
        let center =
            i32::try_from(world::centered_offset(width)).map_err(failure)?;
        self.draws.clear();
        if let Some(art) = self.world_art {
            art.queue(self.background, view, &mut self.draws)?;
        } else {
            self.draws
                .push(self.background.draw(self.background.rect, false));
        }
        self.world.clear();
        self.queue_visible_stations(cabinet, view)?;
        self.queue_enemy_sprites(cabinet, view)?;
        self.world.sort_by_key(|(depth, _)| *depth);
        self.draws.extend(self.world.iter().map(|(_, draw)| *draw));
        self.queue_enemy_meters(cabinet, view)?;
        self.menu.queue(
            cabinet.session(),
            (20i32.saturating_add(center), 16i32),
            &mut self.draws,
        )?;
        if matches!(cabinet.phase(), Phase::MainMenu | Phase::Paused) {
            let shift = if cabinet.phase() == Phase::Paused {
                12i32
            } else {
                0i32
            };
            for (seat, character) in
                Player::ALL.into_iter().zip(cabinet.choices())
            {
                let mut choice = self.hud.draw_choice(seat, character)?;
                choice.destination.y =
                    choice.destination.y.saturating_add(shift);
                choice.destination.x = choice
                    .destination
                    .x
                    .saturating_add(world::station_offset(width, seat)?);
                self.draws.push(choice);
            }
        }
        if let Some(mut panel) = self.panel {
            panel.destination.x = center;
            self.draws.push(panel);
        }
        scene.present([24, 22, 30], &self.draws)
    }

    fn presentation_view(
        &self,
        scene: &mut SceneSession<'_, '_>,
        cabinet: &Cabinet,
    ) -> Result<world::View, RenderError> {
        let width = if let Some(art) = self.world_art {
            let maximum = art.maximum(scene.layout())?;
            scene.fit_adaptive_width(maximum)?.width().get()
        } else {
            scene.layout().width().get()
        };
        self.world_art
            .map_or(Ok(world::View { origin: 0, width }), |art| {
                art.view(width, cabinet.camera_origin())
            })
    }

    fn queue_enemy_meters(
        &mut self,
        cabinet: &Cabinet,
        view: world::View,
    ) -> Result<(), RenderError> {
        if let Some(combat) = &self.combat
            && cabinet.session().world_visible()
        {
            for mut enemy in cabinet.enemies().filter(|enemy| {
                enemy.health > 0
                    && !matches!(enemy.action, crate::EnemyAction::Restrain(_))
            }) {
                enemy.position = view.project(enemy.position);
                combat.enemy_health(
                    &mut self.draws,
                    enemy,
                    self.poses.enemy(enemy.action),
                )?;
            }
        }
        Ok(())
    }

    fn queue_enemy_sprites(
        &mut self,
        cabinet: &Cabinet,
        view: world::View,
    ) -> Result<(), RenderError> {
        if let Some(combat) = &self.combat
            && cabinet.session().world_visible()
        {
            for mut enemy in cabinet.enemies().filter(|enemy| enemy.visible()) {
                enemy.position = view.project(enemy.position);
                self.world.push((
                    enemy.position.y,
                    combat.enemy_draw(
                        enemy,
                        self.poses.enemy(enemy.action),
                        self.poses.repelled_enabled(),
                    )?,
                ));
            }
        }
        Ok(())
    }

    fn queue_pairs(
        &mut self,
        cabinet: &Cabinet,
        view: world::View,
    ) -> Result<(), RenderError> {
        for pair in Pair::ALL {
            let Some(joined) = cabinet.pair_pose(pair) else {
                continue;
            };
            let pose = self
                .poses
                .pair(pair, joined.motion)
                .ok_or(RenderError::InvalidSprite)?;
            if let Some(shadow) = &self.shadow
                && pair == Pair::MargeHomer
                && matches!(
                    joined.motion,
                    crate::PairMotion::Jump { .. }
                        | crate::PairMotion::AfterJump { .. }
                )
            {
                self.draws.push(shadow.draw(view.project(joined.position)));
            }
            self.world.push((
                joined.position.y,
                pose.draw(
                    ground_position(
                        pose,
                        view.project(joined.position),
                        joined.motion.elevation(pair),
                    )?,
                    joined.facing == Facing::Right,
                ),
            ));
        }
        Ok(())
    }

    fn queue_station(
        &mut self,
        (player, seat): (Player, Player),
        (station, paired): (Station, bool),
        view: world::View,
    ) -> Result<(), RenderError> {
        let offset = world::station_offset(view.width, player)?;
        let prompted = self.hud.queue(
            (player, seat),
            (station.participation, offset),
            &mut self.draws,
        )?;
        if !matches!(
            station.participation,
            Participation::Playing | Participation::Recovering { .. }
        ) {
            return Ok(());
        }
        let actor = *self
            .actors
            .get(player.index())
            .ok_or(RenderError::InvalidSprite)?;
        let mut icon = indicator(player)?;
        icon.x = icon.x.saturating_add(offset);
        if !prompted {
            self.draws.push(actor.draw(icon, false));
        }
        if let Some(combat) = &self.combat {
            combat.station_health(&mut self.draws, icon.x, station)?;
        }
        if !paired
            && station.participation == Participation::Playing
            && station.health > 0
        {
            let position = view.project(station.position);
            if let Some(shadow) = &self.shadow
                && shadow::visible(player, station.maneuver)
            {
                self.draws.push(shadow.draw(position));
            }
            let pose = self.poses.select(player, station).unwrap_or(actor);
            self.world.push((
                station.position.y,
                pose.draw(
                    ground_position(
                        pose,
                        position,
                        station.maneuver.elevation(),
                    )?,
                    station.facing == Facing::Left,
                ),
            ));
        }
        Ok(())
    }

    fn queue_visible_stations(
        &mut self,
        cabinet: &Cabinet,
        view: world::View,
    ) -> Result<(), RenderError> {
        if !cabinet.session().world_visible() {
            return Ok(());
        }
        let stations = cabinet.stations();
        for (seat, character) in Player::ALL.into_iter().zip(cabinet.choices())
        {
            let station = *stations
                .get(character.index())
                .ok_or(RenderError::InvalidSprite)?;
            let paired =
                cabinet.pair_pose(Pair::for_player(character)).is_some()
                    || station.maneuver == Maneuver::Restrained;
            self.queue_station((character, seat), (station, paired), view)?;
        }
        self.queue_pairs(cabinet, view)
    }
}

struct OptionalSources {
    audio: Option<launch::AudioSelection>,
    enemy: Option<[OsString; 2]>,
    poses: poses::Catalogue,
    shadow: Option<OsString>,
    world: Option<OsString>,
}

struct Sources<'source> {
    actors: [CanonicalBundle; 4],
    audio: Option<audio::Samples>,
    enemy: Option<[CanonicalBundle; 2]>,
    parent: &'source Path,
    poses: poses::Catalogue,
    shadow: Option<CanonicalBundle>,
    world: Option<CanonicalBundle>,
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
                .map(|selection| audio::Samples::load(parent, selection))
                .transpose()?,
            enemy: options
                .enemy
                .as_ref()
                .map(|hashes| open_bundles(parent, hashes))
                .transpose()?,
            parent,
            poses: options.poses,
            shadow: options
                .shadow
                .as_ref()
                .map(|hash| open_bundle(parent, hash))
                .transpose()?,
            world: options
                .world
                .as_ref()
                .map(|hash| open_bundle(parent, hash))
                .transpose()?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct PreviewOptions<'profiles> {
    audio_device: Option<&'profiles str>,
    controls: &'profiles launch::ResolvedControls,
    display: DisplaySettings,
    hidden: bool,
    measure: bool,
    preferences: Option<&'profiles DisplayStore>,
    touch_enabled: bool,
    volume: MasterVolume,
    volume_store: Option<&'profiles VolumeStore>,
}

/// One already translated native event, retained across fixed-step catch-up.
///
/// Stop consuming the native queue while this batch cannot enter the live
/// input window. Replaying physical events would duplicate router mutation.
#[derive(Debug)]
struct NativeBatch {
    actions: Vec<HostedInput>,
    elapsed: Duration,
}

impl NativeBatch {
    fn submit(
        self,
        live: &mut LiveSimulation<HostedCabinet>,
        measurement: Option<&mut measurement::LiveMeasurement>,
    ) -> Option<Self> {
        match enqueue_local(live, self.elapsed, self.actions) {
            Ok(admitted) => {
                if let (Some(tick), Some(meter)) = (admitted, measurement) {
                    meter.admit(tick, self.elapsed);
                }
                None
            },
            Err(rejected) => Some(Self {
                actions: rejected.actions,
                elapsed: self.elapsed,
            }),
        }
    }
}

struct Interactive<'preferences> {
    applied: DisplaySettings,
    controls: Controls,
    deferred: Option<NativeBatch>,
    host: Option<HostLan>,
    input: NativeInput,
    measurement: Option<measurement::LiveMeasurement>,
    preferences: Option<&'preferences DisplayStore>,
    previous: Duration,
    requested: DisplaySettings,
    simulation: LiveSimulation<HostedCabinet>,
    volume: MasterVolume,
    volume_store: Option<&'preferences VolumeStore>,
}

impl<'preferences> Interactive<'preferences> {
    fn advance_measurement(
        &mut self,
        next_tick: u64,
    ) -> Result<(), RenderError> {
        if let Some(meter) = &mut self.measurement {
            meter.advance(next_tick);
            if self.simulation.state().state().phase() != Phase::Playing {
                meter.finish()?;
            }
        }
        Ok(())
    }

    fn measure_presented(
        &mut self,
        started: Option<Instant>,
        layout: Option<PresentationLayout>,
    ) -> Result<(), RenderError> {
        if self.simulation.state().state().phase() == Phase::Playing
            && let (Some(meter), Some(beginning), Some(submitted)) =
                (&mut self.measurement, started, layout)
        {
            let completed = Instant::now();
            meter.observe(measurement::FrameSample {
                completed,
                display: self.applied,
                elapsed: self.input.elapsed(),
                layout: submitted,
                work: completed.saturating_duration_since(beginning),
            })?;
        }
        Ok(())
    }

    fn new(
        startup: InputStartup,
        options: PreviewOptions<'preferences>,
        cabinet: &Cabinet,
        host: Option<HostLan>,
    ) -> Result<Self, RenderError> {
        let reserved = host.as_ref().map_or([false; 4], HostLan::reserved);
        let hosted = HostedCabinet::new(cabinet, reserved)
            .ok_or_else(|| failure("invalid cabinet reservation"))?;
        let mut simulation = fixed(hosted)?;
        let mut controls = Controls::with_reserved(
            options.touch_enabled,
            &options.controls.profiles,
            reserved,
        )?;
        for event in startup.attached {
            controls.route(event, Duration::ZERO, &mut simulation)?;
        }
        let mut input = startup.input;
        if options.touch_enabled {
            input.enable_touch();
        }
        Ok(Self {
            applied: options.display,
            controls,
            deferred: None,
            host,
            input,
            measurement: options
                .measure
                .then(measurement::LiveMeasurement::default),
            preferences: options.preferences,
            previous: Duration::ZERO,
            requested: cabinet.session().display(),
            simulation,
            volume: cabinet.session().volume(),
            volume_store: options.volume_store,
        })
    }

    fn poll(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        wait_ms: u32,
    ) -> Result<EventBatch, RenderError> {
        if let Some(pending) = self.deferred.take() {
            self.deferred =
                pending.submit(&mut self.simulation, self.measurement.as_mut());
            if self.deferred.is_some() {
                return Ok(EventBatch::Empty);
            }
        }
        if let Some(elapsed) =
            self.input.synchronize_touch(|| scene.visible_layout())?
            && !self.route_native(InputEvent::TouchCancelled, elapsed)?
        {
            return Ok(EventBatch::Limited);
        }
        let batch = scene.dispatch_events(
            size(EVENT_BUDGET)?,
            wait_ms,
            |view, event| {
                let logical = match self.input.translate(event) {
                    Some(HostEvent::Quit) => return Ok(false),
                    Some(HostEvent::Input(logical)) => logical,
                    None => return Ok(true),
                };
                let elapsed = self
                    .input
                    .event_elapsed(event)
                    .ok_or_else(|| failure("ambiguous event timestamp"))?;
                if let Some(projected) = project_contact(view, logical)? {
                    return self.route_native(projected, elapsed);
                }
                Ok(true)
            },
        )?;
        if self.deferred.is_some() {
            return Ok(EventBatch::Limited);
        }
        if matches!(batch, EventBatch::Empty | EventBatch::Drained) {
            let attached = self.input.discover_after_drain()?;
            let elapsed = self.input.elapsed();
            for event in attached {
                self.controls.route(event, elapsed, &mut self.simulation)?;
            }
        }
        Ok(batch)
    }

    fn poll_host(&mut self) -> Result<(), RenderError> {
        if let Some(host) = &mut self.host
            && let Some(admitted) =
                host.poll(&mut self.simulation).map_err(failure)?
            && let Some(meter) = &mut self.measurement
        {
            meter.admit_remote(admitted.tick, admitted.completed_at);
        }
        Ok(())
    }

    fn route_native(
        &mut self,
        event: InputEvent,
        elapsed: Duration,
    ) -> Result<bool, RenderError> {
        let actions = self
            .controls
            .actions(event)?
            .into_iter()
            .map(Into::into)
            .collect();
        self.deferred = NativeBatch { actions, elapsed }
            .submit(&mut self.simulation, self.measurement.as_mut());
        Ok(self.deferred.is_none())
    }

    fn run(
        mut self,
        scene: &mut SceneSession<'_, '_>,
        art: &mut Scene,
        mut playback: Option<&mut audio::Playback<'_>>,
    ) -> Result<(), RenderError> {
        if let Some(audio) = &mut playback {
            audio.enable_output();
        }
        let mut presented = None;
        while self.simulation.state().state().phase() != Phase::Exited {
            let started = self.measurement.as_ref().map(|_| Instant::now());
            let wait_ms = presentation_wait(self.simulation.state());
            let batch = self.poll(scene, wait_ms)?;
            if batch == EventBatch::Stopped {
                break;
            }
            self.poll_host()?;
            let elapsed = self.input.elapsed();
            let delta = elapsed.saturating_sub(self.previous);
            self.previous = elapsed;
            let advanced = self.simulation.advance(delta).map_err(failure)?;
            self.advance_measurement(advanced.next_tick)?;
            let hosted = self.simulation.state();
            let cabinet = hosted.state();
            art.hud.update_connections(hosted.connected_remotes());
            let desired = cabinet.session().effective_display();
            if desired != self.applied {
                scene.apply_display_settings(desired)?;
                self.applied = desired;
            }
            let requested = cabinet.session().display();
            if requested != self.requested {
                save_display(self.preferences, requested)?;
                self.requested = requested;
            }
            let volume = cabinet.session().volume();
            if volume != self.volume {
                save_volume(self.volume_store, volume)?;
                self.volume = volume;
            }
            if let Some(audio) = &mut playback {
                audio.update(cabinet)?;
            }
            if frame_due(hosted, presented.as_ref(), batch)
                || !scene.presentation_is_current()?
            {
                let drawn = art.present(scene, cabinet)?;
                presented = (drawn && cabinet.phase() != Phase::Playing)
                    .then_some(*hosted);
                self.measure_presented(started, scene.last_presented_layout())?;
            }
        }
        if let Some(measurement) = &mut self.measurement {
            measurement.finish()?;
        }
        Ok(())
    }
}

type LocalAdmission<Input> = Result<Option<u64>, RejectedInput<Input>>;

fn configured_profiles(text: &str) -> Result<Profiles, String> {
    DEFAULT_PROFILES.configured(ACTION_NAMES, text, key_from_name)
}

fn load_actors(
    scene: &mut SceneSession<'_, '_>,
    source: &Sources<'_>,
) -> Result<[Art; 4], RenderError> {
    let mut actors = Vec::new();
    for (bundle, key) in source.actors.iter().zip(KEYS) {
        let actor = Art::load(scene, bundle, key)?;
        if actor.rect.width.get() > 128 || actor.rect.height.get() > 128 {
            return Err(failure("unexpected actor extent"));
        }
        actors.push(actor);
    }
    actors.try_into().map_err(|_error| failure("actor count"))
}

fn enqueue_local<Simulation>(
    live: &mut LiveSimulation<Simulation>,
    elapsed: Duration,
    actions: Vec<Simulation::Input>,
) -> LocalAdmission<Simulation::Input>
where
    Simulation: InputStep,
{
    if actions.is_empty() {
        return Ok(None);
    }
    let Some(tick) = live.tick_at(elapsed) else {
        return Err(RejectedInput {
            actions,
            reason: QueueRejection::Future,
        });
    };
    let target = tick.max(live.next_tick());
    live.enqueue(target, actions).map(|()| Some(target))
}

fn desktop_mapping(
    profiles: &Profiles,
    reserved: [bool; 4],
) -> InputMapping<Action> {
    let mut mapping = InputMapping::new();
    let [first, ..] = profiles.keys;
    let [pause, ..] = first;
    if let Some(key) = pause
        && let Some(player) = Player::ALL
            .into_iter()
            .zip(reserved)
            .find_map(|(player, remote)| (!remote).then_some(player))
    {
        mapping.bind_key(key.code(), Action {
            control: Control::Pause,
            player,
        });
    }
    for (((player, keys), buttons), remote) in Player::ALL
        .into_iter()
        .zip(profiles.keys)
        .zip(profiles.mouse)
        .zip(reserved)
    {
        if !remote {
            bind_desktop_seat(&mut mapping, player, keys, buttons);
        }
    }
    mapping
}

fn bind_desktop_seat(
    mapping: &mut InputMapping<Action>,
    player: Player,
    keys: [Option<KeyboardKey>; 8],
    buttons: [Option<PointerButton>; 8],
) {
    for ((control, key), mouse) in CONTROLS.into_iter().zip(keys).zip(buttons) {
        if control != Control::Pause
            && let Some(code) = key
        {
            mapping.bind_key(code.code(), Action { control, player });
        }
        if let Some(button) = mouse {
            mapping.bind_mouse(button.code(), Action { control, player });
        }
    }
}

fn frame_due(
    current: &HostedCabinet,
    presented: Option<&HostedCabinet>,
    batch: EventBatch,
) -> bool {
    current.state().phase() == Phase::Playing
        || batch != EventBatch::Empty
        || presented != Some(current)
}

const fn presentation_wait(current: &HostedCabinet) -> u32 {
    if matches!(current.state().phase(), Phase::Playing) {
        0
    } else {
        IDLE_EVENT_WAIT_MS
    }
}

fn save_display(
    store: Option<&DisplayStore>,
    settings: DisplaySettings,
) -> Result<(), RenderError> {
    if let Some(preferences) = store {
        let receipt = preferences.store(settings).map_err(failure)?;
        receipt.durability.map_err(failure)?;
    }
    Ok(())
}

fn save_volume(
    store: Option<&VolumeStore>,
    volume: MasterVolume,
) -> Result<(), RenderError> {
    if let Some(preferences) = store {
        let receipt = preferences.store(volume).map_err(failure)?;
        receipt.durability.map_err(failure)?;
    }
    Ok(())
}

const fn mouse_label(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Left => "LEFT",
        PointerButton::Middle => "MIDDLE",
        PointerButton::Right => "RIGHT",
        PointerButton::X1 => "X1",
        PointerButton::X2 => "X2",
    }
}

fn key_label(key: KeyboardKey) -> &'static str {
    if key == KeyboardKey::ENTER {
        "ENTER"
    } else {
        key_name(key)
    }
}

const fn adaptive_hint(world_available: bool) -> &'static str {
    if world_available {
        "ADAPTIVE EXTENDS STREET WHEN IT FITS"
    } else {
        "NO WORLD STRIP - ADAPTIVE ASPECT FIT"
    }
}

const fn main_menu_hints(hosted: bool) -> &'static [&'static str] {
    if hosted {
        &[
            "LOCAL LEFT/RIGHT CHOOSE CHARACTER",
            "LOCAL START OPENS HOSTED GAME",
            "REMOTE START JOINS DURING PLAY",
            "UP/DOWN SELECT - PRIMARY CONFIRM",
        ]
    } else {
        &[
            "LEFT/RIGHT CHOOSE YOUR CHARACTER",
            "PRESS YOUR START KEY TO JOIN",
            "GAMEPAD / TOUCH: USE JOIN CONTROL",
            "UP/DOWN SELECT - PRIMARY CONFIRM",
        ]
    }
}

fn pause_hints(profiles: &Profiles, reserved: [bool; 4]) -> [String; 3] {
    let index = reserved.iter().position(|remote| !remote).unwrap_or(0);
    let [pause, ..] = profiles.keys.first().copied().unwrap_or([None; 8]);
    let [_, _, primary, secondary, left, right, up, down] =
        profiles.keys.get(index).copied().unwrap_or([None; 8]);
    let [_, _, mouse, ..] =
        profiles.mouse.get(index).copied().unwrap_or([None; 8]);
    let key = |binding: Option<KeyboardKey>| {
        binding.map_or("NONE", |code| {
            let label = key_label(code);
            if label.len() > 12 {
                "PROFILE"
            } else {
                label
            }
        })
    };
    let mouse_name = mouse.map_or_else(String::new, |button| {
        format!(" / MOUSE {}", mouse_label(button))
    });
    [
        format!(
            "P{} MOVE: {} {} {} {}",
            index.saturating_add(1),
            key(left),
            key(right),
            key(up),
            key(down)
        ),
        format!(
            "ATTACK: {}{mouse_name} - JUMP: {}",
            key(primary),
            key(secondary)
        ),
        format!("{} OR START RESUMES - JOIN BELOW", key(pause)),
    ]
}

fn load_menu(
    scene: &mut SceneSession<'_, '_>,
    world_available: bool,
    profiles: &Profiles,
    reserved: [bool; 4],
) -> Result<MenuArt, RenderError> {
    let hints = pause_hints(profiles, reserved);
    let pause = hints.each_ref().map(String::as_str);
    MenuArt::load(
        scene,
        PlateLayout {
            background: [24, 22, 30, 255],
            height: 192,
            max_bytes: size(1_400_000)?,
            width: 280,
        },
        MenuContent {
            main_hints: main_menu_hints(reserved.contains(&true)),
            pause_hints: &pause,
            settings_hints: &[
                "PRIMARY CHANGES VALUE - PAUSE RETURNS",
                adaptive_hint(world_available),
                "CONTROLS: --CONTROLS / --RESET-CONTROLS",
            ],
            title: "THE SIMPSONS ARCADE",
        },
    )
}

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
    live_with_session(combat, LocalSession::default())
}

fn live_with_session(
    combat: bool,
    session: LocalSession,
) -> Result<LiveSimulation<Cabinet>, RenderError> {
    live_with_scroll(combat, session, 0)
}

fn live_with_scroll(
    combat: bool,
    session: LocalSession,
    scroll_limit: u16,
) -> Result<LiveSimulation<Cabinet>, RenderError> {
    fixed(cabinet_with_scroll(combat, session, scroll_limit)?)
}

fn cabinet_with_scroll(
    combat: bool,
    session: LocalSession,
    scroll_limit: u16,
) -> Result<Cabinet, RenderError> {
    let mut cabinet = if combat {
        Cabinet::with_opening_street()
    } else {
        Cabinet::new(StreetLayout::DOWNTOWN)
    }
    .map_err(failure)?;
    cabinet = cabinet.with_scroll_limit(scroll_limit).map_err(failure)?;
    cabinet.session = session;
    Ok(cabinet)
}

fn fixed<Simulation>(
    simulation: Simulation,
) -> Result<LiveSimulation<Simulation>, RenderError>
where
    Simulation: InputStep,
{
    let clock =
        StepClock::new(TICK_INTERVAL, dimension(8)?).map_err(failure)?;
    Ok(LiveSimulation::new(simulation, clock, LiveInputLimits {
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
    if source.title != "simpsons-arcade" || source.revision != "world-4p-set-1"
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

fn project_contact(
    scene: SceneView<'_>,
    event: InputEvent,
) -> Result<Option<InputEvent>, RenderError> {
    let InputEvent::Contact { id, phase } = event else {
        return Ok(Some(event));
    };
    let projected = match phase {
        ContactPhase::Down(point) => {
            let Some(logical) =
                scene.logical_touch(point)?.and_then(|projected| {
                    touch::to_panel(projected, scene.layout().width().get())
                })
            else {
                return Ok(None);
            };
            ContactPhase::Down(logical)
        },
        ContactPhase::Motion(Some(point)) => ContactPhase::Motion(
            scene.logical_touch(point)?.and_then(|projected| {
                touch::to_panel(projected, scene.layout().width().get())
            }),
        ),
        ContactPhase::Motion(None) | ContactPhase::Up => phase,
    };
    Ok(Some(InputEvent::Contact { id, phase: projected }))
}

/// Launch interactive preview or a hidden finite native smoke.
///
/// # Errors
/// Rejects invalid inputs, missing art, unsupported native device or input.
pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if matches!(args.as_slice(), [arg] if arg == "--list-audio-devices") {
        return audio::list_devices().map_err(|error| format!("{error:?}"));
    }
    if !NativeWindow::supported() {
        return Err(format!("{:?}", RenderError::Unsupported));
    }
    if args.first().is_some_and(|arg| arg == "--lan-controller") {
        return controller::run(args);
    }
    let load_started = Instant::now();
    let launch::Launch {
        assets,
        audio_device,
        controls,
        display,
        host,
        measure,
        preferences,
        smoke,
        touch_enabled,
        volume,
        volume_store,
    } = launch::Launch::parse(args)?;
    let resolved = controls.resolve()?;
    let listener = host.as_deref().map(lan::open_host).transpose()?;
    let catalogue = assets
        .poses
        .as_deref()
        .map(poses::Catalogue::open)
        .transpose()
        .map_err(|error| format!("{error:?}"))?
        .unwrap_or_default();
    let result =
        Sources::load(&assets.directory, &assets.actors, OptionalSources {
            audio: assets.audio,
            enemy: assets.enemy,
            poses: catalogue,
            shadow: assets.shadow,
            world: assets.world,
        })
        .and_then(|source| {
            window(
                &source,
                PreviewOptions {
                    audio_device: audio_device.as_deref(),
                    controls: &resolved,
                    display,
                    hidden: smoke,
                    measure,
                    preferences: preferences.as_ref(),
                    touch_enabled,
                    volume,
                    volume_store: volume_store.as_ref(),
                },
                listener,
                load_started,
            )
        });
    result.map_err(|error| format!("{error:?}"))
}

fn profile_limit() -> Result<NonZeroU64, String> {
    u64::try_from(MAX_PROFILE_BYTES)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| String::from("invalid profile byte limit"))
}

fn read_profile(path: &Path) -> Result<String, String> {
    let bytes = snapshot(path, profile_limit()?).map_err(|error| {
        format!("control profile {}: {error:?}", path.display())
    })?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn size(value: usize) -> Result<NonZeroUsize, RenderError> {
    NonZeroUsize::new(value).ok_or(RenderError::TooLarge)
}

fn smoke(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
    profiles: &Profiles,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene
        .readback()
        .map_err(|error| failure(format!("initial readback: {error:?}")))?;
    let mut controls = Controls::with_profiles(touch_enabled, profiles)?;
    smoke_admission(scene, &mut controls, &mut simulation, touch_enabled)?;
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
    if art.combat.is_some() {
        encounter::verify_active_stations(scene, simulation.state())?;
    }
    if touch_enabled {
        smoke_touch_movement(scene, &mut controls, &mut simulation)?;
    }
    smoke_jump(scene, art, &mut controls, &mut simulation)?;
    smoke_held_jump(scene, art, &mut controls, &mut simulation)?;
    if !art.poses.is_empty() {
        smoke_poses(scene, art, &mut controls, &mut simulation)?;
    }
    smoke_special(scene, art, &mut controls, &mut simulation)?;
    smoke_air_attack(scene, art, &mut controls, &mut simulation)?;
    smoke_movement_benchmark(scene, art, &mut controls, &mut simulation)
}

fn smoke_special(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, ..] = simulation.state().stations();
    let _drawn = art.present(scene, simulation.state())?;
    let pixels = scene.readback()?;
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    for down in [true, false] {
        for control in [Control::Primary, Control::Secondary] {
            let code = controls.key(Player::Marge, control)?.code();
            controls.route(
                InputEvent::Key { code, down },
                TICK_INTERVAL.saturating_mul(tick),
                simulation,
            )?;
        }
    }
    let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [attack, ..] = simulation.state().stations();
    if !matches!(attack.maneuver, Maneuver::Special { tick: 0, .. })
        || attack.position != before.position
    {
        return Err(failure("stationary special did not start"));
    }
    let _draw = art.present(scene, simulation.state())?;
    if pixels == scene.readback()? {
        return Err(failure("stationary special not visibly presented"));
    }
    for _ in 0..29u8 {
        let _cycle_step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        let _cycle_draw = art.present(scene, simulation.state())?;
    }
    smoke_press(controls, simulation, Control::Primary)?;
    let _land_draw = art.present(scene, simulation.state())?;
    let [landed, ..] = simulation.state().stations();
    if landed.maneuver != Maneuver::Idle || landed.position != before.position {
        return Err(failure("special did not land at original position"));
    }
    Ok(())
}

fn smoke_air_attack(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, ..] = simulation.state().stations();
    smoke_jump_edge(controls, simulation, true)?;
    smoke_early_air_attack(controls, simulation, before.position)?;
    for _ in 0..13u8 {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    }
    let _jump = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    smoke_press(controls, simulation, Control::Primary)?;
    let [attack, ..] = simulation.state().stations();
    if !matches!(
        attack.maneuver,
        Maneuver::AirAttack(motion) if motion.tick() == 0
    ) || attack.position.x.abs_diff(before.position.x) != 4
    {
        return Err(failure("aerial attack did not advance"));
    }
    let _draw = art.present(scene, simulation.state())?;
    if pixels == scene.readback().map_err(failure)? {
        return Err(failure("aerial attack not visibly presented"));
    }
    smoke_jump_edge(controls, simulation, false)?;
    for _ in 0..20u8 {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        let _cycle_draw = art.present(scene, simulation.state())?;
    }
    smoke_press(controls, simulation, Control::Primary)?;
    let _land_draw = art.present(scene, simulation.state())?;
    let [landed, ..] = simulation.state().stations();
    if landed.maneuver != Maneuver::Idle
        || landed.position.x.abs_diff(before.position.x) != 84
        || landed.position.y != before.position.y
    {
        return Err(failure("aerial attack did not land at expected position"));
    }
    Ok(())
}

fn smoke_early_air_attack(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    position: Point,
) -> Result<(), RenderError> {
    let _takeoff = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    smoke_press(controls, simulation, Control::Primary)?;
    let [early, ..] = simulation.state().stations();
    if !matches!(early.maneuver, Maneuver::Jump(jump) if jump.tick() == 1)
        || early.position != position
    {
        return Err(failure("early aerial attack interrupted takeoff"));
    }
    Ok(())
}

fn smoke_character_selection(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_profiles(
        options.touch_enabled,
        &options.controls.profiles,
    )?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene.readback()?;
    smoke_press(&mut controls, &mut simulation, Control::Right)?;
    if simulation.state().choices().first() != Some(&Player::Homer) {
        return Err(failure("character selection did not follow the seat"));
    }
    let _updated = art.present(scene, simulation.state())?;
    if scene.readback()? == before {
        return Err(failure("character selection was not visibly presented"));
    }
    smoke_press(&mut controls, &mut simulation, Control::Start)?;
    let [marge, homer, ..] = simulation.state().stations();
    if marge.participation != Participation::Absent
        || homer.participation != Participation::Playing
    {
        return Err(failure("selected character did not join"));
    }
    Ok(())
}

fn smoke_controller_selection(profiles: &Profiles) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_profiles(false, profiles)?;
    let device = 1;
    controls.route(
        InputEvent::Attached { device },
        Duration::ZERO,
        &mut simulation,
    )?;
    let [_, start, _, _, _, right, ..] =
        profiles.buttons.first().ok_or(RenderError::InvalidSprite)?;
    for button in [right, start] {
        for down in [true, false] {
            controls.route(
                InputEvent::Button {
                    device,
                    code: button.code(),
                    down,
                },
                Duration::ZERO,
                &mut simulation,
            )?;
        }
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [marge, homer, ..] = simulation.state().stations();
    if marge.participation != Participation::Absent
        || homer.participation != Participation::Playing
    {
        return Err(failure("controller selection did not join Homer"));
    }
    Ok(())
}

fn smoke_touch_selection(
    scene: &SceneSession<'_, '_>,
    profiles: &Profiles,
) -> Result<(), RenderError> {
    let mut simulation = live(false)?;
    let mut controls = Controls::with_profiles(true, profiles)?;
    let player = touch::seats(profiles)
        .next()
        .ok_or(RenderError::InvalidSprite)?;
    smoke_touch(
        scene,
        &mut controls,
        &mut simulation,
        (player, &[Control::Right, Control::Start]),
    )?;
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let next = player
        .index()
        .saturating_add(1)
        .checked_rem(Player::ALL.len())
        .ok_or(RenderError::InvalidSprite)?;
    let character = simulation.state().stations().get(next).copied();
    if character.map(|station| station.participation)
        != Some(Participation::Playing)
    {
        return Err(failure("touch selection did not join next character"));
    }
    Ok(())
}

fn smoke_settings(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    let session = LocalSession::with_display(
        DisplaySettings::default(),
        options.touch_enabled,
    );
    let mut simulation = live_with_session(false, session)?;
    let mut controls = Controls::with_profiles(
        options.touch_enabled,
        &options.controls.profiles,
    )?;
    scene.apply_display_settings(session.effective_display())?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene.readback()?;
    for control in [Control::Down, Control::Primary, Control::Primary] {
        smoke_press(&mut controls, &mut simulation, control)?;
    }
    let settings = simulation.state().session();
    if settings.phase() != Phase::Settings
        || settings.display().presentation
            != session.display().presentation.next(options.touch_enabled)
    {
        return Err(failure("display menu selection failed"));
    }
    scene.apply_display_settings(settings.effective_display())?;
    let _updated = art.present(scene, simulation.state())?;
    if scene.readback()? == before {
        return Err(failure("settings were not visibly presented"));
    }
    smoke_window_toggle(scene, art, &mut controls, &mut simulation)?;
    scene.apply_display_settings(options.display)
}

fn smoke_window_toggle(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    smoke_press(controls, simulation, Control::Down)?;
    for mode in [WindowMode::Fullscreen, WindowMode::Windowed] {
        smoke_press(controls, simulation, Control::Primary)?;
        let settings = simulation.state().session();
        if settings.display().window != mode {
            return Err(failure("window menu selection failed"));
        }
        scene.apply_display_settings(settings.effective_display())?;
        let _drawn = art.present(scene, simulation.state())?;
    }
    smoke_press(controls, simulation, Control::Down)?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene.readback()?;
    smoke_press(controls, simulation, Control::Primary)?;
    if simulation.state().session().volume() != MasterVolume::ThreeQuarters {
        return Err(failure("master volume menu selection failed"));
    }
    let _updated = art.present(scene, simulation.state())?;
    if scene.readback()? == before {
        return Err(failure("master volume row did not update"));
    }
    smoke_press(controls, simulation, Control::Start)?;
    if simulation.state().phase() != Phase::MainMenu {
        return Err(failure("settings did not return to main menu"));
    }
    Ok(())
}

fn smoke_press(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    control: Control,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    let code = controls.key(Player::Marge, control)?.code();
    for down in [true, false] {
        controls.route(
            InputEvent::Key { code, down },
            TICK_INTERVAL.saturating_mul(tick),
            simulation,
        )?;
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    Ok(())
}

fn smoke_admission(
    scene: &SceneSession<'_, '_>,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    touch_enabled: bool,
) -> Result<(), RenderError> {
    let profiles = controls.profiles;
    for ((player, keys), touch) in Player::ALL
        .into_iter()
        .zip(profiles.keys)
        .zip(profiles.touch_players)
    {
        if touch_enabled && touch {
            smoke_touch(
                scene,
                controls,
                simulation,
                (player, &[Control::Start]),
            )?;
            continue;
        }
        let [_, start, ..] = keys;
        let key = start.ok_or(RenderError::InvalidSprite)?;
        for down in [true, false] {
            controls.route(
                InputEvent::Key { code: key.code(), down },
                Duration::ZERO,
                simulation,
            )?;
        }
    }
    Ok(())
}

fn smoke_held_jump(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    let [before, ..] = simulation.state().stations();
    let _drawn = art.present(scene, simulation.state())?;
    let pixels = scene.readback().map_err(failure)?;
    smoke_jump_edge(controls, simulation, true)?;
    for _ in 0..32u8 {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    }
    let [airborne, ..] = simulation.state().stations();
    if airborne.maneuver.elevation() <= 51
        || airborne.position != before.position
    {
        return Err(failure("held jump did not extend lift"));
    }
    let _redrawn = art.present(scene, simulation.state())?;
    if pixels == scene.readback().map_err(failure)? {
        return Err(failure("held jump not visibly presented"));
    }
    shadow::smoke(scene, art, simulation.state(), true)?;
    for _ in 32..59u8 {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    }
    let [landed, ..] = simulation.state().stations();
    if landed.maneuver != Maneuver::Idle || landed.position != before.position {
        return Err(failure("held jump did not return to ground"));
    }
    shadow::smoke(scene, art, simulation.state(), false)?;
    smoke_jump_edge(controls, simulation, false)?;
    let _released = simulation.advance(TICK_INTERVAL).map_err(failure)?;
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
    for down in [true, false] {
        smoke_jump_edge(controls, simulation, down)?;
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let [airborne, ..] = simulation.state().stations();
    if !matches!(airborne.maneuver, Maneuver::Jump(j) if j.tick() == 0)
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

fn smoke_jump_edge(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    down: bool,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    controls.route(
        InputEvent::Key {
            code: controls.key(Player::Marge, Control::Secondary)?.code(),
            down,
        },
        TICK_INTERVAL.saturating_mul(tick),
        simulation,
    )?;
    Ok(())
}

fn smoke_peak_report() -> Result<(), RenderError> {
    let peak = peak_resident_bytes().map_or_else(
        |_error| String::from("unobserved"),
        |bytes| bytes.to_string(),
    );
    writeln!(
        stdout().lock(),
        "Whole-smoke lifetime peak RSS: {peak} bytes"
    )
    .map_err(failure)
}

fn startup_report(
    scene: &SceneSession<'_, '_>,
    renderer: &str,
    started: Instant,
) -> Result<(), RenderError> {
    let completed = Instant::now();
    let layout = scene.layout();
    let profile = QualityProfile {
        frame_pacing: FramePacing::Uncapped,
        height: layout.height().get(),
        optional_effects: false,
        width: layout.width().get(),
    };
    let mut meter =
        FrameMeter::new(PerformanceCollector::new(profile), completed);
    meter.record_load(started, completed).map_err(failure)?;
    if let Ok(path) = env::current_exe() {
        let _result = meter.record_artifact(&path);
    }
    writeln!(
        stdout().lock(),
        concat!(
            "Native startup: {}; {}x{} initial logical; config, asset and ",
            "scene load {:?}; RSS {:?} bytes; executable {:?} bytes"
        ),
        renderer,
        profile.width,
        profile.height,
        meter.collector().load_duration(),
        resident_bytes().ok(),
        meter.collector().binary_size(),
    )
    .map_err(failure)
}

fn smoke_movement_benchmark(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
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
    for cycle in 0..20u8 {
        let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
        let elapsed = TICK_INTERVAL.saturating_mul(tick);
        let before = simulation.state().stations();
        smoke_movement_inputs(
            controls,
            simulation,
            elapsed,
            cycle.is_multiple_of(2),
        )?;
        let rss = resident_bytes().ok();
        for frame in 0..30u8 {
            let started = Instant::now();
            let _advanced =
                simulation.advance(TICK_INTERVAL).map_err(failure)?;
            let _drawn = art.present(scene, simulation.state())?;
            collector
                .observe(FrameObservation {
                    frame_time: started.elapsed(),
                    input_latency: None,
                    resident_bytes: if frame == 0 {
                        rss
                    } else {
                        None
                    },
                })
                .map_err(failure)?;
        }
        if before
            .into_iter()
            .zip(simulation.state().stations())
            .any(|(previous, current)| previous.position == current.position)
        {
            return Err(failure("movement workload did not advance"));
        }
    }
    smoke_movement_report(&collector)
}

fn smoke_movement_inputs(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    elapsed: Duration,
    rightward: bool,
) -> Result<(), RenderError> {
    for [_, _, _, _, left_binding, right_binding, ..] in controls.profiles.keys
    {
        let left = left_binding.ok_or(RenderError::InvalidSprite)?;
        let right = right_binding.ok_or(RenderError::InvalidSprite)?;
        let (release, press) = if rightward {
            (left, right)
        } else {
            (right, left)
        };
        for (key, down) in [(release, false), (press, true)] {
            controls.route(
                InputEvent::Key { code: key.code(), down },
                elapsed,
                simulation,
            )?;
        }
    }
    Ok(())
}

fn smoke_movement_report(
    collector: &PerformanceCollector,
) -> Result<(), RenderError> {
    let report = collector.report();
    writeln!(
        stdout().lock(),
        concat!(
            "Four-player movement: {} frames; mean {:?}; ",
            "max {:?}; p95 {:?}; budget {:?}; sampled RSS peak {:?} bytes"
        ),
        report.frame_time.samples(),
        report.frame_time.mean(),
        report.frame_time.maximum(),
        collector.frame_percentile(95),
        report.frame_budget,
        report.peak_resident_bytes,
    )
    .map_err(failure)
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
    for [_, _, primary, ..] in controls.profiles.keys {
        for down in [true, false] {
            controls.route(
                InputEvent::Key {
                    code: primary.ok_or(RenderError::InvalidSprite)?.code(),
                    down,
                },
                TICK_INTERVAL.saturating_mul(tick),
                simulation,
            )?;
        }
    }
    let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    if simulation.state().stations().iter().any(|station| {
        station.maneuver != (Maneuver::Attack { moving: false, tick: 0 })
    }) {
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
    smoke_press(controls, simulation, Control::Primary)?;
    let [marge, ..] = simulation.state().stations();
    if marge.maneuver != Maneuver::Idle {
        return Err(failure("ground attack ignored its recovery interval"));
    }
    Ok(())
}

fn smoke_touch(
    scene: &SceneSession<'_, '_>,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    commands: (Player, &[Control]),
) -> Result<(), RenderError> {
    let (player, actions) = commands;
    let id = ContactId { device: 1, finger: 1 };
    for control in actions.iter().copied() {
        let point =
            smoke_touch_point(scene, player, control, &controls.profiles)?;
        for phase in [ContactPhase::Down(point), ContactPhase::Up] {
            let event = project_contact(scene.view(), InputEvent::Contact {
                id,
                phase,
            })?
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
    let profiles = controls.profiles;
    let before = simulation.state().stations();
    let mut contacts = Vec::new();
    for player in touch::seats(&profiles) {
        for control in [Control::Right, Control::Up] {
            let id = ContactId {
                device: 1,
                finger: i64::try_from(contacts.len()).map_err(failure)?,
            };
            let event = project_contact(scene.view(), InputEvent::Contact {
                id,
                phase: ContactPhase::Down(smoke_touch_point(
                    scene, player, control, &profiles,
                )?),
            })?
            .ok_or_else(|| failure("movement touch outside scene"))?;
            controls.route(event, TICK_INTERVAL, simulation)?;
            contacts.push(id);
        }
    }
    let _advanced = simulation.advance(TICK_INTERVAL).map_err(failure)?;
    let moving = simulation.state().stations();
    verify_touch_movement(&before, &moving, profiles.touch_players)?;
    for id in contacts {
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
    let stopped = simulation.state().stations();
    if stopped
        .into_iter()
        .zip(moving)
        .any(|(after, previous)| after.position != previous.position)
    {
        return Err(failure("touch release failed"));
    }
    Ok(())
}

fn verify_touch_movement(
    initial: &[Station; 4],
    moved: &[Station; 4],
    selected: [bool; 4],
) -> Result<(), RenderError> {
    for ((before, moving), touch) in initial.iter().zip(moved).zip(selected) {
        let expected = if touch {
            Point {
                x: before.position.x.saturating_add(2),
                y: before.position.y.saturating_sub(2),
            }
        } else {
            before.position
        };
        if moving.position != expected {
            return Err(failure("simultaneous touch movement failed"));
        }
    }
    Ok(())
}

fn smoke_touch_point(
    scene: &SceneSession<'_, '_>,
    player: Player,
    control: Control,
    profiles: &Profiles,
) -> Result<TouchPoint, RenderError> {
    scene
        .surface_touch(
            touch::to_scene(
                touch::center(player, control, profiles)?,
                scene.layout().width().get(),
            )
            .ok_or(RenderError::InvalidDimensions)?,
        )?
        .ok_or_else(|| failure("touch control outside visible scene"))
}

fn smoke_cooperation(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
) -> Result<(), RenderError> {
    let enabled = art.poses.paired_families();
    let families = enabled.into_iter().filter(|active| *active).count();
    if families == 0 {
        return Ok(());
    }
    let mut cabinet = art.poses.configure_cabinet(
        &Cabinet::new(StreetLayout::DOWNTOWN).map_err(failure)?,
    );
    for player in Player::ALL {
        cabinet.input_step(0, &[
            LogicalAction {
                action: Action {
                    control: Control::Start,
                    player,
                },
                down: true,
            },
            LogicalAction {
                action: Action {
                    control: Control::Start,
                    player,
                },
                down: false,
            },
        ]);
    }
    let _drawn = art.present(scene, &cabinet)?;
    let before = scene.readback()?;
    for tick in 1..40 {
        cabinet.input_step(tick, &[]);
    }
    let _paired = art.present(scene, &cabinet)?;
    if art.world.len() != Player::ALL.len().saturating_sub(families)
        || before == scene.readback()?
    {
        return Err(failure("cooperative actors not visibly combined"));
    }
    smoke_cooperation_movement(scene, art, &mut cabinet, families)?;
    if art.poses.pair_jump(Pair::BartLisa) {
        smoke_cooperation_jump(scene, art, &mut cabinet)?;
        smoke_cooperation_primary(scene, art, &mut cabinet)?;
    }
    if art.poses.pair_jump(Pair::MargeHomer) {
        smoke_marge_homer_jump(scene, art, &mut cabinet)?;
    }
    writeln!(
        stdout().lock(),
        "Cooperative pose smoke: {families} families"
    )
    .map_err(failure)
}

fn smoke_cooperation_jump(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &mut Cabinet,
) -> Result<(), RenderError> {
    let actions = [
        (Player::Homer, Control::Right, false),
        (Player::Lisa, Control::Right, false),
        (Player::Bart, Control::Secondary, true),
        (Player::Bart, Control::Secondary, false),
    ]
    .map(|(player, control, down)| LogicalAction {
        action: Action { control, player },
        down,
    });
    let still = scene.readback()?;
    cabinet.input_step(49, &actions);
    let _rise = art.present(scene, cabinet)?;
    let opening = scene.readback()?;
    if still == opening || cabinet.pair_pose(Pair::BartLisa).is_none() {
        return Err(failure("combined jump opening not visible"));
    }
    for tick in 50..62 {
        cabinet.input_step(tick, &[]);
    }
    let _apex = art.present(scene, cabinet)?;
    if opening == scene.readback()? {
        return Err(failure("combined jump elevation not visible"));
    }
    for tick in 62..79 {
        cabinet.input_step(tick, &[]);
    }
    let _landed = art.present(scene, cabinet)?;
    if cabinet.pair_pose(Pair::BartLisa).is_none()
        || opening == scene.readback()?
    {
        return Err(failure("combined jump landing not visible"));
    }
    writeln!(stdout().lock(), "Bart/Lisa combined jump verified")
        .map_err(failure)
}

fn smoke_cooperation_movement(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &mut Cabinet,
    families: usize,
) -> Result<(), RenderError> {
    let still = scene.readback()?;
    let actions = [Player::Homer, Player::Lisa].map(|player| LogicalAction {
        action: Action {
            control: Control::Right,
            player,
        },
        down: true,
    });
    cabinet.input_step(40, &actions);
    for tick in 41..49 {
        cabinet.input_step(tick, &[]);
    }
    let _moving = art.present(scene, cabinet)?;
    if art.world.len() != Player::ALL.len().saturating_sub(families)
        || still == scene.readback()?
    {
        return Err(failure("cooperative locomotion not visibly presented"));
    }
    Ok(())
}

fn smoke_cooperation_primary(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &mut Cabinet,
) -> Result<(), RenderError> {
    let still = scene.readback()?;
    let actions = [true, false].map(|down| LogicalAction {
        action: Action {
            control: Control::Primary,
            player: Player::Lisa,
        },
        down,
    });
    cabinet.input_step(79, &actions);
    let _pressed = art.present(scene, cabinet)?;
    if cabinet.pair_pose(Pair::BartLisa).map(|pose| pose.motion)
        != Some(crate::PairMotion::Primary)
        || still == scene.readback()?
    {
        return Err(failure("combined primary response not visible"));
    }
    cabinet.input_step(80, &[]);
    let _released = art.present(scene, cabinet)?;
    if still != scene.readback()? {
        return Err(failure("combined primary did not restore idle art"));
    }
    writeln!(stdout().lock(), "Bart/Lisa combined Primary verified")
        .map_err(failure)
}

fn smoke_marge_homer_jump(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &mut Cabinet,
) -> Result<(), RenderError> {
    let still = scene.readback()?;
    let actions = [
        (Player::Homer, Control::Right, false),
        (Player::Lisa, Control::Right, false),
        (Player::Marge, Control::Secondary, true),
        (Player::Marge, Control::Secondary, false),
    ]
    .map(|(player, control, down)| LogicalAction {
        action: Action { control, player },
        down,
    });
    cabinet.input_step(81, &actions);
    let _opening = art.present(scene, cabinet)?;
    let opening = scene.readback()?;
    if still == opening {
        return Err(failure("Marge/Homer jump opening not visible"));
    }
    for tick in 82..93 {
        cabinet.input_step(tick, &[]);
    }
    let _apex = art.present(scene, cabinet)?;
    if opening == scene.readback()? {
        return Err(failure("Marge/Homer jump elevation not visible"));
    }
    for tick in 93..111 {
        cabinet.input_step(tick, &[]);
    }
    let _landed = art.present(scene, cabinet)?;
    if cabinet.pair_pose(Pair::MargeHomer).map(|pose| pose.motion)
        != Some(crate::PairMotion::AfterJump { tick: 0 })
        || opening == scene.readback()?
    {
        return Err(failure("Marge/Homer landing not visible"));
    }
    smoke_marge_homer_primary(scene, art, cabinet)?;
    let landed = scene.readback()?;
    cabinet.input_step(113, &[LogicalAction {
        action: Action {
            control: Control::Left,
            player: Player::Homer,
        },
        down: true,
    }]);
    let _walk = art.present(scene, cabinet)?;
    if cabinet.pair_pose(Pair::MargeHomer).map(|pose| pose.motion)
        != Some(crate::PairMotion::AfterJump { tick: 4 })
        || landed == scene.readback()?
    {
        return Err(failure("Marge/Homer post-landing walk not visible"));
    }
    writeln!(
        stdout().lock(),
        "Marge/Homer combined jump and walk verified"
    )
    .map_err(failure)
}

fn smoke_marge_homer_primary(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    cabinet: &mut Cabinet,
) -> Result<(), RenderError> {
    let landed = scene.readback()?;
    let stations = cabinet.stations();
    for (tick, player) in [(111, Player::Marge), (112, Player::Homer)] {
        let actions = [true, false].map(|down| LogicalAction {
            action: Action {
                control: Control::Primary,
                player,
            },
            down,
        });
        cabinet.input_step(tick, &actions);
        let _phase = art.present(scene, cabinet)?;
        let same = landed == scene.readback()?;
        let expected = if tick == 111 {
            1
        } else {
            2
        };
        if cabinet.stations() != stations
            || cabinet.pair_pose(Pair::MargeHomer).map(|pose| pose.motion)
                != Some(crate::PairMotion::AfterJump { tick: expected })
            || same != (tick == 111)
        {
            return Err(failure("Marge/Homer Primary phase not visible"));
        }
    }
    writeln!(stdout().lock(), "Marge/Homer paired Primary verified")
        .map_err(failure)
}

fn smoke_optional(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
    playback: Option<&mut audio::Playback<'_>>,
) -> Result<(), RenderError> {
    smoke_cooperation(scene, art)?;
    if art.combat.is_some() {
        encounter::smoke(scene, art, touch_enabled)?;
    }
    if let Some(audio) = playback {
        audio.smoke()?;
    }
    Ok(())
}

fn host_reservations(
    options: PreviewOptions<'_>,
    host: Option<&HostLan>,
) -> Result<[bool; 4], RenderError> {
    let reserved = host.map_or([false; 4], HostLan::reserved);
    if options.touch_enabled
        && touch::reserved(&options.controls.profiles, reserved)
    {
        return Err(failure("touch seat reserved for remote input"));
    }
    if let Some(listener) = host {
        writeln!(
            stdout().lock(),
            "LAN host: {}",
            listener.local_addr().map_err(failure)?
        )
        .map_err(failure)?;
    }
    Ok(reserved)
}

fn window(
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
    host: Option<HostLan>,
    load_started: Instant,
) -> Result<(), RenderError> {
    let touch_enabled = options.touch_enabled;
    let reserved = host_reservations(options, host.as_ref())?;
    let mut window = NativeWindow::open(
        "The Simpsons Arcade - movement prototype",
        dimension(if options.hidden && source.world.is_some() {
            1280
        } else {
            960
        })?,
        dimension(if touch_enabled {
            912
        } else {
            672
        })?,
        options.hidden,
    )?;
    window.apply_display_settings(options.display)?;
    let layout = preview_layout(touch_enabled, &options.controls.profiles)?;
    let limits = scene_limits(source, options, reserved)?;
    let startup = window
        .input(size(reserved.into_iter().filter(|remote| !remote).count())?)?;
    let mut playback =
        audio::open_optional(source.audio.as_ref(), options.audio_device)?;
    let renderer =
        (options.hidden || options.measure).then(|| window.renderer_name());
    window.with_scene(layout, limits, |scene| {
        let mut art = Scene::load(scene, source, options, reserved)?;
        if let Some(name) = renderer.as_deref() {
            startup_report(scene, name, load_started)?;
        }
        if options.hidden {
            smoke_settings(scene, &mut art, options)?;
            if let Some(bundle) = &source.world {
                world::smoke(scene, &mut art, bundle, options)?;
            }
            smoke_character_selection(scene, &mut art, options)?;
            smoke_controller_selection(&options.controls.profiles)?;
            if touch_enabled {
                smoke_touch_selection(scene, &options.controls.profiles)?;
            }
            smoke(scene, &mut art, touch_enabled, &options.controls.profiles)?;
            smoke_optional(scene, &mut art, touch_enabled, playback.as_mut())?;
            return smoke_peak_report();
        }
        let scroll_limit =
            art.world_art.map_or(Ok(0), world::WorldArt::scroll_limit)?;
        let session = LocalSession::with_preferences(
            options.display,
            options.volume,
            touch_enabled,
        );
        let cabinet = art.poses.configure_cabinet(&cabinet_with_scroll(
            source.enemy.is_some(),
            session,
            scroll_limit,
        )?);
        let preview = Interactive::new(startup, options, &cabinet, host)?;
        options.controls.persist().map_err(failure)?;
        preview.run(scene, &mut art, playback.as_mut())
    })
}

fn scene_limits(
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
    reserved: [bool; 4],
) -> Result<SceneLimits, RenderError> {
    let touch_enabled = options.touch_enabled;
    let enemy_atlases = if source.enemy.is_some() {
        3
    } else {
        0
    };
    let enemy_sprites: usize = if source.enemy.is_some() {
        encounter::EXTRA_SPRITES
    } else {
        0
    };
    let count = source.poses.count().saturating_add(enemy_atlases);
    let extension = usize::from(source.world.is_some());
    let shadows = usize::from(source.shadow.is_some());
    Ok(SceneLimits {
        atlas_bytes: size(
            3_607_872usize
                .saturating_add(station_hud::extra_bytes(reserved))
                .saturating_add(if touch_enabled {
                    touch::extra_bytes(&options.controls.profiles)
                } else {
                    0
                })
                .saturating_add(count.saturating_mul(65_536))
                .saturating_add(shadows.saturating_mul(shadow::MAX_BYTES))
                .saturating_add(extension.saturating_mul(
                    usize::try_from(world::atlas_budget()).map_err(failure)?,
                )),
        )?,
        atlases: size(
            count
                .saturating_add(extension)
                .saturating_add(shadows)
                .saturating_add(if touch_enabled {
                    9
                } else {
                    8
                }),
        )?,
        sprites: size(
            enemy_sprites
                .saturating_add(extension.saturating_mul(2))
                .saturating_add(shadows.saturating_mul(4))
                .saturating_add(if touch_enabled {
                    19
                } else {
                    18
                }),
        )?,
    })
}

fn world_capacity(combat: bool) -> usize {
    Player::ALL.len().saturating_add(
        usize::from(combat).saturating_mul(super::ENEMY_CAPACITY),
    )
}

fn preview_layout(
    touch_enabled: bool,
    profiles: &Profiles,
) -> Result<FrameLayout, RenderError> {
    let height = if touch_enabled {
        u32::from(touch::height(profiles)?)
    } else {
        world::BASE_HEIGHT
    };
    FrameLayout::new(
        dimension(world::BASE_WIDTH)?,
        dimension(height)?,
        size(
            320usize
                .saturating_mul(usize::from(touch::MAX_SCENE_HEIGHT))
                .saturating_mul(4),
        )?,
    )
}

#[cfg(test)]
mod tests {
    use games::input::physical::{
        ControllerAxis, ControllerButton, KeyboardKey,
    };
    use games::input::{InputEvent, LogicalAction};
    use games::replay::{InputStep as _, LiveSimulation};
    use games_platform::ir::frame::RenderError;
    use games_platform::ir::scene::EventBatch;

    use super::{
        Action, Cabinet, Control, Controls, DEFAULT_PROFILES, LiveInputLimits,
        NativeBatch, Phase, Player, Profiles, Station, StepClock, StreetLayout,
        TICK_INTERVAL, configured_profiles, dimension, failure, frame_due,
        live, presentation_wait, size,
    };
    use crate::remote::{HostedCabinet, HostedInput};

    type TestCabinet = (Controls, LiveSimulation<Cabinet>);

    fn route_now(
        controls: &mut Controls,
        simulation: &mut LiveSimulation<Cabinet>,
        event: InputEvent,
    ) -> Result<(), RenderError> {
        let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
        controls.route(event, TICK_INTERVAL.saturating_mul(tick), simulation)
    }

    fn button(device: u64, button: ControllerButton, down: bool) -> InputEvent {
        InputEvent::Button {
            device,
            code: button.code(),
            down,
        }
    }

    fn advance(
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<[Station; 4], RenderError> {
        let _step = simulation.advance(TICK_INTERVAL).map_err(failure)?;
        Ok(simulation.state().stations())
    }

    fn cabinet(profiles: &Profiles) -> Result<TestCabinet, RenderError> {
        let mut simulation = live(false)?;
        let mut controls = Controls::with_profiles(false, profiles)?;
        for device in 10..14u64 {
            route_now(&mut controls, &mut simulation, InputEvent::Attached {
                device,
            })?;
            for event in [
                button(device, ControllerButton::Start, true),
                button(device, ControllerButton::Start, false),
            ] {
                route_now(&mut controls, &mut simulation, event)?;
            }
        }
        let _stations = advance(&mut simulation)?;
        Ok((controls, simulation))
    }

    fn limited_host() -> Result<LiveSimulation<HostedCabinet>, RenderError> {
        let cabinet = Cabinet::new(StreetLayout::DOWNTOWN).map_err(failure)?;
        let hosted = HostedCabinet::new(&cabinet, [false; 4])
            .ok_or_else(|| failure("invalid hosted fixture"))?;
        let clock =
            StepClock::new(TICK_INTERVAL, dimension(8)?).map_err(failure)?;
        Ok(LiveSimulation::new(hosted, clock, LiveInputLimits {
            future_ticks: 1,
            pending_actions: size(1)?,
        }))
    }

    fn native_batch(control: Control, down: bool, tick: u32) -> NativeBatch {
        NativeBatch {
            actions: vec![HostedInput::Local(LogicalAction {
                action: Action {
                    control,
                    player: Player::Marge,
                },
                down,
            })],
            elapsed: TICK_INTERVAL.saturating_mul(tick),
        }
    }

    #[test]
    fn future_native_event_retains_its_fixed_tick_during_catchup()
    -> Result<(), RenderError> {
        let mut host = limited_host()?;
        let pending = native_batch(Control::Start, true, 3)
            .submit(&mut host, None)
            .ok_or_else(|| failure("future batch was admitted too early"))?;
        let _catchup = host
            .advance(TICK_INTERVAL.saturating_mul(2))
            .map_err(failure)?;
        if pending.submit(&mut host, None).is_some() {
            return Err(failure("batch rejected within future window"));
        }
        let _before_event = host.advance(TICK_INTERVAL).map_err(failure)?;
        if host.state().state().phase() != Phase::MainMenu {
            return Err(failure("future event executed before its tick"));
        }
        let _at_event = host.advance(TICK_INTERVAL).map_err(failure)?;
        if host.state().state().phase() != Phase::Playing {
            return Err(failure("retained native event did not execute"));
        }
        Ok(())
    }

    #[test]
    fn capacity_rejected_native_release_is_retried_without_losing_input()
    -> Result<(), RenderError> {
        let mut host = limited_host()?;
        for (control, tick) in [(Control::Start, 0), (Control::Right, 1)] {
            if native_batch(control, true, tick)
                .submit(&mut host, None)
                .is_some()
            {
                return Err(failure("initial native input rejected"));
            }
            let _step = host.advance(TICK_INTERVAL).map_err(failure)?;
        }
        if native_batch(Control::Start, false, 2)
            .submit(&mut host, None)
            .is_some()
        {
            return Err(failure("could not fill live queue"));
        }
        let pending = native_batch(Control::Right, false, 2)
            .submit(&mut host, None)
            .ok_or_else(|| failure("full live queue accepted release"))?;
        let _full_tick = host.advance(TICK_INTERVAL).map_err(failure)?;
        let [before, ..] = host.state().state().stations();
        if pending.submit(&mut host, None).is_some() {
            return Err(failure("late release was not readmitted"));
        }
        let _released_ticks = host
            .advance(TICK_INTERVAL.saturating_mul(3))
            .map_err(failure)?;
        let [after, ..] = host.state().state().stations();
        if before.position != after.position {
            return Err(failure("rejected release left movement held"));
        }
        Ok(())
    }

    #[test]
    fn menu_hints_follow_actual_launch_capabilities() {
        use super::{adaptive_hint, main_menu_hints};

        assert!(adaptive_hint(true).contains("EXTENDS STREET"));
        assert!(adaptive_hint(false).contains("ASPECT FIT"));
        assert!(
            main_menu_hints(true).contains(&"LOCAL START OPENS HOSTED GAME")
        );
        assert!(
            main_menu_hints(true).contains(&"REMOTE START JOINS DURING PLAY")
        );
        assert!(
            main_menu_hints(false).contains(&"PRESS YOUR START KEY TO JOIN")
        );
        assert!(
            !main_menu_hints(false)
                .iter()
                .any(|hint| hint.contains("REMOTE"))
        );
    }

    #[test]
    fn desktop_defaults_join_one_player_and_attack_with_mouse()
    -> Result<(), RenderError> {
        let mut simulation = live(false)?;
        let mut controls = Controls::new(false)?;
        super::smoke_press(&mut controls, &mut simulation, Control::Start)?;
        let [first, others @ ..] = simulation.state().stations();
        if first.participation != crate::Participation::Playing
            || others.iter().any(|station| {
                station.participation != crate::Participation::Absent
            })
        {
            return Err(failure("default admission joined extra players"));
        }
        for down in [true, false] {
            route_now(&mut controls, &mut simulation, InputEvent::Mouse {
                code: super::PointerButton::Left.code(),
                down,
            })?;
        }
        let [attacking, ..] = advance(&mut simulation)?;
        if !matches!(attacking.maneuver, crate::Maneuver::Attack {
            tick: 0,
            ..
        }) {
            return Err(failure("default mouse attack did not start"));
        }
        super::smoke_press(&mut controls, &mut simulation, Control::Pause)?;
        if simulation.state().phase() != Phase::Paused {
            return Err(failure("Escape did not pause"));
        }
        super::smoke_press(&mut controls, &mut simulation, Control::Pause)?;
        if simulation.state().phase() != Phase::Playing {
            return Err(failure("Escape did not resume"));
        }
        Ok(())
    }

    #[test]
    fn escape_routes_to_first_unreserved_local_seat() -> Result<(), RenderError>
    {
        let mut controls =
            Controls::with_reserved(false, &DEFAULT_PROFILES, [
                true, false, false, false,
            ])?;
        let key = |code: KeyboardKey, down| InputEvent::Key {
            code: code.code(),
            down,
        };
        for down in [true, false] {
            if controls.actions(key(KeyboardKey::ESCAPE, down))?
                != [LogicalAction {
                    action: Action {
                        control: Control::Pause,
                        player: Player::Homer,
                    },
                    down,
                }]
                || !controls.actions(key(KeyboardKey::F2, down))?.is_empty()
            {
                return Err(failure("global pause routed incorrectly"));
            }
        }
        Ok(())
    }

    #[test]
    fn suspended_frames_redraw_for_input_window_or_remote_state()
    -> Result<(), RenderError> {
        let cabinet = Cabinet::new(StreetLayout::DOWNTOWN).map_err(failure)?;
        let mut hosted =
            HostedCabinet::new(&cabinet, [false, true, false, false])
                .ok_or_else(|| failure("invalid hosted fixture"))?;
        let initial = hosted;
        if presentation_wait(&hosted) == 0
            || !frame_due(&hosted, None, EventBatch::Empty)
            || frame_due(&hosted, Some(&initial), EventBatch::Empty)
            || !frame_due(&hosted, Some(&initial), EventBatch::Drained)
            || !frame_due(&hosted, Some(&initial), EventBatch::Limited)
        {
            return Err(failure("static menu frame policy"));
        }
        hosted.input_step(0, &[HostedInput::Connected(1)]);
        if !frame_due(&hosted, Some(&initial), EventBatch::Empty) {
            return Err(failure("authenticated state did not redraw menu"));
        }
        let action = |control| {
            HostedInput::Local(LogicalAction {
                action: Action {
                    control,
                    player: Player::Marge,
                },
                down: true,
            })
        };
        hosted.input_step(1, &[action(Control::Start)]);
        if hosted.state().phase() != Phase::Playing
            || presentation_wait(&hosted) != 0
            || !frame_due(&hosted, Some(&hosted), EventBatch::Empty)
        {
            return Err(failure("gameplay was not uncapped"));
        }
        let playing = hosted;
        hosted.input_step(2, &[action(Control::Pause)]);
        if hosted.state().phase() != Phase::Paused
            || presentation_wait(&hosted) == 0
            || !frame_due(&hosted, Some(&playing), EventBatch::Empty)
            || frame_due(&hosted, Some(&hosted), EventBatch::Empty)
        {
            return Err(failure("paused frame policy"));
        }
        let paused = hosted;
        hosted.input_step(3, &[HostedInput::Disconnected(1)]);
        if !frame_due(&hosted, Some(&paused), EventBatch::Empty) {
            return Err(failure("disconnect did not invalidate paused frame"));
        }
        Ok(())
    }

    #[test]
    fn native_devices_skip_reserved_remote_seats() -> Result<(), RenderError> {
        let mut controls =
            Controls::with_reserved(false, &DEFAULT_PROFILES, [
                true, false, true, false,
            ])?;
        let _first = controls.actions(InputEvent::Attached { device: 10 })?;
        let _second = controls.actions(InputEvent::Attached { device: 11 })?;
        let _extra = controls.actions(InputEvent::Attached { device: 12 })?;
        if controls.devices != [None, Some(10), None, Some(11)] {
            return Err(failure("controller assigned a reserved seat"));
        }
        let local =
            controls.actions(button(10, ControllerButton::Start, true))?;
        if local.first().map(|change| change.action.player)
            != Some(Player::Homer)
        {
            return Err(failure("local controller assigned wrong seat"));
        }
        let remote = controls.actions(InputEvent::Key {
            code: KeyboardKey::ENTER.code(),
            down: true,
        })?;
        if !remote.is_empty()
            || Controls::with_reserved(true, &DEFAULT_PROFILES, [
                true, false, true, false,
            ])
            .is_ok()
        {
            return Err(failure(
                "reserved seat accepted local keyboard or touch",
            ));
        }
        Ok(())
    }

    #[test]
    fn hotplug_releases_only_the_removed_station_and_reuses_it()
    -> Result<(), RenderError> {
        let (mut controls, mut simulation) = cabinet(&DEFAULT_PROFILES)?;
        let start = simulation.state().stations();
        for device in 10..14u64 {
            route_now(
                &mut controls,
                &mut simulation,
                button(device, ControllerButton::DPadRight, true),
            )?;
        }
        let moving = advance(&mut simulation)?;
        if start
            .into_iter()
            .zip(moving)
            .any(|(before, after)| before.position == after.position)
        {
            return Err(failure("controller movement did not reach all slots"));
        }
        for event in [
            InputEvent::Detached { device: 11 },
            InputEvent::Attached { device: 14 },
            button(11, ControllerButton::DPadLeft, true),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let stopped = advance(&mut simulation)?;
        for (index, (before, after)) in
            moving.into_iter().zip(stopped).enumerate()
        {
            if (before.position == after.position) != (index == 1) {
                return Err(failure("disconnect affected the wrong stations"));
            }
        }
        route_now(
            &mut controls,
            &mut simulation,
            button(14, ControllerButton::DPadRight, true),
        )?;
        let [_, resumed, ..] = advance(&mut simulation)?;
        let [_, old_homer, ..] = stopped;
        if resumed.position.x <= old_homer.position.x {
            return Err(failure("replacement did not control vacant slot"));
        }
        Ok(())
    }

    #[test]
    fn remapping_releases_only_that_station_and_keeps_its_assignment()
    -> Result<(), RenderError> {
        let (mut controls, mut simulation) = cabinet(&DEFAULT_PROFILES)?;
        for device in 10..14u64 {
            route_now(
                &mut controls,
                &mut simulation,
                button(device, ControllerButton::DPadRight, true),
            )?;
        }
        let moving = advance(&mut simulation)?;
        for event in [
            InputEvent::ControllerRemapped { device: 11 },
            InputEvent::ControllerRemapped { device: 11 },
            button(11, ControllerButton::DPadRight, false),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let stopped = advance(&mut simulation)?;
        for (index, (before, after)) in
            moving.into_iter().zip(stopped).enumerate()
        {
            if (before.position == after.position) != (index == 1) {
                return Err(failure("remap affected the wrong stations"));
            }
        }
        route_now(
            &mut controls,
            &mut simulation,
            axis(11, ControllerAxis::LeftX, i16::MIN),
        )?;
        let [_, resumed, ..] = advance(&mut simulation)?;
        let [_, old_homer, ..] = stopped;
        if resumed.position.x >= old_homer.position.x {
            return Err(failure("remapped device lost its station"));
        }
        Ok(())
    }

    fn axis(device: u64, axis: ControllerAxis, value: i16) -> InputEvent {
        InputEvent::Axis {
            device,
            code: axis.code(),
            value,
        }
    }

    #[test]
    fn touch_panels_cannot_claim_reserved_remote_stations()
    -> Result<(), RenderError> {
        let profiles = configured_profiles("version=1\ntouch-players=1,3")
            .map_err(failure)?;
        if Controls::with_reserved(true, &profiles, [false, true, false, true])
            .is_err()
            || Controls::with_reserved(true, &profiles, [
                false, false, true, false,
            ])
            .is_ok()
            || Controls::with_reserved(false, &profiles, [
                false, false, true, false,
            ])
            .is_err()
        {
            return Err(failure("touch reservation admission mismatch"));
        }
        Ok(())
    }

    #[test]
    fn axis_profiles_isolate_station_thresholds() -> Result<(), RenderError> {
        let profiles = configured_profiles(concat!(
            "version=1\naxis 2 leftx=none/none\n",
            "axis 2 rightx=right/left\n",
            "axis-threshold 2 rightx=20000/5000\n",
        ))
        .map_err(failure)?;
        let (mut controls, mut simulation) = cabinet(&profiles)?;
        let [first, second, ..] = simulation.state().stations();
        for event in [
            axis(10, ControllerAxis::LeftX, 16_000),
            axis(11, ControllerAxis::LeftX, 32_767),
            axis(11, ControllerAxis::RightX, -19_999),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let [moved, waiting, ..] = advance(&mut simulation)?;
        if moved.position.x <= first.position.x
            || waiting.position != second.position
        {
            return Err(failure("axis profiles affected wrong station"));
        }
        for event in [
            axis(10, ControllerAxis::LeftX, 10_000),
            axis(11, ControllerAxis::RightX, -20_000),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let [stopped, pressed, ..] = advance(&mut simulation)?;
        if stopped.position != moved.position
            || pressed.position.x <= waiting.position.x
        {
            return Err(failure("axis press or release threshold ignored"));
        }
        route_now(
            &mut controls,
            &mut simulation,
            axis(11, ControllerAxis::RightX, -5_001),
        )?;
        let [_, held, ..] = advance(&mut simulation)?;
        if held.position.x <= pressed.position.x {
            return Err(failure("axis hysteresis did not retain hold"));
        }
        route_now(
            &mut controls,
            &mut simulation,
            axis(11, ControllerAxis::RightX, -5_000),
        )?;
        let [_, released, ..] = advance(&mut simulation)?;
        if released.position != held.position {
            return Err(failure("axis hysteresis did not release"));
        }
        verify_axis_replacement(&mut controls, &mut simulation, released)
    }

    fn verify_axis_replacement(
        controls: &mut Controls,
        simulation: &mut LiveSimulation<Cabinet>,
        released: Station,
    ) -> Result<(), RenderError> {
        for event in [
            InputEvent::Detached { device: 11 },
            InputEvent::Attached { device: 14 },
            axis(11, ControllerAxis::RightX, -32_767),
            axis(14, ControllerAxis::RightX, 20_000),
        ] {
            route_now(controls, simulation, event)?;
        }
        let [_, replacement, ..] = advance(simulation)?;
        if replacement.position.x >= released.position.x {
            return Err(failure("replacement ignored station axis profile"));
        }
        Ok(())
    }

    #[test]
    fn custom_profiles_route_only_the_configured_station_and_device()
    -> Result<(), RenderError> {
        let profiles =
            configured_profiles("version=1\nkey 1 right=Q\nbutton 2 right=x\n")
                .map_err(failure)?;
        let (mut controls, mut simulation) = cabinet(&profiles)?;
        let before = simulation.state().stations();
        for event in [
            InputEvent::Key {
                code: KeyboardKey::LETTER_D.code(),
                down: true,
            },
            button(11, ControllerButton::DPadRight, true),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let unchanged = advance(&mut simulation)?;
        if before != unchanged {
            return Err(failure("replaced bindings still routed"));
        }
        for event in [
            InputEvent::Key {
                code: KeyboardKey::LETTER_Q.code(),
                down: true,
            },
            button(11, ControllerButton::West, true),
        ] {
            route_now(&mut controls, &mut simulation, event)?;
        }
        let moved = advance(&mut simulation)?;
        for (index, (old, new)) in before.into_iter().zip(moved).enumerate() {
            if (new.position.x > old.position.x) != (index < 2) {
                return Err(failure("remapped action affected wrong station"));
            }
        }
        verify_remapped_replacement(&mut controls, &mut simulation)
    }

    fn verify_remapped_replacement(
        controls: &mut Controls,
        simulation: &mut LiveSimulation<Cabinet>,
    ) -> Result<(), RenderError> {
        for event in [
            InputEvent::Detached { device: 11 },
            InputEvent::Attached { device: 14 },
            button(14, ControllerButton::DPadRight, true),
        ] {
            route_now(controls, simulation, event)?;
        }
        let [_, waiting, ..] = advance(simulation)?;
        route_now(
            controls,
            simulation,
            button(14, ControllerButton::West, true),
        )?;
        let [_, moved, ..] = advance(simulation)?;
        if moved.position.x <= waiting.position.x {
            return Err(failure("replacement did not inherit profile"));
        }
        Ok(())
    }
}
