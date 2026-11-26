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
//   - Standard boot keyboard report bit and usage translation.
// - Must-Not:
//   - Open devices, access host services or assume a transport.
// - Allows:
//   - Inputs: Admitted boot report and absolute LED states.
//   - Outputs: Stable keyboard usages and a bounded LED report.
//   - Side effects: None.
// - Split-When:
//   - Another keyboard protocol has a different report layout.
// - Merge-When:
//   - Identical keyboard protocol layouts share a decoder.
// - Summary:
//   - USB HID boot keyboard protocol translation.
// - Description:
//   - Device Class Definition HID 1.11, Appendix B and C.
// - Usage:
//   - Invoked through the portable keyboard report contract.
// - Defaults:
//   - Interpret only the admitted fixed boot report prefix.

use crate::ir::hardware::keyboard::{BootKeyboardPort, KeyboardLeds, LedState};

impl BootKeyboardPort for () {
    fn decode(report: [u8; 8]) -> [bool; 256] {
        let [modifiers, _, keys @ ..] = report;
        let mut pressed = [false; 256];
        for (state, usage) in pressed.iter_mut().zip(0u8..=u8::MAX) {
            *state = if (224..=231).contains(&usage) {
                let bit = u32::from(usage.saturating_sub(224));
                modifiers & 1u8.rotate_left(bit) != 0
            } else {
                usage != 0 && keys.contains(&usage)
            };
        }
        pressed
    }

    fn leds(states: KeyboardLeds) -> [u8; 1] {
        [indicator(states.num_lock, 1)
            | indicator(states.caps_lock, 2)
            | indicator(states.scroll_lock, 4)
            | indicator(states.compose, 8)
            | indicator(states.kana, 16)]
    }
}

const fn indicator(state: LedState, bit: u8) -> u8 {
    match state {
        LedState::Off => 0,
        LedState::On => bit,
    }
}
