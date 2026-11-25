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
//   - Portable execution environment descriptors and normalization.
// - Must-Not:
//   - Probe physical hardware or assume capabilities from identity.
// - Allows:
//   - Inputs: Selected adapter observations of the executing program.
//   - Outputs: Typed system and process architecture descriptors.
//   - Side effects: Read-only adapter dispatch.
// - Split-When:
//   - Physical device identification needs independent admission.
// - Merge-When:
//   - An existing contract owns the same execution observations.
// - Summary:
//   - Distinguish execution identity from a requested build target.
// - Description:
//   - Recognize supported descriptors and preserve unknown environments.
// - Usage:
//   - Report execution identity; negotiate actual capabilities separately.
// - Defaults:
//   - Unknown architecture or system implies no capability guarantee.

//! Portable identification of the currently executing program.

/// Architecture of the current program, not necessarily the physical CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    /// 64-bit ARM execution.
    Arm64,
    /// The observation is outside the recognized architecture vocabulary.
    Unknown,
    /// 32-bit WebAssembly execution.
    Wasm32,
    /// 64-bit x86 execution.
    X86_64,
}

/// Operating system targeted by the executing program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatingSystem {
    /// Android.
    Android,
    /// FreeBSD.
    FreeBsd,
    /// iOS.
    Ios,
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
    /// No recognized operating system, including unspecified WebAssembly.
    Unknown,
    /// Windows.
    Windows,
}

/// Execution identity without native handles or physical device claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionIdentity {
    /// Architecture of the executing program.
    pub architecture: Architecture,
    /// Operating system targeted by the executing program.
    pub system: OperatingSystem,
}

/// Private observation source selected by platform composition.
pub(crate) trait IdentificationPort {
    fn execution_names() -> (&'static str, &'static str);
}

/// Identify the currently executing program.
///
/// This reports its execution target, not the physical CPU, browser host,
/// installed toolchains or supported devices. Those require their respective
/// capability observations.
#[must_use]
pub fn execution() -> ExecutionIdentity {
    execution_with::<()>()
}

fn execution_with<Port>() -> ExecutionIdentity
where
    Port: IdentificationPort,
{
    let (architecture, system) = Port::execution_names();
    normalize(architecture, system)
}

fn normalize(architecture: &str, system: &str) -> ExecutionIdentity {
    ExecutionIdentity {
        architecture: match architecture {
            "aarch64" => Architecture::Arm64,
            "wasm32" => Architecture::Wasm32,
            "x86_64" => Architecture::X86_64,
            _ => Architecture::Unknown,
        },
        system: match system {
            "android" => OperatingSystem::Android,
            "freebsd" => OperatingSystem::FreeBsd,
            "ios" => OperatingSystem::Ios,
            "linux" => OperatingSystem::Linux,
            "macos" => OperatingSystem::MacOs,
            "windows" => OperatingSystem::Windows,
            _ => OperatingSystem::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Browser;

    impl IdentificationPort for Browser {
        fn execution_names() -> (&'static str, &'static str) {
            ("wasm32", "unknown")
        }
    }

    #[test]
    fn architecture_and_system_are_independent_observations() {
        for (architecture, system, expected_arch, expected_os) in [
            (
                "aarch64",
                "linux",
                Architecture::Arm64,
                OperatingSystem::Linux,
            ),
            (
                "x86_64",
                "linux",
                Architecture::X86_64,
                OperatingSystem::Linux,
            ),
            (
                "aarch64",
                "android",
                Architecture::Arm64,
                OperatingSystem::Android,
            ),
            (
                "aarch64",
                "windows",
                Architecture::Arm64,
                OperatingSystem::Windows,
            ),
            (
                "x86_64",
                "windows",
                Architecture::X86_64,
                OperatingSystem::Windows,
            ),
            (
                "unexpected",
                "linux",
                Architecture::Unknown,
                OperatingSystem::Linux,
            ),
            (
                "aarch64",
                "unexpected",
                Architecture::Arm64,
                OperatingSystem::Unknown,
            ),
        ] {
            assert_eq!(normalize(architecture, system), ExecutionIdentity {
                architecture: expected_arch,
                system: expected_os,
            });
        }
    }

    #[test]
    fn browser_target_does_not_infer_a_host_system() {
        assert_eq!(execution_with::<Browser>(), ExecutionIdentity {
            architecture: Architecture::Wasm32,
            system: OperatingSystem::Unknown,
        });
    }
}
