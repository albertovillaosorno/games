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
//   - Verifies certificate purpose, root, leaf and key before configuration.
// - Usage:
//   - Exchange public roots and fingerprints through a trusted channel.
// - Defaults:
//   - No credential persistence, network discovery or implicit approval.

//! Independent locally provisioned LAN identities for manual enrolment.

use std::fmt::{self, Debug, Formatter};
use std::io::{self, ErrorKind};
use std::sync::Arc;

use rustls::RootCertStore;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier as _;
use rustls::crypto::ring::default_provider;
use rustls::crypto::ring::sign::any_supported_type;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::sign::CertifiedKey;

use super::{ClientTls, PeerFingerprint, ServerTls};
use crate::ir::lan::identity::PublicIdentity;
use crate::ir::security::lan::CredentialKind;
use crate::ir::security::lan::host::ClientApprovals;

/// Local private leaf credentials with no private-key export operation.
///
/// Accepts a leaf directly signed by an independent local root, with no
/// intermediate chain. Certificate issuance and storage remain external.
pub(super) struct LocalIdentity {
    certified: Arc<CertifiedKey>,
    public: PublicIdentity,
}

impl Debug for LocalIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("fingerprint", &self.public.fingerprint())
            .finish_non_exhaustive()
    }
}

impl LocalIdentity {
    /// Import a distinct client key and currently valid client-auth leaf.
    ///
    /// The encoded key is borrowed; callers own its memory lifetime.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials.
    pub(super) fn client(
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> io::Result<Self> {
        let identity = Self::import(root_der, leaf_der, key_der)?;
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(root_store(identity.public.root_der())?),
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
    /// Rejects invalid TLS configuration; the IR admits the credential role.
    pub(super) fn client_tls(
        &self,
        host: &PublicIdentity,
    ) -> io::Result<ClientTls> {
        ClientTls::from_identity(
            root_store(host.root_der())?,
            Arc::clone(&self.certified),
            host.fingerprint(),
        )
    }

    /// Import a host key and currently valid leaf for the given DNS or IP name.
    ///
    /// The encoded key is borrowed; callers own its memory lifetime.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials,
    /// including certificates issued for a different server name.
    pub(super) fn host(
        name: &str,
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> io::Result<Self> {
        let identity = Self::import(root_der, leaf_der, key_der)?;
        let server_name =
            ServerName::try_from(name).map_err(io::Error::other)?;
        let verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(root_store(identity.public.root_der())?),
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
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> io::Result<Self> {
        CredentialKind::PrivateKey
            .admit(key_der)
            .map_err(|_error| ErrorKind::InvalidInput)?;
        let key = PrivateKeyDer::try_from(key_der).map_err(io::Error::other)?;
        let leaf = certificate(leaf_der)?;
        let public = PublicIdentity::from_der(
            root_der,
            PeerFingerprint::from_der(leaf_der),
        )?;
        let signer = any_supported_type(&key).map_err(io::Error::other)?;
        let checked = CertifiedKey::new(vec![leaf], signer);
        checked.keys_match().map_err(io::Error::other)?;
        Ok(Self {
            certified: Arc::new(checked),
            public,
        })
    }

    /// Public trust material; never contains either private key.
    #[must_use]
    pub(super) const fn public(&self) -> &PublicIdentity {
        &self.public
    }

    /// Configure mandatory client verification using approved device roots.
    ///
    /// The application must also enrol each approved leaf fingerprint in
    /// its `HostSession` before admitting any game messages.
    ///
    /// # Errors
    /// Rejects invalid TLS configuration; the portable facade
    /// has already admitted the role and client list.
    pub(super) fn server_tls(
        &self,
        approvals: &ClientApprovals<'_>,
    ) -> io::Result<ServerTls> {
        let mut roots = RootCertStore::empty();
        for client in approvals.clients() {
            roots
                .add(CertificateDer::from(client.root_der()))
                .map_err(io::Error::other)?;
        }
        ServerTls::from_identity(roots, Arc::clone(&self.certified))
    }
}

fn certificate(der: &[u8]) -> io::Result<CertificateDer<'static>> {
    CredentialKind::Certificate
        .admit(der)
        .map_err(|_error| ErrorKind::InvalidInput)?;
    Ok(CertificateDer::from(der.to_vec()))
}

fn root_store(root_der: &[u8]) -> io::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(root_der))
        .map_err(io::Error::other)?;
    Ok(roots)
}

pub(super) fn validate_root(root_der: &[u8]) -> io::Result<()> {
    root_store(root_der).map(drop)
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind};
    use std::slice::from_ref;

    use games::network::SessionConfig;
    use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair};

    use super::super::super::tests::{
        LOOPBACK, TestCodec, config, require, session_config,
    };
    use super::super::host::HostSession;
    use super::super::tests::{authority, pair};
    use super::super::{PeerFingerprint, SecureLanListener};
    use super::{ClientApprovals, LocalIdentity, PublicIdentity, ServerTls};
    use crate::ir::lan::FrameBackend as _;
    use crate::ir::lan::identity::LocalIdentity as PortableIdentity;

    const HOST: &str = "games.test";

    fn server_tls(
        identity: &LocalIdentity,
        config: SessionConfig,
        clients: &[PublicIdentity],
    ) -> io::Result<ServerTls> {
        identity.server_tls(&ClientApprovals::new(config, clients)?)
    }

    fn key() -> io::Result<KeyPair> {
        KeyPair::generate().map_err(io::Error::other)
    }

    fn device<Identity>(
        host: bool,
        import: impl FnOnce(&[u8], &[u8], &[u8]) -> io::Result<Identity>,
    ) -> io::Result<Identity> {
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
        let encoded = key.serialize_der();
        import(root.der().as_ref(), cert.der().as_ref(), &encoded)
    }

    fn native_device(host: bool) -> io::Result<LocalIdentity> {
        device(host, |root, leaf, key| {
            if host {
                LocalIdentity::host(HOST, root, leaf, key)
            } else {
                LocalIdentity::client(root, leaf, key)
            }
        })
    }

    fn portable_device(host: bool) -> io::Result<PortableIdentity> {
        device(host, |root, leaf, key| {
            if host {
                PortableIdentity::host(HOST, root, leaf, key)
            } else {
                PortableIdentity::client(root, leaf, key)
            }
        })
    }

    #[test]
    fn independent_device_credentials_enable_verified_local_play()
    -> io::Result<()> {
        let host = native_device(true)?;
        let client = native_device(false)?;
        let stranger = native_device(false)?;
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
        let admitted_tls =
            server_tls(&host, session, from_ref(&imported_client))?;
        let listener =
            SecureLanListener::bind(LOOPBACK, settings, admitted_tls)?;
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
        let oversized_key = vec![0; 8193];
        assert!(LocalIdentity::client(&[], &[], &oversized_key).is_err());
    }

    #[test]
    fn wrong_roles_and_duplicate_approvals_fail_closed() -> io::Result<()> {
        let host = portable_device(true)?;
        let client = portable_device(false)?;
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
            LocalIdentity::host(
                HOST,
                root_der,
                leaf_der,
                &wrong.serialize_der(),
            )
            .is_err(),
            "mismatched private key accepted",
        )?;
        require(
            LocalIdentity::host(
                HOST,
                other.der().as_ref(),
                leaf_der,
                &key.serialize_der(),
            )
            .is_err(),
            "untrusted root accepted",
        )?;
        require(
            LocalIdentity::host(
                "wrong.test",
                root_der,
                leaf_der,
                &key.serialize_der(),
            )
            .is_err(),
            "wrong server name accepted",
        )?;
        require(
            LocalIdentity::client(root_der, leaf_der, &key.serialize_der())
                .is_err(),
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
                &key.serialize_der(),
            )
            .is_err(),
            "expired client certificate accepted",
        )
    }
}
