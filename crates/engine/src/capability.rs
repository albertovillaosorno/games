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
//   - Portable controller feedback requests and capability reporting.
// - Must-Not:
//   - Access devices or prescribe title-specific feedback.
// - Allows:
//   - Inputs: Normalized feedback commands and opaque controller identities.
//   - Outputs: Explicit capability and feedback outcomes.
//   - Side effects: Only through an explicitly supplied feedback adapter.
// - Split-When:
//   - A distinct feedback capability requires independent ownership.
// - Merge-When:
//   - Another shared contract covers equivalent feedback semantics.
// - Summary:
//   - Portable bounded controller feedback.
// - Description:
//   - Expresses rumble without native controller or driver types.
// - Usage:
//   - Query support and submit commands through a host adapter.
// - Defaults:
//   - Unavailable feedback is explicit and never changes simulation.
//
//! Portable controller feedback capability contracts.

use std::num::NonZeroU16;

/// Device feedback exposed by a host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HapticSupport {
    /// Low and high frequency command channels; physical motor count is
    /// not implied by this capability.
    DualMotor,
    /// One actuator; combine motor intensities with their maximum.
    SingleMotor,
    /// A device has no supported actuator.
    Unsupported,
}

/// A finite, normalized feedback effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HapticEffect {
    /// Replace the active effect for a bounded duration.
    Rumble {
        /// Low-frequency intensity, zero to `u16::MAX`.
        low: u16,
        /// High-frequency intensity, zero to `u16::MAX`.
        high: u16,
        /// Positive duration in milliseconds, at most 65,535 ms.
        milliseconds: NonZeroU16,
    },
    /// Stop all active feedback on this controller.
    Stop,
}

impl HapticEffect {
    /// Adapt the request to one available motor without changing its timing.
    #[must_use]
    pub const fn single_motor(self) -> Self {
        match self {
            Self::Stop => Self::Stop,
            Self::Rumble { low, high, milliseconds } => {
                let strength = if low > high {
                    low
                } else {
                    high
                };
                Self::Rumble {
                    low: strength,
                    high: strength,
                    milliseconds,
                }
            },
        }
    }
}

/// A feedback command addressed to a host-local controller identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HapticCommand {
    /// Opaque connected controller identifier, not a native handle.
    pub device: u64,
    /// Requested effect.
    pub effect: HapticEffect,
}

/// Explicit non-gameplay feedback failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HapticError {
    /// The selected controller is no longer attached.
    Disconnected,
    /// The host failed to deliver the command.
    Unavailable,
    /// The connected controller lacks the requested capability.
    Unsupported,
}

/// Host-provided feedback port. No native device types cross the boundary.
///
/// Adapters stop feedback on disconnect or focus loss, and replace active
/// effects rather than queueing unbounded requests.
pub trait Haptics {
    /// Deliver a finite effect independently of simulation state.
    ///
    /// # Errors
    /// Reports disconnected devices, unsupported features or host failures.
    fn submit(&mut self, command: HapticCommand) -> Result<(), HapticError>;

    /// Report support, or None for a disconnected device.
    fn support(&self, device: u64) -> Option<HapticSupport>;
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::{
        HapticCommand, HapticEffect, HapticError, HapticSupport, Haptics,
    };

    #[derive(Debug, Default)]
    struct Fake {
        effect: Option<HapticEffect>,
    }

    impl Haptics for Fake {
        fn submit(
            &mut self,
            command: HapticCommand,
        ) -> Result<(), HapticError> {
            match self.support(command.device) {
                None => Err(HapticError::Disconnected),
                Some(HapticSupport::Unsupported) => {
                    Err(HapticError::Unsupported)
                },
                Some(HapticSupport::SingleMotor | HapticSupport::DualMotor) => {
                    self.effect = Some(command.effect.single_motor());
                    Ok(())
                },
            }
        }

        fn support(&self, device: u64) -> Option<HapticSupport> {
            match device {
                1 => Some(HapticSupport::SingleMotor),
                2 => Some(HapticSupport::Unsupported),
                _ => None,
            }
        }
    }

    #[test]
    fn bounded_feedback_and_explicit_unavailability() {
        let mut host = Fake::default();
        let effect = HapticEffect::Rumble {
            low: 100,
            high: 200,
            milliseconds: NonZeroU16::MIN,
        };
        assert_eq!(host.submit(HapticCommand { device: 1, effect }), Ok(()));
        assert_eq!(host.effect, Some(effect.single_motor()));
        assert_eq!(
            host.submit(HapticCommand { device: 2, effect }),
            Err(HapticError::Unsupported),
        );
        assert_eq!(
            host.submit(HapticCommand { device: 3, effect }),
            Err(HapticError::Disconnected),
        );
        assert_eq!(
            host.submit(HapticCommand {
                device: 1,
                effect: HapticEffect::Stop,
            }),
            Ok(())
        );
        assert_eq!(host.effect, Some(HapticEffect::Stop));
    }
}
