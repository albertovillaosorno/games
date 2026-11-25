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
//   - Private host service selection and software adapter composition.
// - Must-Not:
//   - Own game simulation or source media interpretation.
// - Allows:
//   - Inputs: Shared engine configuration and host-provided resources.
//   - Outputs: Native platform capabilities.
//   - Side effects: Explicit host operating-system access.
// - Split-When:
//   - A platform adapter needs an independent responsibility.
// - Merge-When:
//   - Multiple adapters implement the same platform boundary.
// - Summary:
//   - Platform integration outside the transport-neutral engine.
// - Description:
//   - Exposes native storage adapters behind target-specific module boundaries.
// - Usage:
//   - Compose adapters with the engine in a host application.
// - Defaults:
//   - No host resources are accessed by module selection.

//! Private software implementation and current host selection.

mod linux;

/// Explicit absence of the authenticated LAN capability.
#[cfg(any(not(feature = "secure-lan"), target_arch = "wasm32"))]
pub mod lan {
    /// Whether the selected host supports LAN.
    pub const AVAILABLE: bool = false;
}

use std::io;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::io::ErrorKind;
use std::num::NonZeroU32;
use std::time::Duration;

use games::input::physical::KeyboardKey;
#[cfg(all(feature = "secure-lan", not(target_arch = "wasm32")))]
pub use linux::lan;
pub use linux::storage::*;
pub use linux::toolchain;

use crate::ir::audio::{
    AudioDevices, PlaybackError, PlaybackFactory, PlaybackPort,
};
use crate::ir::frame::RenderError;
use crate::ir::graphics::{WindowFactory, WindowPort};
use crate::ir::input::InputNames;
use crate::ir::measurement::MeasurementSource;
use crate::ir::security::audio::PlaybackRequest;

#[cfg(all(feature = "interactive", target_os = "linux"))]
impl PlaybackFactory for () {
    fn devices() -> Result<AudioDevices, PlaybackError> {
        linux::audio::devices()
    }

    fn open(
        request: PlaybackRequest<'_>,
    ) -> Result<PlaybackPort, PlaybackError> {
        linux::audio::open_audio(request)
    }
}

#[cfg(not(all(feature = "interactive", target_os = "linux")))]
impl PlaybackFactory for () {
    fn devices() -> Result<AudioDevices, PlaybackError> {
        Err(PlaybackError::Unsupported)
    }

    fn open(
        _request: PlaybackRequest<'_>,
    ) -> Result<PlaybackPort, PlaybackError> {
        Err(PlaybackError::Unsupported)
    }
}

#[cfg(all(feature = "interactive", target_os = "linux"))]
impl WindowFactory for () {
    const SUPPORTED: bool = true;

    fn open(
        title: &str,
        width: NonZeroU32,
        height: NonZeroU32,
        hidden: bool,
    ) -> Result<WindowPort, RenderError> {
        linux::graphics::open_window(title, width, height, hidden)
    }
}

#[cfg(not(all(feature = "interactive", target_os = "linux")))]
impl WindowFactory for () {
    const SUPPORTED: bool = false;

    fn open(
        _title: &str,
        _width: NonZeroU32,
        _height: NonZeroU32,
        _hidden: bool,
    ) -> Result<WindowPort, RenderError> {
        Err(RenderError::Unsupported)
    }
}

#[cfg(all(feature = "interactive", target_os = "linux"))]
impl InputNames for () {
    fn key_from_name(name: &str) -> Option<KeyboardKey> {
        linux::input::key_from_name(name)
    }

    fn key_name(key: KeyboardKey) -> &'static str {
        linux::input::key_name(key)
    }
}

#[cfg(not(all(feature = "interactive", target_os = "linux")))]
impl InputNames for () {
    fn key_from_name(_name: &str) -> Option<KeyboardKey> {
        None
    }

    fn key_name(_key: KeyboardKey) -> &'static str {
        "UNKNOWN"
    }
}

impl MeasurementSource for () {
    fn executable_bytes() -> io::Result<u64> {
        #[cfg(not(target_arch = "wasm32"))]
        return linux::performance::executable_bytes();
        #[cfg(target_arch = "wasm32")]
        Err(ErrorKind::Unsupported.into())
    }

    fn now() -> io::Result<Duration> {
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(linux::performance::now());
        #[cfg(target_arch = "wasm32")]
        Err(ErrorKind::Unsupported.into())
    }

    fn peak_resident_bytes() -> io::Result<u64> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        return linux::performance::peak_resident_bytes();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        Err(ErrorKind::Unsupported.into())
    }

    fn resident_bytes() -> io::Result<u64> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        return linux::performance::resident_bytes();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        Err(ErrorKind::Unsupported.into())
    }
}
