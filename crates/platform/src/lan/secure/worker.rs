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
//   - Bounded native host I/O work and completion delivery.
// - Must-Not:
//   - Implement title schemas, simulation or a second roster policy.
// - Allows:
//   - Inputs: Authenticated session, bounded command queue and codec.
//   - Outputs: Correlated operation results and explicit backpressure.
//   - Side effects: Owns and joins a worker thread performing socket I/O.
// - Split-When:
//   - Concurrent per-peer I/O requires independent channel scheduling.
// - Merge-When:
//   - Another native worker provides equivalent host operations.
// - Summary:
//   - Keep synchronous host transfers away from simulation updates.
// - Description:
//   - Serialises operations without blocking submission or polling.
// - Usage:
//   - Poll handshakes and receives; poll correlated completions.
// - Defaults:
//   - No implicit LAN listener, polling receives or unbounded queue.

//! Caller-driven bounded host I/O independent of the simulation thread.

use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    Receiver, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::thread::{Builder, JoinHandle};

use games::network::MessageSchema;

use super::host::{HostError, HostSession};
use super::{PeerFingerprint, PendingConnection};

const MAX_QUEUE_CAPACITY: usize = 1024;

/// One locally requested host operation.
#[derive(Debug)]
pub enum HostOperation {
    /// Authenticate the admitted pending socket and join its approved slot.
    Authenticate(Box<PendingConnection>),
    /// Disconnect an active identity, reserving its slot.
    Disconnect(u64),
    /// Join a directly connected local controller.
    JoinLocal(u64),
    /// Advance a pending handshake without blocking on an untrusted socket.
    PollAuthenticate(Box<PendingConnection>),
    /// Poll one frame without blocking on an idle or incomplete peer.
    PollReceive(u64),
    /// Receive one protocol-expected frame; an idle timeout disconnects.
    Receive(u64),
    /// Reconnect a previously reserved local controller.
    ReconnectLocal(u64),
    /// Remove an active or reserved slot.
    Remove(u64),
    /// Revoke a locally approved certificate.
    Revoke(PeerFingerprint),
    /// Send an application payload to an active LAN player.
    Send {
        /// Host-approved player identity.
        peer: u64,
        /// Payload checked against the session's frame budget.
        bytes: Vec<u8>,
    },
}

/// A command paired with a caller-owned correlation identifier.
#[derive(Debug)]
pub struct HostRequest {
    /// Identifier echoed in the corresponding completion.
    pub id: u64,
    /// Work to perform on the I/O thread.
    pub operation: HostOperation,
}

/// Successful host operation result.
#[derive(Debug)]
pub enum HostOutcome<Message> {
    /// Verified peer admitted or reconnected.
    Admitted(u64),
    /// Roster or transport operation completed without a value.
    Applied,
    /// Resubmit this pending handshake or drop it to release its permit.
    Authenticating(Box<PendingConnection>),
    /// Application message passed the host's schema.
    Message {
        /// Identity determined by the host, not the received payload.
        peer: u64,
        /// Title-defined decoded message.
        value: Message,
    },
    /// No complete frame available on the polled peer.
    Pending,
}

/// Failure of an operation performed by the worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationError {
    /// Mutual TLS authentication failed.
    Authentication(ErrorKind),
    /// The host roster, transfer or schema rejected the operation.
    Host(HostError),
}

impl From<HostError> for OperationError {
    fn from(error: HostError) -> Self {
        Self::Host(error)
    }
}

/// Result of a worker operation.
pub type HostResult<Message> = Result<HostOutcome<Message>, OperationError>;

/// One correlated completion, including failures.
#[derive(Debug)]
pub struct HostCompletion<Message> {
    /// Identifier supplied with the original command.
    pub id: u64,
    /// Operation success or failure.
    pub result: HostResult<Message>,
}

/// Reason a request was not admitted to the bounded queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    /// The worker is no longer accepting requests.
    Closed,
    /// The request queue has reached its configured capacity.
    Full,
    /// The outgoing payload exceeds the host's frame budget.
    Oversized,
}

/// A rejected command returned unchanged to the caller.
#[derive(Debug)]
pub struct RejectedRequest {
    /// Why the command was not enqueued.
    pub reason: RejectionReason,
    /// Original command, including ownership of any pending connection.
    pub request: HostRequest,
}

#[derive(Debug)]
struct WorkerChannels<Message> {
    incoming: Receiver<HostRequest>,
    outgoing: SyncSender<HostCompletion<Message>>,
    stopped: Arc<AtomicBool>,
}

#[derive(Debug)]
struct Channels<Message> {
    requests: SyncSender<HostRequest>,
    results: Receiver<HostCompletion<Message>>,
}

/// Owns the queues and lifetime of one native host I/O thread.
///
/// Only the worker performs authentication, schema decoding and transfers.
/// Requests and completions each have the supplied finite capacity. One
/// operation is in flight at a time. A trusted title codec must be bounded
/// and nonblocking. Use `PollAuthenticate` and `PollReceive` to prevent
/// quiet sockets from delaying other requests; blocking operations remain
/// available for protocol-expected transfers.
///
/// Shutdown cancels queued operations and joins the worker. It may wait for
/// an in-flight bounded transfer; do not shut down on the rendering path.
#[derive(Debug)]
pub struct HostWorker<Message> {
    channels: Option<Channels<Message>>,
    frame_budget: usize,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl<Message: Send + 'static> HostWorker<Message> {
    /// Poll for one completion without waiting for the worker.
    ///
    /// # Errors
    /// Reports a closed worker after all deliverable completions are read.
    pub fn poll(&self) -> io::Result<Option<HostCompletion<Message>>> {
        let channels = self.channels.as_ref().ok_or(ErrorKind::BrokenPipe)?;
        match channels.results.try_recv() {
            Ok(completion) => Ok(Some(completion)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err(ErrorKind::BrokenPipe.into())
            },
        }
    }

    /// Cancel queued work and join the native I/O thread.
    ///
    /// # Errors
    /// Reports a worker panic. An active transfer uses its existing deadline.
    pub fn shutdown(mut self) -> io::Result<()> {
        self.stop()
    }

    /// Spawn a worker owning the existing session and trusted title codec.
    ///
    /// # Errors
    /// Rejects capacities above 1024 and reports thread creation failure.
    pub fn spawn<Codec>(
        session: HostSession,
        codec: Codec,
        capacity: NonZeroUsize,
    ) -> io::Result<Self>
    where
        Codec: MessageSchema<Message = Message> + Send + 'static,
    {
        if capacity.get() > MAX_QUEUE_CAPACITY {
            return Err(ErrorKind::InvalidInput.into());
        }
        let frame_budget = session.frame_budget();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let (requests, incoming) = sync_channel(capacity.get());
        let (outgoing, results) = sync_channel(capacity.get());
        let worker_channels = WorkerChannels {
            incoming,
            outgoing,
            stopped: worker_stopped,
        };
        let thread = Builder::new().name(String::from("games-host-io")).spawn(
            move || {
                serve(session, &codec, &worker_channels);
            },
        )?;
        Ok(Self {
            channels: Some(Channels { requests, results }),
            frame_budget,
            stopped,
            thread: Some(thread),
        })
    }

    /// Submit one command without waiting for authentication or I/O.
    ///
    /// # Errors
    /// Returns the original request on saturation, shutdown or oversize.
    pub fn try_submit(
        &self,
        request: HostRequest,
    ) -> Result<(), RejectedRequest> {
        if let HostOperation::Send { bytes, .. } = &request.operation
            && bytes.len() > self.frame_budget
        {
            return Err(RejectedRequest {
                reason: RejectionReason::Oversized,
                request,
            });
        }
        let Some(channels) = &self.channels else {
            return Err(RejectedRequest {
                reason: RejectionReason::Closed,
                request,
            });
        };
        channels
            .requests
            .try_send(request)
            .map_err(|error| match error {
                TrySendError::Full(returned) => RejectedRequest {
                    reason: RejectionReason::Full,
                    request: returned,
                },
                TrySendError::Disconnected(returned) => RejectedRequest {
                    reason: RejectionReason::Closed,
                    request: returned,
                },
            })
    }
}

impl<Message> HostWorker<Message> {
    fn stop(&mut self) -> io::Result<()> {
        // Close both directions before joining: the worker may be waiting
        // for another request or unable to deliver a full completion queue.
        self.stopped.store(true, Ordering::Release);
        drop(self.channels.take());
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_panic| io::Error::other("host worker panicked"))?;
        }
        Ok(())
    }
}

impl<Message> Drop for HostWorker<Message> {
    fn drop(&mut self) {
        let _result = self.stop();
    }
}

fn execute<Codec>(
    session: &mut HostSession,
    codec: &Codec,
    operation: HostOperation,
) -> HostResult<Codec::Message>
where
    Codec: MessageSchema,
{
    match operation {
        HostOperation::Authenticate(pending) => {
            let stream = pending.authenticate().map_err(|error| {
                OperationError::Authentication(error.kind())
            })?;
            Ok(HostOutcome::Admitted(session.admit(stream)?))
        },
        HostOperation::Disconnect(peer) => {
            session.disconnect(peer)?;
            Ok(HostOutcome::Applied)
        },
        HostOperation::JoinLocal(peer) => {
            session.join_local(peer)?;
            Ok(HostOutcome::Applied)
        },
        HostOperation::PollAuthenticate(mut pending) => {
            match pending
                .poll_authenticate()
                .map_err(|error| OperationError::Authentication(error.kind()))?
            {
                Some(stream) => {
                    Ok(HostOutcome::Admitted(session.admit(stream)?))
                },
                None => Ok(HostOutcome::Authenticating(pending)),
            }
        },
        HostOperation::PollReceive(peer) => {
            Ok(session.poll_receive(peer, codec)?.map_or_else(
                || HostOutcome::Pending,
                |value| HostOutcome::Message { peer, value },
            ))
        },
        HostOperation::Receive(peer) => {
            let value = session.receive(peer, codec)?;
            Ok(HostOutcome::Message { peer, value })
        },
        HostOperation::ReconnectLocal(peer) => {
            session.reconnect_local(peer)?;
            Ok(HostOutcome::Applied)
        },
        HostOperation::Remove(peer) => {
            session.remove(peer)?;
            Ok(HostOutcome::Applied)
        },
        HostOperation::Revoke(fingerprint) => {
            session.revoke_enrollment(&fingerprint)?;
            Ok(HostOutcome::Applied)
        },
        HostOperation::Send { peer, bytes } => {
            session.send(peer, &bytes)?;
            Ok(HostOutcome::Applied)
        },
    }
}

fn serve<Codec>(
    mut session: HostSession,
    codec: &Codec,
    channels: &WorkerChannels<Codec::Message>,
) where
    Codec: MessageSchema,
{
    while let Ok(request) = channels.incoming.recv() {
        if channels.stopped.load(Ordering::Acquire) {
            break;
        }
        let completion = HostCompletion {
            id: request.id,
            result: execute(&mut session, codec, request.operation),
        };
        if channels.outgoing.send(completion).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::thread;

    use games::network::SessionError;

    use super::super::super::tests::{
        LOOPBACK, TIMEOUT, TestCodec, config, poll_accept, require,
        session_config,
    };
    use super::super::tests::{Credentials, authority, credentials};
    use super::super::{SecureFrameStream, SecureLanListener, ServerName};
    use super::*;

    #[derive(Debug)]
    struct GatedCodec {
        entered: SyncSender<()>,
        release: Receiver<()>,
    }

    impl MessageSchema for GatedCodec {
        type Message = u8;

        fn decode(&self, bytes: &[u8]) -> Option<Self::Message> {
            self.entered.send(()).ok()?;
            self.release.recv_timeout(TIMEOUT).ok()?;
            TestCodec.decode(bytes)
        }
    }

    fn submit<Message>(
        worker: &HostWorker<Message>,
        id: u64,
        operation: HostOperation,
    ) -> io::Result<()>
    where
        Message: Send + 'static,
    {
        worker
            .try_submit(HostRequest { id, operation })
            .map_err(|error| io::Error::other(format!("{error:?}")))
    }

    fn completion<Message>(
        worker: &HostWorker<Message>,
        id: u64,
    ) -> io::Result<HostResult<Message>>
    where
        Message: Send + 'static,
    {
        let completion = poll_accept(|| worker.poll())?;
        require(completion.id == id, "correlated completion")?;
        Ok(completion.result)
    }

    fn authenticate<Message>(
        worker: &HostWorker<Message>,
        listener: &SecureLanListener,
        keys: &Credentials,
        id: u64,
    ) -> io::Result<SecureFrameStream>
    where
        Message: Send + 'static,
    {
        let address = listener.local_addr()?;
        let settings = config()?;
        let server_name =
            ServerName::try_from("games.test").map_err(io::Error::other)?;
        thread::scope(|scope| {
            let remote = scope.spawn(|| {
                SecureFrameStream::connect(
                    address,
                    settings,
                    &keys.client,
                    server_name,
                )
            });
            let pending = poll_accept(|| listener.try_accept())?;
            submit(worker, id, HostOperation::Authenticate(Box::new(pending)))?;
            require(
                matches!(completion(worker, id)?, Ok(HostOutcome::Admitted(2))),
                "verified identity admitted by worker",
            )?;
            remote
                .join()
                .map_err(|_error| io::Error::other("client panicked"))?
        })
    }

    fn poll_admission(
        worker: &HostWorker<u8>,
        listener: &SecureLanListener,
        id: u64,
    ) -> io::Result<()> {
        let mut pending =
            Some(Box::new(poll_accept(|| listener.try_accept())?));
        poll_accept(|| {
            let next = pending.take().ok_or(ErrorKind::NotConnected)?;
            submit(worker, id, HostOperation::PollAuthenticate(next))?;
            match completion(worker, id)? {
                Ok(HostOutcome::Authenticating(next_state)) => {
                    pending = Some(next_state);
                    Ok(None)
                },
                Ok(HostOutcome::Admitted(2)) => Ok(Some(())),
                result => Err(io::Error::other(format!("{result:?}"))),
            }
        })
    }

    #[test]
    fn offline_commands_preserve_slots_and_return_oversized_requests()
    -> io::Result<()> {
        let session = HostSession::new(session_config(false), &[])
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let worker = HostWorker::spawn(session, TestCodec, NonZeroUsize::MIN)?;
        for (id, operation) in [
            (1, HostOperation::JoinLocal(1)),
            (2, HostOperation::Disconnect(1)),
            (3, HostOperation::ReconnectLocal(1)),
        ] {
            submit(&worker, id, operation)?;
            require(
                matches!(completion(&worker, id)?, Ok(HostOutcome::Applied)),
                "offline roster operation",
            )?;
        }
        let rejected = worker.try_submit(HostRequest {
            id: 4,
            operation: HostOperation::Send {
                peer: 1,
                bytes: vec![0; 9],
            },
        });
        require(
            matches!(rejected, Err(RejectedRequest {
                reason: RejectionReason::Oversized,
                request: HostRequest {
                    id: 4,
                    operation: HostOperation::Send { peer: 1, bytes },
                },
            }) if bytes.len() == 9),
            "oversized request returned intact",
        )?;
        worker.shutdown()
    }

    #[test]
    fn loopback_worker_authenticates_decodes_and_rejoins() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let listener =
            SecureLanListener::bind(LOOPBACK, config()?, keys.server.clone())?;
        let session =
            HostSession::new(session_config(true), &[(keys.client_id, 2)])
                .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let worker = HostWorker::spawn(session, TestCodec, NonZeroUsize::MIN)?;
        let mut client = authenticate(&worker, &listener, &keys, 1)?;
        client.send(b"M\x05")?;
        submit(&worker, 2, HostOperation::Receive(2))?;
        require(
            matches!(
                completion(&worker, 2)?,
                Ok(HostOutcome::Message { peer: 2, value: 5 })
            ),
            "trusted peer and decoded message",
        )?;
        submit(&worker, 3, HostOperation::Send {
            peer: 2,
            bytes: b"reply".to_vec(),
        })?;
        require(
            matches!(completion(&worker, 3)?, Ok(HostOutcome::Applied))
                && client.receive()? == b"reply",
            "worker sends encrypted response",
        )?;
        client.send(b"invalid")?;
        submit(&worker, 4, HostOperation::Receive(2))?;
        require(
            matches!(
                completion(&worker, 4)?,
                Err(OperationError::Host(HostError::Session(
                    SessionError::MalformedMessage
                ),))
            ),
            "malformed payload rejected",
        )?;
        submit(&worker, 5, HostOperation::Disconnect(2))?;
        require(
            matches!(completion(&worker, 5)?, Ok(HostOutcome::Applied)),
            "reserve disconnected slot",
        )?;
        drop(authenticate(&worker, &listener, &keys, 6)?);
        worker.shutdown()
    }

    #[test]
    fn queue_saturation_returns_request_without_waiting() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let listener =
            SecureLanListener::bind(LOOPBACK, config()?, keys.server.clone())?;
        let session =
            HostSession::new(session_config(true), &[(keys.client_id, 2)])
                .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let (entered, decoding) = sync_channel(1);
        let (unblock, release) = sync_channel(1);
        let worker = HostWorker::spawn(
            session,
            GatedCodec { entered, release },
            NonZeroUsize::MIN,
        )?;
        let mut client = authenticate(&worker, &listener, &keys, 1)?;
        client.send(b"M\x03")?;
        submit(&worker, 2, HostOperation::Receive(2))?;
        decoding.recv_timeout(TIMEOUT).map_err(io::Error::other)?;
        require(worker.poll()?.is_none(), "poll while transfer is pending")?;
        submit(&worker, 3, HostOperation::JoinLocal(1))?;
        let rejected = worker.try_submit(HostRequest {
            id: 4,
            operation: HostOperation::Disconnect(1),
        });
        unblock.send(()).map_err(io::Error::other)?;
        require(
            matches!(
                rejected,
                Err(RejectedRequest {
                    reason: RejectionReason::Full,
                    request: HostRequest {
                        id: 4,
                        operation: HostOperation::Disconnect(1)
                    },
                })
            ),
            "full queue returns intact command",
        )?;
        require(
            matches!(
                completion(&worker, 2)?,
                Ok(HostOutcome::Message { peer: 2, value: 3 })
            ) && matches!(completion(&worker, 3)?, Ok(HostOutcome::Applied)),
            "commands complete in submission order",
        )?;
        worker.shutdown()
    }

    #[test]
    fn poll_handshake_keeps_host_commands_available() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let listener =
            SecureLanListener::bind(LOOPBACK, config()?, keys.server.clone())?;
        let session =
            HostSession::new(session_config(true), &[(keys.client_id, 2)])
                .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let worker = HostWorker::spawn(session, TestCodec, NonZeroUsize::MIN)?;
        let address = listener.local_addr()?;
        let _silent = TcpStream::connect(address)?;
        let silent = poll_accept(|| listener.try_accept())?;
        submit(
            &worker,
            1,
            HostOperation::PollAuthenticate(Box::new(silent)),
        )?;
        let Ok(HostOutcome::Authenticating(stalled)) = completion(&worker, 1)?
        else {
            return Err(io::Error::other("silent handshake must be pending"));
        };
        submit(&worker, 2, HostOperation::JoinLocal(1))?;
        require(
            matches!(completion(&worker, 2)?, Ok(HostOutcome::Applied)),
            "local admission independent of silent handshake",
        )?;
        let server_name =
            ServerName::try_from("games.test").map_err(io::Error::other)?;
        thread::scope(|scope| {
            let remote = scope.spawn(|| {
                SecureFrameStream::connect(
                    address,
                    config()?,
                    &keys.client,
                    server_name,
                )
            });
            poll_admission(&worker, &listener, 3)?;
            let mut client = remote
                .join()
                .map_err(|_error| io::Error::other("client panicked"))??;
            client.send(b"M\x07")?;
            submit(&worker, 4, HostOperation::Receive(2))?;
            require(
                matches!(
                    completion(&worker, 4)?,
                    Ok(HostOutcome::Message { peer: 2, value: 7 })
                ),
                "second peer completes and sends while first is silent",
            )
        })?;
        drop(stalled);
        worker.shutdown()
    }

    #[test]
    fn idle_peer_poll_allows_other_commands_and_future_messages()
    -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let listener =
            SecureLanListener::bind(LOOPBACK, config()?, keys.server.clone())?;
        let session =
            HostSession::new(session_config(true), &[(keys.client_id, 2)])
                .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let worker = HostWorker::spawn(session, TestCodec, NonZeroUsize::MIN)?;
        let mut client = authenticate(&worker, &listener, &keys, 1)?;
        submit(&worker, 2, HostOperation::PollReceive(2))?;
        require(
            matches!(completion(&worker, 2)?, Ok(HostOutcome::Pending)),
            "idle peer is pending",
        )?;
        submit(&worker, 3, HostOperation::JoinLocal(1))?;
        require(
            matches!(completion(&worker, 3)?, Ok(HostOutcome::Applied)),
            "other player admitted while LAN peer is idle",
        )?;
        client.send(b"M\x09")?;
        poll_accept(|| {
            submit(&worker, 4, HostOperation::PollReceive(2))?;
            match completion(&worker, 4)? {
                Ok(HostOutcome::Pending) => Ok(None),
                Ok(HostOutcome::Message { peer: 2, value: 9 }) => Ok(Some(())),
                result => Err(io::Error::other(format!("{result:?}"))),
            }
        })?;
        worker.shutdown()
    }
}
