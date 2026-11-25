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

/// Explicit absence of authenticated LAN; no backend handles can exist.
#[cfg(any(not(feature = "secure-lan"), target_arch = "wasm32"))]
mod unavailable_lan {
    use core::net::SocketAddr;
    use std::io::{ErrorKind, Result};

    use games::network::MessageSchema;

    use crate::ir::lan::identity::IdentityPort;
    use crate::ir::lan::pump::{HostInputConfig, PumpPort};
    use crate::ir::lan::{
        ClientTls, FramePort, LanAvailability, LanConfig, LanFactory, ServerTls,
    };

    impl LanAvailability for () {
        const SUPPORTED: bool = false;
    }

    impl LanFactory for () {
        fn bind<Message, Codec>(
            _config: HostInputConfig<'_>,
            _tls: ServerTls,
            _codec: Codec,
        ) -> Result<PumpPort<Message>>
        where
            Message: Send + 'static,
            Codec: MessageSchema<Message = Message> + Send + 'static,
        {
            Err(ErrorKind::Unsupported.into())
        }

        fn client_identity(
            _root_der: &[u8],
            _leaf_der: &[u8],
            _key_der: &[u8],
        ) -> Result<IdentityPort> {
            Err(ErrorKind::Unsupported.into())
        }

        fn connect(
            _address: SocketAddr,
            _config: LanConfig,
            _tls: &ClientTls,
            _name: &str,
        ) -> Result<FramePort> {
            Err(ErrorKind::Unsupported.into())
        }

        fn host_identity(
            _name: &str,
            _root_der: &[u8],
            _leaf_der: &[u8],
            _key_der: &[u8],
        ) -> Result<IdentityPort> {
            Err(ErrorKind::Unsupported.into())
        }

        fn validate_root(_root_der: &[u8]) -> Result<()> {
            Err(ErrorKind::Unsupported.into())
        }
    }
}

#[cfg(not(target_os = "linux"))]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::io::ErrorKind;
use std::num::NonZeroU32;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::time::Duration;

use games::input::physical::KeyboardKey;
pub use linux::storage::*;

use crate::ir::audio::{
    AudioDevices, PlaybackError, PlaybackFactory, PlaybackPort,
};
use crate::ir::frame::RenderError;
use crate::ir::graphics::{WindowFactory, WindowPort};
use crate::ir::input::InputNames;
use crate::ir::measurement::MeasurementSource;
use crate::ir::security::audio::PlaybackRequest;
#[cfg(not(target_os = "linux"))]
use crate::ir::security::toolchain::CargoArguments;
#[cfg(not(target_os = "linux"))]
use crate::ir::security::toolchain::validation::Arguments;
use crate::ir::storage::ExecutableLocation;
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::validation::{Report, ValidationBackend};
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::{
    Target, ToolVersion, ToolchainError, ToolchainFactory, ValidationTool,
    reference::ReferenceTool,
};

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
        return linux::performance::artifact_bytes(&PathBuf::from(
            <() as ExecutableLocation>::executable_path()?,
        ));
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

impl ExecutableLocation for () {
    fn executable_path() -> io::Result<OsString> {
        #[cfg(not(target_arch = "wasm32"))]
        return executable_path().map(PathBuf::into_os_string);
        #[cfg(target_arch = "wasm32")]
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(not(target_os = "linux"))]
impl ToolchainFactory for () {
    fn arguments() -> Result<Vec<OsString>, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn available(_target: Target) -> bool {
        false
    }

    fn compile(
        _args: CargoArguments<'_>,
        _root: &OsStr,
        _target: Target,
    ) -> Result<OsString, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn download_reference(
        _tool: ReferenceTool,
    ) -> Result<OsString, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn metadata(
        _args: CargoArguments<'_>,
        _root: &OsStr,
        _manifest: &OsStr,
    ) -> Result<Vec<u8>, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn probe_version(
        _root: &OsStr,
        _tool: ValidationTool,
    ) -> Result<ToolVersion, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn workspace_root() -> Result<OsString, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn write_line(_line: &str) -> Result<(), ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(not(target_os = "linux"))]
impl ValidationBackend for () {
    fn execute_validation(
        _root: &OsStr,
        _args: &Arguments,
        _command: String,
    ) -> Result<Report, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}
