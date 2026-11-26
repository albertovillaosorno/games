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
//   - Portable host binding and initial roster admission.
// - Must-Not:
//   - Acquire sockets, authenticate credentials or read native clocks.
// - Allows:
//   - Inputs: Explicit host endpoint, enrolled and local identities, limits.
//   - Outputs: Admitted request and uniquely allocated initial roster.
//   - Side effects: Bounded in-memory allocation for approved identities.
// - Split-When:
//   - A distinct host capability needs another initial roster policy.
// - Merge-When:
//   - Another shared admission contract owns the same host roster.
// - Summary:
//   - Reject invalid host configuration before transport dispatch.
// - Description:
//   - Reserve capacity for remote approvals and distinct local identities.
// - Usage:
//   - Admit a request before passing it to a native host factory.
// - Defaults:
//   - No anonymous peers, duplicate identities or implicit LAN permission.

//! Backend-independent authenticated host binding admission.

use core::net::SocketAddr;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use games::network::{PeerKind, Session, SessionConfig, SessionError};

use super::{LanAdmissionError, admit_endpoint};
use crate::ir::lan::pump::HostInputConfig;
use crate::ir::lan::{LanConfig, PeerFingerprint};

/// Host admission rejection before any native resource access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostAdmissionError {
    /// A fingerprint or player identity has more than one approval.
    DuplicateEnrollment,
    /// No remote identity was explicitly approved for the host.
    EmptyEnrollment,
    /// Transport or endpoint policy rejected the configuration.
    Lan(LanAdmissionError),
    /// A local controller claims a remotely enrolled player identity.
    LocalIdentityReserved,
    /// The shared roster rejected an initial local identity.
    Session(SessionError),
    /// Local identities and remote approvals exceed the session capacity.
    TooManyEnrollments,
}

impl From<LanAdmissionError> for HostAdmissionError {
    fn from(error: LanAdmissionError) -> Self {
        Self::Lan(error)
    }
}

impl From<SessionError> for HostAdmissionError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

/// Map of admitted leaf fingerprints to host-owned player identities.
pub type EnrolledPeers = BTreeMap<PeerFingerprint, u64>;

type PlayerIds = BTreeSet<u64>;
type EnrolmentIndex = (EnrolledPeers, PlayerIds);

/// Initial local membership and unique remote identity approvals.
#[derive(Debug)]
pub struct HostRoster {
    config: SessionConfig,
    enrolled: EnrolledPeers,
    session: Session,
}

impl HostRoster {
    /// Session budgets used for this admitted roster.
    #[must_use]
    pub const fn config(&self) -> SessionConfig {
        self.config
    }

    /// Transfer approved remote identities and active local membership.
    #[must_use]
    pub fn into_parts(self) -> (EnrolledPeers, Session) {
        (self.enrolled, self.session)
    }

    /// Admit the complete initial roster without opening a connection.
    /// # Errors
    /// Rejects disabled remote approval, duplicate identities or excess seats.
    pub fn new(
        config: SessionConfig,
        enrolments: &[(PeerFingerprint, u64)],
        local_peers: &[u64],
    ) -> Result<Self, HostAdmissionError> {
        if !config.allow_lan && !enrolments.is_empty() {
            return Err(SessionError::LanDisabled.into());
        }
        let count = enrolments.len().checked_add(local_peers.len());
        if !count.is_some_and(|combined| config.admits_players(combined)) {
            return Err(HostAdmissionError::TooManyEnrollments);
        }
        let (enrolled, remote) = enrolment_map(enrolments)?;
        let session = local_session(config, local_peers, &remote)?;
        Ok(Self {
            config,
            enrolled,
            session,
        })
    }
}

/// A host bind request admitted before selection of a native transport.
#[derive(Debug)]
pub struct HostBindRequest {
    address: SocketAddr,
    idle_timeout: Option<Duration>,
    roster: HostRoster,
    transport: LanConfig,
}

impl HostBindRequest {
    /// Explicitly admitted private listening endpoint.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Optional nonzero inbound lease duration.
    #[must_use]
    pub const fn idle_timeout(&self) -> Option<Duration> {
        self.idle_timeout
    }

    /// Transfer the admitted roster to the host worker.
    #[must_use]
    pub fn into_roster(self) -> HostRoster {
        self.roster
    }

    /// Admit all transport-independent host bind requirements.
    /// # Errors
    /// Rejects disabled LAN, invalid endpoints, limits and roster approvals.
    pub fn new(
        config: HostInputConfig<'_>,
    ) -> Result<Self, HostAdmissionError> {
        admit_endpoint(config.address)?;
        let transport = LanConfig::from_limits(super::LanLimits::new(
            config.session,
            config.timeout,
        )?);
        if config.idle_timeout.is_some_and(|limit| limit.is_zero()) {
            return Err(LanAdmissionError::ZeroTimeout.into());
        }
        if config.enrolments.is_empty() {
            return Err(HostAdmissionError::EmptyEnrollment);
        }
        let roster = HostRoster::new(
            config.session,
            config.enrolments,
            config.local_peers,
        )?;
        Ok(Self {
            address: config.address,
            idle_timeout: config.idle_timeout,
            roster,
            transport,
        })
    }

    /// Wire and connection limits admitted with the roster.
    #[must_use]
    pub const fn transport(&self) -> LanConfig {
        self.transport
    }
}

fn enrolment_map(
    enrolments: &[(PeerFingerprint, u64)],
) -> Result<EnrolmentIndex, HostAdmissionError> {
    let mut enrolled = BTreeMap::new();
    let mut remote = BTreeSet::new();
    for &(fingerprint, peer) in enrolments {
        if enrolled.insert(fingerprint, peer).is_some() || !remote.insert(peer)
        {
            return Err(HostAdmissionError::DuplicateEnrollment);
        }
    }
    Ok((enrolled, remote))
}

fn local_session(
    config: SessionConfig,
    local_peers: &[u64],
    remote: &PlayerIds,
) -> Result<Session, HostAdmissionError> {
    let mut session = Session::new(config);
    for &peer in local_peers {
        if remote.contains(&peer) {
            return Err(HostAdmissionError::LocalIdentityReserved);
        }
        session.join(peer, PeerKind::Local)?;
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::*;

    const FIRST: PeerFingerprint = PeerFingerprint::from_bytes([1; 32]);
    const SECOND: PeerFingerprint = PeerFingerprint::from_bytes([2; 32]);
    const TIMEOUT: Duration = Duration::from_secs(1);
    const LOOPBACK: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    type RosterCase<'roster> = (
        &'roster [(PeerFingerprint, u64)],
        &'roster [u64],
        HostAdmissionError,
    );

    fn config() -> SessionConfig {
        SessionConfig {
            allow_lan: true,
            max_message_bytes: NonZeroUsize::new(8)
                .unwrap_or(NonZeroUsize::MIN),
            max_players: NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
        }
    }

    fn request(enrolments: &[(PeerFingerprint, u64)]) -> HostInputConfig<'_> {
        HostInputConfig {
            address: LOOPBACK,
            enrolments,
            idle_timeout: Some(TIMEOUT),
            local_peers: &[0],
            session: config(),
            timeout: TIMEOUT,
        }
    }

    #[test]
    fn admission_owns_only_active_local_membership_and_remote_approvals()
    -> Result<(), String> {
        let admitted =
            HostBindRequest::new(request(&[(FIRST, 1), (SECOND, 2)]))
                .map_err(|error| format!("{error:?}"))?;
        if admitted.address() != LOOPBACK
            || admitted.idle_timeout() != Some(TIMEOUT)
            || admitted.transport().limits().max_bytes() != 8
        {
            return Err(String::from("admitted request changed its limits"));
        }
        let roster = admitted.into_roster();
        if roster.config() != config() {
            return Err(String::from("admitted roster changed its settings"));
        }
        let (enrolled, mut session) = roster.into_parts();
        if enrolled.get(&FIRST) != Some(&1)
            || session.accept(0, b"local") != Ok(())
            || session.accept(1, b"remote") != Err(SessionError::UnknownPeer)
            || session.join(1, PeerKind::Lan).is_err()
            || session.join(2, PeerKind::Lan).is_err()
        {
            return Err(String::from(
                "initial roster membership is inconsistent",
            ));
        }
        Ok(())
    }

    #[test]
    fn rejects_duplicate_and_overlapping_initial_identities() {
        let cases: [RosterCase<'_>; 5] = [
            (
                &[(FIRST, 1), (FIRST, 2)],
                &[],
                HostAdmissionError::DuplicateEnrollment,
            ),
            (
                &[(FIRST, 1), (SECOND, 1)],
                &[],
                HostAdmissionError::DuplicateEnrollment,
            ),
            (
                &[(FIRST, 1)],
                &[1],
                HostAdmissionError::LocalIdentityReserved,
            ),
            (
                &[(FIRST, 1)],
                &[0, 0],
                HostAdmissionError::Session(SessionError::AlreadyConnected),
            ),
            (
                &[(FIRST, 1), (SECOND, 2)],
                &[0, 3],
                HostAdmissionError::TooManyEnrollments,
            ),
        ];
        for (enrolments, local_peers, expected) in cases {
            assert_eq!(
                HostRoster::new(config(), enrolments, local_peers).err(),
                Some(expected),
            );
        }
    }

    #[test]
    fn host_request_rejects_unapproved_or_disabled_lan() {
        assert_eq!(
            HostBindRequest::new(request(&[])).err(),
            Some(HostAdmissionError::EmptyEnrollment),
        );
        let mut disabled = request(&[(FIRST, 1)]);
        disabled.session.allow_lan = false;
        assert_eq!(
            HostBindRequest::new(disabled).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::Disabled)),
        );
    }

    #[test]
    fn host_request_rejects_zero_transfer_or_idle_deadlines() {
        let mut invalid = request(&[(FIRST, 1)]);
        invalid.timeout = Duration::ZERO;
        assert_eq!(
            HostBindRequest::new(invalid).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::ZeroTimeout)),
        );
        invalid.timeout = TIMEOUT;
        invalid.idle_timeout = Some(Duration::ZERO);
        assert_eq!(
            HostBindRequest::new(invalid).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::ZeroTimeout)),
        );
    }

    #[test]
    fn host_request_rejects_wildcard_binding() {
        let mut invalid = request(&[(FIRST, 1)]);
        invalid.address.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(
            HostBindRequest::new(invalid).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::PublicEndpoint)),
        );
    }
}
