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
//   - Portable public identity and opaque private credential handles.
// - Must-Not:
//   - Expose signing keys, generate credentials or establish implicit trust.
// - Allows:
//   - Inputs: Bounded locally supplied DER and approved fingerprints.
//   - Outputs: Public enrolment material and admitted transport credentials.
//   - Side effects: Delegated credential verification only.
// - Split-When:
//   - Another credential family requires separate admission.
// - Merge-When:
//   - The existing identity contract expresses the same trust requirements.
// - Summary:
//   - Explicit local enrolment without exposing a backend or private key.
// - Description:
//   - Separates portable public bytes from opaque verified credentials.
// - Usage:
//   - Provision identities out of band and admit approved trust explicitly.
// - Defaults:
//   - No credential persistence, public trust store or implicit approval.

//! Portable public and locally provisioned private identities.

use std::fmt::{self, Debug, Formatter};
use std::io::{ErrorKind, Result};

use games::network::SessionConfig;

use super::{ClientTls, LanFactory, PeerFingerprint, ServerTls};
use crate::ir::security::lan::CredentialKind;

/// Public material to exchange and verify through a trusted local channel.
/// Receiving this value does not establish trust or enrol a player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicIdentity {
    fingerprint: PeerFingerprint,
    root: Vec<u8>,
}

impl PublicIdentity {
    /// The fingerprint that must match the verified TLS leaf certificate.
    #[must_use]
    pub const fn fingerprint(&self) -> PeerFingerprint {
        self.fingerprint
    }

    /// Admit a public root certificate and its separately approved pin.
    ///
    /// # Errors
    /// Rejects empty, oversized or malformed roots or absent verification.
    pub fn from_der(
        root_der: &[u8],
        fingerprint: PeerFingerprint,
    ) -> Result<Self> {
        CredentialKind::Certificate
            .admit(root_der)
            .map_err(|_error| ErrorKind::InvalidInput)?;
        <() as LanFactory>::validate_root(root_der)?;
        Ok(Self {
            fingerprint,
            root: root_der.to_vec(),
        })
    }

    /// DER-encoded public root, suitable for out-of-band exchange.
    #[must_use]
    pub fn root_der(&self) -> &[u8] {
        &self.root
    }
}

/// Private verified identity port; never exposes a private key.
pub(crate) trait IdentityBackend: Send + Sync {
    fn client_credentials(&self, host: &PublicIdentity) -> Result<ClientTls>;
    fn public_material(&self) -> &PublicIdentity;
    fn server_credentials(
        &self,
        config: SessionConfig,
        clients: &[PublicIdentity],
    ) -> Result<ServerTls>;
}

pub(crate) type IdentityPort = Box<dyn IdentityBackend>;

/// Locally provisioned private identity with no private-key export operation.
/// Certificate issuance and storage remain external.
pub struct LocalIdentity {
    backend: IdentityPort,
}

impl Debug for LocalIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("fingerprint", &self.public().fingerprint())
            .finish_non_exhaustive()
    }
}

impl LocalIdentity {
    /// Import a distinct client key and currently valid client-auth leaf.
    /// The encoded key is borrowed; callers own its memory lifetime.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials,
    /// or an unavailable verification backend.
    pub fn client(
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<Self> {
        admit_credentials(root_der, leaf_der, key_der)?;
        let backend =
            <() as LanFactory>::client_identity(root_der, leaf_der, key_der)?;
        Ok(Self { backend })
    }

    /// Configure a client to accept only the separately approved host.
    ///
    /// # Errors
    /// Rejects host-role credentials or invalid TLS configuration.
    pub fn client_tls(&self, host: &PublicIdentity) -> Result<ClientTls> {
        self.backend.client_credentials(host)
    }

    /// Import a host key and currently valid leaf for the given DNS or IP name.
    /// The encoded key is borrowed; callers own its memory lifetime.
    ///
    /// # Errors
    /// Rejects malformed, oversized, mismatched or untrusted credentials,
    /// including certificates issued for a different server name.
    pub fn host(
        name: &str,
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<Self> {
        admit_credentials(root_der, leaf_der, key_der)?;
        let backend = <() as LanFactory>::host_identity(
            name, root_der, leaf_der, key_der,
        )?;
        Ok(Self { backend })
    }

    /// Public trust material; never contains either private key.
    #[must_use]
    pub fn public(&self) -> &PublicIdentity {
        self.backend.public_material()
    }

    /// Configure mandatory client verification with approved device roots.
    ///
    /// # Errors
    /// Rejects disabled LAN, wrong role, empty or excessive approval lists,
    /// duplicate fingerprints and invalid TLS configuration.
    pub fn server_tls(
        &self,
        config: SessionConfig,
        clients: &[PublicIdentity],
    ) -> Result<ServerTls> {
        self.backend.server_credentials(config, clients)
    }
}

fn admit_credentials(root: &[u8], leaf: &[u8], key: &[u8]) -> Result<()> {
    CredentialKind::Certificate
        .admit(root)
        .map_err(|_error| ErrorKind::InvalidInput)?;
    CredentialKind::Certificate
        .admit(leaf)
        .map_err(|_error| ErrorKind::InvalidInput)?;
    CredentialKind::PrivateKey
        .admit(key)
        .map_err(|_error| ErrorKind::InvalidInput)?;
    Ok(())
}
