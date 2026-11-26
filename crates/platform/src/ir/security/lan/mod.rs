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
//   - Neutral LAN permission, address and wire-size admission.
// - Must-Not:
//   - Open sockets, read clocks, authenticate peers or decode game actions.
// - Allows:
//   - Inputs: Session limits, timeout, endpoint and frame lengths.
//   - Outputs: Admitted bounded values or explicit policy failures.
//   - Side effects: None.
// - Split-When:
//   - Another transport has independent admission semantics.
// - Merge-When:
//   - The shared session already enforces the same wire-level contract.
// - Summary:
//   - Shared mandatory checks for opted-in private LAN transports.
// - Description:
//   - Enforces permission and frame budgets before native transport access.
// - Usage:
//   - Admit configuration and endpoints before creating a connection.
// - Defaults:
//   - Reject public endpoints and unrepresentable or excessive lengths.

//! Host-independent security admission for private LAN transport.

/// Incremental bounded framing over injected byte operations.
pub mod framing;

use core::net::{IpAddr, SocketAddr};
use std::num::NonZeroU64;
use std::time::Duration;

use games::network::SessionConfig;

/// Rejection by the transport-independent admission policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LanAdmissionError {
    /// The session did not explicitly permit local network clients.
    Disabled,
    /// The session frame budget cannot be represented in the wire format.
    FrameBudget,
    /// The inbound frame exceeds the admitted frame budget.
    InboundLength,
    /// The outbound frame exceeds the admitted frame budget.
    OutboundLength,
    /// The endpoint is not an explicitly private or loopback address.
    PublicEndpoint,
    /// The timeout must be nonzero.
    ZeroTimeout,
}

/// Bound failure before decoding a locally provisioned DER credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialAdmissionError {
    /// The credential has no encoded contents.
    Empty,
    /// The encoded credential exceeds its admitted byte budget.
    TooLarge,
}

/// Credential class with a shared byte budget for storage and transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialKind {
    /// Public root or leaf certificate.
    Certificate,
    /// Private signing key.
    PrivateKey,
}

impl CredentialKind {
    /// Reject empty or oversized DER before backend parsing or copying.
    /// This does not authenticate or structurally decode the contents.
    /// # Errors
    /// Returns the corresponding credential-size rejection.
    pub fn admit(self, der: &[u8]) -> Result<(), CredentialAdmissionError> {
        if der.is_empty() {
            return Err(CredentialAdmissionError::Empty);
        }
        let size = u64::try_from(der.len())
            .map_err(|_error| CredentialAdmissionError::TooLarge)?;
        super::file::admit_size(size, self.max_bytes().get())
            .map_err(|_error| CredentialAdmissionError::TooLarge)
    }

    /// Maximum DER byte count for this credential class.
    #[must_use]
    pub const fn max_bytes(self) -> NonZeroU64 {
        match self {
            Self::Certificate => NonZeroU64::MIN.saturating_add(4095),
            Self::PrivateKey => NonZeroU64::MIN.saturating_add(8191),
        }
    }
}

/// Admitted LAN transport limits inherited from the shared session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanLimits {
    max_bytes: u32,
    max_pending: u32,
    timeout: Duration,
}

impl LanLimits {
    /// Admit a wire payload length before native reads or allocations.
    ///
    /// # Errors
    /// Rejects payload lengths above the configured or addressable limit.
    pub fn inbound_length(
        self,
        length: u32,
    ) -> Result<usize, LanAdmissionError> {
        if length > self.max_bytes {
            return Err(LanAdmissionError::InboundLength);
        }
        usize::try_from(length)
            .map_err(|_error| LanAdmissionError::InboundLength)
    }

    /// Maximum permitted wire payload.
    #[must_use]
    pub const fn max_bytes(self) -> u32 {
        self.max_bytes
    }

    /// Maximum outstanding peer connections.
    #[must_use]
    pub const fn max_pending(self) -> u32 {
        self.max_pending
    }

    /// Admit the configuration before accessing a native transport.
    ///
    /// # Errors
    /// Rejects disabled LAN, a zero timeout or a nonrepresentable wire budget.
    /// The backend must also check that its own clock can represent the
    /// requested deadline before acquiring native resources.
    pub fn new(
        session: SessionConfig,
        timeout: Duration,
    ) -> Result<Self, LanAdmissionError> {
        if !session.allow_lan {
            return Err(LanAdmissionError::Disabled);
        }
        if timeout.is_zero() {
            return Err(LanAdmissionError::ZeroTimeout);
        }
        let max_bytes = u32::try_from(session.max_message_bytes.get())
            .map_err(|_error| LanAdmissionError::FrameBudget)?;
        Ok(Self {
            max_bytes,
            max_pending: session.max_players.get(),
            timeout,
        })
    }

    /// Admit an outbound payload length before encoding a frame.
    ///
    /// # Errors
    /// Rejects payloads exceeding the admitted or representable wire size.
    pub fn outbound_length(
        self,
        length: usize,
    ) -> Result<u32, LanAdmissionError> {
        let count = u32::try_from(length)
            .map_err(|_error| LanAdmissionError::OutboundLength)?;
        if count > self.max_bytes {
            return Err(LanAdmissionError::OutboundLength);
        }
        Ok(count)
    }

    /// Maximum duration allowed for the whole transfer.
    #[must_use]
    pub const fn timeout(self) -> Duration {
        self.timeout
    }
}

/// Admit only loopback, private and link-local unicast endpoints.
///
/// IPv4-mapped IPv6 addresses receive the same policy as native IPv4.
///
/// # Errors
/// Rejects public, wildcard, multicast and other non-private endpoints.
pub const fn admit_endpoint(
    address: SocketAddr,
) -> Result<(), LanAdmissionError> {
    let admitted = match address.ip().to_canonical() {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local()
        },
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        },
    };
    if admitted {
        Ok(())
    } else {
        Err(LanAdmissionError::PublicEndpoint)
    }
}

#[cfg(test)]
mod tests {
    use core::net::SocketAddr;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::time::Duration;

    use games::network::SessionConfig;

    use super::{LanAdmissionError, LanLimits, admit_endpoint};

    fn config(allow_lan: bool) -> SessionConfig {
        SessionConfig {
            allow_lan,
            max_message_bytes: NonZeroUsize::new(8)
                .unwrap_or(NonZeroUsize::MIN),
            max_players: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        }
    }

    #[test]
    fn config_rejects_disabled_sessions_and_unrepresentable_limits()
    -> Result<(), String> {
        let timeout = Duration::from_secs(2);
        if LanLimits::new(config(false), timeout)
            != Err(LanAdmissionError::Disabled)
            || LanLimits::new(config(true), Duration::ZERO)
                != Err(LanAdmissionError::ZeroTimeout)
        {
            return Err(String::from("invalid configuration was admitted"));
        }
        if let Some(max_message_bytes) =
            usize::try_from(u64::from(u32::MAX).saturating_add(1))
                .ok()
                .and_then(NonZeroUsize::new)
        {
            let limits = SessionConfig {
                max_message_bytes,
                ..config(true)
            };
            if LanLimits::new(limits, timeout)
                != Err(LanAdmissionError::FrameBudget)
            {
                return Err(String::from("unrepresentable wire budget"));
            }
        }
        Ok(())
    }

    #[test]
    fn pending_capacity_preserves_wide_budget() -> Result<(), String> {
        for count in [NonZeroU32::MIN.saturating_add(255), NonZeroU32::MAX] {
            let session = SessionConfig {
                max_players: count,
                ..config(true)
            };
            let limits = LanLimits::new(session, Duration::from_secs(2))
                .map_err(|error| format!("{error:?}"))?;
            if limits.max_pending() != count.get() {
                return Err(String::from("pending roster budget was narrowed"));
            }
        }
        Ok(())
    }

    #[test]
    fn exact_frame_bounds_are_shared_by_both_directions() -> Result<(), String>
    {
        let timeout = Duration::from_secs(2);
        let limits = LanLimits::new(config(true), timeout)
            .map_err(|error| format!("{error:?}"))?;
        if limits.max_bytes() == 8
            && limits.max_pending() == 2
            && limits.timeout() == timeout
            && limits.inbound_length(0) == Ok(0)
            && limits.inbound_length(8) == Ok(8)
            && limits.inbound_length(9) == Err(LanAdmissionError::InboundLength)
            && limits.outbound_length(0) == Ok(0)
            && limits.outbound_length(8) == Ok(8)
            && limits.outbound_length(9)
                == Err(LanAdmissionError::OutboundLength)
        {
            Ok(())
        } else {
            Err(String::from("inconsistent wire length admission"))
        }
    }

    #[test]
    fn private_address_policy_canonicalizes_and_rejects_public_ranges()
    -> Result<(), String> {
        for (address, expected) in [
            ("127.0.0.1:1", true),
            ("10.0.0.1:1", true),
            ("172.15.255.255:1", false),
            ("172.16.0.0:1", true),
            ("172.31.255.255:1", true),
            ("172.32.0.0:1", false),
            ("192.168.0.1:1", true),
            ("169.254.0.1:1", true),
            ("100.64.0.1:1", false),
            ("[::1]:1", true),
            ("[::]:1", false),
            ("[::127.0.0.1]:1", false),
            ("[fc00::1]:1", true),
            ("[fdff::1]:1", true),
            ("[fe80::1]:1", true),
            ("[febf::1]:1", true),
            ("[fec0::1]:1", false),
            ("[2001:db8::1]:1", false),
            ("[ff02::1]:1", false),
            ("[::ffff:127.0.0.1]:1", true),
            ("[::ffff:192.168.0.1]:1", true),
            ("[::ffff:8.8.8.8]:1", false),
            ("[::ffff:0.0.0.0]:1", false),
            ("[::ffff:100.64.0.1]:1", false),
        ] {
            let endpoint = address
                .parse::<SocketAddr>()
                .map_err(|error| error.to_string())?;
            if admit_endpoint(endpoint).is_ok() != expected {
                return Err(format!("incorrect endpoint admission: {address}"));
            }
        }
        Ok(())
    }
}
