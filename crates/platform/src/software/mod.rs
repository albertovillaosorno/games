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

#[cfg(all(target_os = "android", feature = "application-entry"))]
mod android;
mod linux;
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
mod web;

#[cfg(all(
    test,
    not(all(
        target_arch = "wasm32",
        target_os = "unknown",
        feature = "interactive"
    ))
))]
mod tests {
    use super::*;

    #[test]
    fn unavailable_host_does_not_call_application() {
        use std::cell::Cell;
        use std::rc::Rc;

        use crate::ir::graphics::frame_loop::start;
        let called = Rc::new(Cell::new(false));
        let completion = Rc::clone(&called);
        assert_eq!(
            start(
                |_| Err(RenderError::InvalidTimestamp),
                move |_| completion.set(true),
            ),
            Err(RenderError::Unsupported)
        );
        assert!(!called.get());
    }
}

/// Explicit absence of authenticated LAN; no backend handles can exist.
#[cfg(any(not(feature = "secure-lan"), target_arch = "wasm32"))]
mod unavailable_lan {
    use core::net::SocketAddr;
    use std::io::{ErrorKind, Result};

    use games::network::MessageSchema;

    use crate::ir::lan::identity::IdentityPort;
    use crate::ir::lan::pump::PumpPort;
    use crate::ir::lan::{
        ClientTls, FramePort, LanAvailability, LanConfig, LanFactory, ServerTls,
    };
    use crate::ir::security::lan::host_binding::HostBindRequest;

    impl LanAvailability for () {
        const SUPPORTED: bool = false;
    }

    impl LanFactory for () {
        fn bind<Message, Codec>(
            _config: HostBindRequest,
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

use std::env::consts::{ARCH, OS};
#[cfg(not(target_os = "linux"))]
use std::ffi::OsStr;
use std::ffi::OsString;
#[cfg(not(any(
    target_os = "linux",
    all(target_os = "android", feature = "application-entry"),
    all(target_arch = "wasm32", target_os = "unknown")
)))]
use std::fmt::Arguments as StatusMessage;
use std::io;
use std::io::ErrorKind;
use std::num::NonZeroU32;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::time::Duration;

use games::input::physical::{ControllerButton, KeyboardKey};

#[cfg(any(test, feature = "test-fixtures"))]
use crate::ir::audio::fixtures::AudioFixturePort;
use crate::ir::audio::{
    AudioDevices, PlaybackError, PlaybackFactory, PlaybackPort,
};
use crate::ir::frame::RenderError;
#[cfg(any(test, feature = "test-fixtures"))]
use crate::ir::graphics::fixtures::SceneFixturePort;
use crate::ir::graphics::frame_loop::{
    FrameComplete, FrameLoopPort, FrameStep,
};
use crate::ir::graphics::{WindowFactory, WindowPort};
use crate::ir::identification::IdentificationPort;
use crate::ir::input::InputNames;
#[cfg(any(test, feature = "test-fixtures"))]
use crate::ir::input::fixtures::EventFixturePort;
use crate::ir::measurement::MeasurementSource;
#[cfg(not(any(
    target_os = "linux",
    all(target_os = "android", feature = "application-entry"),
    all(target_arch = "wasm32", target_os = "unknown")
)))]
use crate::ir::process::ProcessPort;
#[cfg(all(
    not(target_os = "linux"),
    any(test, feature = "test-fixtures")
))]
use crate::ir::process::fixtures::{FixturePort, NativeSmoke};
use crate::ir::security::audio::PlaybackRequest;
#[cfg(not(target_os = "linux"))]
use crate::ir::security::toolchain::CargoArguments;
#[cfg(not(target_os = "linux"))]
use crate::ir::security::toolchain::validation::{Arguments, Completion};
use crate::ir::storage::ExecutableLocation;
#[cfg(not(all(
    feature = "interactive",
    target_arch = "wasm32",
    target_os = "unknown"
)))]
use crate::ir::storage::bundle::{
    BundleError,
    directory::{
        DirectoryComplete, DirectoryPicker, DirectoryPlan, DirectorySelector,
    },
};
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::environment::{Environment, EnvironmentBackend};
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::reference::retention::{
    Operation as RetentionOperation, Plan as RetentionPlan,
    Policy as RetentionPolicy, RetentionBackend,
};
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::validation::ValidationBackend;
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::validation::state::{StateFactory, StatePort};
#[cfg(not(target_os = "linux"))]
use crate::ir::toolchain::{
    BrowserBindings, Target, ToolVersion, ToolchainError, ToolchainFactory,
    ValidationTool, reference::ReferenceTool,
};

#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
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

#[cfg(all(
    feature = "interactive",
    target_arch = "wasm32",
    target_os = "unknown"
))]
impl PlaybackFactory for () {
    fn devices() -> Result<AudioDevices, PlaybackError> {
        web::audio::devices()
    }

    fn open(
        request: PlaybackRequest<'_>,
    ) -> Result<PlaybackPort, PlaybackError> {
        web::audio::open_audio(request)
    }
}

#[cfg(not(all(
    feature = "interactive",
    any(
        target_os = "linux",
        target_os = "android",
        all(target_arch = "wasm32", target_os = "unknown")
    )
)))]
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

#[cfg(any(test, feature = "test-fixtures"))]
impl AudioFixturePort for () {
    fn verify_playback() -> Result<(), PlaybackError> {
        #[cfg(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "interactive",
            feature = "test-fixtures"
        ))]
        return web::audio::verify_fixture();
        #[cfg(not(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "interactive",
            feature = "test-fixtures"
        )))]
        Err(PlaybackError::Unsupported)
    }
}

#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
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

#[cfg(all(
    feature = "interactive",
    target_arch = "wasm32",
    target_os = "unknown"
))]
impl WindowFactory for () {
    const SUPPORTED: bool = true;

    fn open(
        title: &str,
        width: NonZeroU32,
        height: NonZeroU32,
        hidden: bool,
    ) -> Result<WindowPort, RenderError> {
        web::graphics::open_window(title, width, height, hidden)
    }
}

#[cfg(not(all(
    feature = "interactive",
    any(
        target_os = "linux",
        target_os = "android",
        all(target_arch = "wasm32", target_os = "unknown")
    )
)))]
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

#[cfg(all(
    feature = "interactive",
    any(target_os = "linux", target_os = "android")
))]
impl InputNames for () {
    fn controller_button_supported(_button: ControllerButton) -> bool {
        true
    }

    fn key_from_name(name: &str) -> Option<KeyboardKey> {
        KeyboardKey::from_profile_name(name)
            .or_else(|| linux::input::key_from_name(name))
    }

    fn key_name(key: KeyboardKey) -> &'static str {
        linux::input::key_name(key)
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl InputNames for () {
    fn controller_button_supported(button: ControllerButton) -> bool {
        web::controller_mapping::supports(button)
    }

    fn key_from_name(name: &str) -> Option<KeyboardKey> {
        web::keyboard::profile_key(name)
    }

    fn key_name(_key: KeyboardKey) -> &'static str {
        "UNKNOWN"
    }
}

#[cfg(not(any(
    all(
        feature = "interactive",
        any(target_os = "linux", target_os = "android")
    ),
    all(target_arch = "wasm32", target_os = "unknown")
)))]
impl InputNames for () {
    fn controller_button_supported(_button: ControllerButton) -> bool {
        true
    }

    fn key_from_name(name: &str) -> Option<KeyboardKey> {
        KeyboardKey::from_profile_name(name)
    }

    fn key_name(_key: KeyboardKey) -> &'static str {
        "UNKNOWN"
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl EventFixturePort for () {
    fn verify_event_source() -> Result<(), RenderError> {
        #[cfg(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        ))]
        return web::input::verify_event_source();
        #[cfg(not(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        )))]
        Err(RenderError::Unsupported)
    }
}

impl IdentificationPort for () {
    fn execution_names() -> (&'static str, &'static str) {
        (ARCH, OS)
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
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        return web::performance::now();
        #[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
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
    fn application_data_path() -> io::Result<OsString> {
        #[cfg(all(target_os = "android", feature = "application-entry"))]
        return android::process::application_data_path();
        #[cfg(not(all(target_os = "android", feature = "application-entry")))]
        Err(ErrorKind::Unsupported.into())
    }

    fn executable_path() -> io::Result<OsString> {
        #[cfg(all(target_os = "android", feature = "application-entry"))]
        return android::process::executable_path();
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        return linux::storage::executable_path().map(PathBuf::into_os_string);
        #[cfg(any(
            target_arch = "wasm32",
            all(target_os = "android", not(feature = "application-entry"))
        ))]
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(not(any(
    target_os = "linux",
    all(target_os = "android", feature = "application-entry"),
    all(target_arch = "wasm32", target_os = "unknown")
)))]
impl ProcessPort for () {
    fn arguments() -> io::Result<Vec<OsString>> {
        Err(ErrorKind::Unsupported.into())
    }

    fn wait_for_retry(_delay: Duration) -> io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn write_diagnostic(_message: StatusMessage<'_>) -> io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn write_status(_message: StatusMessage<'_>) -> io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(all(
    not(target_os = "linux"),
    any(test, feature = "test-fixtures")
))]
impl FixturePort for () {
    fn smoke_requested(_check: NativeSmoke) -> io::Result<bool> {
        Err(ErrorKind::Unsupported.into())
    }

    fn with_worker<Output, Worker, Foreground>(
        _worker: Worker,
        _foreground: Foreground,
    ) -> io::Result<Output>
    where
        Output: Send,
        Worker: FnOnce() -> io::Result<Output> + Send,
        Foreground: FnOnce() -> io::Result<()>,
    {
        Err(ErrorKind::Unsupported.into())
    }

    fn yield_execution() -> io::Result<()> {
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(not(target_os = "linux"))]
impl RetentionBackend for () {
    fn retain(
        _policy: RetentionPolicy,
        _operation: RetentionOperation,
    ) -> Result<RetentionPlan, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(not(target_os = "linux"))]
impl ToolchainFactory for () {
    fn available(_target: Target) -> bool {
        false
    }

    fn bind_browser(
        _args: CargoArguments<'_>,
        _root: &OsStr,
        _parent: &OsStr,
    ) -> Result<BrowserBindings, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
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

    fn package_android(
        _args: CargoArguments<'_>,
        _root: &OsStr,
        _parent: &OsStr,
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

    fn smoke(
        _args: CargoArguments<'_>,
        _root: &OsStr,
        _catalogue: &OsStr,
    ) -> Result<(), ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn workspace_root() -> Result<OsString, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(not(target_os = "linux"))]
impl ValidationBackend for () {
    fn execute_validation(
        _root: &OsStr,
        _args: &Arguments,
    ) -> Result<Completion, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(not(target_os = "linux"))]
impl EnvironmentBackend for () {
    fn environment() -> Result<Environment, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn validation_log_name() -> Result<String, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(not(target_os = "linux"))]
impl StateFactory for () {
    fn open_state(_root: &OsStr) -> io::Result<StatePort> {
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(all(
    not(target_os = "linux"),
    any(test, feature = "test-fixtures")
))]
impl crate::ir::toolchain::validation::fixtures::FixtureBackend for () {
    fn git(
        _root: &crate::ir::storage::fixtures::TestRoot,
        _args: &Arguments,
    ) -> Result<(), ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn hook_syntax(
        _hook: crate::ir::toolchain::validation::fixtures::FixtureHook,
    ) -> Result<bool, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }

    fn run_hook(
        _root: &crate::ir::storage::fixtures::TestRoot,
        _hook: crate::ir::toolchain::validation::fixtures::FixtureHook,
        _failure: crate::ir::toolchain::validation::fixtures::FixtureDelegate,
    ) -> Result<bool, ToolchainError> {
        Err(ToolchainError::UnsupportedHost)
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl SceneFixturePort for () {
    fn verify_gpu_scene() -> Result<(), RenderError> {
        #[cfg(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        ))]
        return web::graphics::verify_webgl_scene();
        #[cfg(not(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        )))]
        Err(RenderError::Unsupported)
    }

    fn verify_resident_scene() -> Result<(), RenderError> {
        #[cfg(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        ))]
        return web::graphics::verify_resident_scene();
        #[cfg(not(all(
            target_arch = "wasm32",
            target_os = "unknown",
            feature = "test-fixtures"
        )))]
        Err(RenderError::Unsupported)
    }
}

impl FrameLoopPort for () {
    fn start(
        step: FrameStep,
        complete: FrameComplete,
    ) -> Result<(), RenderError> {
        #[cfg(all(
            feature = "interactive",
            target_arch = "wasm32",
            target_os = "unknown"
        ))]
        {
            web::frame_loop::start(step, complete)
        }
        #[cfg(not(all(
            feature = "interactive",
            target_arch = "wasm32",
            target_os = "unknown"
        )))]
        {
            drop((step, complete));
            Err(RenderError::Unsupported)
        }
    }
}

#[cfg(not(all(
    feature = "interactive",
    target_arch = "wasm32",
    target_os = "unknown"
)))]
impl DirectoryPicker for () {
    fn select<State, Error>(
        plan: DirectoryPlan,
        prompt: &str,
        choose: DirectorySelector<State, Error>,
        complete: DirectoryComplete<State, Error>,
    ) -> Result<(), BundleError>
    where
        State: 'static,
        Error: 'static,
    {
        drop((plan, prompt, choose, complete));
        Err(BundleError::Io(ErrorKind::Unsupported))
    }
}
