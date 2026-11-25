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
//   - Local credential import and bounded public identity exchange.
// - Must-Not:
//   - Issue certificates, transfer keys or authorize player slots.
// - Allows:
//   - Inputs: Locally provisioned DER keys and approved public identities.
//   - Outputs: Role-specific TLS configurations and public trust anchors.
//   - Side effects: Reads the clock to verify certificate validity.
// - Split-When:
//   - Credential generation or persistence requires an independent boundary.
// - Merge-When:
//   - Another adapter supplies equivalent bounded credential import.
// - Summary:
//   - Import independent LAN identities and assemble explicit trust stores.
// - Description:
//   - Verifies the role, root, leaf and key before creating TLS configs.
// - Usage:
//   - Exchange public roots and fingerprints through a trusted channel.
// - Defaults:
//   - No credential persistence, network discovery or implicit approval.

//! Independent locally provisioned LAN identities for manual enrolment.

use std::collections::BTreeSet;
use std::fmt::{self, Debug, Formatter};
use std::io::{self, ErrorKind};
use std::sync::Arc;

use games::network::SessionConfig;
use rustls::RootCertStore;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier as _;
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::sign::CertifiedKey;

use super::{ClientTls, PeerFingerprint, ServerTls};

const MAX_CERTIFICATE_BYTES: usize = 4096;
const MAX_KEY_BYTES: usize = 8192;

/// Public material to exchange and verify through a trusted local channel.
///
/// The leaf fingerprint is independent of the root's certificate hash.
/// Receiving this value does not itself establish trust or enrol a player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicIdentity {
    fingerprint: PeerFingerprint,
    root: CertificateDer<'static>,
}

impl PublicIdentity {
    /// The fingerprint that must match the verified TLS leaf certificate.
    #[must_use]
    pub const fn fingerprint(&self) -> PeerFingerprint {
        self.fingerprint
    }

    /// Admit one public root certificate and its separately approved pin.
    ///
    /// # Errors
    /// Rejects malformed roots or DER larger than 4,096 bytes.
    pub fn from_der(
        root_der: &[u8],
        fingerprint: PeerFingerprint,
    ) -> io::Result<Self> {
        let root = certificate(root_der)?;
        let identity = Self { fingerprint, root };
        let _validated = identity.roots()?;
        Ok(identity)
    }

    /// DER-encoded public root, suitable for out-of-band exchange.
    #[must_use]
    pub fn root_der(&self) -> &[u8] {
        self.root.as_ref()
    }

    fn roots(&self) -> io::Result<RootCertStore> {
        let mut roots = RootCertStore::empty();
        roots.add(self.root.clone()).map_err(io::Error::other)?;
        Ok(roots)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Client,
    Host,
}

/// Local private leaf credentials with no private-key export operation.
///
/// Accepts a leaf directly signed by an independent local root, with no
/// intermediate chain. Certificate issuance and storage remain external.
pub struct LocalIdentity {
    certified: Arc<CertifiedKey>,
    public: PublicIdentity,
    role: Role,
}

impl Debug for LocalIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("role", &self.role)
            .field("fingerprint", &self.public.fingerprint)
            .finish_non_exhaustive()
    }
}

impl LocalIdentity {
    /// Import a distinct client key and currently valid client-auth leaf.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials.
    pub fn client(
        root_der: &[u8],
        leaf_der: &[u8],
        key: PrivateKeyDer<'static>,
    ) -> io::Result<Self> {
        let identity = Self::import(Role::Client, root_der, leaf_der, key)?;
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(identity.public.roots()?),
            Arc::new(default_provider()),
        )
        .build()
        .map_err(io::Error::other)?;
        let _verified = verifier
            .verify_client_cert(
                identity
                    .certified
                    .end_entity_cert()
                    .map_err(io::Error::other)?,
                &[],
                UnixTime::now(),
            )
            .map_err(io::Error::other)?;
        Ok(identity)
    }

    /// Configure a client to accept only the separately approved host.
    ///
    /// # Errors
    /// Rejects host-role credentials or invalid TLS configuration.
    pub fn client_tls(&self, host: &PublicIdentity) -> io::Result<ClientTls> {
        if self.role != Role::Client {
            return Err(ErrorKind::InvalidInput.into());
        }
        ClientTls::from_identity(
            host.roots()?,
            Arc::clone(&self.certified),
            host.fingerprint,
        )
    }

    /// Import a host key and currently valid leaf for the given DNS or IP name.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials,
    /// including certificates issued for a different server name.
    pub fn host(
        name: &str,
        root_der: &[u8],
        leaf_der: &[u8],
        key: PrivateKeyDer<'static>,
    ) -> io::Result<Self> {
        let identity = Self::import(Role::Host, root_der, leaf_der, key)?;
        let server_name =
            ServerName::try_from(name).map_err(io::Error::other)?;
        let verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(identity.public.roots()?),
            Arc::new(default_provider()),
        )
        .build()
        .map_err(io::Error::other)?;
        let _verified = verifier
            .verify_server_cert(
                identity
                    .certified
                    .end_entity_cert()
                    .map_err(io::Error::other)?,
                &[],
                &server_name,
                &[],
                UnixTime::now(),
            )
            .map_err(io::Error::other)?;
        Ok(identity)
    }

    fn import(
        role: Role,
        root_der: &[u8],
        leaf_der: &[u8],
        key: PrivateKeyDer<'static>,
    ) -> io::Result<Self> {
        if key.secret_der().is_empty() || key.secret_der().len() > MAX_KEY_BYTES
        {
            return Err(ErrorKind::InvalidInput.into());
        }
        let leaf = certificate(leaf_der)?;
        let public = PublicIdentity::from_der(
            root_der,
            PeerFingerprint::from_der(leaf_der),
        )?;
        let checked =
            CertifiedKey::from_der(vec![leaf], key, &default_provider())
                .map_err(io::Error::other)?;
        checked.keys_match().map_err(io::Error::other)?;
        Ok(Self {
            certified: Arc::new(checked),
            public,
            role,
        })
    }

    /// Public trust material; never contains either private key.
    #[must_use]
    pub const fn public(&self) -> &PublicIdentity {
        &self.public
    }

    /// Configure mandatory client verification using approved device roots.
    ///
    /// The application must also enrol each approved leaf fingerprint in
    /// its `HostSession` before admitting any game messages.
    ///
    /// # Errors
    /// Rejects disabled LAN, wrong role, empty or excessive approval lists,
    /// duplicate fingerprints and invalid TLS configuration.
    pub fn server_tls(
        &self,
        config: SessionConfig,
        clients: &[PublicIdentity],
    ) -> io::Result<ServerTls> {
        if !config.allow_lan {
            return Err(ErrorKind::PermissionDenied.into());
        }
        if self.role != Role::Host
            || clients.is_empty()
            || clients.len() > usize::from(config.max_players.get())
        {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut roots = RootCertStore::empty();
        let mut fingerprints = BTreeSet::new();
        for client in clients {
            if !fingerprints.insert(client.fingerprint) {
                return Err(ErrorKind::InvalidInput.into());
            }
            roots.add(client.root.clone()).map_err(io::Error::other)?;
        }
        ServerTls::from_identity(roots, Arc::clone(&self.certified))
    }
}

fn certificate(der: &[u8]) -> io::Result<CertificateDer<'static>> {
    if der.is_empty() || der.len() > MAX_CERTIFICATE_BYTES {
        return Err(ErrorKind::InvalidInput.into());
    }
    Ok(CertificateDer::from(der.to_vec()))
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind};
    use std::slice::from_ref;

    use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair};
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

    use super::super::super::tests::{
        LOOPBACK, TestCodec, config, require, session_config,
    };
    use super::super::host::HostSession;
    use super::super::tests::{authority, pair};
    use super::super::{PeerFingerprint, SecureLanListener};
    use super::{LocalIdentity, PublicIdentity};

    const HOST: &str = "games.test";

    fn key() -> io::Result<KeyPair> {
        KeyPair::generate().map_err(io::Error::other)
    }

    fn der(key: &KeyPair) -> PrivateKeyDer<'static> {
        PrivatePkcs8KeyDer::from(key.serialize_der()).into()
    }

    fn device(host: bool) -> io::Result<LocalIdentity> {
        let root = authority()?;
        let key = key()?;
        let mut params = if host {
            CertificateParams::new(vec![String::from(HOST)])
                .map_err(io::Error::other)?
        } else {
            CertificateParams::default()
        };
        params.extended_key_usages = vec![if host {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        let cert = params.signed_by(&key, &root).map_err(io::Error::other)?;
        if host {
            LocalIdentity::host(
                HOST,
                root.der().as_ref(),
                cert.der().as_ref(),
                der(&key),
            )
        } else {
            LocalIdentity::client(
                root.der().as_ref(),
                cert.der().as_ref(),
                der(&key),
            )
        }
    }

    #[test]
    fn independent_device_credentials_enable_verified_local_play()
    -> io::Result<()> {
        let host = device(true)?;
        let client = device(false)?;
        let stranger = device(false)?;
        require(
            client.public() != stranger.public(),
            "independent clients reused credentials",
        )?;
        let imported_host = PublicIdentity::from_der(
            host.public().root_der(),
            host.public().fingerprint(),
        )?;
        let imported_client = PublicIdentity::from_der(
            client.public().root_der(),
            client.public().fingerprint(),
        )?;
        let settings = config()?;
        let session = session_config(true);
        let server_tls =
            host.server_tls(session, from_ref(&imported_client))?;
        let listener = SecureLanListener::bind(LOOPBACK, settings, server_tls)?;
        let client_tls = client.client_tls(&imported_host)?;
        let mut roster =
            HostSession::new(session, &[(imported_client.fingerprint(), 2)])
                .map_err(|_error| io::Error::other("invalid enrolment"))?;
        require(roster.join_local(1).is_ok(), "local player rejected")?;
        let (inbound, outbound) = pair(&listener, settings, &client_tls, HOST)?;
        let mut remote = outbound?;
        require(roster.admit(inbound?) == Ok(2), "approved peer rejected")?;
        remote.send(b"M\x05")?;
        require(roster.receive(2, &TestCodec) == Ok(5), "action rejected")?;
        let outsider_tls = stranger.client_tls(&imported_host)?;
        let (outsider, _remote) =
            pair(&listener, settings, &outsider_tls, HOST)?;
        require(outsider.is_err(), "unapproved root accepted")
    }

    #[test]
    fn public_root_and_key_admission_are_bounded() {
        let pin = PeerFingerprint::from_bytes([0; 32]);
        assert!(PublicIdentity::from_der(&[], pin).is_err());
        assert!(PublicIdentity::from_der(b"invalid", pin).is_err());
        assert!(PublicIdentity::from_der(&[0; 4097], pin).is_err());
        let oversized_key = PrivatePkcs8KeyDer::from(vec![0; 8193]).into();
        assert!(LocalIdentity::client(&[], &[], oversized_key).is_err());
    }

    #[test]
    fn wrong_roles_and_duplicate_approvals_fail_closed() -> io::Result<()> {
        let host = device(true)?;
        let client = device(false)?;
        let approved = from_ref(client.public());
        let duplicate = [client.public().clone(), client.public().clone()];
        let session = session_config(true);
        require(
            host.client_tls(host.public()).is_err()
                && client.server_tls(session, approved).is_err()
                && host.server_tls(session, &[]).is_err()
                && host.server_tls(session, &duplicate).is_err(),
            "invalid role or approval list accepted",
        )?;
        require(
            host.server_tls(session_config(false), approved)
                .err()
                .is_some_and(|error| {
                    error.kind() == ErrorKind::PermissionDenied
                }),
            "LAN opt-in was bypassed",
        )
    }

    #[test]
    fn invalid_credential_combinations_are_rejected() -> io::Result<()> {
        let root = authority()?;
        let other = authority()?;
        let key = key()?;
        let wrong = KeyPair::generate().map_err(io::Error::other)?;
        let mut params = CertificateParams::new(vec![String::from(HOST)])
            .map_err(io::Error::other)?;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, &root).map_err(io::Error::other)?;
        let root_der = root.der().as_ref();
        let leaf_der = cert.der().as_ref();
        require(
            LocalIdentity::host(HOST, root_der, leaf_der, der(&wrong)).is_err(),
            "mismatched private key accepted",
        )?;
        require(
            LocalIdentity::host(
                HOST,
                other.der().as_ref(),
                leaf_der,
                der(&key),
            )
            .is_err(),
            "untrusted root accepted",
        )?;
        require(
            LocalIdentity::host("wrong.test", root_der, leaf_der, der(&key))
                .is_err(),
            "wrong server name accepted",
        )?;
        require(
            LocalIdentity::client(root_der, leaf_der, der(&key)).is_err(),
            "server-only leaf accepted for client authentication",
        )
    }

    #[test]
    fn expired_device_leaf_cannot_be_imported() -> io::Result<()> {
        let root = authority()?;
        let key = key()?;
        let mut params = CertificateParams::default();
        params.not_before = rcgen::date_time_ymd(2000, 1, 1);
        params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = params.signed_by(&key, &root).map_err(io::Error::other)?;
        require(
            LocalIdentity::client(
                root.der().as_ref(),
                cert.der().as_ref(),
                der(&key),
            )
            .is_err(),
            "expired client certificate accepted",
        )
    }
}
