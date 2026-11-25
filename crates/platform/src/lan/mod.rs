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
//   - Native TCP endpoints, transfer deadlines and shared frame boundaries.
// - Must-Not:
//   - Interpret game actions or expose a public service.
// - Allows:
//   - Inputs: Explicit LAN configuration, endpoints and byte frames.
//   - Outputs: Bounded frames and untrusted sockets for secure negotiation.
//   - Side effects: Local socket I/O and bounded frame allocation.
// - Split-When:
//   - Secure negotiation needs a separate authentication adapter.
// - Merge-When:
//   - A shared native transport covers these same framing guarantees.
// - Summary:
//   - Opt-in native TCP transport and reusable deadline framing.
// - Description:
//   - Bounds entire frame transfers and closes failed connections.
// - Usage:
//   - Use authenticated connections from the secure adapter on I/O workers.
// - Defaults:
//   - No socket access without explicit LAN permission.

/// Locally provisioned, mutually authenticated TLS transport.
pub mod secure;

use std::io::{self, ErrorKind, Read as _, Write as _};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use games::network::SessionConfig;

/// Explicit native transport limits inherited from a session.
#[derive(Clone, Copy, Debug)]
pub struct LanConfig {
    max_bytes: u32,
    timeout: Duration,
}

impl LanConfig {
    fn deadline(self) -> io::Result<Instant> {
        Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| ErrorKind::InvalidInput.into())
    }

    fn frame_length(self, length: u32) -> io::Result<usize> {
        if length > self.max_bytes {
            return Err(ErrorKind::InvalidData.into());
        }
        usize::try_from(length).map_err(|_error| ErrorKind::InvalidData.into())
    }

    /// Derive a transport configuration from an opted-in session.
    ///
    /// # Errors
    ///
    /// Rejects disabled LAN, an unrepresentable frame budget or timeout.
    pub fn new(session: SessionConfig, timeout: Duration) -> io::Result<Self> {
        if !session.allow_lan {
            return Err(ErrorKind::PermissionDenied.into());
        }
        if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let max_bytes = u32::try_from(session.max_message_bytes.get())
            .map_err(|_error| ErrorKind::InvalidInput)?;
        Ok(Self { max_bytes, timeout })
    }
}

#[derive(Debug)]
struct LanListener {
    config: LanConfig,
    listener: TcpListener,
}

impl LanListener {
    fn bind(address: SocketAddr, config: LanConfig) -> io::Result<Self> {
        require_private(address)?;
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self { config, listener })
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    #[cfg(test)]
    fn try_accept(&self) -> io::Result<Option<FrameStream>> {
        self.try_socket().map(|stream| {
            stream.map(|socket| FrameStream {
                channel: FrameChannel::new(socket, self.config),
            })
        })
    }

    fn try_socket(&self) -> io::Result<Option<DeadlineSocket>> {
        match self.listener.accept() {
            Ok((stream, address)) => {
                require_private(address)?;
                DeadlineSocket::new(stream, self.config).map(Some)
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug)]
struct DeadlineSocket {
    deadline: Instant,
    socket: TcpStream,
}

impl DeadlineSocket {
    fn new(socket: TcpStream, config: LanConfig) -> io::Result<Self> {
        socket.set_nonblocking(false)?;
        socket.set_nodelay(true)?;
        Ok(Self {
            deadline: config.deadline()?,
            socket,
        })
    }

    fn prepare_read(&self) -> io::Result<()> {
        self.socket
            .set_read_timeout(Some(remaining(self.deadline)?))
    }

    fn prepare_write(&self) -> io::Result<()> {
        self.socket
            .set_write_timeout(Some(remaining(self.deadline)?))
    }
}

trait TimedStream {
    fn read_chunk(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
    fn reset_deadline(&mut self, deadline: Instant);
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<usize>;
}

impl TimedStream for DeadlineSocket {
    fn read_chunk(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.prepare_read()?;
        self.socket.read(bytes)
    }

    fn reset_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.prepare_write()?;
        self.socket.write(bytes)
    }
}

#[derive(Debug)]
struct FrameChannel<Stream> {
    config: LanConfig,
    stream: Option<Stream>,
}

impl<Stream> FrameChannel<Stream>
where
    Stream: TimedStream,
{
    const fn new(stream: Stream, config: LanConfig) -> Self {
        Self {
            config,
            stream: Some(stream),
        }
    }

    fn receive(&mut self) -> io::Result<Vec<u8>> {
        self.transfer(receive_frame)
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let length = u32::try_from(bytes.len())
            .map_err(|_error| ErrorKind::InvalidInput)?;
        if length > self.config.max_bytes {
            return Err(ErrorKind::InvalidInput.into());
        }
        self.transfer(|stream, _config| {
            write_all(stream, &length.to_be_bytes())?;
            write_all(stream, bytes)
        })
    }

    fn transfer<Output, Operation>(
        &mut self,
        operation: Operation,
    ) -> io::Result<Output>
    where
        Operation: FnOnce(&mut Stream, LanConfig) -> io::Result<Output>,
    {
        let stream = self.stream.as_mut().ok_or(ErrorKind::NotConnected)?;
        stream.reset_deadline(self.config.deadline()?);
        let result = operation(stream, self.config);
        if result.is_err() {
            drop(self.stream.take());
        }
        result
    }
}

#[cfg(test)]
#[derive(Debug)]
struct FrameStream {
    channel: FrameChannel<DeadlineSocket>,
}

#[cfg(test)]
impl FrameStream {
    fn connect(address: SocketAddr, config: LanConfig) -> io::Result<Self> {
        let socket = connect_socket(address, config)?;
        Ok(Self {
            channel: FrameChannel::new(socket, config),
        })
    }

    fn receive(&mut self) -> io::Result<Vec<u8>> {
        self.channel.receive()
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.channel.send(bytes)
    }
}

fn connect_socket(
    address: SocketAddr,
    config: LanConfig,
) -> io::Result<DeadlineSocket> {
    require_private(address)?;
    let stream = TcpStream::connect_timeout(&address, config.timeout)?;
    DeadlineSocket::new(stream, config)
}

const fn private_address(address: SocketAddr) -> bool {
    match address.ip().to_canonical() {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local()
        },
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        },
    }
}

fn read_all<Stream>(stream: &mut Stream, mut bytes: &mut [u8]) -> io::Result<()>
where
    Stream: TimedStream,
{
    while !bytes.is_empty() {
        match stream.read_chunk(bytes) {
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(count) => {
                bytes = bytes.get_mut(count..).ok_or(ErrorKind::InvalidData)?;
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn receive_frame<Stream>(
    stream: &mut Stream,
    config: LanConfig,
) -> io::Result<Vec<u8>>
where
    Stream: TimedStream,
{
    let mut header = [0; 4];
    read_all(stream, &mut header)?;
    let length = config.frame_length(u32::from_be_bytes(header))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| ErrorKind::OutOfMemory)?;
    bytes.resize(length, 0);
    read_all(stream, &mut bytes)?;
    Ok(bytes)
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| ErrorKind::TimedOut.into())
}

fn require_private(address: SocketAddr) -> io::Result<()> {
    if !private_address(address) {
        return Err(ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

fn write_all<Stream>(stream: &mut Stream, mut bytes: &[u8]) -> io::Result<()>
where
    Stream: TimedStream,
{
    while !bytes.is_empty() {
        match stream.write_chunk(bytes) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(count) => {
                bytes = bytes.get(count..).ok_or(ErrorKind::InvalidData)?;
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind, Write as _};
    use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpStream};
    use std::num::{NonZeroU8, NonZeroUsize};
    use std::thread;
    use std::time::{Duration, Instant};

    use games::network::{MessageSchema, PeerKind, Session, SessionConfig};

    use super::{FrameStream, LanConfig, LanListener};

    pub(super) const LOOPBACK: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    pub(super) const TIMEOUT: Duration = Duration::from_secs(2);

    #[derive(Debug)]
    struct TestCodec;

    impl MessageSchema for TestCodec {
        type Message = u8;

        fn decode(&self, bytes: &[u8]) -> Option<Self::Message> {
            match bytes {
                [b'M', value] if *value <= 10 => Some(*value),
                _ => None,
            }
        }
    }

    pub(super) fn poll_accept<Connection, Accept>(
        mut accept: Accept,
    ) -> io::Result<Connection>
    where
        Accept: FnMut() -> io::Result<Option<Connection>>,
    {
        let deadline = Instant::now()
            .checked_add(TIMEOUT)
            .ok_or(ErrorKind::TimedOut)?;
        loop {
            if let Some(stream) = accept()? {
                return Ok(stream);
            }
            if Instant::now() >= deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            thread::yield_now();
        }
    }

    fn accepted(listener: &LanListener) -> io::Result<FrameStream> {
        poll_accept(|| listener.try_accept())
    }

    pub(super) fn config() -> io::Result<LanConfig> {
        LanConfig::new(session_config(true), TIMEOUT)
    }

    pub(super) fn error_kind<Value>(
        result: io::Result<Value>,
    ) -> Option<ErrorKind> {
        result.err().map(|error| error.kind())
    }

    #[test]
    fn bidirectional_frames_preserve_boundaries_and_limits() -> io::Result<()> {
        let settings = config()?;
        let listener = LanListener::bind(LOOPBACK, settings)?;
        require(listener.try_accept()?.is_none(), "idle listener")?;
        let mut client =
            FrameStream::connect(listener.local_addr()?, settings)?;
        let mut host = accepted(&listener)?;
        require(
            error_kind(client.send(b"123456789"))
                == Some(ErrorKind::InvalidInput),
            "oversized outgoing frame",
        )?;
        client.send(b"12345678")?;
        client.send(b"")?;
        client.send(b"next")?;
        require(host.receive()? == b"12345678", "maximum length frame")?;
        require(host.receive()?.is_empty(), "empty frame")?;
        require(host.receive()? == b"next", "next frame")?;
        host.send(b"reply")?;
        require(client.receive()? == b"reply", "reply frame")
    }

    #[test]
    fn incomplete_header_times_out_and_closes_connection() -> io::Result<()> {
        let settings =
            LanConfig::new(session_config(true), Duration::from_millis(50))?;
        let listener = LanListener::bind(LOOPBACK, settings)?;
        let mut client = TcpStream::connect(listener.local_addr()?)?;
        let mut host = accepted(&listener)?;
        client.write_all(&[0, 0])?;
        let error = error_kind(host.receive());
        require(
            matches!(error, Some(ErrorKind::TimedOut | ErrorKind::WouldBlock)),
            "incomplete header must time out",
        )?;
        require(
            error_kind(host.receive()) == Some(ErrorKind::NotConnected),
            "timed-out connection must close",
        )
    }

    #[test]
    fn oversized_wire_header_is_rejected_before_payload() -> io::Result<()> {
        let listener = LanListener::bind(LOOPBACK, config()?)?;
        let mut client = TcpStream::connect(listener.local_addr()?)?;
        let mut host = accepted(&listener)?;
        client.write_all(&u32::MAX.to_be_bytes())?;
        require(
            error_kind(host.receive()) == Some(ErrorKind::InvalidData),
            "oversized wire header",
        )?;
        require(
            error_kind(host.receive()) == Some(ErrorKind::NotConnected),
            "oversized wire frame closes connection",
        )
    }

    pub(super) fn require(
        condition: bool,
        context: &'static str,
    ) -> io::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(io::Error::other(context))
        }
    }

    #[test]
    fn session_can_decode_and_rejoin_over_real_loopback() -> io::Result<()> {
        let settings = config()?;
        let listener = LanListener::bind(LOOPBACK, settings)?;
        let mut session = Session::new(session_config(true));
        require(session.join(1, PeerKind::Local).is_ok(), "local controller")?;
        let address = listener.local_addr()?;
        let mut client = FrameStream::connect(address, settings)?;
        let mut host = accepted(&listener)?;
        // The test owns both endpoints and supplies the trusted identity.
        require(session.join(2, PeerKind::Lan).is_ok(), "trusted test peer")?;
        client.send(b"bad")?;
        require(
            session.decode(2, &host.receive()?, &TestCodec).is_err(),
            "malformed message must not enter simulation",
        )?;
        client.send(b"M\x04")?;
        require(
            session.decode(2, &host.receive()?, &TestCodec) == Ok(4),
            "valid first message",
        )?;
        drop(client);
        require(
            error_kind(host.receive()) == Some(ErrorKind::UnexpectedEof),
            "client disconnect",
        )?;
        require(session.disconnect(2).is_ok(), "reserve disconnected slot")?;
        require(
            error_kind(host.receive()) == Some(ErrorKind::NotConnected),
            "disconnected stream closes",
        )?;
        let mut replacement = FrameStream::connect(address, settings)?;
        let mut next_host = accepted(&listener)?;
        require(session.reconnect(2, PeerKind::Lan).is_ok(), "rejoin")?;
        replacement.send(b"M\x07")?;
        require(
            session.decode(2, &next_host.receive()?, &TestCodec) == Ok(7),
            "valid rejoined message",
        )
    }

    pub(super) fn session_config(lan: bool) -> SessionConfig {
        SessionConfig {
            allow_lan: lan,
            max_message_bytes: NonZeroUsize::new(8)
                .unwrap_or(NonZeroUsize::MIN),
            max_players: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
        }
    }

    #[test]
    fn transport_requires_opt_in_and_a_private_address() -> io::Result<()> {
        require(
            error_kind(LanConfig::new(session_config(false), TIMEOUT))
                == Some(ErrorKind::PermissionDenied),
            "offline configuration must not open sockets",
        )?;
        require(
            error_kind(LanConfig::new(session_config(true), Duration::ZERO))
                == Some(ErrorKind::InvalidInput),
            "zero timeout",
        )?;
        let settings = config()?;
        for address in [
            SocketAddr::from(([0, 0, 0, 0], 0)),
            SocketAddr::from(([8, 8, 8, 8], 0)),
            SocketAddr::from(([0; 8], 0)),
            SocketAddr::from(([0, 0, 0, 0, 0, 0xffff, 0x0808, 0x0808], 0)),
        ] {
            require(
                error_kind(LanListener::bind(address, settings))
                    == Some(ErrorKind::PermissionDenied),
                "public or wildcard listener",
            )?;
            require(
                error_kind(FrameStream::connect(address, settings))
                    == Some(ErrorKind::PermissionDenied),
                "public or wildcard connection",
            )?;
        }
        Ok(())
    }

    #[test]
    fn truncated_frame_closes_without_partial_data() -> io::Result<()> {
        let listener = LanListener::bind(LOOPBACK, config()?)?;
        let mut client = TcpStream::connect(listener.local_addr()?)?;
        let mut host = accepted(&listener)?;
        client.write_all(&4u32.to_be_bytes())?;
        client.write_all(b"ab")?;
        client.shutdown(Shutdown::Write)?;
        require(
            error_kind(host.receive()) == Some(ErrorKind::UnexpectedEof),
            "truncated frame",
        )?;
        require(
            error_kind(host.send(b"reply")) == Some(ErrorKind::NotConnected),
            "truncated frame closes connection",
        )
    }
}
