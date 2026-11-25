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
//   - Neutral host input and application lifecycle event exchange.
// - Must-Not:
//   - Open devices, interpret native handles or select device families.
// - Allows:
//   - Inputs: Normalized input transitions and lifecycle notifications.
//   - Outputs: Events and existing portable feedback contracts.
//   - Side effects: None.
// - Split-When:
//   - A separate device report protocol requires its own contract.
// - Merge-When:
//   - Existing engine input contracts cover equivalent exchange.
// - Summary:
//   - Bridge host device events into the shared action router.
// - Description:
//   - Preserve input semantics without exposing the native event source.
// - Usage:
//   - Translate admitted native events before routing them to an application.
// - Defaults:
//   - No device or feedback capability is assumed.

//! Portable host input and feedback exchange.

use std::time::Duration;

pub use games::capability::{
    HapticCommand, HapticEffect, HapticError, HapticSupport, Haptics,
};
pub use games::input::InputEvent;

/// Events delivered by a host to its action router or application loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEvent {
    /// A platform-neutral input transition.
    Input(InputEvent),
    /// The application or its window received a close request.
    Quit,
}

/// A window-scoped input observation before controller admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputTransition {
    /// A device disconnected; the adapter must release its resources.
    Disconnected(u64),
    /// A change in device availability requests discovery after queue drain.
    DiscoveryHint,
    /// An already normalized event still subject to session admission.
    Event(HostEvent),
    /// The window regained focus and may accept feedback again.
    FocusGained,
    /// The polled event has no portable input meaning.
    Ignored,
}

/// One polled event, including observations ignored by the input router.
///
/// Timestamps use the originating host's monotonic time domain, never a
/// native wrapping counter or a wall-clock timestamp. An absent timestamp
/// means the source could not establish an unambiguous observation time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputSample {
    timestamp: Option<Duration>,
    transition: InputTransition,
    window_scope: Option<u64>,
}

impl InputSample {
    /// Convert the observation to time since the input session began.
    ///
    /// Observations queued before startup belong to time zero. The origin
    /// must come from the same clock as the sample.
    #[must_use]
    pub const fn elapsed_since(self, origin: Duration) -> Option<Duration> {
        match self.timestamp {
            Some(timestamp) => Some(timestamp.saturating_sub(origin)),
            None => None,
        }
    }

    /// Check whether an observation is global or belongs to the given window.
    #[must_use]
    pub fn matches_window(self, scope: u64) -> bool {
        self.window_scope.is_none_or(|window| window == scope)
    }

    /// Construct a normalized observation from an adapter or event source.
    /// The timestamp and window scope must use the owning host's domain.
    #[must_use]
    pub const fn new(
        timestamp: Option<Duration>,
        transition: InputTransition,
        window_scope: Option<u64>,
    ) -> Self {
        Self {
            timestamp,
            transition,
            window_scope,
        }
    }

    /// Observe the normalized transition, not a backend event handle.
    #[must_use]
    pub const fn transition(self) -> InputTransition {
        self.transition
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{InputSample, InputTransition};

    #[test]
    fn window_scope_does_not_accept_another_window() {
        let scoped = InputSample::new(None, InputTransition::Ignored, Some(7));
        let global = InputSample::new(None, InputTransition::Ignored, None);
        assert!(scoped.matches_window(7));
        assert!(!scoped.matches_window(8));
        assert!(global.matches_window(8));
    }

    #[test]
    fn elapsed_sample_preserves_pre_start_and_missing_time() {
        let origin = Duration::from_millis(100);
        for (timestamp, expected) in [
            (Some(90), Some(Duration::ZERO)),
            (Some(100), Some(Duration::ZERO)),
            (Some(123), Some(Duration::from_millis(23))),
            (None, None),
        ] {
            let sample = InputSample::new(
                timestamp.map(Duration::from_millis),
                InputTransition::Ignored,
                None,
            );
            assert_eq!(sample.elapsed_since(origin), expected);
        }
    }
}
