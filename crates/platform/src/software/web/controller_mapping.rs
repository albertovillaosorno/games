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
//   - Correspondence between standard browser buttons and physical codes.
// - Must-Not:
//   - Assign stations, interpret title actions or query connected hardware.
// - Allows:
//   - Inputs: Standard button indices and portable controller identities.
//   - Outputs: Button correspondence and profile representability.
//   - Side effects: None.
// - Split-When:
//   - Browser button mapping requires independent device-specific policies.
// - Merge-When:
//   - A shared browser mapping owns the same correspondence.
// - Summary:
//   - Share one mapping between profile admission and event translation.
// - Description:
//   - Reject extended native buttons outside the standard browser mapping.
// - Usage:
//   - Admit configured buttons and translate standard mapped observations.
// - Defaults:
//   - No extended device-specific buttons.

use games::input::physical::ControllerButton;

pub(super) const BUTTONS: [(u32, ControllerButton); 15] = [
    (0, ControllerButton::South),
    (1, ControllerButton::East),
    (2, ControllerButton::West),
    (3, ControllerButton::North),
    (4, ControllerButton::LeftShoulder),
    (5, ControllerButton::RightShoulder),
    (8, ControllerButton::Back),
    (9, ControllerButton::Start),
    (10, ControllerButton::LeftStick),
    (11, ControllerButton::RightStick),
    (12, ControllerButton::DPadUp),
    (13, ControllerButton::DPadDown),
    (14, ControllerButton::DPadLeft),
    (15, ControllerButton::DPadRight),
    (16, ControllerButton::Guide),
];

pub(in crate::software) fn supports(button: ControllerButton) -> bool {
    BUTTONS.iter().any(|&(_, mapped)| mapped == button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_profiles_exclude_extended_native_buttons() {
        let extended = [
            ControllerButton::Misc1,
            ControllerButton::Paddle1,
            ControllerButton::Paddle2,
            ControllerButton::Paddle3,
            ControllerButton::Paddle4,
            ControllerButton::Touchpad,
        ];
        for button in ControllerButton::ALL {
            assert_eq!(supports(button), !extended.contains(&button));
        }
    }
}
