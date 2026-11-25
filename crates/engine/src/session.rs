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

const MAIN_ITEMS: [MenuItem; 2] = [MenuItem::Play, MenuItem::Exit];
const PAUSE_ITEMS: [MenuItem; 3] =
    [MenuItem::Resume, MenuItem::MainMenu, MenuItem::Exit];

/// Current screen of a local gameplay session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionPhase {
    /// The user has requested that the host close.
    Exited,
    /// Initial menu, before any character has joined.
    #[default]
    MainMenu,
    /// The world is suspended while the pause menu is shown.
    Paused,
    /// The world is running.
    Playing,
}

/// Available shared menu choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuItem {
    /// Close the host.
    Exit,
    /// Abandon the current world and return to the main menu.
    MainMenu,
    /// Start a new session.
    Play,
    /// Resume the suspended world.
    Resume,
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
    pub const fn items(self) -> &'static [MenuItem] {
        match self {
            Self::MainMenu => &MAIN_ITEMS,
            Self::Paused => &PAUSE_ITEMS,
            Self::Playing | Self::Exited => &[],
        }
    }
}

/// Deterministic local session state with no device or title dependencies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalSession {
    phase: SessionPhase,
    selection: usize,
}

impl LocalSession {
    fn confirm(&mut self) -> SessionEffect {
        match self.selected() {
            Some(MenuItem::Play) => self.start(),
            Some(MenuItem::Resume) => {
                self.transition(SessionPhase::Playing);
                SessionEffect::None
            },
            Some(MenuItem::MainMenu) => {
                self.return_to_menu();
                SessionEffect::Reset
            },
            Some(MenuItem::Exit) => {
                self.transition(SessionPhase::Exited);
                SessionEffect::Exit
            },
            None => SessionEffect::None,
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
        self.phase.items()
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
            SessionPhase::Exited => SessionEffect::None,
        }
    }

    const fn toggle_pause(&mut self) {
        match self.phase {
            SessionPhase::Playing => self.transition(SessionPhase::Paused),
            SessionPhase::Paused => self.transition(SessionPhase::Playing),
            SessionPhase::MainMenu | SessionPhase::Exited => {},
        }
    }

    const fn transition(&mut self, phase: SessionPhase) {
        self.phase = phase;
        self.selection = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
