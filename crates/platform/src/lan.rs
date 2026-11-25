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
//   - Native TCP binding, connection and bounded message framing.
// - Must-Not:
//   - Authenticate peers, interpret game actions or expose a public service.
// - Allows:
//   - Inputs: Explicit LAN configuration, endpoints and byte frames.
//   - Outputs: Untrusted connections and bounded frames or I/O errors.
//   - Side effects: Local socket I/O and bounded frame allocation.
// - Split-When:
//   - A secure transport or nonblocking I/O runtime needs its own adapter.
// - Merge-When:
//   - A shared native transport covers these same framing guarantees.
// - Summary:
//   - Opt-in private-address TCP transport.
// - Description:
//   - Bounds entire frame transfers and closes failed connections.
// - Usage:
//   - Use on an I/O worker; authenticate peers before session admission.
// - Defaults:
//   - No socket access without explicit LAN permission.

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

/// Nonblocking TCP listener bound to one explicit local network interface.
///
/// Accepted connections are untrusted until a separate identity mechanism
/// verifies them. This is not an authenticated or encrypted transport.
#[derive(Debug)]
pub struct LanListener {
    config: LanConfig,
    listener: TcpListener,
}

impl LanListener {
    /// Open a listener on an explicitly selected private address.
    ///
    /// # Errors
    ///
    /// Rejects public and wildcard addresses or operating-system failures.
    pub fn bind(address: SocketAddr, config: LanConfig) -> io::Result<Self> {
        require_private(address)?;
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self { config, listener })
    }

    /// Report the assigned local address, including an ephemeral port.
    ///
    /// # Errors
    ///
    /// Propagates socket inspection failures.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Poll for one untrusted connection without blocking the host loop.
    ///
    /// # Errors
    ///
    /// Rejects non-private peers and propagates socket failures.
    pub fn try_accept(&self) -> io::Result<Option<FrameStream>> {
        match self.listener.accept() {
            Ok((stream, address)) => {
                require_private(address)?;
                FrameStream::from_socket(stream, self.config).map(Some)
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// A bounded native stream carrying big-endian u32 length-prefixed frames.
///
/// Transfers may block until the configured deadline. Use an I/O worker,
/// never the simulation or render thread. Bytes are not authenticated.
#[derive(Debug)]
pub struct FrameStream {
    config: LanConfig,
    stream: Option<TcpStream>,
}

impl FrameStream {
    /// Connect to an explicit local network peer.
    ///
    /// # Errors
    ///
    /// Rejects public and wildcard destinations or connection failures.
    pub fn connect(address: SocketAddr, config: LanConfig) -> io::Result<Self> {
        require_private(address)?;
        let stream = TcpStream::connect_timeout(&address, config.timeout)?;
        Self::from_socket(stream, config)
    }

    fn from_socket(stream: TcpStream, config: LanConfig) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            config,
            stream: Some(stream),
        })
    }

    /// Receive one complete frame, allocating only after its size is checked.
    ///
    /// # Errors
    ///
    /// Rejects oversized, incomplete or timed-out frames. A failed transfer
    /// closes the connection, so an invalid frame cannot desynchronise it.
    pub fn receive(&mut self) -> io::Result<Vec<u8>> {
        self.transfer(receive_frame)
    }

    /// Send one bounded frame without fragmenting its application boundary.
    ///
    /// # Errors
    ///
    /// Rejects oversized outgoing data before I/O. Transfer failures close
    /// the connection.
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let length = u32::try_from(bytes.len())
            .map_err(|_error| ErrorKind::InvalidInput)?;
        if length > self.config.max_bytes {
            return Err(ErrorKind::InvalidInput.into());
        }
        self.transfer(|stream, config| {
            let deadline = config.deadline()?;
            write_until(stream, &length.to_be_bytes(), deadline)?;
            write_until(stream, bytes, deadline)
        })
    }

    fn transfer<Output, Operation>(
        &mut self,
        operation: Operation,
    ) -> io::Result<Output>
    where
        Operation: FnOnce(&mut TcpStream, LanConfig) -> io::Result<Output>,
    {
        let stream = self.stream.as_mut().ok_or(ErrorKind::NotConnected)?;
        let result = operation(stream, self.config);
        if result.is_err() {
            drop(self.stream.take());
        }
        result
    }
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

fn read_until(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(bytes) {
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

fn receive_frame(
    stream: &mut TcpStream,
    config: LanConfig,
) -> io::Result<Vec<u8>> {
    let deadline = config.deadline()?;
    let mut header = [0; 4];
    read_until(stream, &mut header, deadline)?;
    let length = config.frame_length(u32::from_be_bytes(header))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| ErrorKind::OutOfMemory)?;
    bytes.resize(length, 0);
    read_until(stream, &mut bytes, deadline)?;
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

fn write_until(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
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

    const LOOPBACK: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    const TIMEOUT: Duration = Duration::from_secs(2);

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

    fn accepted(listener: &LanListener) -> io::Result<FrameStream> {
        let deadline = Instant::now()
            .checked_add(TIMEOUT)
            .ok_or(ErrorKind::TimedOut)?;
        loop {
            if let Some(stream) = listener.try_accept()? {
                return Ok(stream);
            }
            if Instant::now() >= deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            thread::yield_now();
        }
    }

    fn config() -> io::Result<LanConfig> {
        LanConfig::new(session_config(true), TIMEOUT)
    }

    fn error_kind<Value>(result: io::Result<Value>) -> Option<ErrorKind> {
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

    fn require(condition: bool, context: &'static str) -> io::Result<()> {
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

    fn session_config(lan: bool) -> SessionConfig {
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
