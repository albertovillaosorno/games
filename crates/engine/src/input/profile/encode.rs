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
//   - Canonical, bounded version-one control profile serialization.
// - Must-Not:
//   - Open persistent storage, assign devices or select title actions.
// - Allows:
//   - Inputs: Validated portable profiles and caller-selected action names.
//   - Outputs: Complete round-trippable UTF-8 profile records.
//   - Side effects: Bounded allocation only.
// - Split-When:
//   - Another persisted profile format has independent encoding rules.
// - Merge-When:
//   - Another profile encoder duplicates version-one records.
// - Summary:
//   - Persist editable digital profiles without losing unmodified bindings.
// - Description:
//   - Emit every admitted field and verify the existing decoder round trip.
// - Usage:
//   - Serialize an edited candidate before requesting atomic persistence.
// - Defaults:
//   - Reject invalid, ambiguous or over-budget profiles.

use std::fmt::{self, Write};

use super::super::physical::{ControllerAxis, PointerButton};
use super::{DigitalProfiles, KeyNamer, KeyResolver, ProfileText};

#[derive(Debug)]
struct BoundedProfile {
    maximum: usize,
    text: String,
}

impl Write for BoundedProfile {
    fn write_char(&mut self, c: char) -> fmt::Result {
        self.write_str(c.encode_utf8(&mut [0; 4]))
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        fmt::write(self, args)
    }

    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self
            .text
            .len()
            .checked_add(s.len())
            .is_none_or(|length| length > self.maximum)
        {
            return Err(fmt::Error);
        }
        self.text
            .try_reserve(s.len())
            .map_err(|_error| fmt::Error)?;
        self.text.push_str(s);
        Ok(())
    }
}

pub(super) fn encode<const PLAYERS: usize, const ACTIONS: usize>(
    profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    actions: [&str; ACTIONS],
    maximum: usize,
    key_names: (KeyNamer, KeyResolver),
) -> Result<String, String> {
    profiles.validate()?;
    let (key_name, resolve_key) = key_names;
    let mut output = BoundedProfile {
        maximum,
        text: String::new(),
    };
    write_profile(&mut output, profiles, &actions, key_name)
        .map_err(|_error| String::from("cannot encode control profile"))?;
    let parsed = profiles.configured(
        actions,
        ProfileText {
            max_bytes: maximum,
            text: &output.text,
        },
        resolve_key,
    )?;
    if &parsed != profiles {
        return Err(String::from("control profile cannot round trip"));
    }
    Ok(output.text)
}

fn write_profile<const PLAYERS: usize, const ACTIONS: usize>(
    output: &mut BoundedProfile,
    profiles: &DigitalProfiles<PLAYERS, ACTIONS>,
    actions: &[&str; ACTIONS],
    key_name: KeyNamer,
) -> fmt::Result {
    writeln!(output, "version=1")?;
    write_touch_players(output, &profiles.touch_players)?;
    for (index, keys) in profiles.keys.iter().enumerate() {
        let player = index.saturating_add(1);
        let mice = profiles.mouse.get(index).ok_or(fmt::Error)?;
        let buttons = profiles.buttons.get(index).ok_or(fmt::Error)?;
        let slots = profiles.touch_slots.get(index).ok_or(fmt::Error)?;
        for (slot, (((key, mouse), button), touch)) in
            keys.iter().zip(mice).zip(buttons).zip(slots).enumerate()
        {
            let action = actions.get(slot).ok_or(fmt::Error)?;
            let key_label = key.map_or("none", key_name);
            writeln!(output, "key {player} {action}={key_label}")?;
            let mouse_label = mouse.map_or("none", PointerButton::profile_name);
            writeln!(output, "mouse {player} {action}={mouse_label}")?;
            let button_label = button.profile_name();
            writeln!(output, "button {player} {action}={button_label}")?;
            let touch_label = actions.get(*touch).ok_or(fmt::Error)?;
            writeln!(output, "touch {player} {action}={touch_label}")?;
        }
        let axes = profiles.axes.get(index).ok_or(fmt::Error)?;
        for (axis, profile) in ControllerAxis::ALL.into_iter().zip(axes) {
            let name = axis.profile_name();
            let negative = axis_name(actions, profile.negative)?;
            let positive = axis_name(actions, profile.positive)?;
            writeln!(output, "axis {player} {name}={negative}/{positive}")?;
            let (press, release) = (profile.press, profile.release);
            writeln!(
                output,
                "axis-threshold {player} {name}={press}/{release}"
            )?;
        }
    }
    Ok(())
}

fn axis_name<'text>(
    actions: &[&'text str],
    slot: Option<usize>,
) -> Result<&'text str, fmt::Error> {
    slot.map_or(Ok("none"), |index| {
        actions.get(index).copied().ok_or(fmt::Error)
    })
}

fn write_touch_players(
    output: &mut BoundedProfile,
    selected: &[bool],
) -> fmt::Result {
    write!(output, "touch-players=")?;
    let mut separator = "";
    for (index, enabled) in selected.iter().enumerate() {
        if *enabled {
            write!(output, "{separator}{}", index.saturating_add(1))?;
            separator = ",";
        }
    }
    writeln!(output)
}
