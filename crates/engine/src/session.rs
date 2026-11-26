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
//   - Free-play menu navigation and local session transitions.
// - Must-Not:
//   - Own title characters, combat, rendering, devices or saved settings.
// - Allows:
//   - Inputs: Edge-triggered logical menu commands.
//   - Outputs: Current phase, selection and session effects.
//   - Side effects: Deterministic in-memory transitions only.
// - Split-When:
//   - Settings editing requires its own persistent configuration contract.
// - Merge-When:
//   - Another shared module duplicates local menu or pause transitions.
// - Summary:
//   - Shared modern free-play session flow.
// - Description:
//   - Start plays immediately; pause suspends the world; menus wrap.
// - Usage:
//   - Route logical input edges and apply effects in the title simulation.
// - Defaults:
//   - Main menu with Play selected; no payment or external service.

//! Local free-play navigation independent of the host and title rules.

use crate::audio::MasterVolume;
use crate::presentation::{DisplaySettings, PresentationMode};

const MAIN_ITEMS: [MenuItem; 3] =
    [MenuItem::Play, MenuItem::Settings, MenuItem::Exit];
const PAUSE_ITEMS: [MenuItem; 4] = [
    MenuItem::Resume,
    MenuItem::Settings,
    MenuItem::MainMenu,
    MenuItem::Exit,
];
const SETTINGS_ITEMS: [MenuItem; 4] = [
    MenuItem::Display,
    MenuItem::Window,
    MenuItem::Volume,
    MenuItem::Back,
];
const SETTINGS_WITH_CONTROL_RESET: [MenuItem; 6] = [
    MenuItem::Display,
    MenuItem::Window,
    MenuItem::Volume,
    MenuItem::ResetControls,
    MenuItem::EditControls,
    MenuItem::Back,
];

/// Availability and requested next-launch reset of a control profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ControlReset {
    /// A host can persist defaults for the next launch.
    Available,
    /// The host does not offer a persistent reset.
    #[default]
    Disabled,
    /// The user has requested defaults; the host must persist them.
    Requested,
}

/// Current screen of a local gameplay session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionPhase {
    /// The host is editing a next-launch control profile.
    Controls,
    /// The user has requested that the host close.
    Exited,
    /// Initial menu, before any character has joined.
    #[default]
    MainMenu,
    /// The world is suspended while the pause menu is shown.
    Paused,
    /// The world is running.
    Playing,
    /// Settings; the originating menu remains suspended.
    Settings,
}

/// Available shared menu choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuItem {
    /// Return to the menu from which settings were opened.
    Back,
    /// Cycle the logical presentation mode.
    Display,
    /// Open the host control profile editor.
    EditControls,
    /// Close the host.
    Exit,
    /// Abandon the current world and return to the main menu.
    MainMenu,
    /// Start a new session.
    Play,
    /// Request default controls for the next launch.
    ResetControls,
    /// Resume the suspended world.
    Resume,
    /// Open settings.
    Settings,
    /// Cycle master audio volume.
    Volume,
    /// Toggle windowed or desktop-fullscreen presentation.
    Window,
}

/// Edge-triggered logical menu input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuInput {
    /// Confirm the selected menu item.
    Confirm,
    /// Select the next item.
    Next,
    /// Toggle pause when a world is active.
    Pause,
    /// Select the preceding item.
    Previous,
    /// Play immediately, join an active world, or resume.
    Start,
}

/// A title or host operation requested by the shared session flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEffect {
    /// Close the host.
    Exit,
    /// Admit the requesting local player.
    Join,
    /// Nothing outside the shared menu state needs to change.
    None,
    /// Reset the title world.
    Reset,
}

impl SessionPhase {
    /// Visible menu choices, empty while playing or after exit.
    #[must_use]
    pub const fn items(self, control_reset: bool) -> &'static [MenuItem] {
        match self {
            Self::MainMenu => &MAIN_ITEMS,
            Self::Paused => &PAUSE_ITEMS,
            Self::Settings if control_reset => &SETTINGS_WITH_CONTROL_RESET,
            Self::Settings => &SETTINGS_ITEMS,
            Self::Controls | Self::Playing | Self::Exited => &[],
        }
    }
}

/// Deterministic local session state with no device or title dependencies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalSession {
    control_reset: ControlReset,
    display: DisplaySettings,
    phase: SessionPhase,
    restrict_fill: bool,
    selection: usize,
    settings_origin: SessionPhase,
    volume: MasterVolume,
}

impl LocalSession {
    const fn close_controls(&mut self) {
        self.transition(SessionPhase::Settings);
        self.selection = 4;
        self.control_reset = ControlReset::Available;
    }

    const fn close_settings(&mut self) {
        self.transition(self.settings_origin);
        self.selection = 1;
    }

    fn confirm(&mut self) -> SessionEffect {
        match self.selected() {
            Some(MenuItem::Back) => self.close_settings(),
            Some(MenuItem::Display) => self.cycle_display(),
            Some(MenuItem::EditControls) => {
                self.transition(SessionPhase::Controls);
            },
            Some(MenuItem::Settings) => {
                self.settings_origin = self.phase;
                self.transition(SessionPhase::Settings);
            },
            Some(MenuItem::Volume) => self.volume = self.volume.next(),
            Some(MenuItem::Window) => {
                self.display.window = self.display.window.toggled();
            },
            Some(MenuItem::Play) => return self.start(),
            Some(MenuItem::ResetControls) => {
                self.control_reset = ControlReset::Requested;
            },
            Some(MenuItem::Resume) => {
                self.transition(SessionPhase::Playing);
            },
            Some(MenuItem::MainMenu) => {
                self.return_to_menu();
                return SessionEffect::Reset;
            },
            Some(MenuItem::Exit) => {
                self.transition(SessionPhase::Exited);
                return SessionEffect::Exit;
            },
            None => {},
        }
        SessionEffect::None
    }

    /// Requested control reset, not an acknowledgement of persistence.
    #[must_use]
    pub const fn control_reset(self) -> ControlReset {
        self.control_reset
    }

    const fn cycle_display(&mut self) {
        self.display.presentation =
            self.display.presentation.next(self.restrict_fill);
    }

    /// Current requested portable display settings.
    #[must_use]
    pub const fn display(self) -> DisplaySettings {
        self.display
    }

    /// Keep menu choices reachable when gameplay uses centered cropping.
    #[must_use]
    pub const fn effective_display(self) -> DisplaySettings {
        if !matches!(self.phase, SessionPhase::Playing)
            && matches!(self.display.presentation, PresentationMode::Fill)
        {
            DisplaySettings {
                presentation: PresentationMode::AspectFit,
                ..self.display
            }
        } else {
            self.display
        }
    }

    /// Apply a single logical input edge and report title or host effects.
    pub fn input(&mut self, input: MenuInput) -> SessionEffect {
        match input {
            MenuInput::Start => self.start(),
            MenuInput::Confirm => self.confirm(),
            MenuInput::Pause => {
                self.toggle_pause();
                SessionEffect::None
            },
            MenuInput::Next | MenuInput::Previous => {
                self.navigate(input);
                SessionEffect::None
            },
        }
    }

    /// Visible choices for the current screen, empty while playing.
    #[must_use]
    pub const fn items(self) -> &'static [MenuItem] {
        self.phase
            .items(!matches!(self.control_reset, ControlReset::Disabled))
    }

    fn navigate(&mut self, input: MenuInput) {
        let count = self.items().len();
        if count == 0 {
            return;
        }
        let offset = if input == MenuInput::Next {
            1
        } else {
            count.saturating_sub(1)
        };
        self.selection = self
            .selection
            .saturating_add(offset)
            .checked_rem(count)
            .unwrap_or_default();
    }

    /// Current gameplay or menu phase.
    #[must_use]
    pub const fn phase(self) -> SessionPhase {
        self.phase
    }

    /// Return to the initial menu after a title-owned game-over condition.
    pub const fn return_to_menu(&mut self) {
        self.transition(SessionPhase::MainMenu);
    }

    /// Highlighted item, or None when no menu is open.
    #[must_use]
    pub fn selected(self) -> Option<MenuItem> {
        self.items().get(self.selection).copied()
    }

    const fn start(&mut self) -> SessionEffect {
        match self.phase {
            SessionPhase::MainMenu | SessionPhase::Playing => {
                self.transition(SessionPhase::Playing);
                SessionEffect::Join
            },
            SessionPhase::Paused => {
                self.transition(SessionPhase::Playing);
                SessionEffect::None
            },
            SessionPhase::Controls => {
                self.close_controls();
                SessionEffect::None
            },
            SessionPhase::Settings => {
                self.close_settings();
                SessionEffect::None
            },
            SessionPhase::Exited => SessionEffect::None,
        }
    }

    const fn toggle_pause(&mut self) {
        match self.phase {
            SessionPhase::Playing => self.transition(SessionPhase::Paused),
            SessionPhase::Paused => self.transition(SessionPhase::Playing),
            SessionPhase::Controls => self.close_controls(),
            SessionPhase::Settings => self.close_settings(),
            SessionPhase::MainMenu | SessionPhase::Exited => {},
        }
    }

    const fn transition(&mut self, phase: SessionPhase) {
        self.phase = phase;
        self.selection = 0;
    }

    /// Current requested master volume.
    #[must_use]
    pub const fn volume(self) -> MasterVolume {
        self.volume
    }

    /// Offer the optional next-launch control reset at session creation.
    #[must_use]
    pub const fn with_control_reset(mut self, enabled: bool) -> Self {
        self.control_reset = if enabled {
            ControlReset::Available
        } else {
            ControlReset::Disabled
        };
        self
    }

    /// Initialise a session with portable display preferences and whether
    /// centered cropping must be excluded to keep controls reachable.
    #[must_use]
    pub const fn with_display(
        display: DisplaySettings,
        restrict_fill: bool,
    ) -> Self {
        Self::with_preferences(display, MasterVolume::Full, restrict_fill)
    }

    /// Initialise a session with portable display and volume preferences.
    #[must_use]
    pub const fn with_preferences(
        display: DisplaySettings,
        volume: MasterVolume,
        restrict_fill: bool,
    ) -> Self {
        let adjusted = if restrict_fill
            && matches!(display.presentation, PresentationMode::Fill)
        {
            DisplaySettings {
                presentation: PresentationMode::AspectFit,
                ..display
            }
        } else {
            display
        };
        Self {
            control_reset: ControlReset::Disabled,
            display: adjusted,
            phase: SessionPhase::MainMenu,
            restrict_fill,
            selection: 0,
            settings_origin: SessionPhase::MainMenu,
            volume,
        }
    }

    /// Whether the current screen suspends or displays an existing world.
    #[must_use]
    pub const fn world_visible(self) -> bool {
        matches!(self.phase, SessionPhase::Playing | SessionPhase::Paused)
            || (matches!(
                self.phase,
                SessionPhase::Settings | SessionPhase::Controls
            ) && matches!(self.settings_origin, SessionPhase::Paused))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::WindowMode;

    #[test]
    fn control_reset_is_hidden_without_host_support() {
        let mut session = LocalSession::default();
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        assert_eq!(session.items(), &SETTINGS_ITEMS);
        assert_eq!(session.control_reset(), ControlReset::Disabled);
    }

    fn select_reset_in_pause() -> LocalSession {
        let mut session = LocalSession::default().with_control_reset(true);
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        for input in [MenuInput::Pause, MenuInput::Next, MenuInput::Confirm] {
            assert_eq!(session.input(input), SessionEffect::None);
        }
        assert_eq!(session.items(), &SETTINGS_WITH_CONTROL_RESET);
        for _ in 0u8..3 {
            assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        }
        assert_eq!(session.selected(), Some(MenuItem::ResetControls));
        assert_eq!(session.control_reset(), ControlReset::Available);
        session
    }

    #[test]
    fn editor_returns_to_suspended_settings_without_world_reset() {
        let mut session = select_reset_in_pause();
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        assert_eq!(session.selected(), Some(MenuItem::EditControls));
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Controls);
        assert!(session.world_visible());
        assert_eq!(session.items(), []);
        leave_editor(&mut session);
    }

    fn leave_editor(session: &mut LocalSession) {
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Controls);
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(session.selected(), Some(MenuItem::EditControls));
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Paused);
    }

    #[test]
    fn control_reset_preserves_the_session() {
        let mut session = select_reset_in_pause();
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::None);
        assert_eq!(session.control_reset(), ControlReset::Requested);
        assert_eq!(session.phase(), SessionPhase::Settings);
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Paused);
        assert_eq!(session.control_reset(), ControlReset::Requested);
    }

    #[test]
    fn start_joins_without_prerequisites_and_pause_is_reversible() {
        let mut session = LocalSession::default();
        assert_eq!(session.selected(), Some(MenuItem::Play));
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        assert_eq!(session.phase(), SessionPhase::Playing);
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Paused);
        assert_eq!(session.selected(), Some(MenuItem::Resume));
        assert_eq!(session.input(MenuInput::Start), SessionEffect::None);
        assert_eq!(session.phase(), SessionPhase::Playing);
    }

    #[test]
    fn menus_wrap_and_can_reset() {
        let mut session = LocalSession::default();
        assert_eq!(session.input(MenuInput::Previous), SessionEffect::None);
        assert_eq!(session.selected(), Some(MenuItem::Exit));
        // Start is always immediate, irrespective of the highlighted item.
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        assert_eq!(session.input(MenuInput::Next), SessionEffect::None);
        assert_eq!(session.selected(), Some(MenuItem::MainMenu));
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::Reset);
        assert_eq!(session.phase(), SessionPhase::MainMenu);
    }

    #[test]
    fn menu_exit_is_terminal() {
        let mut session = LocalSession::default();
        assert_eq!(session.input(MenuInput::Previous), SessionEffect::None);
        assert_eq!(session.input(MenuInput::Confirm), SessionEffect::Exit);
        assert_eq!(session.phase(), SessionPhase::Exited);
        assert_eq!(session.input(MenuInput::Start), SessionEffect::None);
    }

    #[test]
    fn settings_cycle_display_and_return_to_origin() {
        let mut session = LocalSession::default();
        menu_input(&mut session, MenuInput::Next);
        menu_input(&mut session, MenuInput::Confirm);
        assert_eq!(session.phase(), SessionPhase::Settings);
        assert!(!session.world_visible());
        cycle_modes(&mut session, &[
            PresentationMode::Integer,
            PresentationMode::Fill,
            PresentationMode::Stretch,
            PresentationMode::Adaptive,
            PresentationMode::AspectFit,
        ]);
        menu_input(&mut session, MenuInput::Next);
        menu_input(&mut session, MenuInput::Confirm);
        assert_eq!(session.display().window, WindowMode::Fullscreen);
        menu_input(&mut session, MenuInput::Next);
        assert_eq!(session.selected(), Some(MenuItem::Volume));
        menu_input(&mut session, MenuInput::Confirm);
        assert_eq!(session.volume(), MasterVolume::ThreeQuarters);
        menu_input(&mut session, MenuInput::Next);
        menu_input(&mut session, MenuInput::Confirm);
        assert_eq!(session.phase(), SessionPhase::MainMenu);
        assert_eq!(session.selected(), Some(MenuItem::Settings));
        assert_eq!(session.volume(), MasterVolume::ThreeQuarters);
    }

    #[test]
    fn paused_settings_preserve_world_and_avoid_cropped_touch_controls() {
        let mut session = LocalSession::with_display(
            DisplaySettings {
                presentation: PresentationMode::Fill,
                ..DisplaySettings::default()
            },
            true,
        );
        assert_eq!(session.display().presentation, PresentationMode::AspectFit);
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        menu_input(&mut session, MenuInput::Pause);
        menu_input(&mut session, MenuInput::Next);
        menu_input(&mut session, MenuInput::Confirm);
        assert!(session.world_visible());
        cycle_modes(&mut session, &[
            PresentationMode::Integer,
            PresentationMode::Stretch,
            PresentationMode::Adaptive,
            PresentationMode::AspectFit,
        ]);
        menu_input(&mut session, MenuInput::Start);
        assert_eq!(session.phase(), SessionPhase::Paused);
        menu_input(&mut session, MenuInput::Start);
        assert_eq!(session.phase(), SessionPhase::Playing);
    }

    fn menu_input(session: &mut LocalSession, input: MenuInput) {
        assert_eq!(session.input(input), SessionEffect::None);
    }

    fn cycle_modes(session: &mut LocalSession, modes: &[PresentationMode]) {
        for &mode in modes {
            menu_input(session, MenuInput::Confirm);
            assert_eq!(session.display().presentation, mode);
        }
    }

    #[test]
    fn fill_is_requested_only_during_gameplay() {
        let mut session = LocalSession::with_display(
            DisplaySettings {
                presentation: PresentationMode::Fill,
                ..DisplaySettings::default()
            },
            false,
        );
        assert_eq!(
            session.effective_display().presentation,
            PresentationMode::AspectFit
        );
        assert_eq!(session.input(MenuInput::Start), SessionEffect::Join);
        assert_eq!(
            session.effective_display().presentation,
            PresentationMode::Fill
        );
        assert_eq!(session.input(MenuInput::Pause), SessionEffect::None);
        assert_eq!(
            session.effective_display().presentation,
            PresentationMode::AspectFit
        );
        assert_eq!(session.display().presentation, PresentationMode::Fill);
    }
}
