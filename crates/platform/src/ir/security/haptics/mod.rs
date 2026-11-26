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
//   - Portable feedback permission and capability admission.
// - Must-Not:
//   - Open devices or submit native effects.
// - Allows:
//   - Inputs: Device capability, focus permission and finite feedback effect.
//   - Outputs: Adapted admitted effect or explicit rejection.
//   - Side effects: None.
// - Split-When:
//   - A separate effect family needs a different admission contract.
// - Merge-When:
//   - Another portable contract provides identical feedback admission.
// - Summary:
//   - Enforce foreground feedback without preventing safety stops.
// - Description:
//   - Adapt single motor effects and reject unavailable capabilities.
// - Usage:
//   - Admit every effect before dispatching it to a connected device.
// - Defaults:
//   - Disconnected, unsupported or disallowed rumble is rejected.

//! Shared admission of finite device feedback requests.

use games::capability::{
    HapticCommand, HapticEffect, HapticError, HapticSupport,
};

/// A feedback command admitted against the current device and permission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedbackRequest(HapticCommand);

impl FeedbackRequest {
    /// Return the admitted command, including any single-channel adaptation.
    #[must_use]
    pub const fn command(self) -> HapticCommand {
        self.0
    }

    /// Admit one bounded command against an observed capability and permission.
    /// # Errors
    /// Rejects unavailable, unsupported or disconnected devices.
    pub fn new(
        command: HapticCommand,
        support: Option<HapticSupport>,
        allowed: bool,
    ) -> Result<Self, HapticError> {
        let effect = admit_feedback(support, allowed, command.effect)?;
        Ok(Self(HapticCommand { effect, ..command }))
    }
}

/// Admit feedback according to the current device and focus permission.
///
/// A stop remains available when foreground permission has been withdrawn.
///
/// # Errors
/// Reports a disconnected device, unsupported capability or a rumble
/// request without current permission.
pub const fn admit_feedback(
    support: Option<HapticSupport>,
    allowed: bool,
    effect: HapticEffect,
) -> Result<HapticEffect, HapticError> {
    let admitted = match support {
        None => return Err(HapticError::Disconnected),
        Some(HapticSupport::Unsupported) => {
            return Err(HapticError::Unsupported);
        },
        Some(HapticSupport::SingleMotor) => effect.single_motor(),
        Some(HapticSupport::DualMotor) => effect,
    };
    match admitted {
        HapticEffect::Rumble { .. } if !allowed => {
            Err(HapticError::Unavailable)
        },
        HapticEffect::Rumble { .. } | HapticEffect::Stop => Ok(admitted),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    #[test]
    fn admits_only_supported_foreground_effects_and_safety_stops() {
        let effect = HapticEffect::Rumble {
            low: 100,
            high: 300,
            milliseconds: NonZeroU16::MIN,
        };
        assert_eq!(
            admit_feedback(None, true, effect),
            Err(HapticError::Disconnected)
        );
        assert_eq!(
            admit_feedback(Some(HapticSupport::Unsupported), true, effect),
            Err(HapticError::Unsupported)
        );
        let dual = Some(HapticSupport::DualMotor);
        let single = Some(HapticSupport::SingleMotor);
        assert_eq!(admit_feedback(dual, true, effect), Ok(effect));
        assert_eq!(
            admit_feedback(single, true, effect),
            Ok(effect.single_motor())
        );
        for support in [dual, single] {
            assert_eq!(
                admit_feedback(support, false, effect),
                Err(HapticError::Unavailable)
            );
            assert_eq!(
                admit_feedback(support, false, HapticEffect::Stop),
                Ok(HapticEffect::Stop)
            );
        }
    }
}
