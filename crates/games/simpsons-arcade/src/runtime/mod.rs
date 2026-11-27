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
mod axis_smoke;
mod backdrop;
/// Local canonical directory browser launch.
pub mod browser_preview;
mod controller;
mod encounter;
mod lan;
mod launch;
mod measurement;
/// Path-independent event-driven title presentation.
pub mod memory_preview;
mod poses;
mod scenery;
mod shadow;
mod station_hud;
mod touch;
mod world;

use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::mem::take;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::rc::Rc;
use std::time::Duration;

use games::audio::{AudioSource, MasterVolume, SourceSettings};
use games::input::physical::{
    ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
};
use games::input::profile::editor::{
    ControlEditor, EditorEffect, EditorNavigation, EditorStatus,
};
use games::input::profile::{
    AxisProfile, DigitalBinding, DigitalProfiles, ProfileText,
};
use games::input::touch::{ContactId, ContactPhase, TouchPoint};
use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
use games::performance::{
    FrameObservation, PerformanceCollector, QualityProfile,
};
use games::presentation::{
    DisplaySettings, FramePacing, PresentationLayout, PresentationMode,
    WindowMode,
};
use games::replay::{
    InputStep, LiveInputLimits, LiveSimulation, QueueRejection, RejectedInput,
};
use games::session::{ControlReset, LocalSession};
use games::timing::StepClock;
use games_assets::{AssetFormat, ContentHash, ManifestLimits};
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::NativeWindow;
use games_platform::ir::graphics::scene_renderer::{SceneSession, SceneView};
use games_platform::ir::input::{
    HostEvent, InputStartup, NativeInput, controller_button_supported,
    key_from_name, key_name,
};
use games_platform::ir::measurement::{
    FrameMeter, elapsed_between, executable_bytes, now, peak_resident_bytes,
    resident_bytes,
};
use games_platform::ir::menu::{MenuArt, MenuContent};
use games_platform::ir::process::write_status;
use games_platform::ir::scene::{
    AtlasId, EventBatch, SceneLimits, SpriteDraw, SpriteRect,
};
use games_platform::ir::storage::bundle::{
    BundleSource, CanonicalBundle, PayloadView,
};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::preferences::{
    DisplayStore, PublicationPolicy, VolumeStore,
};
use games_platform::ir::ui::PlateLayout;
use games_platform::ir::ui::editor::{ControlEditorArt, ControlEditorView};

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
// Apply the same title-owned budget to snapshot, storage and parsing.
const MAX_PROFILE_BYTES: usize = 8192;
const MENU_WIDTH: u16 = 280;
const MENU_HEIGHT: u16 = 192;

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
type ModalActions = (LocalActions, Option<EditorNavigation>);
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

    fn close_editor(
        &mut self,
        releases: &mut LocalActions,
    ) -> Result<(), RenderError> {
        drop(self.router.release_all());
        // Closing is independent of the active gameplay binding and the
        // selected editing station.
        let player = Player::ALL
            .into_iter()
            .zip(self.reserved)
            .find_map(|(player, remote)| (!remote).then_some(player))
            .ok_or(RenderError::InvalidSprite)?;
        releases.extend([true, false].map(|down| LogicalAction {
            action: Action {
                control: Control::Pause,
                player,
            },
            down,
        }));
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

    fn modal_actions(
        &mut self,
        event: InputEvent,
        closing: bool,
    ) -> Result<ModalActions, RenderError> {
        // Release only holds already delivered to the running simulation.
        // Modal capture itself must not emit gameplay presses or releases.
        let releases = self.router.release_holds();
        let actions = self.actions(event)?;
        if closing {
            // Do not let a new contact outlive a pending modal exit.
            drop(self.router.release_all());
            return Ok((releases, None));
        }
        drop(self.router.release_holds());
        let navigation = if matches!(
            event,
            InputEvent::Contact {
                phase: ContactPhase::Down(_),
                ..
            } | InputEvent::Touch { down: true, .. }
        ) {
            actions
                .iter()
                .find(|change| change.down)
                .map(|change| touch_editor_command(change.action.control))
        } else {
            None
        };
        Ok((releases, navigation))
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

    fn smoke_event(
        &self,
        player: Player,
        control: Control,
        down: bool,
    ) -> Result<InputEvent, RenderError> {
        let index = CONTROLS
            .iter()
            .position(|item| *item == control)
            .ok_or(RenderError::InvalidSprite)?;
        let seat = player.index();
        if let Ok(key) = self.key(player, control) {
            return Ok(InputEvent::Key { code: key.code(), down });
        }
        if let Some(mouse) = self
            .profiles
            .mouse
            .get(seat)
            .and_then(|buttons| buttons.get(index))
            .copied()
            .flatten()
        {
            return Ok(InputEvent::Mouse { code: mouse.code(), down });
        }
        let device = self
            .devices
            .get(seat)
            .copied()
            .flatten()
            .ok_or_else(|| failure("smoke controller not assigned"))?;
        let button = self
            .profiles
            .buttons
            .get(seat)
            .and_then(|buttons| buttons.get(index))
            .ok_or(RenderError::InvalidSprite)?;
        Ok(InputEvent::Button {
            device,
            code: button.code(),
            down,
        })
    }

    fn with_panel(
        panel: Option<touch::PanelRole>,
        profiles: &Profiles,
        reserved: [bool; 4],
    ) -> Result<Self, RenderError> {
        profiles.validate().map_err(failure)?;
        validate_control_bindings(profiles).map_err(failure)?;
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

    fn with_smoke_profiles(
        touch_enabled: bool,
        profiles: &Profiles,
    ) -> Result<Self, RenderError> {
        let mut controls = Self::with_profiles(touch_enabled, profiles)?;
        for player in Player::ALL {
            let device = u64::try_from(player.index())
                .map_err(failure)?
                .saturating_add(1);
            if !controls
                .actions(InputEvent::Attached { device })?
                .is_empty()
            {
                return Err(failure("smoke controller attached with holds"));
            }
        }
        Ok(controls)
    }
}

#[derive(Clone, Copy, Debug)]
struct Art {
    atlas: AtlasId,
    rect: SpriteRect,
}

type LoadedArt<'bundle> = (Art, PayloadView<'bundle>);

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

    fn load_with_pixels<'bundle>(
        scene: &mut SceneSession<'_, '_>,
        bundle: &'bundle CanonicalBundle,
        key: &str,
    ) -> Result<LoadedArt<'bundle>, RenderError> {
        let record = bundle
            .manifest()
            .record(key)
            .ok_or_else(|| failure("missing art record"))?;
        let AssetFormat::Rgba8 { height, width } = record.format else {
            return Err(failure("art must be RGBA8"));
        };
        let pixels = bundle.view_payload(key).map_err(failure)?;
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
        Self::upload_layout(scene, layout, pixels)
    }

    fn upload_layout(
        scene: &mut SceneSession<'_, '_>,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<Self, RenderError> {
        let atlas = scene.load_atlas(layout, pixels)?;
        Ok(Self {
            atlas,
            rect: SpriteRect {
                height: layout.height(),
                width: layout.width(),
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
    editor: Option<ControlEditorArt>,
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
        source.poses.admit_world(world_art)?;
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
                .saturating_add(source.poses.scenic_sprite_count())
                .saturating_add(
                    usize::from(source.shadow.is_some()).saturating_mul(4),
                ),
            ),
            editor: load_editor(
                scene,
                options.can_reset_controls(reserved),
                touch_enabled,
            )?,
            hud: station_hud::StationHud::load(scene, profiles, reserved)?,
            menu: load_menu(scene, world_art.is_some(), options, reserved)?,
            panel: if touch_enabled {
                Some(touch::load(scene, profiles, touch::PanelRole::Cabinet)?)
            } else {
                None
            },
            poses: source.poses.load(source.store, scene)?,
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
        self.present_with_editor(scene, cabinet, None)
    }

    fn present_with_editor(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        cabinet: &Cabinet,
        editor: Option<(ControlEditor, &Profiles)>,
    ) -> Result<bool, RenderError> {
        let view = self.presentation_view(scene, cabinet)?;
        let width = view.width;
        let center =
            i32::try_from(world::centered_offset(width)).map_err(failure)?;
        self.draws.clear();
        self.queue_background(cabinet, view)?;
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
        if cabinet.phase() == Phase::Controls {
            let (selection, profiles) =
                editor.ok_or(RenderError::InvalidSprite)?;
            self.editor
                .as_ref()
                .ok_or(RenderError::InvalidSprite)?
                .queue(
                    ControlEditorView {
                        actions: ACTION_NAMES,
                        editor: selection,
                        key_name,
                        profiles,
                    },
                    (20i32.saturating_add(center), 16i32),
                    &mut self.draws,
                )?;
        }
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
        let width = if self.poses.backdrop.draw(cabinet).is_some() {
            let base = FrameLayout::new(
                dimension(world::BASE_WIDTH)?,
                scene.layout().height(),
                size(
                    320usize
                        .saturating_mul(usize::from(touch::MAX_SCENE_HEIGHT))
                        .saturating_mul(4),
                )?,
            )?;
            scene.fit_adaptive_width(base)?.width().get()
        } else if let Some(art) = self.world_art {
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

    fn queue_background(
        &mut self,
        cabinet: &Cabinet,
        view: world::View,
    ) -> Result<(), RenderError> {
        if let Some(draw) = self.poses.backdrop.draw(cabinet) {
            self.draws.push(draw);
        } else if let Some(art) = self.world_art {
            art.queue(self.background, view, &mut self.draws)?;
        } else {
            self.draws
                .push(self.background.draw(self.background.rect, false));
        }
        if cabinet.session().world_visible() {
            self.poses.scenery.queue(cabinet, view, &mut self.draws)?;
        }
        Ok(())
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
    audio: Option<audio::AudioSelection>,
    enemy: Option<[ContentHash; 2]>,
    poses: poses::Catalogue,
    shadow: Option<ContentHash>,
    world: Option<ContentHash>,
}

struct Sources<'source> {
    actors: [CanonicalBundle; 4],
    audio: Option<audio::Samples>,
    enemy: Option<[CanonicalBundle; 2]>,
    poses: poses::Catalogue,
    shadow: Option<CanonicalBundle>,
    store: BundleSource<'source>,
    world: Option<CanonicalBundle>,
}

impl<'source> Sources<'source> {
    fn load(
        store: BundleSource<'source>,
        actors: [ContentHash; 4],
        options: OptionalSources,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            actors: open_bundles(store, actors)?,
            audio: options
                .audio
                .map(|selection| audio::Samples::load(store, selection))
                .transpose()?,
            enemy: options
                .enemy
                .as_ref()
                .map(|hashes| open_bundles(store, *hashes))
                .transpose()?,
            poses: options.poses,
            shadow: options
                .shadow
                .as_ref()
                .map(|hash| open_bundle(store, *hash))
                .transpose()?,
            store,
            world: options
                .world
                .as_ref()
                .map(|hash| {
                    open_selected_bundle(store, *hash, world::bundle_limits()?)
                })
                .transpose()?,
        })
    }

    fn local(assets: &'source launch::Assets) -> Result<Self, RenderError> {
        Self::load(
            BundleSource::Local(&assets.directory),
            parse_hashes(&assets.actors)?,
            OptionalSources {
                audio: assets
                    .audio
                    .as_ref()
                    .map(audio::AudioSelection::from_local)
                    .transpose()?,
                enemy: assets.enemy.as_ref().map(parse_hashes).transpose()?,
                poses: assets
                    .poses
                    .as_deref()
                    .map(poses::Catalogue::open)
                    .transpose()?
                    .unwrap_or_default(),
                shadow: assets.shadow.as_deref().map(parse_hash).transpose()?,
                world: assets.world.as_deref().map(parse_hash).transpose()?,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowSwitching {
    Fixed,
    Supported,
}

#[derive(Clone, Copy, Debug)]
struct PreviewOptions<'profiles> {
    audio_device: Option<&'profiles str>,
    controls: &'profiles Rc<launch::ResolvedControls>,
    display: DisplaySettings,
    hidden: bool,
    measure: bool,
    preferences: Option<&'profiles Rc<DisplayStore>>,
    publication: PublicationPolicy,
    source_settings: SourceSettings,
    touch_enabled: bool,
    volume: MasterVolume,
    volume_store: Option<&'profiles Rc<VolumeStore>>,
    window_switching: WindowSwitching,
}

impl PreviewOptions<'_> {
    fn can_reset_controls(self, reserved: [bool; 4]) -> bool {
        !self.hidden
            && self.controls.can_reset()
            && (!self.touch_enabled
                || !touch::reserved(&DEFAULT_PROFILES, reserved))
    }

    fn session(self, reserved: [bool; 4]) -> LocalSession {
        LocalSession::with_preferences(self.display, self.volume, false)
            .with_source_settings(self.source_settings)
            .with_control_reset(self.can_reset_controls(reserved))
            .with_fixed_window(self.window_switching == WindowSwitching::Fixed)
    }
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

struct Interactive {
    applied: DisplaySettings,
    control_preferences: Rc<launch::ResolvedControls>,
    controls: Controls,
    deferred: Option<NativeBatch>,
    editor: ControlEditor,
    host: Option<HostLan>,
    input: NativeInput,
    measurement: Option<measurement::LiveMeasurement>,
    next_profiles: Profiles,
    preferences: Option<Rc<DisplayStore>>,
    presented: Option<HostedCabinet>,
    previous: Duration,
    publication: PublicationPolicy,
    requested: DisplaySettings,
    reset_persisted: bool,
    simulation: LiveSimulation<HostedCabinet>,
    source_settings: SourceSettings,
    stopped: bool,
    volume: MasterVolume,
    volume_store: Option<Rc<VolumeStore>>,
}

impl Interactive {
    const fn acknowledge_edit(&mut self, candidate: Option<&Profiles>) {
        self.editor.acknowledge(candidate.is_some());
        if let Some(profiles) = candidate {
            self.next_profiles = *profiles;
        }
    }

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

    fn edit_axis(
        &mut self,
        axis: ControllerAxis,
        profile: AxisProfile,
    ) -> Result<(), RenderError> {
        let candidate = self
            .control_preferences
            .persist_axis_edit(
                &self.next_profiles,
                (self.editor.station(), axis),
                profile,
            )
            .map_err(failure)?;
        self.acknowledge_edit(candidate.as_ref());
        Ok(())
    }

    fn edit_binding(
        &mut self,
        binding: DigitalBinding,
    ) -> Result<(), RenderError> {
        let candidate = self
            .control_preferences
            .persist_edit(
                &self.next_profiles,
                (self.editor.station(), self.editor.action()),
                binding,
            )
            .map_err(failure)?;
        self.acknowledge_edit(candidate.as_ref());
        Ok(())
    }

    fn edit_touch_player(&mut self, enabled: bool) -> Result<(), RenderError> {
        let station = self.editor.station();
        if enabled && self.controls.reserved.get(station) != Some(&false) {
            self.acknowledge_edit(None);
            return Ok(());
        }
        let candidate = self
            .control_preferences
            .persist_touch_player_edit(&self.next_profiles, station, enabled)
            .map_err(failure)?;
        self.acknowledge_edit(candidate.as_ref());
        Ok(())
    }

    fn editor_actions(
        &mut self,
        event: InputEvent,
    ) -> Result<LocalActions, RenderError> {
        let closing = self.editor.status() == EditorStatus::Closing;
        let (mut releases, navigation) =
            self.controls.modal_actions(event, closing)?;
        let effect = if let Some(command) = navigation {
            self.editor.navigate(command, &self.next_profiles)
        } else if matches!(event, InputEvent::Button { device, .. }
            if !self.controls.devices.contains(&Some(device)))
        {
            EditorEffect::None
        } else {
            self.editor.input(event, &self.next_profiles)
        };
        match effect {
            EditorEffect::Propose(binding) => self.edit_binding(binding)?,
            EditorEffect::ProposeTouchPlayer(enabled) => {
                self.edit_touch_player(enabled)?;
            },
            EditorEffect::ProposeAxis { axis, profile } => {
                self.edit_axis(axis, profile)?;
            },
            EditorEffect::Close => self.controls.close_editor(&mut releases)?,
            EditorEffect::None => {},
        }
        Ok(releases)
    }

    fn for_scene(
        startup: InputStartup,
        options: PreviewOptions<'_>,
        art: &Scene,
        host: Option<HostLan>,
    ) -> Result<Self, RenderError> {
        let reserved = host.as_ref().map_or([false; 4], HostLan::reserved);
        let scroll_limit =
            art.world_art.map_or(Ok(0), world::WorldArt::scroll_limit)?;
        let cabinet = art.poses.configure_cabinet(&cabinet_with_scroll(
            art.combat.is_some(),
            options.session(reserved),
            scroll_limit,
        )?);
        let preview = Self::new(startup, options, &cabinet, host)?;
        options.controls.persist().map_err(failure)?;
        Ok(preview)
    }

    fn measure_presented(
        &mut self,
        started: Option<Duration>,
        layout: Option<PresentationLayout>,
    ) -> Result<(), RenderError> {
        if self.simulation.state().state().phase() == Phase::Playing
            && let (Some(meter), Some(beginning)) =
                (&mut self.measurement, started)
        {
            let Some(submitted) = layout else {
                return meter.close_segment();
            };
            let completed = now().map_err(failure)?;
            meter.observe(&measurement::FrameSample {
                completed,
                display: self.applied,
                elapsed: self.input.elapsed()?,
                layout: submitted,
                work: elapsed_between(beginning, completed).map_err(failure)?,
            })?;
        }
        Ok(())
    }

    fn new(
        startup: InputStartup,
        options: PreviewOptions<'_>,
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
            control_preferences: Rc::clone(options.controls),
            controls,
            deferred: None,
            editor: ControlEditor::default(),
            host,
            input,
            measurement: options
                .measure
                .then(measurement::LiveMeasurement::default),
            next_profiles: options.controls.profiles,
            preferences: options.preferences.map(Rc::clone),
            presented: None,
            previous: Duration::ZERO,
            publication: options.publication,
            requested: cabinet.session().display(),
            reset_persisted: false,
            simulation,
            source_settings: cabinet.session().source_settings(),
            stopped: false,
            volume: cabinet.session().volume(),
            volume_store: options.volume_store.map(Rc::clone),
        })
    }

    fn next_delta(&mut self) -> Result<Duration, RenderError> {
        let elapsed = self.input.elapsed()?;
        let delta = elapsed
            .checked_sub(self.previous)
            .ok_or(RenderError::InvalidTimestamp)?;
        self.previous = elapsed;
        Ok(delta)
    }

    fn persist_preferences(
        &mut self,
        session: LocalSession,
    ) -> Result<(), RenderError> {
        let requested = session.display();
        if requested != self.requested {
            save_display(
                self.preferences.as_deref(),
                requested,
                self.publication,
            )?;
            self.requested = requested;
        }
        let volume = session.volume();
        if volume != self.volume {
            save_volume(
                self.volume_store.as_deref(),
                volume,
                self.publication,
            )?;
            self.volume = volume;
        }
        let sources = session.source_settings();
        if self.source_settings != sources {
            save_sources(
                self.volume_store.as_deref(),
                self.source_settings,
                sources,
                self.publication,
            )?;
            self.source_settings = sources;
        }
        if session.control_reset() != ControlReset::Requested {
            self.reset_persisted = false;
        } else if !self.reset_persisted {
            self.control_preferences
                .persist_defaults()
                .map_err(failure)?;
            self.next_profiles = DEFAULT_PROFILES;
            self.reset_persisted = true;
        }
        Ok(())
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
            let elapsed = self.input.elapsed()?;
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
        let changes =
            if self.simulation.state().state().phase() == Phase::Controls {
                self.editor_actions(event)?
            } else {
                self.controls.actions(event)?
            };
        let actions = changes.into_iter().map(Into::into).collect();
        self.deferred = NativeBatch { actions, elapsed }
            .submit(&mut self.simulation, self.measurement.as_mut());
        Ok(self.deferred.is_none())
    }

    fn run(
        mut self,
        scene: &mut SceneSession<'_, '_>,
        art: &mut Scene,
        mut playback: Option<&mut audio::Playback>,
    ) -> Result<(), RenderError> {
        if let Some(audio) = &mut playback {
            audio.enable_output();
        }
        loop {
            let wait_ms = presentation_wait(
                self.simulation.state(),
                scene.last_presented_layout().is_some(),
            );
            if !self.step(scene, (art, playback.as_deref_mut()), wait_ms)? {
                break;
            }
        }
        self.measurement
            .as_mut()
            .map_or(Ok(()), measurement::LiveMeasurement::finish)
    }

    /// Advance at most one event batch and one presentation. A caller that
    /// yields to its host between steps can select zero event wait.
    fn step(
        &mut self,
        scene: &mut SceneSession<'_, '_>,
        (art, playback): (&mut Scene, Option<&mut audio::Playback>),
        wait_ms: u32,
    ) -> Result<bool, RenderError> {
        if self.stopped
            || self.simulation.state().state().phase() == Phase::Exited
        {
            return Ok(false);
        }
        let started = self
            .measurement
            .as_ref()
            .map(|_| now())
            .transpose()
            .map_err(failure)?;
        let batch = self.poll(scene, wait_ms)?;
        if batch == EventBatch::Stopped {
            self.stopped = true;
            return Ok(false);
        }
        self.poll_host()?;
        let delta = self.next_delta()?;
        let old_phase = self.simulation.state().state().phase();
        let advanced = self.simulation.advance(delta).map_err(failure)?;
        self.advance_measurement(advanced.next_tick)?;
        let session = self.simulation.state().state().session();
        if old_phase != Phase::Controls && session.phase() == Phase::Controls {
            self.editor = ControlEditor::default();
        }
        let desired = session.effective_display();
        if desired != self.applied {
            scene.apply_display_settings(desired)?;
            self.applied = desired;
        }
        self.persist_preferences(session)?;
        let hosted = self.simulation.state();
        let cabinet = hosted.state();
        art.hud.update_connections(hosted.connected_remotes());
        if let Some(audio) = playback {
            audio.update(cabinet)?;
        }
        if frame_due(hosted, self.presented.as_ref(), batch)
            || !scene.presentation_is_current()?
        {
            let drawn = art.present_with_editor(
                scene,
                cabinet,
                Some((self.editor, &self.next_profiles)),
            )?;
            self.presented =
                (drawn && cabinet.phase() != Phase::Playing).then_some(*hosted);
            let submitted =
                scene.last_presented_layout().filter(|_layout| drawn);
            self.measure_presented(started, submitted)?;
        }
        Ok(session.phase() != Phase::Exited)
    }
}

type LocalAdmission<Input> = Result<Option<u64>, RejectedInput<Input>>;

fn configured_profiles(text: &str) -> Result<Profiles, String> {
    let profiles = DEFAULT_PROFILES.configured(
        ACTION_NAMES,
        ProfileText {
            max_bytes: MAX_PROFILE_BYTES,
            text,
        },
        key_from_name,
    )?;
    validate_control_bindings(&profiles)?;
    Ok(profiles)
}

fn validate_control_bindings(profiles: &Profiles) -> Result<(), String> {
    validate_keyboard_pause(profiles)?;
    if !profiles
        .buttons
        .iter()
        .flatten()
        .copied()
        .all(controller_button_supported)
    {
        return Err("unsupported controller button binding".into());
    }
    Ok(())
}

fn validate_keyboard_pause(profiles: &Profiles) -> Result<(), String> {
    let [first, others @ ..] = profiles.keys;
    let [pause, ..] = first;
    if pause != Some(KeyboardKey::ESCAPE)
        || others.iter().any(|[key, ..]| key.is_some())
    {
        return Err("Escape is the sole keyboard pause binding".into());
    }
    Ok(())
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

const fn presentation_wait(
    current: &HostedCabinet,
    drawable_available: bool,
) -> u32 {
    if drawable_available && matches!(current.state().phase(), Phase::Playing) {
        0
    } else {
        IDLE_EVENT_WAIT_MS
    }
}

fn save_display(
    store: Option<&DisplayStore>,
    settings: DisplaySettings,
    publication: PublicationPolicy,
) -> Result<(), RenderError> {
    if let Some(preferences) = store {
        let receipt = preferences.store(settings).map_err(failure)?;
        publication.confirm(receipt).map_err(failure)?;
    }
    Ok(())
}

fn save_volume(
    store: Option<&VolumeStore>,
    volume: MasterVolume,
    publication: PublicationPolicy,
) -> Result<(), RenderError> {
    if let Some(preferences) = store {
        let receipt = preferences.store(volume).map_err(failure)?;
        publication.confirm(receipt).map_err(failure)?;
    }
    Ok(())
}

fn save_sources(
    store: Option<&VolumeStore>,
    previous: SourceSettings,
    requested: SourceSettings,
    publication: PublicationPolicy,
) -> Result<(), RenderError> {
    let Some(preferences) = store else {
        return Ok(());
    };
    for source in [AudioSource::Effects, AudioSource::Music] {
        let Some(level) = requested.get(source) else {
            continue;
        };
        if previous.get(source) != Some(level) {
            let receipt =
                preferences.store_source(source, level).map_err(failure)?;
            publication.confirm(receipt).map_err(failure)?;
        }
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

const fn settings_hint(
    hidden: bool,
    persistent: bool,
    editor: bool,
) -> &'static str {
    if hidden {
        "SMOKE DOES NOT SAVE SETTINGS"
    } else if !persistent {
        "SETTINGS ARE SESSION ONLY"
    } else if editor {
        "CONTROL EDITS APPLY NEXT LAUNCH"
    } else {
        "DISPLAY AND VOLUME SAVED"
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
            "PRESS YOUR START CONTROL TO JOIN",
            "GAMEPAD / TOUCH: USE JOIN CONTROL",
            "UP/DOWN SELECT - PRIMARY CONFIRM",
        ]
    }
}

fn desktop_hint(
    key: Option<KeyboardKey>,
    mouse: Option<PointerButton>,
) -> String {
    let keyboard = key.map(|code| {
        let label = key_label(code);
        // Two 13-character keys with mouse labels fit the pause plate.
        if label.len() > 13 {
            "PROFILE"
        } else {
            label
        }
    });
    match (keyboard, mouse) {
        (Some(label), Some(button)) => {
            format!("{label}/MOUSE {}", mouse_label(button))
        },
        (None, Some(button)) => format!("MOUSE {}", mouse_label(button)),
        (Some(label), None) => String::from(label),
        (None, None) => String::from("NONE"),
    }
}

fn pause_hints(profiles: &Profiles, reserved: [bool; 4]) -> [String; 4] {
    let index = reserved.iter().position(|remote| !remote).unwrap_or(0);
    let [pause, ..] = profiles.keys.first().copied().unwrap_or([None; 8]);
    let [_, _, primary, secondary, left, right, up, down] =
        profiles.keys.get(index).copied().unwrap_or([None; 8]);
    let [mouse_pause, _, attack, jump, m_left, m_right, m_up, m_down] =
        profiles.mouse.get(index).copied().unwrap_or([None; 8]);
    [
        format!(
            "P{} LEFT: {} RIGHT: {}",
            index.saturating_add(1),
            desktop_hint(left, m_left),
            desktop_hint(right, m_right)
        ),
        format!(
            "UP: {} DOWN: {}",
            desktop_hint(up, m_up),
            desktop_hint(down, m_down)
        ),
        format!(
            "ATTACK: {} JUMP: {}",
            desktop_hint(primary, attack),
            desktop_hint(secondary, jump)
        ),
        format!(
            "{} OR START RESUMES - JOIN BELOW",
            desktop_hint(pause, mouse_pause)
        ),
    ]
}

const fn touch_editor_command(control: Control) -> EditorNavigation {
    match control {
        Control::Pause => EditorNavigation::Cancel,
        Control::Start => EditorNavigation::Clear,
        Control::Primary => EditorNavigation::Confirm,
        Control::Secondary => EditorNavigation::Family,
        Control::Left => EditorNavigation::Left,
        Control::Right => EditorNavigation::Right,
        Control::Up => EditorNavigation::Up,
        Control::Down => EditorNavigation::Down,
    }
}

fn load_editor(
    scene: &mut SceneSession<'_, '_>,
    enabled: bool,
    touch_enabled: bool,
) -> Result<Option<ControlEditorArt>, RenderError> {
    enabled
        .then(|| {
            ControlEditorArt::load(
                scene,
                PlateLayout {
                    background: [24, 22, 30, 255],
                    height: MENU_HEIGHT,
                    max_bytes: size(ControlEditorArt::bytes(
                        MENU_WIDTH,
                        MENU_HEIGHT,
                    ))?,
                    width: MENU_WIDTH,
                },
                touch_enabled.then_some([
                    "TOUCH: ARROWS, B SOURCE, A EDIT",
                    "TOUCH: START CLEAR, PAUSE BACK",
                ]),
            )
        })
        .transpose()
}

fn load_menu(
    scene: &mut SceneSession<'_, '_>,
    world_available: bool,
    options: PreviewOptions<'_>,
    reserved: [bool; 4],
) -> Result<MenuArt, RenderError> {
    let hints = pause_hints(&options.controls.profiles, reserved);
    let pause = hints.each_ref().map(String::as_str);
    let reset = options.can_reset_controls(reserved);
    MenuArt::load(
        scene,
        PlateLayout {
            background: [24, 22, 30, 255],
            height: MENU_HEIGHT,
            max_bytes: size(MenuArt::bytes(
                MENU_WIDTH,
                MENU_HEIGHT,
                reset,
                options.source_settings,
            ))?,
            width: MENU_WIDTH,
        },
        MenuContent {
            control_reset: reset,
            main_hints: main_menu_hints(reserved.contains(&true)),
            pause_hints: &pause,
            settings_hints: &[
                "PRIMARY CHANGES VALUE - PAUSE RETURNS",
                adaptive_hint(world_available),
                settings_hint(
                    options.hidden,
                    options.preferences.is_some(),
                    options.can_reset_controls(reserved),
                ),
            ],
            sources: options.source_settings,
            title: "THE SIMPSONS ARCADE",
            window_switching: options.window_switching
                == WindowSwitching::Supported,
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

fn parse_hash(digest: &OsStr) -> Result<ContentHash, RenderError> {
    digest
        .to_str()
        .ok_or_else(|| failure("invalid hash"))?
        .parse()
        .map_err(failure)
}

fn parse_hashes<const COUNT: usize>(
    hashes: &[OsString; COUNT],
) -> Result<[ContentHash; COUNT], RenderError> {
    hashes
        .iter()
        .map(|digest| parse_hash(digest))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_error| failure("bundle count"))
}

fn art_bundle_limits() -> Result<ManifestLimits, RenderError> {
    Ok(ManifestLimits {
        manifest_bytes: size(32_768)?,
        payload_bytes: NonZeroU64::new(1_048_576)
            .ok_or(RenderError::TooLarge)?,
        records: size(33)?,
        total_bytes: NonZeroU64::new(33_554_432)
            .ok_or(RenderError::TooLarge)?,
    })
}

fn open_bundle(
    store: BundleSource<'_>,
    digest: ContentHash,
) -> Result<CanonicalBundle, RenderError> {
    open_selected_bundle(store, digest, art_bundle_limits()?)
}

fn open_selected_bundle(
    store: BundleSource<'_>,
    digest: ContentHash,
    limits: ManifestLimits,
) -> Result<CanonicalBundle, RenderError> {
    let bundle = store.open(digest, limits).map_err(failure)?;
    let source = bundle.manifest().source();
    if source.title != "simpsons-arcade" || source.revision != "world-4p-set-1"
    {
        return Err(failure("unexpected source edition"));
    }
    Ok(bundle)
}

fn open_bundles<const COUNT: usize>(
    store: BundleSource<'_>,
    hashes: [ContentHash; COUNT],
) -> Result<[CanonicalBundle; COUNT], RenderError> {
    hashes
        .into_iter()
        .map(|hash| open_bundle(store, hash))
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
    if args.first().is_some_and(|arg| arg == "--edit-control") {
        return launch::edit_control(&args);
    }
    if matches!(args.as_slice(), [arg] if arg == "--list-audio-devices") {
        return audio::list_devices().map_err(|error| format!("{error:?}"));
    }
    if !NativeWindow::supported() {
        return Err(format!("{:?}", RenderError::Unsupported));
    }
    if args.first().is_some_and(|arg| arg == "--lan-controller") {
        return controller::run(args);
    }
    run_preview(args)
}

fn run_preview(args: Vec<OsString>) -> Result<(), String> {
    let load_started = now().map_err(|error| error.to_string())?;
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
    let resolved = Rc::new(controls.resolve()?);
    let display_store = preferences.map(Rc::new);
    let shared_volume_store = volume_store.map(Rc::new);
    let listener = host.as_deref().map(lan::open_host).transpose()?;
    let result = Sources::local(&assets).and_then(|source| {
        let source_settings = audio::source_settings(
            source.audio.as_ref(),
            shared_volume_store.as_deref(),
        )?;
        window(
            &source,
            PreviewOptions {
                audio_device: audio_device.as_deref(),
                controls: &resolved,
                display,
                hidden: smoke,
                measure,
                preferences: display_store.as_ref(),
                publication: PublicationPolicy::Durable,
                source_settings,
                touch_enabled,
                volume,
                volume_store: shared_volume_store.as_ref(),
                window_switching: WindowSwitching::Supported,
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

fn read_profile(path: &OsStr) -> Result<String, String> {
    let bytes = snapshot(path, profile_limit()?).map_err(|error| {
        format!("control profile {}: {error:?}", path.to_string_lossy())
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
    let mut controls = Controls::with_smoke_profiles(touch_enabled, profiles)?;
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
            controls.route(
                controls.smoke_event(Player::Marge, control, down)?,
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
    let mut controls = Controls::with_smoke_profiles(
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
    let directions = axis_smoke::verify(profiles)?;
    write_status(format_args!(
        "Synthetic controller axes: {directions} directions verified"
    ))
    .map_err(failure)
}

fn smoke_touch_selection(
    scene: &SceneSession<'_, '_>,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    if options.display.presentation == PresentationMode::Fill {
        smoke_protected_touch(scene)?;
    }
    let profiles = &options.controls.profiles;
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

fn smoke_protected_touch(
    scene: &SceneSession<'_, '_>,
) -> Result<(), RenderError> {
    let layout = scene
        .visible_layout()?
        .ok_or(RenderError::InvalidDimensions)?;
    let panel = layout.protected.ok_or(RenderError::InvalidDimensions)?;
    if panel.source.y != world::BASE_HEIGHT
        || panel.source.x != 0
        || panel.source.width != layout.design.width.get()
        || panel.source.y.checked_add(panel.source.height)
            != Some(layout.design.height.get())
        || layout.destination.height > panel.destination.y
    {
        return Err(failure("Fill did not preserve the full touch panel"));
    }
    write_status(format_args!(
        "Protected Fill touch: {}x{} drawable; {}x{} panel at {},{}",
        layout.surface.width,
        layout.surface.height,
        panel.destination.width,
        panel.destination.height,
        panel.destination.x,
        panel.destination.y,
    ))
    .map_err(failure)
}

fn smoke_settings(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    options: PreviewOptions<'_>,
) -> Result<(), RenderError> {
    let session = LocalSession::with_display(DisplaySettings::default(), false)
        .with_source_settings(options.source_settings);
    let mut simulation = live_with_session(false, session)?;
    let mut controls = Controls::with_smoke_profiles(
        options.touch_enabled,
        &options.controls.profiles,
    )?;
    scene.apply_display_settings(session.effective_display())?;
    let _drawn = art.present(scene, simulation.state())?;
    let before = scene.readback()?;
    for control in [Control::Down, Control::Primary] {
        smoke_press(&mut controls, &mut simulation, control)?;
    }
    for mode in [PresentationMode::Integer, PresentationMode::Fill] {
        smoke_press(&mut controls, &mut simulation, Control::Primary)?;
        let settings = simulation.state().session();
        if settings.phase() != Phase::Settings
            || settings.display().presentation != mode
        {
            return Err(failure("display menu selection failed"));
        }
        if mode == PresentationMode::Fill
            && settings.effective_display().presentation
                != PresentationMode::AspectFit
        {
            return Err(failure("Fill settings menu must remain visible"));
        }
        scene.apply_display_settings(settings.effective_display())?;
        let _updated = art.present(scene, simulation.state())?;
        if scene.readback()? == before {
            return Err(failure("settings were not visibly presented"));
        }
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
    smoke_audio_menu(scene, art, controls, simulation)
}

fn smoke_audio_menu(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    smoke_press(controls, simulation, Control::Primary)?;
    if simulation.state().phase() != Phase::Audio {
        return Err(failure("audio submenu did not open"));
    }
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
    smoke_source_levels(scene, art, controls, simulation)?;
    smoke_press(controls, simulation, Control::Start)?;
    if simulation.state().phase() != Phase::Settings {
        return Err(failure("audio did not return to settings"));
    }
    smoke_press(controls, simulation, Control::Start)?;
    if simulation.state().phase() != Phase::MainMenu {
        return Err(failure("settings did not return to main menu"));
    }
    Ok(())
}

fn smoke_source_levels(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
) -> Result<(), RenderError> {
    for source in [AudioSource::Effects, AudioSource::Music] {
        let mut expected = simulation.state().session().source_settings();
        let Some(level) = expected.get(source) else {
            continue;
        };
        match source {
            AudioSource::Effects => expected.effects = Some(level.next()),
            AudioSource::Music => expected.music = Some(level.next()),
        }
        smoke_press(controls, simulation, Control::Down)?;
        let _drawn = art.present(scene, simulation.state())?;
        let before = scene.readback()?;
        smoke_press(controls, simulation, Control::Primary)?;
        let session = simulation.state().session();
        if session.source_settings() != expected
            || session.volume() != MasterVolume::ThreeQuarters
        {
            return Err(failure("source volume menu selection failed"));
        }
        let _updated = art.present(scene, simulation.state())?;
        if scene.readback()? == before {
            return Err(failure("source volume row did not update"));
        }
    }
    Ok(())
}

fn smoke_press(
    controls: &mut Controls,
    simulation: &mut LiveSimulation<Cabinet>,
    control: Control,
) -> Result<(), RenderError> {
    let tick = u32::try_from(simulation.next_tick()).map_err(failure)?;
    for down in [true, false] {
        controls.route(
            controls.smoke_event(Player::Marge, control, down)?,
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
    for (player, touch) in Player::ALL.into_iter().zip(profiles.touch_players) {
        if touch_enabled && touch {
            smoke_touch(
                scene,
                controls,
                simulation,
                (player, &[Control::Start]),
            )?;
            continue;
        }
        for down in [true, false] {
            controls.route(
                controls.smoke_event(player, Control::Start, down)?,
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
        controls.smoke_event(Player::Marge, Control::Secondary, down)?,
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
    write_status(format_args!("Whole-smoke lifetime peak RSS: {peak} bytes"))
        .map_err(failure)
}

fn startup_report(
    scene: &SceneSession<'_, '_>,
    renderer: &str,
    started: Duration,
) -> Result<(), RenderError> {
    let completed = now().map_err(failure)?;
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
    if let Ok(bytes) = executable_bytes() {
        meter.record_artifact_bytes(bytes);
    }
    write_status(format_args!(
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
    ))
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
            let started = now().map_err(failure)?;
            let _advanced =
                simulation.advance(TICK_INTERVAL).map_err(failure)?;
            if !art.present(scene, simulation.state())? {
                return Err(failure("movement frame was not submitted"));
            }
            collector
                .observe(FrameObservation {
                    frame_time: elapsed_between(
                        started,
                        now().map_err(failure)?,
                    )
                    .map_err(failure)?,
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
    for player in Player::ALL {
        let (release, press) = if rightward {
            (Control::Left, Control::Right)
        } else {
            (Control::Right, Control::Left)
        };
        for (control, down) in [(release, false), (press, true)] {
            controls.route(
                controls.smoke_event(player, control, down)?,
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
    write_status(format_args!(
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
    ))
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
    for player in Player::ALL {
        for down in [true, false] {
            controls.route(
                controls.smoke_event(player, Control::Primary, down)?,
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
    write_status(format_args!("Cooperative pose smoke: {families} families"))
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
    write_status(format_args!("Bart/Lisa combined jump verified"))
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
    write_status(format_args!("Bart/Lisa combined Primary verified"))
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
    write_status(format_args!("Marge/Homer combined jump and walk verified"))
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
    write_status(format_args!("Marge/Homer paired Primary verified"))
        .map_err(failure)
}

fn smoke_optional(
    scene: &mut SceneSession<'_, '_>,
    art: &mut Scene,
    touch_enabled: bool,
    playback: Option<&mut audio::Playback>,
) -> Result<(), RenderError> {
    smoke_cooperation(scene, art)?;
    scenery::smoke(scene, art)?;
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
        write_status(format_args!(
            "LAN host: {}",
            listener.local_addr().map_err(failure)?
        ))
        .map_err(failure)?;
    }
    Ok(reserved)
}

fn window(
    source: &Sources<'_>,
    options: PreviewOptions<'_>,
    host: Option<HostLan>,
    load_started: Duration,
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
    let mut playback = audio::open_optional(
        source.audio.as_ref(),
        options.audio_device,
        options.source_settings,
        options.volume,
    )?;
    let renderer =
        (options.hidden || options.measure).then(|| window.renderer_name());
    window.with_scene(layout, limits, |scene| {
        scene.protect_bottom(NonZeroU32::new(
            layout.height().get().saturating_sub(world::BASE_HEIGHT),
        ))?;
        let mut art = Scene::load(scene, source, options, reserved)?;
        if let Some(name) = renderer.as_deref() {
            startup_report(scene, name, load_started)?;
        }
        if options.hidden {
            smoke_settings(scene, &mut art, options)?;
            if let Some(bundle) = &source.world {
                let backdrop = take(&mut art.poses.backdrop);
                let result = world::smoke(scene, &mut art, bundle, options);
                art.poses.backdrop = backdrop;
                result?;
            }
            smoke_character_selection(scene, &mut art, options)?;
            smoke_controller_selection(&options.controls.profiles)?;
            if touch_enabled {
                smoke_touch_selection(scene, options)?;
            }
            smoke(scene, &mut art, touch_enabled, &options.controls.profiles)?;
            smoke_optional(scene, &mut art, touch_enabled, playback.as_mut())?;
            backdrop::smoke(scene, &mut art, source)?;
            return smoke_peak_report();
        }
        let preview = Interactive::for_scene(startup, options, &art, host)?;
        preview.run(scene, &mut art, playback.as_mut())
    })
}

fn editor_limits(enabled: bool) -> (usize, usize) {
    if enabled {
        (
            ControlEditorArt::bytes(MENU_WIDTH, MENU_HEIGHT),
            ControlEditorArt::sprites(MENU_WIDTH).saturating_add(2),
        )
    } else {
        (0, 0)
    }
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
    let editor = options.can_reset_controls(reserved);
    let (editor_bytes, editor_sprites) = editor_limits(editor);
    Ok(SceneLimits {
        atlas_bytes: size(
            2_317_632usize
                .saturating_add(MenuArt::bytes(
                    MENU_WIDTH,
                    MENU_HEIGHT,
                    editor,
                    options.source_settings,
                ))
                .saturating_add(editor_bytes)
                .saturating_add(station_hud::extra_bytes(reserved))
                .saturating_add(if touch_enabled {
                    touch::extra_bytes(&options.controls.profiles)
                } else {
                    0
                })
                .saturating_add(source.poses.atlas_bytes())
                .saturating_add(enemy_atlases.saturating_mul(65_536))
                .saturating_add(shadows.saturating_mul(shadow::MAX_BYTES))
                .saturating_add(extension.saturating_mul(
                    usize::try_from(world::atlas_budget()).map_err(failure)?,
                )),
        )?,
        atlases: size(
            count
                .saturating_add(extension)
                .saturating_add(shadows)
                .saturating_add(usize::from(editor).saturating_mul(2))
                .saturating_add(
                    8usize.saturating_add(usize::from(touch_enabled)),
                ),
        )?,
        sprites: size(
            enemy_sprites
                .saturating_add(source.poses.scenic_sprite_count())
                .saturating_add(editor_sprites)
                .saturating_add(extension.saturating_mul(2))
                .saturating_add(shadows.saturating_mul(4))
                .saturating_add(
                    18usize.saturating_add(usize::from(touch_enabled)),
                ),
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
    use std::collections::BTreeMap;
    use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

    use games::input::physical::{
        ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
    };
    use games::input::profile::editor::EditorNavigation;
    use games::input::{InputEvent, LogicalAction};
    use games::replay::{InputStep as _, LiveSimulation};
    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
        SourceIdentity,
    };
    use games_platform::ir::frame::RenderError;
    use games_platform::ir::scene::EventBatch;
    use games_platform::ir::storage::bundle::{
        BundleSource, MemoryBundleLimits, MemoryBundleStore,
    };

    use super::{
        Action, CONTROLS, Cabinet, Control, Controls, DEFAULT_PROFILES, KEYS,
        LiveInputLimits, NativeBatch, OptionalSources, Phase, Player, Profiles,
        Sources, Station, StepClock, StreetLayout, TICK_INTERVAL,
        configured_profiles, dimension, failure, frame_due, live, poses,
        presentation_wait, size,
    };
    use crate::remote::{HostedCabinet, HostedInput};

    type TestCabinet = (Controls, LiveSimulation<Cabinet>);
    type MemoryActor = (ContentHash, Vec<u8>);

    fn memory_actor(
        revision: &str,
        object: ContentHash,
    ) -> Result<MemoryActor, RenderError> {
        let limits = ManifestLimits {
            manifest_bytes: size(4096)?,
            payload_bytes: NonZeroU64::MIN.saturating_add(3),
            records: size(KEYS.len())?,
            total_bytes: NonZeroU64::MIN.saturating_add(15),
        };
        let records = KEYS
            .into_iter()
            .map(|key| AssetRecord {
                digest: object,
                format: AssetFormat::Rgba8 {
                    height: NonZeroU32::MIN,
                    width: NonZeroU32::MIN,
                },
                key: String::from(key),
                size: 4,
            })
            .collect();
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from(revision),
                title: String::from("simpsons-arcade"),
            },
            records,
            limits,
        )
        .map_err(failure)?;
        let bytes = manifest.encode(limits).map_err(failure)?;
        Ok((ContentHash::calculate(&bytes), bytes))
    }

    #[test]
    fn title_source_admits_pinned_memory_bundles_without_local_paths()
    -> Result<(), RenderError> {
        let pixel = [20, 40, 60, 255];
        let object = ContentHash::calculate(&pixel);
        let (valid, valid_bytes) = memory_actor("world-4p-set-1", object)?;
        let (foreign, foreign_bytes) = memory_actor("other-edition", object)?;
        let store = MemoryBundleStore::new(
            BTreeMap::from([(valid, valid_bytes), (foreign, foreign_bytes)]),
            BTreeMap::from([(object, pixel.to_vec())]),
            MemoryBundleLimits {
                bytes: NonZeroU64::MIN.saturating_add(8191),
                manifests: NonZeroUsize::MIN.saturating_add(1),
                objects: NonZeroUsize::MIN,
            },
        )
        .map_err(failure)?;
        let actors = [valid; 4];
        let options = || OptionalSources {
            audio: None,
            enemy: None,
            poses: poses::Catalogue::default(),
            shadow: None,
            world: None,
        };
        let source =
            Sources::load(BundleSource::Memory(&store), actors, options())?;
        for (actor, key) in source.actors.iter().zip(KEYS) {
            if actor.read_payload(key).map_err(failure)? != pixel {
                return Err(failure("selected memory actor changed"));
            }
        }
        let foreign_actors = [foreign; 4];
        if Sources::load(
            BundleSource::Memory(&store),
            foreign_actors,
            options(),
        )
        .is_ok()
        {
            return Err(failure("foreign memory edition admitted"));
        }
        Ok(())
    }

    #[test]
    fn editor_capture_releases_holds_without_leaking_gameplay_presses()
    -> Result<(), RenderError> {
        let mut controls = Controls::new(false)?;
        let held = controls.actions(InputEvent::Key {
            code: KeyboardKey::LETTER_W.code(),
            down: true,
        })?;
        if held
            != [LogicalAction {
                action: Action {
                    control: Control::Up,
                    player: Player::Marge,
                },
                down: true,
            }]
        {
            return Err(failure("initial key not routed"));
        }
        let (modal, navigation) = controls.modal_actions(
            InputEvent::Key {
                code: KeyboardKey::LETTER_D.code(),
                down: true,
            },
            false,
        )?;
        if navigation.is_some() {
            return Err(failure("non-touch event produced touch command"));
        }
        if modal.iter().any(|change| change.down)
            || !modal.contains(&LogicalAction {
                action: Action {
                    control: Control::Up,
                    player: Player::Marge,
                },
                down: false,
            })
        {
            return Err(failure("modal captured presses or left holds"));
        }
        verify_editor_close(&mut controls)
    }

    fn verify_editor_close(controls: &mut Controls) -> Result<(), RenderError> {
        let (mut close, navigation) = controls.modal_actions(
            InputEvent::Key {
                code: KeyboardKey::ESCAPE.code(),
                down: true,
            },
            false,
        )?;
        if navigation.is_some() {
            return Err(failure("non-touch event produced touch command"));
        }
        controls.close_editor(&mut close)?;
        if close
            != [true, false].map(|down| LogicalAction {
                action: Action {
                    control: Control::Pause,
                    player: Player::Marge,
                },
                down,
            })
        {
            return Err(failure("close did not route one pause"));
        }
        if !controls
            .actions(InputEvent::Key {
                code: KeyboardKey::LETTER_D.code(),
                down: false,
            })?
            .is_empty()
        {
            return Err(failure("captured release leaked"));
        }
        Ok(())
    }

    #[test]
    fn modal_close_does_not_depend_on_the_current_gamepad_binding()
    -> Result<(), RenderError> {
        let mut controls =
            Controls::with_reserved(false, &DEFAULT_PROFILES, [
                true, false, true, true,
            ])?;
        drop(controls.actions(InputEvent::Attached { device: 1 })?);
        let (mut close, navigation) = controls.modal_actions(
            InputEvent::Button {
                code: ControllerButton::East.code(),
                device: 1,
                down: true,
            },
            false,
        )?;
        if navigation.is_some() {
            return Err(failure("non-touch event produced touch command"));
        }
        controls.close_editor(&mut close)?;
        if close
            != [true, false].map(|down| LogicalAction {
                action: Action {
                    control: Control::Pause,
                    player: Player::Homer,
                },
                down,
            })
        {
            return Err(failure(
                "modal close was routed through an active binding",
            ));
        }
        Ok(())
    }

    #[test]
    fn remapped_touch_panel_navigates_editor_without_gameplay_edges()
    -> Result<(), RenderError> {
        use games::input::profile::DigitalBinding;
        let profiles = DEFAULT_PROFILES
            .edited(0, 2, DigitalBinding::TouchSlot(3))
            .map_err(failure)?;
        let mut controls = Controls::with_profiles(true, &profiles)?;
        for (control, command) in [
            (Control::Primary, EditorNavigation::Confirm),
            (Control::Secondary, EditorNavigation::Family),
            (Control::Pause, EditorNavigation::Cancel),
            (Control::Start, EditorNavigation::Clear),
            (Control::Right, EditorNavigation::Right),
            (Control::Left, EditorNavigation::Left),
            (Control::Up, EditorNavigation::Up),
            (Control::Down, EditorNavigation::Down),
        ] {
            verify_modal_touch(&mut controls, control, command)?;
        }
        Ok(())
    }

    fn verify_modal_touch(
        controls: &mut Controls,
        control: Control,
        command: EditorNavigation,
    ) -> Result<(), RenderError> {
        use games::input::touch::{ContactId, ContactPhase};
        let point =
            super::touch::center(Player::Marge, control, &controls.profiles)?;
        let id = ContactId { device: 1, finger: 1 };
        for (phase, expected) in [
            (ContactPhase::Down(point), Some(command)),
            (ContactPhase::Down(point), None),
            (ContactPhase::Motion(Some(point)), None),
            (ContactPhase::Up, None),
            (ContactPhase::Down(point), Some(command)),
            (ContactPhase::Up, None),
        ] {
            let (edges, navigation) = controls
                .modal_actions(InputEvent::Contact { id, phase }, false)?;
            if !edges.is_empty() || navigation != expected {
                return Err(failure(
                    "touch editor routed unexpected command or edge",
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn closing_editor_rejects_stale_touch_motion_after_return()
    -> Result<(), RenderError> {
        use games::input::touch::{ContactId, ContactPhase};
        let mut controls = Controls::new(true)?;
        let point = super::touch::center(
            Player::Marge,
            Control::Pause,
            &controls.profiles,
        )?;
        let id = ContactId { device: 1, finger: 1 };
        let (mut edges, navigation) = controls.modal_actions(
            InputEvent::Contact {
                id,
                phase: ContactPhase::Down(point),
            },
            false,
        )?;
        if !edges.is_empty() || navigation != Some(EditorNavigation::Cancel) {
            return Err(failure("touch close did not produce cancel"));
        }
        controls.close_editor(&mut edges)?;
        for phase in [ContactPhase::Motion(Some(point)), ContactPhase::Up] {
            if !controls
                .actions(InputEvent::Contact { id, phase })?
                .is_empty()
            {
                return Err(failure("closed editor leaked a stale contact"));
            }
        }
        Ok(())
    }

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
    fn settings_hint_follows_persistence_availability() {
        use super::settings_hint;

        assert_eq!(
            settings_hint(true, true, false),
            "SMOKE DOES NOT SAVE SETTINGS"
        );
        assert_eq!(
            settings_hint(false, false, false),
            "SETTINGS ARE SESSION ONLY"
        );
        assert_eq!(
            settings_hint(false, true, false),
            "DISPLAY AND VOLUME SAVED"
        );
        assert_eq!(
            settings_hint(false, true, true),
            "CONTROL EDITS APPLY NEXT LAUNCH"
        );
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
            main_menu_hints(false)
                .contains(&"PRESS YOUR START CONTROL TO JOIN")
        );
        assert!(
            !main_menu_hints(false)
                .iter()
                .any(|hint| hint.contains("REMOTE"))
        );
    }

    #[test]
    fn pause_hints_include_mouse_only_and_combined_desktop_bindings() {
        use super::{PointerButton, pause_hints};

        let mut profiles = DEFAULT_PROFILES;
        let [_, keys, ..] = &mut profiles.keys;
        let [_, mouse, ..] = &mut profiles.mouse;
        let [_, _, _, jump, left, ..] = keys;
        *jump = None;
        *left = None;
        *mouse = [
            Some(PointerButton::X1),
            None,
            None,
            Some(PointerButton::Middle),
            Some(PointerButton::Right),
            Some(PointerButton::X2),
            None,
            None,
        ];
        let [first, ..] = &mut profiles.mouse;
        *first = [None; 8];
        let [horizontal, vertical, combat, resume] =
            pause_hints(&profiles, [true, false, false, false]);
        assert_eq!(horizontal, "P2 LEFT: MOUSE RIGHT RIGHT: Right/MOUSE X2");
        assert_eq!(vertical, "UP: Up DOWN: Down");
        assert_eq!(combat, "ATTACK: F JUMP: MOUSE MIDDLE");
        assert!(resume.contains("MOUSE X1"));
        assert!(resume.contains("START RESUMES"));
    }

    #[test]
    fn pause_hints_fit_extended_keyboard_and_mouse_labels() {
        use games_platform::ir::ui::TextLine;

        use super::{MENU_WIDTH, PointerButton, desktop_hint};

        let binding = desktop_hint(
            Some(KeyboardKey::from_code(100)),
            Some(PointerButton::Middle),
        );
        assert_eq!(binding, "IntlBackslash/MOUSE MIDDLE");
        let text = format!("P4 LEFT: {binding} RIGHT: {binding}");
        assert!(
            TextLine {
                color: [255; 4],
                scale: 2,
                text: &text,
                top: 0,
            }
            .fit_width(MENU_WIDTH)
            .scale
                > 0
        );
    }

    #[test]
    fn smoke_inputs_route_admitted_mixed_bindings_for_all_stations()
    -> Result<(), RenderError> {
        let profiles = configured_profiles(concat!(
            "version=1\nkey 1 primary=none\nkey 1 secondary=none\n",
            "key 2 start=none\nbutton 2 start=y\n",
            "key 3 left=none\nkey 4 right=none\n",
        ))
        .map_err(failure)?;
        let mut controls = Controls::with_smoke_profiles(false, &profiles)?;
        if controls.smoke_event(Player::Marge, Control::Primary, true)?
            != (InputEvent::Mouse {
                code: PointerButton::Left.code(),
                down: true,
            })
            || !matches!(
                controls.smoke_event(Player::Homer, Control::Start, true)?,
                InputEvent::Button { code, down: true, .. }
                    if code == ControllerButton::North.code()
            )
        {
            return Err(failure("smoke ignored a configured fallback"));
        }
        let bindings = Player::ALL
            .into_iter()
            .flat_map(|player| CONTROLS.map(|control| (player, control)))
            .flat_map(|(player, control)| {
                [true, false].map(|down| (player, control, down))
            });
        for (player, control, down) in bindings {
            let event = controls.smoke_event(player, control, down)?;
            if controls.actions(event)?
                != [LogicalAction {
                    action: Action { control, player },
                    down,
                }]
            {
                return Err(failure("smoke routed the wrong binding"));
            }
        }
        Ok(())
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
                code: PointerButton::Left.code(),
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
    fn profiles_keep_escape_as_the_only_keyboard_pause()
    -> Result<(), RenderError> {
        for text in [
            "version=1\nkey 1 pause=none\n",
            "version=1\nkey 1 pause=F1\n",
            "version=1\nkey 2 pause=F1\n",
            "version=1\nkey 1 pause=F1\nkey 1 primary=Escape\n",
        ] {
            if configured_profiles(text).is_ok() {
                return Err(failure("accepted a conflicting keyboard pause"));
            }
        }
        let profiles = configured_profiles(concat!(
            "version=1\nkey 1 pause=Escape\n",
            "key 2 pause=none\nkey 1 right=Q\n"
        ))
        .map_err(failure)?;
        let mut controls = Controls::with_profiles(false, &profiles)?;
        let mut simulation = live(false)?;
        super::smoke_press(&mut controls, &mut simulation, Control::Start)?;
        super::smoke_press(&mut controls, &mut simulation, Control::Pause)?;
        if simulation.state().phase() != Phase::Paused {
            return Err(failure("custom profile lost global Escape pause"));
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
        if presentation_wait(&hosted, true) == 0
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
            || presentation_wait(&hosted, true) != 0
            || presentation_wait(&hosted, false) == 0
            || !frame_due(&hosted, Some(&hosted), EventBatch::Empty)
        {
            return Err(failure("gameplay was not uncapped"));
        }
        let playing = hosted;
        hosted.input_step(2, &[action(Control::Pause)]);
        if hosted.state().phase() != Phase::Paused
            || presentation_wait(&hosted, true) == 0
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

    #[test]
    fn pending_editor_close_rejects_new_touch_lifetimes()
    -> Result<(), RenderError> {
        use games::input::touch::{ContactId, ContactPhase};
        let mut controls = Controls::new(true)?;
        let point = super::touch::center(
            Player::Marge,
            Control::Primary,
            &controls.profiles,
        )?;
        let id = ContactId { device: 1, finger: 2 };
        let event = InputEvent::Contact {
            id,
            phase: ContactPhase::Down(point),
        };
        let (edges, navigation) = controls.modal_actions(event, true)?;
        if !edges.is_empty() || navigation.is_some() {
            return Err(failure("pending close admitted touch input"));
        }
        for phase in [ContactPhase::Motion(Some(point)), ContactPhase::Up] {
            if !controls
                .actions(InputEvent::Contact { id, phase })?
                .is_empty()
            {
                return Err(failure("pending close left a stale contact"));
            }
        }
        if controls.actions(event)?
            != [LogicalAction {
                action: Action {
                    control: Control::Primary,
                    player: Player::Marge,
                },
                down: true,
            }]
        {
            return Err(failure("fresh gameplay contact was not admitted"));
        }
        Ok(())
    }
}
