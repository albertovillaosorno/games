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
//   - Portable client approval, host binding and remote observations.
// - Must-Not:
//   - Acquire sockets, authenticate credentials or read native clocks.
// - Allows:
//   - Inputs: Explicit host endpoint, enrolled and local identities, limits.
//   - Outputs: Admitted requests, rosters and ordered remote observations.
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

use super::{LanAdmissionError, LanLimits, admit_endpoint};
use crate::ir::lan::identity::PublicIdentity;
use crate::ir::lan::pump::{HostInputConfig, RemoteEvent};
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
        let transport = LanConfig::from_limits(LanLimits::new(
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

/// Rejection of a transport observation or reply request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostObservationError {
    Lan(LanAdmissionError),
    UnavailablePeer,
    UnexpectedAddress,
    UnexpectedEvent,
}

#[derive(Debug, Default)]
struct RemoteObservation {
    active: bool,
    last_completed: Option<Duration>,
}

/// Observable remote membership and the admitted binding contract.
pub(crate) struct PumpAdmission {
    address: SocketAddr,
    limits: LanLimits,
    peers: BTreeMap<u64, RemoteObservation>,
}

impl PumpAdmission {
    pub(crate) fn address(
        &self,
        observed: SocketAddr,
    ) -> Result<SocketAddr, HostObservationError> {
        if !matches_binding(self.address, observed) {
            return Err(HostObservationError::UnexpectedAddress);
        }
        Ok(self.address)
    }

    pub(crate) fn admit_address(
        &mut self,
        observed: SocketAddr,
    ) -> Result<(), HostObservationError> {
        if !matches_binding(self.address, observed) {
            return Err(HostObservationError::UnexpectedAddress);
        }
        self.address = observed;
        Ok(())
    }

    pub(crate) fn admit_event<Message>(
        &mut self,
        event: &RemoteEvent<Message>,
    ) -> Result<(), HostObservationError> {
        let peer = match *event {
            RemoteEvent::Connected(peer)
            | RemoteEvent::Disconnected { peer, .. }
            | RemoteEvent::Message { peer, .. } => peer,
        };
        let state = self
            .peers
            .get_mut(&peer)
            .ok_or(HostObservationError::UnexpectedEvent)?;
        match *event {
            RemoteEvent::Connected(_) if !state.active => {
                state.active = true;
            },
            RemoteEvent::Disconnected { .. } if state.active => {
                state.active = false;
            },
            RemoteEvent::Message { completed_at, .. }
                if state.active
                    && state
                        .last_completed
                        .is_none_or(|last| completed_at >= last) =>
            {
                state.last_completed = Some(completed_at);
            },
            RemoteEvent::Connected(_)
            | RemoteEvent::Disconnected { .. }
            | RemoteEvent::Message { .. } => {
                return Err(HostObservationError::UnexpectedEvent);
            },
        }
        Ok(())
    }

    pub(crate) fn admit_reply(
        &self,
        peer: u64,
        length: usize,
    ) -> Result<(), HostObservationError> {
        let _length = self
            .limits
            .outbound_length(length)
            .map_err(HostObservationError::Lan)?;
        if !self.peers.get(&peer).is_some_and(|state| state.active) {
            return Err(HostObservationError::UnavailablePeer);
        }
        Ok(())
    }

    pub(crate) fn new(request: &HostBindRequest) -> Self {
        let peers = request
            .roster
            .enrolled
            .values()
            .copied()
            .map(|peer| (peer, RemoteObservation::default()))
            .collect();
        Self {
            address: request.address(),
            limits: request.transport().limits(),
            peers,
        }
    }
}

fn matches_binding(requested: SocketAddr, observed: SocketAddr) -> bool {
    let same_interface = match (requested, observed) {
        (SocketAddr::V4(left), SocketAddr::V4(right)) => {
            left.ip() == right.ip()
        },
        (SocketAddr::V6(left), SocketAddr::V6(right)) => {
            left.ip() == right.ip() && left.scope_id() == right.scope_id()
        },
        (SocketAddr::V4(_), SocketAddr::V6(_))
        | (SocketAddr::V6(_), SocketAddr::V4(_)) => false,
    };
    same_interface
        && observed.port() != 0
        && (requested.port() == 0 || requested.port() == observed.port())
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
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV6};
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
    fn observed_binding_preserves_ipv6_interface_scope() {
        let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let requested = SocketAddr::V6(SocketAddrV6::new(ip, 0, 0, 3));
        let assigned = SocketAddr::V6(SocketAddrV6::new(ip, 42, 0, 3));
        let wrong_scope = SocketAddr::V6(SocketAddrV6::new(ip, 42, 0, 4));
        let wrong_port = SocketAddr::V6(SocketAddrV6::new(ip, 43, 0, 3));
        assert!(matches_binding(requested, assigned));
        assert!(!matches_binding(requested, wrong_scope));
        assert!(!matches_binding(requested, LOOPBACK));
        assert!(!matches_binding(assigned, wrong_port));
        assert!(!matches_binding(assigned, requested));
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
