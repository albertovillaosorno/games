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
//   - Admission of per-station native digital input profiles.
// - Must-Not:
//   - Select title actions, assign devices or route simulation events.
// - Allows:
//   - Inputs: Bounded local text, title action names and default bindings.
//   - Outputs: Validated SDL keyboard and controller-button assignments.
//   - Side effects: None.
// - Split-When:
//   - Other device families require independent profile representations.
// - Merge-When:
//   - Another native profile parser duplicates these admission rules.
// - Summary:
//   - Override digital bindings without ambiguous physical assignments.
// - Description:
//   - Keyboard keys are globally unique; controller buttons are unique within
//     each station. Unspecified bindings retain title defaults.
// - Usage:
//   - Configure before constructing the shared action router.
// - Defaults:
//   - Reject unknown controls, duplicates and malformed values.

//! Bounded, reusable native digital input profiles.

use std::collections::BTreeSet;

use super::{Button, Scancode, button_code, key_code};

/// Maximum UTF-8 profile size, in bytes.
pub const MAX_PROFILE_BYTES: usize = 8192;

/// Digital bindings in title-supplied station and action order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DigitalProfiles<const PLAYERS: usize, const ACTIONS: usize> {
    /// Per-station controller button bindings.
    pub buttons: [[Button; ACTIONS]; PLAYERS],
    /// Globally unique keyboard bindings.
    pub keys: [[Scancode; ACTIONS]; PLAYERS],
    /// Zero-based station selected for the visible touch panel.
    pub touch_player: usize,
    /// Per-station logical-action to physical touch-slot permutation.
    pub touch_slots: [[usize; ACTIONS]; PLAYERS],
}

impl<const PLAYERS: usize, const ACTIONS: usize>
    DigitalProfiles<PLAYERS, ACTIONS>
{
    /// Apply explicit overrides to a complete set of default bindings.
    ///
    /// Each record is `key PLAYER ACTION=NAME` or
    /// `button PLAYER ACTION=NAME` or `touch PLAYER ACTION=SLOT`.
    /// Player numbers start at one; touch slots use the action names.
    /// Optional `touch-player=N` selects one station for the panel.
    /// The first nonempty line must be `version=1`.
    /// # Errors
    /// Rejects oversized, ambiguous, unknown or invalid mappings.
    pub fn configured(
        mut self,
        actions: [&str; ACTIONS],
        text: &str,
    ) -> Result<Self, String> {
        if text.len() > MAX_PROFILE_BYTES {
            return Err(String::from("control profile exceeds byte limit"));
        }
        let mut lines =
            text.lines().map(str::trim).filter(|line| !line.is_empty());
        if lines.next() != Some("version=1") {
            return Err(String::from("expected control profile version=1"));
        }
        let mut seen = BTreeSet::new();
        let mut touch_selected = false;
        for line in lines {
            if touch_selected && line.starts_with("touch-player=") {
                return Err(String::from("duplicate touch-player"));
            }
            if let Some(value) = line.strip_prefix("touch-player=") {
                self.touch_player = value
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .ok_or("invalid touch player")?;
                touch_selected = true;
                continue;
            }
            let (kind, player, action, name) = parse_line(line, &actions)?;
            if !seen.insert((kind, player, action)) {
                return Err(String::from("duplicate control override"));
            }
            self.override_binding((kind, player, action, name), &actions)?;
        }
        self.validate()?;
        Ok(self)
    }

    fn override_binding(
        &mut self,
        record: Record<'_>,
        actions: &[&str],
    ) -> Result<(), String> {
        let (kind, player, action, name) = record;
        let invalid = || String::from("unknown player, action or binding");
        match kind {
            Kind::Button => {
                let slot = self
                    .buttons
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = Button::from_string(name).ok_or_else(invalid)?;
            },
            Kind::Touch => {
                let slot = self
                    .touch_slots
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = actions
                    .iter()
                    .position(|position| *position == name)
                    .ok_or_else(invalid)?;
            },
            Kind::Key => {
                let slot = self
                    .keys
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = Scancode::from_name(name).ok_or_else(invalid)?;
            },
        }
        Ok(())
    }

    /// Verify that no physical binding silently overwrites another.
    /// # Errors
    /// Rejects duplicate keyboard keys or same-station controller buttons.
    pub fn validate(&self) -> Result<(), String> {
        if self.touch_player >= PLAYERS {
            return Err(String::from("invalid touch player"));
        }
        for row in &self.touch_slots {
            if row.iter().any(|slot| *slot >= ACTIONS)
                || row.iter().collect::<BTreeSet<_>>().len() != ACTIONS
            {
                return Err(String::from("invalid or duplicate touch slot"));
            }
        }
        let mut keys = BTreeSet::new();
        for key in self.keys.iter().flatten().copied() {
            if !keys.insert(key_code(key)) {
                return Err(String::from("duplicate keyboard binding"));
            }
        }
        for row in &self.buttons {
            if row
                .iter()
                .copied()
                .map(button_code)
                .collect::<BTreeSet<_>>()
                .len()
                != ACTIONS
            {
                return Err(String::from("duplicate controller binding"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Button,
    Key,
    Touch,
}

type Record<'text> = (Kind, usize, usize, &'text str);

fn parse_line<'text>(
    line: &'text str,
    actions: &[&str],
) -> Result<Record<'text>, String> {
    let invalid = || String::from("invalid control profile record");
    let (left, raw_name) = line.split_once('=').ok_or_else(invalid)?;
    let mut fields = left.split_whitespace();
    let kind = match fields.next() {
        Some("key") => Kind::Key,
        Some("button") => Kind::Button,
        Some("touch") => Kind::Touch,
        _ => return Err(invalid()),
    };
    let player = fields
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|number| number.checked_sub(1))
        .ok_or_else(invalid)?;
    let action = fields
        .next()
        .and_then(|value| actions.iter().position(|name| *name == value))
        .ok_or_else(invalid)?;
    let name = raw_name.trim();
    if fields.next().is_some()
        || name.is_empty()
        || name.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    Ok((kind, player, action, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: DigitalProfiles<2, 2> = DigitalProfiles {
        buttons: [[Button::A, Button::B]; 2],
        keys: [[Scancode::A, Scancode::B], [Scancode::C, Scancode::D]],
        touch_player: 0,
        touch_slots: [[0, 1]; 2],
    };
    const ACTIONS: [&str; 2] = ["primary", "secondary"];

    #[test]
    fn profile_overrides_only_selected_station_and_binding() {
        let configured = DEFAULT.configured(
            ACTIONS,
            concat!(
                "version=1\nkey 2 primary=Space\nbutton 2 primary=x\n",
                "touch-player=2\ntouch 2 primary=secondary\n",
                "touch 2 secondary=primary\n"
            ),
        );
        assert_eq!(
            configured,
            Ok(DigitalProfiles {
                buttons: [[Button::A, Button::B], [Button::X, Button::B]],
                keys: [[Scancode::A, Scancode::B], [
                    Scancode::Space,
                    Scancode::D
                ]],
                touch_player: 1,
                touch_slots: [[0, 1], [1, 0]],
            })
        );
    }

    #[test]
    fn rejects_ambiguous_or_invalid_mappings() {
        for invalid in [
            "version=2",
            "version=1\nkey 1 primary=C",
            "version=1\nbutton 2 primary=b",
            "version=1\nkey 3 primary=Space",
            "version=1\nkey 1 missing=Space",
            "version=1\nkey 0 primary=Space",
            "version=1\nkey 1 primary=not-a-key",
            "version=1\nbutton 2 primary=not-a-button",
            "version=1\nkey 1 primary=Q\nkey 1 primary=R",
            "version=1\ntouch-player=3",
            "version=1\ntouch 2 primary=secondary",
            "version=1\ntouch-player=1\ntouch-player=2",
        ] {
            assert!(DEFAULT.configured(ACTIONS, invalid).is_err());
        }
        assert!(DEFAULT.configured(ACTIONS, &" ".repeat(8193)).is_err());
    }
}
