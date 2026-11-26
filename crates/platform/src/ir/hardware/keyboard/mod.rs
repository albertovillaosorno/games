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
//   - Portable admission and observation of boot keyboard reports.
// - Must-Not:
//   - Open devices or select native transport implementations.
// - Allows:
//   - Inputs: Explicitly selected boot report bytes and LED states.
//   - Outputs: Normalized key state, transitions and output reports.
//   - Side effects: None.
// - Split-When:
//   - Another protocol requires independent report semantics.
// - Merge-When:
//   - Shared device admission owns equivalent report behaviour.
// - Summary:
//   - Bounded physical keyboard protocol capability.
// - Description:
//   - Decode only the fixed boot keyboard format, not arbitrary HID.
// - Usage:
//   - Select this contract only for verified boot-mode interfaces.
// - Defaults:
//   - Invalid reports never change a prior admitted observation.

use games::input::InputEvent;
use games::input::physical::KeyboardKey;

use crate::ir::security::input::admit_boot_keyboard;

/// A report cannot be interpreted as a complete boot keyboard observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardReportError {
    /// The caller did not provide the exact eight-byte boot report.
    Length,
    /// The array contains a modifier rather than a non-modifier usage.
    Modifier,
    /// A rollover, failure or undefined-error usage invalidates the report.
    UncertainKeys,
}

/// A physical keyboard indicator's absolute state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LedState {
    /// Indicator disabled.
    #[default]
    Off,
    /// Indicator enabled.
    On,
}

/// Absolute LED state for the five standard boot keyboard indicators.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardLeds {
    /// Caps lock indicator.
    pub caps_lock: LedState,
    /// Compose indicator.
    pub compose: LedState,
    /// Kana indicator.
    pub kana: LedState,
    /// Num lock indicator.
    pub num_lock: LedState,
    /// Scroll lock indicator.
    pub scroll_lock: LedState,
}

impl KeyboardLeds {
    /// Encode the complete output report without performing device I/O.
    #[must_use]
    pub fn encode(self) -> [u8; 1] {
        <() as BootKeyboardPort>::leds(self)
    }
}

/// Immutable admitted keyboard observation, independent of array order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootKeyboardReport {
    pressed: [bool; 256],
}

impl BootKeyboardReport {
    /// Generate deterministic releases followed by presses.
    ///
    /// Simultaneous changes do not imply a physical event order.
    pub fn changes<'report>(
        &'report self,
        previous: &'report Self,
    ) -> impl Iterator<Item = InputEvent> + 'report {
        [false, true].into_iter().flat_map(move |down| {
            self.pressed
                .iter()
                .zip(&previous.pressed)
                .zip(0u8..=u8::MAX)
                .filter_map(move |((&current, &prior), code)| {
                    (current != prior && current == down).then_some(
                        InputEvent::Key {
                            code: u32::from(code),
                            down,
                        },
                    )
                })
        })
    }

    /// Decode a verified boot-mode interface's exact eight-byte report.
    ///
    /// The reserved byte is ignored; array ordering and duplicate usages
    /// do not affect the observation. No report-mode or transport detection
    /// is performed here.
    /// # Errors
    /// Rejects invalid length, error usages and misplaced modifier usages.
    pub fn decode(bytes: &[u8]) -> Result<Self, KeyboardReportError> {
        let report = admit_boot_keyboard(bytes)?;
        Ok(Self {
            pressed: <() as BootKeyboardPort>::decode(*report),
        })
    }

    /// An empty observation for startup or explicit release on detach.
    #[must_use]
    pub const fn empty() -> Self {
        Self { pressed: [false; 256] }
    }

    /// Observe currently pressed keys in stable usage order.
    pub fn pressed(&self) -> impl Iterator<Item = KeyboardKey> + '_ {
        self.pressed
            .iter()
            .zip(0u8..=u8::MAX)
            .filter_map(|(&down, code)| {
                down.then_some(KeyboardKey::from_code(u32::from(code)))
            })
    }
}

/// Private, host-independent physical keyboard translation port.
pub(crate) trait BootKeyboardPort {
    fn decode(report: [u8; 8]) -> [bool; 256];
    fn leds(states: KeyboardLeds) -> [u8; 1];
}

#[cfg(test)]
mod tests {
    use games::input::InputEvent;
    use games::input::physical::KeyboardKey;

    use super::{
        BootKeyboardReport, KeyboardLeds, KeyboardReportError, LedState,
    };
    use crate::ir::check;
    use crate::ir::frame::RenderError;

    #[test]
    fn report_admits_absolute_state_without_array_order()
    -> Result<(), RenderError> {
        let decode = |bytes: &[u8]| {
            BootKeyboardReport::decode(bytes)
                .map_err(|error| RenderError::Backend(format!("{error:?}")))
        };
        let first = decode(&[2, 0, 4, 7, 0, 0, 0, 0])?;
        let same = decode(&[2, 9, 7, 4, 4, 0, 0, 0])?;
        check(&first, &same)?;
        check(&first.pressed().collect::<Vec<_>>(), &[
            KeyboardKey::LETTER_A,
            KeyboardKey::LETTER_D,
            KeyboardKey::from_code(225),
        ])?;
        let next = decode(&[0, 0, 5, 7, 0, 0, 0, 0])?;
        check(&next.changes(&first).collect::<Vec<_>>(), &[
            InputEvent::Key { code: 4, down: false },
            InputEvent::Key { code: 225, down: false },
            InputEvent::Key { code: 5, down: true },
        ])?;
        check(&BootKeyboardReport::empty().changes(&next).count(), &2)
    }

    #[test]
    fn uncertain_reports_are_not_empty_key_states() {
        for usage in [1, 2, 3] {
            assert_eq!(
                BootKeyboardReport::decode(&[0, 0, usage, 0, 0, 0, 0, 0]),
                Err(KeyboardReportError::UncertainKeys)
            );
        }
        assert_eq!(
            BootKeyboardReport::decode(&[0; 7]),
            Err(KeyboardReportError::Length)
        );
        assert_eq!(
            BootKeyboardReport::decode(&[0; 9]),
            Err(KeyboardReportError::Length)
        );
        assert_eq!(
            BootKeyboardReport::decode(&[0, 0, 224, 0, 0, 0, 0, 0]),
            Err(KeyboardReportError::Modifier)
        );
    }

    #[test]
    fn absolute_led_report_has_zero_padding() {
        assert_eq!(KeyboardLeds::default().encode(), [0]);
        assert_eq!(
            KeyboardLeds {
                caps_lock: LedState::On,
                kana: LedState::On,
                ..KeyboardLeds::default()
            }
            .encode(),
            [18]
        );
        assert_eq!(
            KeyboardLeds {
                caps_lock: LedState::On,
                compose: LedState::On,
                kana: LedState::On,
                num_lock: LedState::On,
                scroll_lock: LedState::On,
            }
            .encode(),
            [31]
        );
    }
}
