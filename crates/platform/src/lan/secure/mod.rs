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
//   - Native TLS peer verification and secure frame transport.
// - Must-Not:
//   - Issue production credentials, own accounts or interpret game actions.
// - Allows:
//   - Inputs: Locally provisioned trust roots, keys and LAN endpoints.
//   - Outputs: Verified certificate identities and encrypted frames.
//   - Side effects: Bounded TLS handshakes and local socket access.
// - Split-When:
//   - Certificate provisioning needs an independent host workflow.
// - Merge-When:
//   - A shared authenticated transport covers these same trust guarantees.
// - Summary:
//   - Mutual TLS 1.3 with explicit local trust roots.
// - Description:
//   - Uses Rustls verification and the common bounded frame channel.
// - Usage:
//   - Match verified peer fingerprints to host-approved player identities.
// - Defaults:
//   - No anonymous peers, public root stores or session resumption.

/// Host-side enrolment and session admission.
pub mod host;
/// Local credential import and explicit public trust exchange.
pub mod identity;

use std::io::{self, ErrorKind, Read as _, Write as _};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::client::{Resumption, WebPkiServerVerifier};
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{NoServerSessionStorage, WebPkiClientVerifier};
use rustls::sign::{CertifiedKey, SingleCertAndKey};
use rustls::version::TLS13;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection,
    DigitallySignedStruct, DistinguishedName, Error, RootCertStore,
    ServerConfig, ServerConnection, SignatureScheme,
};
use sha2::{Digest as _, Sha256};

use super::{
    DeadlineSocket, FrameChannel, LanConfig, LanListener, TimedStream,
    connect_socket,
};

const PROTOCOL: &[u8] = b"games-lan/1";

/// SHA-256 identity of a verified leaf certificate.
///
/// A host must explicitly associate the fingerprint with a player slot;
/// certification alone is not an authorisation to join a specific session.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PeerFingerprint([u8; 32]);

impl PeerFingerprint {
    /// Restore an enrolled fingerprint from its canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derive the fingerprint of a certificate supplied for local enrolment.
    #[must_use]
    pub fn from_der(certificate: &[u8]) -> Self {
        Self(Sha256::digest(certificate).into())
    }

    /// Return the canonical fingerprint bytes for host-side enrolment.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Client credentials, local trust roots and an explicitly approved host.
#[derive(Clone, Debug)]
pub struct ClientTls(Arc<ClientConfig>);

impl ClientTls {
    fn from_identity(
        roots: RootCertStore,
        certified: Arc<CertifiedKey>,
        expected_host: PeerFingerprint,
    ) -> io::Result<Self> {
        if roots.is_empty() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let provider = Arc::new(default_provider());
        let verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::clone(&provider),
        )
        .build()
        .map_err(io::Error::other)?;
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&TLS13])
            .map_err(io::Error::other)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedHost {
                expected_host,
                verifier,
            }))
            .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(
                certified,
            )));
        config.alpn_protocols = vec![PROTOCOL.to_vec()];
        config.resumption = Resumption::disabled();
        Ok(Self(Arc::new(config)))
    }

    /// Configure mutual TLS with local roots and an approved host fingerprint.
    ///
    /// Obtain the fingerprint through a trusted local enrolment process.
    /// Changing the host leaf certificate requires explicit re-enrolment.
    ///
    /// # Errors
    ///
    /// Rejects empty trust roots, invalid keys and certificate chains.
    pub fn new(
        roots: RootCertStore,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        expected_host: PeerFingerprint,
    ) -> io::Result<Self> {
        let certified = CertifiedKey::from_der(chain, key, &default_provider())
            .map_err(io::Error::other)?;
        Self::from_identity(roots, Arc::new(certified), expected_host)
    }
}

#[derive(Debug)]
struct PinnedHost {
    expected_host: PeerFingerprint,
    verifier: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinnedHost {
    fn requires_raw_public_keys(&self) -> bool {
        self.verifier.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.verifier.root_hint_subjects()
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.verifier.supported_verify_schemes()
    }

    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let verified = self.verifier.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        if PeerFingerprint::from_der(end_entity.as_ref()) != self.expected_host
        {
            return Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.verifier.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.verifier.verify_tls13_signature(message, cert, dss)
    }
}

/// Host credentials and explicitly supplied trusted client roots.
#[derive(Clone, Debug)]
pub struct ServerTls(Arc<ServerConfig>);

impl ServerTls {
    fn from_identity(
        roots: RootCertStore,
        certified: Arc<CertifiedKey>,
    ) -> io::Result<Self> {
        let provider = Arc::new(default_provider());
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::clone(&provider),
        )
        .build()
        .map_err(io::Error::other)?;
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&TLS13])
            .map_err(io::Error::other)?
            .with_client_cert_verifier(verifier)
            .with_cert_resolver(Arc::new(SingleCertAndKey::from(certified)));
        config.alpn_protocols = vec![PROTOCOL.to_vec()];
        config.send_tls13_tickets = 0;
        config.session_storage = Arc::new(NoServerSessionStorage {});
        Ok(Self(Arc::new(config)))
    }

    /// Configure mandatory client verification with local trust roots.
    ///
    /// # Errors
    ///
    /// Rejects empty trust roots, invalid keys and certificate chains.
    pub fn new(
        roots: RootCertStore,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> io::Result<Self> {
        let certified = CertifiedKey::from_der(chain, key, &default_provider())
            .map_err(io::Error::other)?;
        Self::from_identity(roots, Arc::new(certified))
    }
}

/// A nonblocking listener which accepts only private-address TLS peers.
#[derive(Debug)]
pub struct SecureLanListener {
    listener: LanListener,
    pending: Arc<AtomicU8>,
    tls: ServerTls,
}

impl SecureLanListener {
    /// Open an explicitly selected private interface with host credentials.
    ///
    /// # Errors
    ///
    /// Rejects public or wildcard interfaces and operating-system errors.
    pub fn bind(
        address: SocketAddr,
        config: LanConfig,
        tls: ServerTls,
    ) -> io::Result<Self> {
        let listener = LanListener::bind(address, config)?;
        Ok(Self {
            listener,
            pending: Arc::new(AtomicU8::new(0)),
            tls,
        })
    }

    /// Return the listening address, including an assigned ephemeral port.
    ///
    /// # Errors
    ///
    /// Propagates socket inspection failures.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Poll for a pending connection without performing a blocking handshake.
    ///
    /// Returns no connection while the configured player-count budget is full.
    ///
    /// # Errors
    ///
    /// Rejects non-private peers and propagates socket failures.
    pub fn try_accept(&self) -> io::Result<Option<PendingConnection>> {
        let Some(permit) = PendingPermit::acquire(
            &self.pending,
            self.listener.config.max_pending,
        ) else {
            return Ok(None);
        };
        let Some(stream) = self.listener.try_socket()? else {
            return Ok(None);
        };
        Ok(Some(PendingConnection {
            config: self.listener.config,
            permit,
            stream,
            tls: self.tls.clone(),
        }))
    }
}

#[derive(Debug)]
struct PendingPermit(Arc<AtomicU8>);

impl PendingPermit {
    fn acquire(counter: &Arc<AtomicU8>, limit: u8) -> Option<Self> {
        let _previous = counter
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_add(1).filter(|next| *next <= limit)
            })
            .ok()?;
        Some(Self(Arc::clone(counter)))
    }
}

impl Drop for PendingPermit {
    fn drop(&mut self) {
        let _previous = self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// An untrusted accepted socket that can only be authenticated or dropped.
#[derive(Debug)]
pub struct PendingConnection {
    config: LanConfig,
    permit: PendingPermit,
    stream: DeadlineSocket,
    tls: ServerTls,
}

impl PendingConnection {
    /// Complete the bounded mutual TLS handshake on an I/O worker.
    ///
    /// # Errors
    ///
    /// Rejects anonymous, untrusted or incompatible peers and timed-out I/O.
    pub fn authenticate(self) -> io::Result<SecureFrameStream> {
        let Self {
            config,
            permit: _permit,
            stream,
            tls,
        } = self;
        let connection =
            ServerConnection::new(tls.0).map_err(io::Error::other)?;
        handshake(Connection::Server(connection), stream, config)
    }
}

/// An encrypted framed connection with a verified peer certificate.
///
/// Frames can block for the configured deadline. Only use on I/O workers.
#[derive(Debug)]
pub struct SecureFrameStream {
    channel: FrameChannel<TlsSocket>,
    fingerprint: PeerFingerprint,
}

impl SecureFrameStream {
    /// Connect and verify the server name against locally trusted roots.
    ///
    /// # Errors
    ///
    /// Rejects public endpoints, untrusted hosts and I/O failures.
    pub fn connect(
        address: SocketAddr,
        config: LanConfig,
        tls: &ClientTls,
        name: ServerName<'static>,
    ) -> io::Result<Self> {
        let socket = connect_socket(address, config)?;
        let connection = ClientConnection::new(Arc::clone(&tls.0), name)
            .map_err(io::Error::other)?;
        handshake(Connection::Client(connection), socket, config)
    }

    /// Get the authenticated peer's leaf certificate fingerprint.
    #[must_use]
    pub const fn peer_fingerprint(&self) -> PeerFingerprint {
        self.fingerprint
    }

    /// Receive one bounded, authenticated application frame.
    ///
    /// # Errors
    ///
    /// Closes the connection after a failed, oversized or incomplete frame.
    pub fn receive(&mut self) -> io::Result<Vec<u8>> {
        self.channel.receive()
    }

    /// Send one bounded application frame through the encrypted stream.
    ///
    /// # Errors
    ///
    /// Rejects oversized data and closes the connection on transfer failure.
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.channel.send(bytes)
    }
}

#[derive(Debug)]
struct TlsSocket {
    connection: Connection,
    socket: DeadlineSocket,
}

impl TlsSocket {
    fn flush_tls(&mut self) -> io::Result<()> {
        while self.connection.wants_write() {
            self.socket.prepare_write()?;
            match self.connection.write_tls(&mut self.socket.socket) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(_) => {},
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_tls(&mut self) -> io::Result<()> {
        loop {
            self.socket.prepare_read()?;
            match self.connection.read_tls(&mut self.socket.socket) {
                Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(_) => {
                    let _state = self
                        .connection
                        .process_new_packets()
                        .map_err(io::Error::other)?;
                    return Ok(());
                },
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }
    }
}

impl TimedStream for TlsSocket {
    fn read_chunk(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.connection.reader().read(bytes) {
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.flush_tls()?;
                    self.read_tls()?;
                },
                result => return result,
            }
        }
    }

    fn reset_deadline(&mut self, deadline: Instant) {
        self.socket.reset_deadline(deadline);
    }

    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.connection.writer().write(bytes)?;
        self.flush_tls()?;
        Ok(count)
    }
}

fn handshake(
    connection: Connection,
    socket: DeadlineSocket,
    config: LanConfig,
) -> io::Result<SecureFrameStream> {
    let mut stream = TlsSocket { connection, socket };
    while stream.connection.is_handshaking() {
        stream.flush_tls()?;
        stream.read_tls()?;
    }
    stream.flush_tls()?;
    if stream.connection.alpn_protocol() != Some(PROTOCOL) {
        return Err(ErrorKind::InvalidData.into());
    }
    let certificate = stream
        .connection
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or(ErrorKind::PermissionDenied)?;
    let fingerprint = PeerFingerprint::from_der(certificate.as_ref());
    Ok(SecureFrameStream {
        channel: FrameChannel::new(stream, config),
        fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::num::NonZeroU8;
    use std::thread;
    use std::time::Duration;

    use games::network::{PeerKind, Session};
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer,
        ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
    };
    use rustls::pki_types::PrivatePkcs8KeyDer;

    use super::super::tests::{
        LOOPBACK, TIMEOUT, config, error_kind, poll_accept, require,
        session_config,
    };
    use super::{
        ClientTls, ErrorKind, LanConfig, PeerFingerprint, RootCertStore,
        SecureFrameStream, SecureLanListener, ServerName, ServerTls, io,
    };

    const HOST: &str = "games.test";

    pub(super) type ConnectionPair =
        (io::Result<SecureFrameStream>, io::Result<SecureFrameStream>);

    pub(super) struct Credentials {
        pub(super) client: ClientTls,
        pub(super) client_id: PeerFingerprint,
        pub(super) server: ServerTls,
        pub(super) server_id: PeerFingerprint,
    }

    pub(super) fn authority() -> io::Result<CertifiedIssuer<'static, KeyPair>> {
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let key = KeyPair::generate().map_err(io::Error::other)?;
        CertifiedIssuer::self_signed(params, key).map_err(io::Error::other)
    }

    fn roots(
        issuer: &CertifiedIssuer<'_, KeyPair>,
    ) -> io::Result<RootCertStore> {
        let mut roots = RootCertStore::empty();
        roots.add(issuer.der().clone()).map_err(io::Error::other)?;
        Ok(roots)
    }

    pub(super) fn credentials(
        host_ca: &CertifiedIssuer<'_, KeyPair>,
        client_ca: &CertifiedIssuer<'_, KeyPair>,
    ) -> io::Result<Credentials> {
        let mut host_params = CertificateParams::new(vec![String::from(HOST)])
            .map_err(io::Error::other)?;
        host_params.extended_key_usages =
            vec![ExtendedKeyUsagePurpose::ServerAuth];
        let host_key = KeyPair::generate().map_err(io::Error::other)?;
        let host_cert = host_params
            .signed_by(&host_key, host_ca)
            .map_err(io::Error::other)?;
        let server_id = PeerFingerprint::from_der(host_cert.der().as_ref());
        let server = ServerTls::new(
            roots(host_ca)?,
            vec![host_cert.der().clone()],
            PrivatePkcs8KeyDer::from(host_key.serialize_der()).into(),
        )?;

        let (client, client_id) =
            client_credentials(host_ca, client_ca, server_id)?;
        Ok(Credentials {
            client,
            client_id,
            server,
            server_id,
        })
    }

    pub(super) fn client_credentials(
        host_ca: &CertifiedIssuer<'_, KeyPair>,
        client_ca: &CertifiedIssuer<'_, KeyPair>,
        expected_host: PeerFingerprint,
    ) -> io::Result<(ClientTls, PeerFingerprint)> {
        let mut client_params = CertificateParams::default();
        client_params.extended_key_usages =
            vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_key = KeyPair::generate().map_err(io::Error::other)?;
        let client_cert = client_params
            .signed_by(&client_key, client_ca)
            .map_err(io::Error::other)?;
        let client_id = PeerFingerprint::from_der(client_cert.der().as_ref());
        let client = ClientTls::new(
            roots(host_ca)?,
            vec![client_cert.der().clone()],
            PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
            expected_host,
        )?;
        Ok((client, client_id))
    }

    pub(super) fn pair(
        listener: &SecureLanListener,
        settings: LanConfig,
        client: &ClientTls,
        name: &'static str,
    ) -> io::Result<ConnectionPair> {
        let address = listener.local_addr()?;
        let server_name =
            ServerName::try_from(name).map_err(io::Error::other)?;
        thread::scope(|scope| {
            let worker = scope.spawn(|| {
                poll_accept(|| listener.try_accept())?.authenticate()
            });
            let remote = SecureFrameStream::connect(
                address,
                settings,
                client,
                server_name,
            );
            let host = worker
                .join()
                .map_err(|_error| io::Error::other("TLS worker failed"))?;
            Ok((host, remote))
        })
    }

    #[test]
    fn authenticated_frames_and_identity_survive_reconnect() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server.clone())?;
        let (host, client) = pair(&listener, settings, &keys.client, HOST)?;
        let mut host_stream = host?;
        let mut client_stream = client?;
        require(
            host_stream.peer_fingerprint() == keys.client_id
                && client_stream.peer_fingerprint() == keys.server_id,
            "verified leaf identities",
        )?;
        let mut session = Session::new(session_config(true));
        require(
            session.join(1, PeerKind::Local).is_ok()
                && session.join(2, PeerKind::Lan).is_ok(),
            "local and authenticated LAN players",
        )?;
        client_stream.send(b"12345678")?;
        client_stream.send(b"")?;
        require(host_stream.receive()? == b"12345678", "maximum frame")?;
        require(host_stream.receive()?.is_empty(), "empty frame")?;
        host_stream.send(b"reply")?;
        require(client_stream.receive()? == b"reply", "encrypted reply")?;
        require(
            error_kind(client_stream.send(b"123456789"))
                == Some(ErrorKind::InvalidInput),
            "outgoing budget",
        )?;
        drop(client_stream);
        require(host_stream.receive().is_err(), "lost connection")?;
        require(session.disconnect(2).is_ok(), "reserve identity")?;
        let (new_host, new_client) =
            pair(&listener, settings, &keys.client, HOST)?;
        let mut rejoined = new_host?;
        let mut remote = new_client?;
        require(
            rejoined.peer_fingerprint() == keys.client_id
                && session.reconnect(2, PeerKind::Lan).is_ok(),
            "same authenticated identity reconnects",
        )?;
        remote.send(b"move")?;
        require(
            session.accept(2, &rejoined.receive()?).is_ok(),
            "rejoined frame accepted",
        )
    }

    #[test]
    fn wrong_name_and_untrusted_clients_are_rejected() -> io::Result<()> {
        let issuer = authority()?;
        let outsider = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let (untrusted, _identity) =
            client_credentials(&issuer, &outsider, keys.server_id)?;
        let mut limits = session_config(true);
        limits.max_players = NonZeroU8::MIN;
        let settings = LanConfig::new(limits, TIMEOUT)?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let (host, remote) =
            pair(&listener, settings, &keys.client, "wrong.test")?;
        require(
            host.is_err() && remote.is_err(),
            "wrong host name must not authenticate",
        )?;
        let (rejected_host, _remote) =
            pair(&listener, settings, &untrusted, HOST)?;
        require(
            rejected_host.is_err(),
            "untrusted client must not authenticate",
        )?;
        let (approved, client) = pair(&listener, settings, &keys.client, HOST)?;
        require(
            approved.is_ok() && client.is_ok(),
            "failed permits released",
        )
    }

    #[test]
    fn client_requires_approved_leaf_as_well_as_valid_ca() -> io::Result<()> {
        let issuer = authority()?;
        let outsider = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let different_host = credentials(&issuer, &issuer)?;
        let (untrusted_ca, _identity) =
            client_credentials(&outsider, &issuer, keys.server_id)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let (wrong_leaf_host, wrong_leaf_client) =
            pair(&listener, settings, &different_host.client, HOST)?;
        require(
            wrong_leaf_host.is_err() && wrong_leaf_client.is_err(),
            "valid CA and name cannot replace approved leaf",
        )?;
        let (wrong_ca_host, wrong_ca_client) =
            pair(&listener, settings, &untrusted_ca, HOST)?;
        require(
            wrong_ca_host.is_err() && wrong_ca_client.is_err(),
            "pin cannot replace CA verification",
        )?;
        let (host, client) = pair(&listener, settings, &keys.client, HOST)?;
        require(
            host.is_ok() && client.is_ok(),
            "approved host and CA authenticate",
        )
    }

    #[test]
    fn silent_untrusted_socket_cannot_hold_handshake() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings =
            LanConfig::new(session_config(true), Duration::from_millis(50))?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let _silent = TcpStream::connect(listener.local_addr()?)?;
        let connection = poll_accept(|| listener.try_accept())?;
        let error = error_kind(connection.authenticate());
        require(
            matches!(error, Some(ErrorKind::TimedOut | ErrorKind::WouldBlock)),
            "silent TLS handshake must time out",
        )
    }

    #[test]
    fn pending_socket_budget_releases_on_drop() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let settings = config()?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, keys.server)?;
        let address = listener.local_addr()?;
        let _first_client = TcpStream::connect(address)?;
        let first = poll_accept(|| listener.try_accept())?;
        let _second_client = TcpStream::connect(address)?;
        let _second = poll_accept(|| listener.try_accept())?;
        let _third_client = TcpStream::connect(address)?;
        require(
            listener.try_accept()?.is_none(),
            "pending sockets cannot exceed player budget",
        )?;
        drop(first);
        let _third = poll_accept(|| listener.try_accept())?;
        require(
            listener.try_accept()?.is_none(),
            "dropped pending socket frees exactly one slot",
        )
    }

    #[test]
    fn expired_pending_socket_cannot_authenticate() -> io::Result<()> {
        let issuer = authority()?;
        let keys = credentials(&issuer, &issuer)?;
        let client_settings = config()?;
        let host_settings =
            LanConfig::new(session_config(true), Duration::from_millis(50))?;
        let listener =
            SecureLanListener::bind(LOOPBACK, host_settings, keys.server)?;
        let address = listener.local_addr()?;
        let name = ServerName::try_from(HOST).map_err(io::Error::other)?;
        thread::scope(|scope| {
            let client = scope.spawn(|| {
                SecureFrameStream::connect(
                    address,
                    client_settings,
                    &keys.client,
                    name,
                )
            });
            let pending = poll_accept(|| listener.try_accept())?;
            // Deliberately exceed the real accepted-socket deadline.
            thread::sleep(Duration::from_millis(80));
            let error = error_kind(pending.authenticate());
            let remote = client
                .join()
                .map_err(|_error| io::Error::other("TLS worker failed"))?;
            require(
                error == Some(ErrorKind::TimedOut) && remote.is_err(),
                "queued handshake must not obtain a fresh deadline",
            )
        })
    }
}
