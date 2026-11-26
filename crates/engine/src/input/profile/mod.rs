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
//   - Admission of per-station portable digital input profiles.
// - Must-Not:
//   - Select title actions, assign devices or route simulation events.
// - Allows:
//   - Inputs: Bounded local text, title action names and default bindings.
//   - Outputs: Validated keyboard, mouse, button and digital axis assignments.
//   - Side effects: None.
// - Split-When:
//   - Other device families require independent profile representations.
// - Merge-When:
//   - Another digital profile parser duplicates these admission rules.
// - Summary:
//   - Override digital bindings without ambiguous physical assignments.
// - Description:
//   - Keyboard keys are globally unique; controller buttons are unique within
//     each station. Signed axes use explicit action slots and thresholds.
//     Unspecified bindings retain title defaults.
// - Usage:
//   - Configure before constructing the shared action router.
// - Defaults:
//   - Reject unknown controls, duplicates and malformed values.

//! Bounded, reusable portable digital input profiles.

/// Shared digital binding editor.
pub mod editor;
mod encode;

use std::collections::{BTreeMap, BTreeSet};

use super::AxisBinding;
use super::physical::{
    ControllerAxis, ControllerButton, KeyboardKey, PointerButton,
};

/// One signed axis's digital action slots and hysteresis thresholds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AxisProfile {
    /// Title action slot on the negative side, or disabled.
    pub negative: Option<usize>,
    /// Title action slot on the positive side, or disabled.
    pub positive: Option<usize>,
    /// Inclusive press magnitude.
    pub press: u16,
    /// Inclusive release magnitude.
    pub release: u16,
}

impl AxisProfile {
    /// Disabled axis with the standard digital movement hysteresis.
    pub const UNBOUND: Self = Self {
        negative: None,
        positive: None,
        press: 16_000,
        release: 10_000,
    };

    fn bind<Action>(
        &self,
        actions: &[Action],
    ) -> Result<AxisBinding<Action>, String>
    where
        Action: Copy,
    {
        let find = |slot: Option<usize>| {
            slot.map(|index| {
                actions
                    .get(index)
                    .copied()
                    .ok_or_else(|| String::from("invalid axis action"))
            })
            .transpose()
        };
        AxisBinding::new(
            find(self.negative)?,
            find(self.positive)?,
            self.press,
            self.release,
        )
        .ok_or_else(|| String::from("invalid axis thresholds"))
    }

    fn validate(
        self,
        axis: ControllerAxis,
        actions: usize,
    ) -> Result<(), String> {
        if [self.negative, self.positive]
            .into_iter()
            .flatten()
            .any(|slot| slot >= actions)
            || (matches!(
                axis,
                ControllerAxis::TriggerLeft | ControllerAxis::TriggerRight
            ) && self.negative.is_some())
        {
            return Err(String::from("invalid axis action"));
        }
        let _binding = AxisBinding::new(
            self.negative,
            self.positive,
            self.press,
            self.release,
        )
        .ok_or_else(|| String::from("invalid axis thresholds"))?;
        Ok(())
    }
}

/// Mapped physical axes ready for a single controller assignment.
pub type ControllerAxes<Action> = BTreeMap<u16, AxisBinding<Action>>;
/// Adapter-provided resolution of a persisted physical keyboard name.
pub type KeyResolver = fn(&str) -> Option<KeyboardKey>;

/// Adapter-provided canonical name of a physical keyboard key.
pub type KeyNamer = fn(KeyboardKey) -> &'static str;

/// A selected action's editable digital binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigitalBinding {
    /// Assign a gamepad button, swapping an occupied same-seat button.
    Button(ControllerButton),
    /// Assign or clear a globally unique keyboard key.
    Key(Option<KeyboardKey>),
    /// Assign or clear a globally unique physical pointer button.
    Mouse(Option<PointerButton>),
    /// Move to a physical touch slot, swapping the occupied slot.
    TouchSlot(usize),
}

/// Digital bindings in title-supplied station and action order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DigitalProfiles<const PLAYERS: usize, const ACTIONS: usize> {
    /// Per-station signed digital controller axes.
    pub axes: [[AxisProfile; ControllerAxis::ALL.len()]; PLAYERS],
    /// Per-station controller button bindings.
    pub buttons: [[ControllerButton; ACTIONS]; PLAYERS],
    /// Globally unique optional keyboard bindings.
    pub keys: [[Option<KeyboardKey>; ACTIONS]; PLAYERS],
    /// Globally unique optional mouse button bindings.
    pub mouse: [[Option<PointerButton>; ACTIONS]; PLAYERS],
    /// Stations selected for visible touch panels, in station order.
    pub touch_players: [bool; PLAYERS],
    /// Per-station logical-action to physical touch-slot permutation.
    pub touch_slots: [[usize; ACTIONS]; PLAYERS],
}

/// Bounded UTF-8 text submitted for profile admission.
#[derive(Clone, Copy, Debug)]
pub struct ProfileText<'text> {
    /// Maximum bytes admitted, including whitespace.
    pub max_bytes: usize,
    /// Complete version-one profile text.
    pub text: &'text str,
}

impl<const PLAYERS: usize, const ACTIONS: usize>
    DigitalProfiles<PLAYERS, ACTIONS>
{
    /// Build the selected station's mapped axes for a shared input router.
    ///
    /// The action array follows the same order as the profile action names.
    /// Disabled axes are omitted; button and axis sources may share an action.
    /// # Errors
    /// Rejects an invalid station, action slot or hysteresis threshold.
    pub fn axis_bindings<Action>(
        &self,
        player: usize,
        actions: [Action; ACTIONS],
    ) -> Result<ControllerAxes<Action>, String>
    where
        Action: Copy,
    {
        let row = self.axes.get(player).ok_or("invalid axis station")?;
        let mut mapped = BTreeMap::new();
        for (axis, profile) in ControllerAxis::ALL.into_iter().zip(row) {
            profile.validate(axis, ACTIONS)?;
            if profile.negative.is_some() || profile.positive.is_some() {
                let _old = mapped.insert(axis.code(), profile.bind(&actions)?);
            }
        }
        Ok(mapped)
    }

    /// Apply explicit overrides to a complete set of default bindings.
    ///
    /// Each record is `key PLAYER ACTION=NAME`,
    /// `button PLAYER ACTION=NAME`, `touch PLAYER ACTION=SLOT`,
    /// `axis PLAYER AXIS=NEGATIVE/POSITIVE` or
    /// `axis-threshold PLAYER AXIS=PRESS/RELEASE`. Axis names follow the
    /// version-one profile contract;
    /// use `none` to disable a direction. The supplied keyboard resolver
    /// translates legacy profile names to portable identities.
    /// Player numbers start at one; touch slots use the action names.
    /// Optional `touch-player=N` or `touch-players=N,M` selects stations.
    /// The selection is nonempty, unique and ordered by station number.
    /// The first nonempty line must be `version=1`. The caller supplies
    /// the maximum UTF-8 byte length, including ignored whitespace.
    /// # Errors
    /// Rejects oversized, ambiguous, unknown or invalid mappings.
    pub fn configured(
        mut self,
        actions: [&str; ACTIONS],
        source: ProfileText<'_>,
        resolve_key: KeyResolver,
    ) -> Result<Self, String> {
        let ProfileText { max_bytes, text } = source;
        if text.len() > max_bytes {
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
            if let Some(value) = touch_selection(line, touch_selected)? {
                self.touch_players = selected_players(value)?;
                touch_selected = true;
                continue;
            }
            let (kind, player, action, name) = parse_line(line, &actions)?;
            if !seen.insert((kind, player, action)) {
                return Err(String::from("duplicate control override"));
            }
            self.override_binding(
                (kind, player, action, name),
                &actions,
                resolve_key,
            )?;
        }
        self.validate()?;
        Ok(self)
    }

    /// Return an edited copy, retaining all other bindings and assignments.
    ///
    /// Controller buttons and touch slots swap within the selected station
    /// when occupied. Keyboard and mouse collisions are rejected.
    /// # Errors
    /// Rejects unknown stations, actions or slots and ambiguous profiles.
    pub fn edited(
        &self,
        station: usize,
        action: usize,
        binding: DigitalBinding,
    ) -> Result<Self, String> {
        self.validate()?;
        let invalid = || String::from("invalid control edit");
        let mut candidate = *self;
        match binding {
            DigitalBinding::Button(button) => {
                let row =
                    candidate.buttons.get_mut(station).ok_or_else(invalid)?;
                swap_binding(row, action, button)?;
            },
            DigitalBinding::Key(key) => {
                *candidate
                    .keys
                    .get_mut(station)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)? = key;
            },
            DigitalBinding::Mouse(mouse) => {
                *candidate
                    .mouse
                    .get_mut(station)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)? = mouse;
            },
            DigitalBinding::TouchSlot(slot) => {
                let row = candidate
                    .touch_slots
                    .get_mut(station)
                    .ok_or_else(invalid)?;
                swap_binding(row, action, slot)?;
            },
        }
        candidate.validate()?;
        Ok(candidate)
    }

    /// Encode all bindings using the version-one persisted profile format.
    ///
    /// The existing decoder must recover the identical profile using the
    /// supplied physical key name resolver. The byte budget is caller-owned.
    /// # Errors
    /// Rejects invalid, unrepresentable or over-budget mappings.
    pub fn encoded(
        &self,
        actions: [&str; ACTIONS],
        maximum: usize,
        key_names: (KeyNamer, KeyResolver),
    ) -> Result<String, String> {
        encode::encode(self, actions, maximum, key_names)
    }

    fn override_binding(
        &mut self,
        record: Record<'_>,
        actions: &[&str],
        resolve_key: KeyResolver,
    ) -> Result<(), String> {
        let (kind, player, action, name) = record;
        let invalid = || String::from("unknown player, action or binding");
        match kind {
            Kind::Axis | Kind::AxisThreshold => {
                let profile = self
                    .axes
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                let (negative, positive) =
                    name.split_once('/').ok_or_else(invalid)?;
                if kind == Kind::Axis {
                    profile.negative = axis_action(negative, actions)?;
                    profile.positive = axis_action(positive, actions)?;
                } else {
                    profile.press =
                        negative.parse().map_err(|_error| invalid())?;
                    profile.release =
                        positive.parse().map_err(|_error| invalid())?;
                }
            },
            Kind::Button => {
                let slot = self
                    .buttons
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = ControllerButton::from_profile_name(name)
                    .ok_or_else(invalid)?;
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
            Kind::Mouse => {
                let slot = self
                    .mouse
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = parse_mouse(name)?;
            },
            Kind::Key => {
                let slot = self
                    .keys
                    .get_mut(player)
                    .and_then(|row| row.get_mut(action))
                    .ok_or_else(invalid)?;
                *slot = parse_key(name, resolve_key)?;
            },
        }
        Ok(())
    }

    /// Verify that no physical binding silently overwrites another.
    /// # Errors
    /// Rejects duplicate keyboard keys, same-station controller buttons,
    /// invalid axis slots or thresholds and invalid touch permutations.
    pub fn validate(&self) -> Result<(), String> {
        if !self.touch_players.contains(&true) {
            return Err(String::from("invalid touch player"));
        }
        for row in &self.axes {
            for (axis, profile) in ControllerAxis::ALL.into_iter().zip(row) {
                profile.validate(axis, ACTIONS)?;
            }
        }
        for row in &self.touch_slots {
            if row.iter().any(|slot| *slot >= ACTIONS)
                || row.iter().collect::<BTreeSet<_>>().len() != ACTIONS
            {
                return Err(String::from("invalid or duplicate touch slot"));
            }
        }
        let mut keys = BTreeSet::new();
        for key in self.keys.iter().flatten().flatten().copied() {
            if !keys.insert(key.code()) {
                return Err(String::from("duplicate keyboard binding"));
            }
        }
        let mut mouse = BTreeSet::new();
        for button in self.mouse.iter().flatten().flatten().copied() {
            if !mouse.insert(button.code()) {
                return Err(String::from("invalid or duplicate mouse binding"));
            }
        }
        for row in &self.buttons {
            if row
                .iter()
                .copied()
                .map(ControllerButton::code)
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
    Axis,
    AxisThreshold,
    Button,
    Key,
    Mouse,
    Touch,
}

type Record<'text> = (Kind, usize, usize, &'text str);

fn swap_binding<Value>(
    row: &mut [Value],
    action: usize,
    binding: Value,
) -> Result<(), String>
where
    Value: Copy + Eq,
{
    let invalid = || String::from("invalid control edit");
    if row.get(action).is_none() {
        return Err(invalid());
    }
    if let Some(other) = row.iter().position(|value| *value == binding) {
        row.swap(action, other);
    } else {
        *row.get_mut(action).ok_or_else(invalid)? = binding;
    }
    Ok(())
}

fn touch_selection(line: &str, selected: bool) -> Result<Option<&str>, String> {
    if let Some(value) = line.strip_prefix("touch-player=") {
        if selected || value.contains(',') {
            return Err(String::from("invalid touch-player selection"));
        }
        return Ok(Some(value));
    }
    let value = line.strip_prefix("touch-players=");
    if selected && value.is_some() {
        return Err(String::from("duplicate touch selection"));
    }
    Ok(value)
}

fn selected_players<const PLAYERS: usize>(
    value: &str,
) -> Result<[bool; PLAYERS], String> {
    let mut selected = [false; PLAYERS];
    for field in value.split(',') {
        let slot = field
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| selected.get_mut(index))
            .ok_or("invalid touch player")?;
        if *slot {
            return Err(String::from("duplicate touch player"));
        }
        *slot = true;
    }
    Ok(selected)
}

fn axis_action(value: &str, actions: &[&str]) -> Result<Option<usize>, String> {
    if value == "none" {
        return Ok(None);
    }
    actions
        .iter()
        .position(|name| *name == value)
        .map(Some)
        .ok_or_else(|| String::from("unknown axis action"))
}

fn parse_key(
    name: &str,
    resolve_key: KeyResolver,
) -> Result<Option<KeyboardKey>, String> {
    if name == "none" {
        return Ok(None);
    }
    resolve_key(name)
        .map(Some)
        .ok_or_else(|| String::from("unknown keyboard binding"))
}

fn parse_mouse(name: &str) -> Result<Option<PointerButton>, String> {
    if name == "none" {
        return Ok(None);
    }
    PointerButton::from_profile_name(name)
        .map(Some)
        .ok_or_else(|| String::from("unknown mouse binding"))
}

fn parse_line<'text>(
    line: &'text str,
    actions: &[&str],
) -> Result<Record<'text>, String> {
    let invalid = || String::from("invalid control profile record");
    let (left, raw_name) = line.split_once('=').ok_or_else(invalid)?;
    let mut fields = left.split_whitespace();
    let kind = match fields.next() {
        Some("axis") => Kind::Axis,
        Some("axis-threshold") => Kind::AxisThreshold,
        Some("key") => Kind::Key,
        Some("mouse") => Kind::Mouse,
        Some("button") => Kind::Button,
        Some("touch") => Kind::Touch,
        _ => return Err(invalid()),
    };
    let player = fields
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|number| number.checked_sub(1))
        .ok_or_else(invalid)?;
    let field = fields.next().ok_or_else(invalid)?;
    let action = match kind {
        Kind::Axis | Kind::AxisThreshold => {
            ControllerAxis::from_profile_name(field).and_then(|axis| {
                ControllerAxis::ALL.iter().position(|item| *item == axis)
            })
        },
        Kind::Button | Kind::Key | Kind::Mouse | Kind::Touch => {
            actions.iter().position(|name| *name == field)
        },
    }
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
        axes: [[AxisProfile::UNBOUND; ControllerAxis::ALL.len()]; 2],
        buttons: [[ControllerButton::South, ControllerButton::East]; 2],
        keys: [
            [Some(KeyboardKey::LETTER_A), Some(KeyboardKey::LETTER_B)],
            [Some(KeyboardKey::LETTER_C), Some(KeyboardKey::LETTER_D)],
        ],
        mouse: [[None; 2]; 2],
        touch_players: [true, false],
        touch_slots: [[0, 1]; 2],
    };
    const ACTIONS: [&str; 2] = ["primary", "secondary"];

    fn configure(text: &str) -> Result<DigitalProfiles<2, 2>, String> {
        DEFAULT.configured(
            ACTIONS,
            ProfileText { max_bytes: 8192, text },
            resolve_key,
        )
    }

    fn resolve_key(name: &str) -> Option<KeyboardKey> {
        match name {
            "A" => Some(KeyboardKey::LETTER_A),
            "B" => Some(KeyboardKey::LETTER_B),
            "C" => Some(KeyboardKey::LETTER_C),
            "D" => Some(KeyboardKey::LETTER_D),
            "Q" => Some(KeyboardKey::LETTER_Q),
            "R" => Some(KeyboardKey::LETTER_R),
            "Space" => Some(KeyboardKey::SPACE),
            _ => None,
        }
    }

    #[test]
    fn profile_overrides_only_selected_station_and_binding() {
        let configured = configure(concat!(
            "version=1\nkey 2 primary=Space\nbutton 2 primary=x\n",
            "touch-player=2\ntouch 2 primary=secondary\n",
            "touch 2 secondary=primary\n"
        ));
        assert_eq!(
            configured,
            Ok(DigitalProfiles {
                axes: DEFAULT.axes,
                buttons: [[ControllerButton::South, ControllerButton::East], [
                    ControllerButton::West,
                    ControllerButton::East
                ]],
                keys: [
                    [Some(KeyboardKey::LETTER_A), Some(KeyboardKey::LETTER_B)],
                    [Some(KeyboardKey::SPACE), Some(KeyboardKey::LETTER_D)]
                ],
                mouse: DEFAULT.mouse,
                touch_players: [false, true],
                touch_slots: [[0, 1], [1, 0]],
            })
        );
    }

    #[test]
    fn mouse_bindings_are_optional_and_unique_across_stations() {
        let configured = configure(
            "version=1\nmouse 1 primary=left\nmouse 2 secondary=right",
        );
        assert_eq!(
            configured,
            Ok(DigitalProfiles {
                mouse: [[Some(PointerButton::Left), None], [
                    None,
                    Some(PointerButton::Right)
                ]],
                ..DEFAULT
            })
        );
        assert!(
            configure(
                "version=1\nmouse 1 primary=left\nmouse 2 secondary=left"
            )
            .is_err()
        );
        assert!(configure("version=1\nmouse 1 primary=unknown").is_err());
        assert_eq!(configure("version=1\nmouse 1 primary=none"), Ok(DEFAULT));
    }

    #[test]
    fn axis_overrides_admit_only_selected_station() -> Result<(), String> {
        let profiles = configure(concat!(
            "version=1\naxis 2 rightx=secondary/primary\n",
            "axis-threshold 2 rightx=20000/5000\n",
            "axis 2 lefttrigger=none/primary\n",
        ))?;
        let axes = profiles.axis_bindings(1, ACTIONS)?;
        if profiles.axes.first() != DEFAULT.axes.first()
            || !profiles.axis_bindings(0, ACTIONS)?.is_empty()
            || axes.len() != 2
            || !axes.contains_key(&ControllerAxis::RightX.code())
            || !axes.contains_key(&ControllerAxis::TriggerLeft.code())
            || profiles.axis_bindings(2, ACTIONS).is_ok()
        {
            return Err(String::from("axis profile station mismatch"));
        }
        Ok(())
    }

    #[test]
    fn multiple_touch_seats_are_canonical_and_backward_compatible() {
        let configured = configure("version=1\ntouch-players=2,1");
        assert_eq!(
            configured,
            Ok(DigitalProfiles {
                touch_players: [true, true],
                ..DEFAULT
            })
        );
        assert_eq!(
            configure("version=1\ntouch-players=2"),
            configure("version=1\ntouch-player=2")
        );
    }

    #[test]
    fn profile_byte_budget_belongs_to_the_caller() {
        let text = format!("version=1\n{}", " ".repeat(8192));
        let admitted = DEFAULT.configured(
            ACTIONS,
            ProfileText {
                max_bytes: text.len(),
                text: &text,
            },
            resolve_key,
        );
        assert_eq!(admitted, Ok(DEFAULT));
        assert!(
            DEFAULT
                .configured(
                    ACTIONS,
                    ProfileText {
                        max_bytes: text.len() - 1,
                        text: &text
                    },
                    resolve_key
                )
                .is_err()
        );
        let unicode = "version=1\n\u{2003}";
        assert_eq!(
            DEFAULT.configured(
                ACTIONS,
                ProfileText {
                    max_bytes: unicode.len(),
                    text: unicode
                },
                resolve_key
            ),
            Ok(DEFAULT)
        );
        assert!(
            DEFAULT
                .configured(
                    ACTIONS,
                    ProfileText {
                        max_bytes: unicode.chars().count(),
                        text: unicode
                    },
                    resolve_key,
                )
                .is_err()
        );
    }

    #[test]
    fn edited_profiles_preserve_other_bindings_and_reject_conflicts()
    -> Result<(), String> {
        let edited = DEFAULT
            .edited(0, 0, DigitalBinding::Button(ControllerButton::East))?
            .edited(1, 0, DigitalBinding::TouchSlot(1))?
            .edited(0, 1, DigitalBinding::Key(None))?
            .edited(1, 1, DigitalBinding::Mouse(Some(PointerButton::Left)))?;
        let expected = DigitalProfiles {
            buttons: [[ControllerButton::East, ControllerButton::South], [
                ControllerButton::South,
                ControllerButton::East,
            ]],
            keys: [[Some(KeyboardKey::LETTER_A), None], [
                Some(KeyboardKey::LETTER_C),
                Some(KeyboardKey::LETTER_D),
            ]],
            mouse: [[None, None], [None, Some(PointerButton::Left)]],
            touch_slots: [[0, 1], [1, 0]],
            ..DEFAULT
        };
        if edited != expected {
            return Err(String::from("unrelated bindings changed"));
        }
        for result in [
            edited.edited(
                0,
                0,
                DigitalBinding::Key(Some(KeyboardKey::LETTER_C)),
            ),
            edited.edited(
                0,
                0,
                DigitalBinding::Mouse(Some(PointerButton::Left)),
            ),
            edited.edited(2, 0, DigitalBinding::Key(None)),
            edited.edited(0, 2, DigitalBinding::Key(None)),
            edited.edited(1, 0, DigitalBinding::TouchSlot(2)),
        ] {
            if result.is_ok() {
                return Err(String::from("invalid edit accepted"));
            }
        }
        Ok(())
    }

    fn key_name(key: KeyboardKey) -> &'static str {
        key.profile_name().unwrap_or("unknown")
    }

    #[test]
    fn complete_encoding_round_trips_every_profile_field_and_budget()
    -> Result<(), String> {
        let profiles = configure(concat!(
            "version=1\nkey 1 primary=Q\nkey 2 secondary=none\n",
            "mouse 1 primary=left\nmouse 2 secondary=right\n",
            "button 2 primary=x\ntouch-players=2,1\n",
            "touch 2 primary=secondary\ntouch 2 secondary=primary\n",
            "axis 2 rightx=secondary/primary\n",
            "axis-threshold 2 rightx=20000/5000\n",
            "axis 2 lefttrigger=none/primary\n",
        ))?;
        let encoded =
            profiles.encoded(ACTIONS, 8192, (key_name, resolve_key))?;
        if configure(&encoded) != Ok(profiles)
            || profiles.encoded(
                ACTIONS,
                encoded.len(),
                (key_name, resolve_key),
            )? != encoded
            || profiles
                .encoded(ACTIONS, encoded.len() - 1, (key_name, resolve_key))
                .is_ok()
            || profiles
                .encoded(ACTIONS, 8192, (|_key| "unknown", resolve_key))
                .is_ok()
        {
            return Err(String::from(
                "profile encoding failed round-trip or bounds",
            ));
        }
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_invalid_mappings() {
        for invalid in [
            "version=2",
            "version=1\naxis 1 missing=none/primary",
            "version=1\naxis 3 leftx=none/primary",
            "version=1\naxis 1 leftx=missing/primary",
            "version=1\naxis 1 lefttrigger=primary/none",
            "version=1\naxis 1 leftx=primary",
            "version=1\naxis 1 leftx=none/primary\naxis 1 leftx=none/none",
            "version=1\naxis-threshold 1 leftx=32768/1",
            "version=1\naxis-threshold 1 leftx=100/100",
            "version=1\naxis-threshold 1 leftx=0/0",
            concat!(
                "version=1\naxis-threshold 1 leftx=100/20\n",
                "axis-threshold 1 leftx=200/20",
            ),
            "version=1\nkey 1 primary=C",
            "version=1\nbutton 2 primary=b",
            "version=1\nkey 3 primary=Space",
            "version=1\nkey 1 missing=Space",
            "version=1\nkey 0 primary=Space",
            "version=1\nkey 1 primary=not-a-key",
            "version=1\nbutton 2 primary=not-a-button",
            "version=1\nkey 1 primary=Q\nkey 1 primary=R",
            "version=1\ntouch-player=3",
            "version=1\ntouch-players=",
            "version=1\ntouch-players=1,1",
            "version=1\ntouch-players=1,3",
            "version=1\ntouch-players=0",
            "version=1\ntouch-players=1,",
            "version=1\ntouch-player=1,2",
            "version=1\ntouch-player=1\ntouch-players=2",
            "version=1\ntouch-players=2\ntouch-player=1",
            "version=1\ntouch 2 primary=secondary",
            "version=1\ntouch-player=1\ntouch-player=2",
        ] {
            assert!(configure(invalid).is_err());
        }
        assert!(configure(&" ".repeat(8193)).is_err());
    }
}
