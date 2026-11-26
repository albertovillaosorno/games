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
//   - Native authenticated input composition and fixed-tick admission.
// - Must-Not:
//   - Verify certificates, interpret raw frames or render a second world.
// - Allows:
//   - Inputs: Host-approved seats, TLS settings and an existing simulation.
//   - Outputs: Ordered remote input and safe queue backpressure.
//   - Side effects: Explicit private listener and bounded worker polling.
// - Split-When:
//   - A different client presentation protocol needs its own boundary.
// - Merge-When:
//   - A shared title host covers equivalent seat and tick admission.
// - Summary:
//   - Apply verified LAN controller events to the existing arcade cabinet.
// - Description:
//   - Retry a rejected event before polling another network operation.
// - Usage:
//   - Bind explicitly, construct the reserved cabinet and poll before updates.
// - Defaults:
//   - No implicit network access or account service.

use core::net::SocketAddr;
use std::collections::VecDeque;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::mem;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

use games::network::SessionConfig;
use games::replay::{LiveSimulation, QueueRejection};
use games_platform::ir::lan::pump::{
    HostInputConfig, HostInputPump, PumpProgress, RemoteEvent,
};
use games_platform::ir::lan::{PeerFingerprint, SecureFrameStream, ServerTls};
use games_platform::ir::measurement::now;

use super::Cabinet;
use super::remote::{
    CONTROL_BYTES, HOST_ACK, HostedCabinet, HostedInput, RemoteCodec,
    RemoteControl, RemoteMessage,
};

const POLL_BUDGET: usize = 32;
const CLIENT_QUEUE: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const REPLY_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);

/// Bounded controller channel with authenticated host liveness detection.
#[derive(Debug)]
pub(crate) struct ClientLan {
    idle_timeout: Duration,
    last_received: Duration,
    last_sent: Duration,
    pending: VecDeque<RemoteControl>,
    stream: SecureFrameStream,
}

impl ClientLan {
    fn check_lease(&self, observed_at: Duration) -> IoResult<()> {
        if observed_at.saturating_sub(self.last_received) >= self.idle_timeout {
            return Err(ErrorKind::TimedOut.into());
        }
        Ok(())
    }

    // Admit the complete native transition batch or return it unchanged.
    pub(crate) fn enqueue_batch(
        &mut self,
        values: Vec<RemoteControl>,
    ) -> Result<(), Vec<RemoteControl>> {
        if values.len() > CLIENT_QUEUE.saturating_sub(self.pending.len()) {
            return Err(values);
        }
        self.pending.extend(values);
        Ok(())
    }

    pub(crate) fn new(stream: SecureFrameStream) -> IoResult<Self> {
        Self::with_idle(stream, IDLE_TIMEOUT)
    }

    fn observe_reply(
        &mut self,
        frame: Option<&[u8]>,
        observed_at: Duration,
    ) -> IoResult<()> {
        self.check_lease(observed_at)?;
        if let Some(bytes) = frame {
            if bytes != HOST_ACK {
                return Err(ErrorKind::InvalidData.into());
            }
            self.last_received = observed_at;
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> IoResult<()> {
        self.check_lease(now()?)?;
        // Only a timely, exact versioned acknowledgement renews liveness.
        let reply = self.stream.poll_receive()?;
        self.observe_reply(reply.as_deref(), now()?)?;
        for _step in 0..POLL_BUDGET {
            self.check_lease(now()?)?;
            if !self.stream.has_pending_send() && !self.queue_next()? {
                break;
            }
            self.poll_queued_send(now()?)?;
            if self.stream.has_pending_send() {
                break;
            }
        }
        self.check_lease(now()?)
    }

    fn poll_queued_send(&mut self, observed_at: Duration) -> IoResult<()> {
        self.check_lease(observed_at)?;
        let _progress = self.stream.poll_send()?;
        Ok(())
    }

    fn queue_next(&mut self) -> IoResult<bool> {
        let message = match self.pending.pop_front() {
            Some(value) => RemoteMessage::Control(value),
            None if now()?.saturating_sub(self.last_sent)
                >= HEARTBEAT_INTERVAL =>
            {
                RemoteMessage::Heartbeat
            },
            None => return Ok(false),
        };
        self.stream.queue_send(message.encode().to_vec())?;
        self.last_sent = now()?;
        Ok(true)
    }

    fn with_idle(
        stream: SecureFrameStream,
        idle_timeout: Duration,
    ) -> IoResult<Self> {
        let now = now()?;
        Ok(Self {
            idle_timeout,
            last_received: now,
            last_sent: now,
            pending: VecDeque::new(),
            stream,
        })
    }
}

/// Oldest host-observed control admitted at a fixed simulation tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteAdmission {
    /// Host monotonic timestamp of successful worker decoding.
    pub completed_at: Duration,
    /// Actual unexecuted fixed simulation tick receiving the control.
    pub tick: u64,
}

/// Native input bridge for explicitly hosted free play.
///
/// The host's local screen remains authoritative. Each approved certificate
/// is assigned a cabinet seat by the host. All other seats remain local.
/// Network events target the next unexecuted fixed tick. A saturated
/// simulation queue retains the single rejected event and stops polling
/// until that event can be accepted; releases must never be discarded.
#[derive(Debug)]
pub struct HostLan {
    last_reply: [Option<Duration>; 4],
    pending: Vec<HostedInput>,
    pending_received: Option<Duration>,
    pump: HostInputPump<RemoteMessage>,
    reserved: [bool; 4],
}

impl HostLan {
    /// Bind an explicit LAN endpoint for up to three remote controllers.
    ///
    /// # Errors
    /// Rejects no remote seats, all remote seats, duplicate identities,
    /// unapproved credentials, invalid limits or listener/worker failure.
    pub fn bind(
        address: SocketAddr,
        tls: ServerTls,
        approved: &[Option<PeerFingerprint>; 4],
    ) -> IoResult<Self> {
        Self::bind_with_idle(address, tls, approved, IDLE_TIMEOUT)
    }

    fn bind_with_idle(
        address: SocketAddr,
        tls: ServerTls,
        approved: &[Option<PeerFingerprint>; 4],
        idle_timeout: Duration,
    ) -> IoResult<Self> {
        let reserved = approved.map(|peer| peer.is_some());
        if reserved.into_iter().all(|remote| remote) {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut enrolments = Vec::new();
        let mut local_peers = Vec::new();
        for (peer, approval) in (0u64..).zip(approved.iter().copied()) {
            if let Some(fingerprint) = approval {
                enrolments.push((fingerprint, peer));
            } else {
                local_peers.push(peer);
            }
        }
        let pump = HostInputPump::bind(
            HostInputConfig {
                address,
                enrolments: &enrolments,
                idle_timeout: Some(idle_timeout),
                local_peers: &local_peers,
                session: session_config(),
                timeout: TIMEOUT,
            },
            tls,
            RemoteCodec,
        )?;
        Ok(Self {
            last_reply: [None; 4],
            pending: Vec::new(),
            pending_received: None,
            pump,
            reserved,
        })
    }

    /// Return the address assigned to the private host listener.
    ///
    /// # Errors
    /// Reports a failed native socket inspection.
    pub fn local_addr(&self) -> IoResult<SocketAddr> {
        self.pump.local_addr()
    }

    /// Poll bounded network work before advancing the shared fixed clock.
    ///
    /// Returns the target tick and oldest host decode timestamp among
    /// newly admitted controls, excluding heartbeats and seat lifecycle.
    ///
    /// # Errors
    /// Reports listener/worker failures or an invalid simulation tick.
    /// Capacity rejection retains the event for retry without error.
    pub fn poll(
        &mut self,
        live: &mut LiveSimulation<HostedCabinet>,
    ) -> IoResult<Option<RemoteAdmission>> {
        let tick = live.next_tick();
        let mut oldest = None;
        if !self.retry(live, &mut oldest)? {
            return Ok(None);
        }
        for _step in 0..POLL_BUDGET {
            match self.pump.poll()? {
                PumpProgress::Event(event) => {
                    self.reply(&event)?;
                    self.pending_received = control_timestamp(&event);
                    self.pending.extend(hosted_input(&event));
                },
                PumpProgress::Idle => break,
                PumpProgress::Progress => {},
            }
            if !self.retry(live, &mut oldest)? {
                break;
            }
        }
        Ok(oldest.map(|completed_at| RemoteAdmission { completed_at, tick }))
    }

    fn reply(&mut self, event: &RemoteEvent<RemoteMessage>) -> IoResult<()> {
        let (peer, message) = match *event {
            RemoteEvent::Connected(peer)
            | RemoteEvent::Disconnected { peer, .. } => (peer, false),
            RemoteEvent::Message { peer, .. } => (peer, true),
        };
        let index =
            usize::try_from(peer).map_err(|_error| ErrorKind::InvalidData)?;
        let last = self
            .last_reply
            .get_mut(index)
            .ok_or(ErrorKind::InvalidData)?;
        if !message {
            *last = None;
            return Ok(());
        }
        let now = now()?;
        if last.is_none_or(|previous| {
            now.saturating_sub(previous) >= REPLY_INTERVAL
        }) {
            self.pump.queue_reply(peer, HOST_ACK.to_vec())?;
            *last = Some(now);
        }
        Ok(())
    }

    /// Return the fixed remote seat reservation, in cabinet order.
    #[must_use]
    pub const fn reserved(&self) -> [bool; 4] {
        self.reserved
    }

    fn retry(
        &mut self,
        live: &mut LiveSimulation<HostedCabinet>,
        oldest: &mut Option<Duration>,
    ) -> IoResult<bool> {
        if self.pending.is_empty() {
            return Ok(true);
        }
        match live.enqueue(live.next_tick(), mem::take(&mut self.pending)) {
            Ok(()) => {
                if let Some(received) = self.pending_received.take() {
                    *oldest = Some(
                        oldest.map_or(received, |prior| prior.min(received)),
                    );
                }
                Ok(true)
            },
            Err(rejection) => {
                self.pending = rejection.actions;
                if rejection.reason == QueueRejection::Capacity {
                    Ok(false)
                } else {
                    Err(Error::other(format!("{:?}", rejection.reason)))
                }
            },
        }
    }

    /// Construct the title's hosted cabinet with the bound reservation.
    #[must_use]
    pub fn simulation(&self, cabinet: &Cabinet) -> Option<HostedCabinet> {
        HostedCabinet::new(cabinet, self.reserved)
    }
}

/// The bounded title-specific transport settings shared by host and client.
#[must_use]
pub fn session_config() -> SessionConfig {
    SessionConfig {
        allow_lan: true,
        max_message_bytes: NonZeroUsize::new(CONTROL_BYTES)
            .unwrap_or(NonZeroUsize::MIN),
        max_players: NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
    }
}

const fn control_timestamp(
    event: &RemoteEvent<RemoteMessage>,
) -> Option<Duration> {
    match *event {
        RemoteEvent::Message {
            completed_at,
            value: RemoteMessage::Control(_),
            ..
        } => Some(completed_at),
        RemoteEvent::Connected(_)
        | RemoteEvent::Disconnected { .. }
        | RemoteEvent::Message { .. } => None,
    }
}

const fn hosted_input(
    event: &RemoteEvent<RemoteMessage>,
) -> Option<HostedInput> {
    match *event {
        RemoteEvent::Connected(peer) => Some(HostedInput::Connected(peer)),
        RemoteEvent::Disconnected { peer, reason: _reason } => {
            Some(HostedInput::Disconnected(peer))
        },
        RemoteEvent::Message {
            peer,
            value: RemoteMessage::Control(value),
            ..
        } => Some(HostedInput::Remote { peer, value }),
        RemoteEvent::Message { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use std::slice::from_ref;

    use games::input::LogicalAction;
    use games::replay::LiveInputLimits;
    use games::timing::StepClock;
    use games_platform::ir::lan::identity::LocalIdentity;
    use games_platform::ir::lan::{ClientTls, LanConfig};
    use games_platform::ir::process::fixtures::{with_worker, yield_execution};
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer,
        ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
    };

    use super::*;
    use crate::{
        Action, Control, Participation, Player, StreetLayout, TICK_INTERVAL,
    };

    const HOST: &str = "games.test";

    fn identity(host: bool) -> IoResult<LocalIdentity> {
        let mut authority = CertificateParams::default();
        authority.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        authority.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let root = CertifiedIssuer::self_signed(
            authority,
            KeyPair::generate().map_err(Error::other)?,
        )
        .map_err(Error::other)?;
        let key = KeyPair::generate().map_err(Error::other)?;
        let mut leaf = if host {
            CertificateParams::new(vec![String::from(HOST)])
                .map_err(Error::other)?
        } else {
            CertificateParams::default()
        };
        leaf.extended_key_usages = vec![if host {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        let certificate = leaf.signed_by(&key, &root).map_err(Error::other)?;
        let private = key.serialize_der();
        if host {
            LocalIdentity::host(HOST, root.der(), certificate.der(), &private)
        } else {
            LocalIdentity::client(root.der(), certificate.der(), &private)
        }
    }

    fn host() -> IoResult<(HostLan, ClientTls)> {
        host_with_idle(IDLE_TIMEOUT)
    }

    #[test]
    fn different_credential_approval_cannot_reserve_a_player() -> IoResult<()> {
        let server = identity(true)?;
        let approved = identity(false)?;
        let other = identity(false)?;
        let tls =
            server.server_tls(session_config(), from_ref(approved.public()))?;
        let rejected =
            HostLan::bind(SocketAddr::from(([127, 0, 0, 1], 0)), tls, &[
                None,
                Some(other.public().fingerprint()),
                None,
                None,
            ]);
        if rejected.err().map(|error| error.kind())
            != Some(ErrorKind::PermissionDenied)
        {
            return Err(Error::other(
                "unapproved credential reserved a player",
            ));
        }
        Ok(())
    }

    fn host_with_idle(
        idle_timeout: Duration,
    ) -> IoResult<(HostLan, ClientTls)> {
        let server = identity(true)?;
        let client = identity(false)?;
        let tls =
            server.server_tls(session_config(), from_ref(client.public()))?;
        let listener = HostLan::bind_with_idle(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            tls,
            &[None, Some(client.public().fingerprint()), None, None],
            idle_timeout,
        )?;
        Ok((listener, client.client_tls(server.public())?))
    }

    fn live(host: &HostLan) -> IoResult<LiveSimulation<HostedCabinet>> {
        let cabinet = Cabinet::new(StreetLayout::DOWNTOWN)
            .map_err(|error| Error::other(format!("{error:?}")))?;
        let hosted = host.simulation(&cabinet).ok_or(ErrorKind::InvalidData)?;
        let clock = StepClock::new(TICK_INTERVAL, NonZeroU32::MIN)
            .map_err(|error| Error::other(format!("{error:?}")))?;
        Ok(LiveSimulation::new(hosted, clock, LiveInputLimits {
            future_ticks: 0,
            pending_actions: NonZeroUsize::MIN,
        }))
    }

    fn await_poll<Ready>(
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        ready: Ready,
    ) -> IoResult<()>
    where
        Ready: Fn(&HostLan, &LiveSimulation<HostedCabinet>) -> bool,
    {
        let deadline =
            now()?.checked_add(TIMEOUT).ok_or(ErrorKind::TimedOut)?;
        loop {
            let _admitted = host.poll(live)?;
            if ready(host, live) {
                return Ok(());
            }
            if now()? >= deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            yield_execution()?;
        }
    }

    fn advance(live: &mut LiveSimulation<HostedCabinet>) -> IoResult<()> {
        let _report = live
            .advance(TICK_INTERVAL)
            .map_err(|error| Error::other(format!("{error:?}")))?;
        Ok(())
    }

    fn local(
        live: &mut LiveSimulation<HostedCabinet>,
        control: Control,
    ) -> IoResult<()> {
        live.enqueue(live.next_tick(), vec![HostedInput::Local(
            LogicalAction {
                action: Action {
                    control,
                    player: Player::Marge,
                },
                down: true,
            },
        )])
        .map_err(|error| Error::other(format!("{:?}", error.reason)))
    }

    fn connect(
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        client: &ClientTls,
    ) -> IoResult<SecureFrameStream> {
        let address = host.local_addr()?;
        let config = LanConfig::new(session_config(), TIMEOUT)?;
        with_worker(
            || SecureFrameStream::connect(address, config, client, HOST),
            || {
                await_poll(host, live, |_host, state| {
                    state.pending_actions() == 1
                })?;
                advance(live)
            },
        )
    }

    fn send(
        client: &mut SecureFrameStream,
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        control: Control,
    ) -> IoResult<()> {
        let value =
            RemoteControl::new(control, true).ok_or(ErrorKind::InvalidInput)?;
        client.send(&value.encode())?;
        await_poll(host, live, |_host, state| state.pending_actions() == 1)?;
        advance(live)
    }

    fn require(condition: bool, reason: &'static str) -> IoResult<()> {
        if condition {
            Ok(())
        } else {
            Err(Error::other(reason))
        }
    }

    fn join_and_move(
        client: &mut SecureFrameStream,
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
    ) -> IoResult<()> {
        local(live, Control::Start)?;
        advance(live)?;
        send(client, host, live, Control::Start)?;
        let [marge, homer, bart, lisa] = live.state().state().stations();
        require(
            marge.participation == Participation::Playing
                && homer.participation == Participation::Playing
                && bart.participation == Participation::Absent
                && lisa.participation == Participation::Absent,
            "only the local and approved remote seats joined",
        )?;
        send(client, host, live, Control::Right)?;
        let [local, remote, ..] = live.state().state().stations();
        require(
            remote.position.x > homer.position.x && local == marge,
            "authenticated remote controls only its assigned station",
        )
    }

    fn disconnect(
        client: SecureFrameStream,
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
    ) -> IoResult<()> {
        local(live, Control::Right)?;
        drop(client);
        // Fill the fixed queue before loss arrives. Wait for the host to
        // receive that loss while it cannot yet enqueue its release.
        await_poll(host, live, |bridge, _state| !bridge.pending.is_empty())?;
        require(live.pending_actions() == 1, "local action retained")?;
        advance(live)?;
        let admitted = host.poll(live)?;
        require(
            live.pending_actions() == 1 && admitted.is_none(),
            "disconnect retried without a control timestamp",
        )?;
        advance(live)?;
        let [marge, homer, ..] = live.state().state().stations();
        advance(live)?;
        let [moving, stopped, ..] = live.state().state().stations();
        require(
            moving.position.x > marge.position.x
                && stopped.position == homer.position,
            "remote hold released without stopping local control",
        )
    }

    #[test]
    fn remote_admission_retains_decode_time_through_full_queue() -> IoResult<()>
    {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let mut client = connect(&mut host, &mut live, &credentials)?;
        local(&mut live, Control::Start)?;
        let right = RemoteControl::new(Control::Right, true)
            .ok_or(ErrorKind::InvalidInput)?;
        client.send(&right.encode())?;
        await_poll(&mut host, &mut live, |bridge, _state| {
            bridge.pending_received.is_some()
        })?;
        let decoded = host.pending_received.ok_or(ErrorKind::InvalidData)?;
        require(
            host.poll(&mut live)?.is_none()
                && host.pending_received == Some(decoded),
            "rejected action retains its original worker timestamp",
        )?;
        advance(&mut live)?;
        let tick = live.next_tick();
        require(
            host.poll(&mut live)?
                == Some(RemoteAdmission {
                    completed_at: decoded,
                    tick,
                })
                && host.pending_received.is_none()
                && live.pending_actions() == 1,
            "successful retry reports the actual newly admitted tick",
        )?;
        require(
            host.poll(&mut live)?.is_none(),
            "admitted timestamp is not reported twice",
        )
    }

    #[test]
    fn real_tls_input_moves_and_rejoins_the_reserved_station() -> IoResult<()> {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let mut client = connect(&mut host, &mut live, &credentials)?;
        join_and_move(&mut client, &mut host, &mut live)?;
        disconnect(client, &mut host, &mut live)?;
        let [_, stopped, ..] = live.state().state().stations();
        let mut rejoined = connect(&mut host, &mut live, &credentials)?;
        send(&mut rejoined, &mut host, &mut live, Control::Left)?;
        let [_, reversed, ..] = live.state().state().stations();
        require(
            reversed.position.x < stopped.position.x,
            "same authenticated identity reclaimed its station",
        )
    }

    fn await_client(
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        client: &mut ClientLan,
    ) -> IoResult<()> {
        let deadline =
            now()?.checked_add(TIMEOUT).ok_or(ErrorKind::TimedOut)?;
        loop {
            client.poll()?;
            let _admitted = host.poll(live)?;
            if live.pending_actions() == 1 {
                return advance(live);
            }
            if now()? >= deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            yield_execution()?;
        }
    }

    #[test]
    fn controller_preserves_order_and_rejects_queue_overflow() -> IoResult<()> {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let stream = connect(&mut host, &mut live, &credentials)?;
        let mut client = ClientLan::new(stream)?;
        local(&mut live, Control::Start)?;
        advance(&mut live)?;
        let start = RemoteControl::new(Control::Start, true)
            .ok_or(ErrorKind::InvalidInput)?;
        let right = RemoteControl::new(Control::Right, true)
            .ok_or(ErrorKind::InvalidInput)?;
        let release = RemoteControl::new(Control::Right, false)
            .ok_or(ErrorKind::InvalidInput)?;
        client
            .enqueue_batch(vec![start, right, release])
            .map_err(|_rejected| ErrorKind::WouldBlock)?;
        await_client(&mut host, &mut live, &mut client)?;
        let [_, joined, ..] = live.state().state().stations();
        await_client(&mut host, &mut live, &mut client)?;
        let [_, moved, ..] = live.state().state().stations();
        await_client(&mut host, &mut live, &mut client)?;
        let [_, stopped, ..] = live.state().state().stations();
        require(
            joined.participation == Participation::Playing
                && moved.position.x > joined.position.x
                && stopped.position == moved.position,
            "controller edge ordering",
        )?;
        saturation_and_retry(&mut host, &mut live, &mut client)
    }

    fn saturation_and_retry(
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        client: &mut ClientLan,
    ) -> IoResult<()> {
        let right = RemoteControl::new(Control::Right, true)
            .ok_or(ErrorKind::InvalidInput)?;
        let release = RemoteControl::new(Control::Right, false)
            .ok_or(ErrorKind::InvalidInput)?;
        let left = RemoteControl::new(Control::Left, true)
            .ok_or(ErrorKind::InvalidInput)?;
        client
            .enqueue_batch(vec![right; CLIENT_QUEUE.saturating_sub(1)])
            .map_err(|_rejected| ErrorKind::WouldBlock)?;
        let rejected = client
            .enqueue_batch(vec![release, left])
            .err()
            .ok_or_else(|| Error::other("partial batch admitted"))?;
        require(
            rejected == [release, left]
                && client.pending.len() == CLIENT_QUEUE.saturating_sub(1)
                && client.pending.back() == Some(&right),
            "outgoing queue rejects the complete batch without mutation",
        )?;
        await_client(host, live, client)?;
        client
            .enqueue_batch(rejected)
            .map_err(|_rejected| ErrorKind::WouldBlock)?;
        for _edge in 1..CLIENT_QUEUE.saturating_sub(1) {
            await_client(host, live, client)?;
        }
        let [_, moving, ..] = live.state().state().stations();
        await_client(host, live, client)?;
        let [_, stopped, ..] = live.state().state().stations();
        await_client(host, live, client)?;
        let [_, reversed, ..] = live.state().state().stations();
        require(
            moving.position == stopped.position
                && reversed.position.x < stopped.position.x,
            "rejected release and subsequent opposite edge arrive in order",
        )
    }

    fn maintain_lease(
        host: &mut HostLan,
        live: &mut LiveSimulation<HostedCabinet>,
        client: &mut ClientLan,
    ) -> IoResult<()> {
        let deadline = now()?
            .checked_add(Duration::from_millis(1300))
            .ok_or(ErrorKind::TimedOut)?;
        while now()? < deadline {
            client.poll()?;
            let admitted = host.poll(live)?;
            require(
                live.pending_actions() == 0 && admitted.is_none(),
                "heartbeats refresh the lease without simulation input",
            )?;
            yield_execution()?;
        }
        Ok(())
    }

    #[test]
    fn only_timely_valid_acknowledgements_renew_client_lease() -> IoResult<()> {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let stream = connect(&mut host, &mut live, &credentials)?;
        let timeout = Duration::from_secs(1);
        let mut client = ClientLan::with_idle(stream, timeout)?;
        let original = client.last_received;
        let timely = original
            .checked_add(Duration::from_millis(999))
            .ok_or(ErrorKind::InvalidInput)?;
        client.observe_reply(None, timely)?;
        let malformed = client.observe_reply(Some(&[0]), timely);
        require(
            malformed
                .is_err_and(|error| error.kind() == ErrorKind::InvalidData)
                && client.last_received == original,
            "empty and malformed replies do not renew the lease",
        )?;
        client.observe_reply(Some(&HOST_ACK), timely)?;
        require(client.last_received == timely, "valid reply renews lease")?;
        let deadline =
            timely.checked_add(timeout).ok_or(ErrorKind::InvalidInput)?;
        for observed in [
            deadline,
            deadline
                .checked_add(Duration::from_nanos(1))
                .ok_or(ErrorKind::InvalidInput)?,
        ] {
            let late = client.observe_reply(Some(&HOST_ACK), observed);
            require(
                late.is_err_and(|error| error.kind() == ErrorKind::TimedOut)
                    && client.last_received == timely,
                "expired lease cannot be renewed by an acknowledgement",
            )?;
        }
        Ok(())
    }

    #[test]
    fn expired_client_lease_rejects_queued_control_before_dispatch()
    -> IoResult<()> {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let stream = connect(&mut host, &mut live, &credentials)?;
        let timeout = Duration::from_secs(1);
        let mut client = ClientLan::with_idle(stream, timeout)?;
        let right = RemoteControl::new(Control::Right, true)
            .ok_or(ErrorKind::InvalidInput)?;
        client
            .enqueue_batch(vec![right])
            .map_err(|_rejected| ErrorKind::WouldBlock)?;
        require(client.queue_next()?, "control queued without transport I/O")?;
        let deadline = client
            .last_received
            .checked_add(timeout)
            .ok_or(ErrorKind::InvalidInput)?;
        require(
            client
                .poll_queued_send(deadline)
                .is_err_and(|error| error.kind() == ErrorKind::TimedOut)
                && client.stream.has_pending_send(),
            "expired lease prevents dispatch of the queued control",
        )?;
        drop(client);
        let disconnect_deadline =
            now()?.checked_add(TIMEOUT).ok_or(ErrorKind::TimedOut)?;
        loop {
            require(
                host.poll(&mut live)?.is_none(),
                "expired control never reaches host",
            )?;
            if live.pending_actions() == 1 {
                break;
            }
            if now()? >= disconnect_deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            yield_execution()?;
        }
        advance(&mut live)?;
        require(
            live.state().connected_remotes().get(1) == Some(&false),
            "only the disconnect reaches the simulation",
        )
    }

    #[test]
    fn controller_expires_when_connected_host_stops_polling() -> IoResult<()> {
        let (mut host, credentials) = host()?;
        let mut live = live(&host)?;
        let stream = connect(&mut host, &mut live, &credentials)?;
        let mut client = ClientLan::with_idle(stream, Duration::from_secs(1))?;
        maintain_lease(&mut host, &mut live, &mut client)?;

        // Stop host processing without closing either authenticated endpoint.
        let deadline =
            now()?.checked_add(TIMEOUT).ok_or(ErrorKind::TimedOut)?;
        loop {
            match client.poll() {
                Ok(()) if now()? < deadline => yield_execution()?,
                Err(error) if error.kind() == ErrorKind::TimedOut => break,
                _ => {
                    return Err(Error::other(
                        "controller did not expire on host silence",
                    ));
                },
            }
        }
        require(
            live.state().connected_remotes().get(1) == Some(&true),
            "host still owns the remote seat",
        )?;
        drop(client);
        await_poll(&mut host, &mut live, |_bridge, state| {
            state.pending_actions() == 1
        })?;
        advance(&mut live)?;
        let _rejoined = connect(&mut host, &mut live, &credentials)?;
        require(
            live.state().connected_remotes().get(1) == Some(&true),
            "the same identity can rejoin after host silence",
        )
    }

    #[test]
    fn silent_controller_expires_and_releases_direction() -> IoResult<()> {
        let (mut host, credentials) = host_with_idle(Duration::from_secs(1))?;
        let mut live = live(&host)?;
        let mut stream = connect(&mut host, &mut live, &credentials)?;
        join_and_move(&mut stream, &mut host, &mut live)?;
        let mut client = ClientLan::with_idle(stream, Duration::from_secs(1))?;
        maintain_lease(&mut host, &mut live, &mut client)?;

        // Keep the TCP connection alive locally but stop sending heartbeats.
        let [_, moving, ..] = live.state().state().stations();
        await_poll(&mut host, &mut live, |_bridge, state| {
            state.pending_actions() == 1
        })?;
        advance(&mut live)?;
        advance(&mut live)?;
        let [_, stopped, ..] = live.state().state().stations();
        require(
            stopped.position == moving.position,
            "idle expiry released the direction without a client close",
        )?;
        let mut rejoined = connect(&mut host, &mut live, &credentials)?;
        send(&mut rejoined, &mut host, &mut live, Control::Left)?;
        let [_, reversed, ..] = live.state().state().stations();
        drop(client);
        require(
            reversed.position.x < stopped.position.x,
            "expired identity reclaimed its station",
        )
    }
}
