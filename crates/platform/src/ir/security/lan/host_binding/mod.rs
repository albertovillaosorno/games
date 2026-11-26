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
//   - Portable client approval, host binding and initial roster admission.
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
//   - Admit credential approvals and the roster before native dispatch.
// - Defaults:
//   - No anonymous peers, duplicate identities or implicit LAN permission.

//! Backend-independent authenticated host binding admission.

use core::net::SocketAddr;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use games::network::{PeerKind, Session, SessionConfig, SessionError};

use super::{LanAdmissionError, admit_endpoint};
use crate::ir::lan::identity::PublicIdentity;
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
    /// A requested peer is not in the credential approval list.
    UnapprovedIdentity,
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

/// Bounded, unique client approvals admitted before TLS configuration.
#[derive(Debug)]
pub struct ClientApprovals<'clients> {
    clients: &'clients [PublicIdentity],
    pins: BTreeSet<PeerFingerprint>,
}

impl<'clients> ClientApprovals<'clients> {
    /// Explicitly approved public identities and trust roots.
    #[must_use]
    pub const fn clients(&self) -> &'clients [PublicIdentity] {
        self.clients
    }

    /// Transfer the exact leaf approvals to the opaque TLS handle.
    #[must_use]
    pub fn into_pins(self) -> BTreeSet<PeerFingerprint> {
        self.pins
    }

    /// Admit a nonempty unique list within the selected session budget.
    /// # Errors
    /// Rejects disabled LAN, empty, excessive or duplicate approvals.
    pub fn new(
        config: SessionConfig,
        clients: &'clients [PublicIdentity],
    ) -> Result<Self, HostAdmissionError> {
        if !config.allow_lan {
            return Err(LanAdmissionError::Disabled.into());
        }
        if clients.is_empty() {
            return Err(HostAdmissionError::EmptyEnrollment);
        }
        if !config.admits_players(clients.len()) {
            return Err(HostAdmissionError::TooManyEnrollments);
        }
        let mut pins = BTreeSet::new();
        for client in clients {
            if !pins.insert(client.fingerprint()) {
                return Err(HostAdmissionError::DuplicateEnrollment);
            }
        }
        Ok(Self { clients, pins })
    }
}

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
    /// Every enrolled remote leaf must have an explicit credential approval.
    /// A subset may be selected without granting additional identities.
    /// # Errors
    /// Rejects disabled LAN, invalid endpoints, limits and roster approvals.
    pub fn new(
        config: HostInputConfig<'_>,
        approved: &BTreeSet<PeerFingerprint>,
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
        if !roster.enrolled.keys().all(|pin| approved.contains(pin)) {
            return Err(HostAdmissionError::UnapprovedIdentity);
        }
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

    fn approved() -> BTreeSet<PeerFingerprint> {
        BTreeSet::from([FIRST, SECOND])
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
        let admitted = HostBindRequest::new(
            request(&[(FIRST, 1), (SECOND, 2)]),
            &approved(),
        )
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
            HostBindRequest::new(request(&[]), &approved()).err(),
            Some(HostAdmissionError::EmptyEnrollment),
        );
        let mut disabled = request(&[(FIRST, 1)]);
        disabled.session.allow_lan = false;
        assert_eq!(
            HostBindRequest::new(disabled, &approved()).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::Disabled)),
        );
    }

    #[test]
    fn host_request_rejects_zero_transfer_or_idle_deadlines() {
        let mut invalid = request(&[(FIRST, 1)]);
        invalid.timeout = Duration::ZERO;
        assert_eq!(
            HostBindRequest::new(invalid, &approved()).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::ZeroTimeout)),
        );
        invalid.timeout = TIMEOUT;
        invalid.idle_timeout = Some(Duration::ZERO);
        assert_eq!(
            HostBindRequest::new(invalid, &approved()).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::ZeroTimeout)),
        );
    }

    #[test]
    fn host_request_cannot_enrol_a_leaf_without_credential_approval() {
        let approved = BTreeSet::from([FIRST]);
        assert_eq!(
            HostBindRequest::new(request(&[(SECOND, 1)]), &approved).err(),
            Some(HostAdmissionError::UnapprovedIdentity)
        );
        assert!(
            HostBindRequest::new(request(&[(FIRST, 1)]), &approved).is_ok()
        );
    }

    #[test]
    fn host_request_accepts_a_subset_of_the_approved_client_pins() {
        let accepted =
            HostBindRequest::new(request(&[(FIRST, 1)]), &approved());
        assert!(accepted.is_ok());
    }

    #[test]
    fn host_request_rejects_wildcard_binding() {
        let mut invalid = request(&[(FIRST, 1)]);
        invalid.address.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(
            HostBindRequest::new(invalid, &approved()).err(),
            Some(HostAdmissionError::Lan(LanAdmissionError::PublicEndpoint)),
        );
    }
}
