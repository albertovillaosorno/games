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
//   - Typed motion sensor identity, units and observations.
// - Must-Not:
//   - Open hardware or assume a sensor is present.
// - Allows:
//   - Inputs: Explicit sensor requests and admitted observations.
//   - Outputs: Motion readings or typed unavailable errors.
//   - Side effects: None.
// - Split-When:
//   - A distinct sensor needs different measurement semantics.
// - Merge-When:
//   - Existing motion contracts cover equivalent observations.
// - Summary:
//   - Optional portable three-axis motion sensor contract.
// - Description:
//   - Reports sensor-local values without claiming calibration.
// - Usage:
//   - Read through an admitted input session.
// - Defaults:
//   - No implicit motion input or device capability.

/// Standard motion sensor kind and units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionSensor {
    /// Acceleration in metres per second squared.
    Accelerometer,
    /// Angular velocity in radians per second.
    Gyroscope,
    /// Left-side acceleration in metres per second squared.
    LeftAccelerometer,
    /// Left-side angular velocity in radians per second.
    LeftGyroscope,
    /// Right-side acceleration in metres per second squared.
    RightAccelerometer,
    /// Right-side angular velocity in radians per second.
    RightGyroscope,
}

impl MotionSensor {
    /// All admitted motion sensor kinds.
    pub const ALL: [Self; 6] = [
        Self::Accelerometer,
        Self::Gyroscope,
        Self::LeftAccelerometer,
        Self::LeftGyroscope,
        Self::RightAccelerometer,
        Self::RightGyroscope,
    ];
}

/// Sensor observation in its own coordinate frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionReading {
    /// Admitted controller identity.
    pub device: u64,
    /// Measurement kind and units.
    pub sensor: MotionSensor,
    /// Three sensor-local axes; no calibration or freshness claim.
    pub values: [f32; 3],
}

/// Explicit rejection of an optional sensor request or observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionError {
    /// The input session is not foreground.
    Background,
    /// The device is not admitted to this session.
    Disconnected,
    /// The returned observation contains invalid values.
    InvalidReading,
    /// The host failed to obtain or stop the requested observation.
    Unavailable,
    /// The device does not expose the requested sensor.
    Unsupported,
}
