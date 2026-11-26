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
//   - Portable public identity, credential roles and opaque private handles.
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
//   - Admits role-specific use of opaque verified credentials.
// - Usage:
//   - Provision identities out of band and admit approved trust explicitly.
// - Defaults:
//   - No credential persistence, public trust store or implicit approval.

//! Portable public and locally provisioned private identities.

use std::fmt::{self, Debug, Formatter};
use std::io::{ErrorKind, Result};
use std::sync::Arc;

use games::network::SessionConfig;

use super::{
    ClientTls, CredentialPort, LanFactory, PeerFingerprint, ServerTls,
};
use crate::ir::security::lan::CredentialKind;
use crate::ir::security::lan::host::ClientApprovals;

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
        approvals: &ClientApprovals<'_>,
    ) -> Result<CredentialPort>;
}

pub(crate) type IdentityPort = Box<dyn IdentityBackend>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityRole {
    Client,
    Host,
}

impl IdentityRole {
    fn admit(self, expected: Self) -> Result<()> {
        if self != expected {
            return Err(ErrorKind::InvalidInput.into());
        }
        Ok(())
    }
}

/// Locally provisioned private identity with no private-key export operation.
/// Certificate issuance and storage remain external.
pub struct LocalIdentity {
    backend: IdentityPort,
    role: IdentityRole,
}

impl Debug for LocalIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("fingerprint", &self.public().fingerprint())
            .field("role", &self.role)
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
        Ok(Self {
            backend,
            role: IdentityRole::Client,
        })
    }

    /// Configure a client to accept only the separately approved host.
    ///
    /// # Errors
    /// Rejects host-role credentials or invalid TLS configuration.
    pub fn client_tls(&self, host: &PublicIdentity) -> Result<ClientTls> {
        self.role.admit(IdentityRole::Client)?;
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
        Ok(Self {
            backend,
            role: IdentityRole::Host,
        })
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
        let approvals = ClientApprovals::new(config, clients)?;
        self.role.admit(IdentityRole::Host)?;
        let backend = self.backend.server_credentials(&approvals)?;
        Ok(ServerTls {
            approved: Arc::new(approvals.into_pins()),
            backend,
        })
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::Error;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct ObservedIdentity {
        dispatched: Arc<AtomicBool>,
        public: PublicIdentity,
    }

    impl IdentityBackend for ObservedIdentity {
        fn client_credentials(
            &self,
            _host: &PublicIdentity,
        ) -> Result<ClientTls> {
            self.dispatched.store(true, Ordering::Release);
            Ok(ClientTls(Arc::new(())))
        }

        fn public_material(&self) -> &PublicIdentity {
            &self.public
        }

        fn server_credentials(
            &self,
            approvals: &ClientApprovals<'_>,
        ) -> Result<CredentialPort> {
            self.dispatched
                .store(!approvals.clients().is_empty(), Ordering::Release);
            Ok(Arc::new(()))
        }
    }

    fn public(tag: u8) -> PublicIdentity {
        PublicIdentity {
            fingerprint: PeerFingerprint::from_bytes([tag; 32]),
            root: vec![tag],
        }
    }

    fn fixture(role: IdentityRole) -> (LocalIdentity, Arc<AtomicBool>) {
        let dispatched = Arc::new(AtomicBool::new(false));
        let backend = Box::new(ObservedIdentity {
            dispatched: Arc::clone(&dispatched),
            public: public(0),
        });
        (LocalIdentity { backend, role }, dispatched)
    }

    fn config() -> SessionConfig {
        SessionConfig {
            allow_lan: true,
            max_message_bytes: NonZeroUsize::MIN,
            max_players: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        }
    }

    #[test]
    fn opposite_credential_roles_never_dispatch() {
        let (host, host_dispatch) = fixture(IdentityRole::Host);
        let (client, client_dispatch) = fixture(IdentityRole::Client);
        let approved = public(1);
        assert_eq!(
            host.client_tls(&approved).err().map(|error| error.kind()),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(
            client
                .server_tls(config(), &[approved])
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidInput)
        );
        assert!(!host_dispatch.load(Ordering::Acquire));
        assert!(!client_dispatch.load(Ordering::Acquire));
    }

    #[test]
    fn client_credentials_dispatch_only_for_a_client_identity() {
        let (client, dispatched) = fixture(IdentityRole::Client);
        assert!(client.client_tls(&public(1)).is_ok());
        assert!(dispatched.load(Ordering::Acquire));
    }

    #[test]
    fn duplicate_or_excessive_approvals_never_dispatch() {
        let (identity, dispatched) = fixture(IdentityRole::Host);
        for approvals in [vec![], vec![public(1), public(1)], vec![
            public(1),
            public(2),
            public(3),
        ]] {
            assert_eq!(
                identity
                    .server_tls(config(), &approvals)
                    .err()
                    .map(|error| error.kind()),
                Some(ErrorKind::InvalidInput)
            );
        }
        assert!(!dispatched.load(Ordering::Acquire));
    }

    #[test]
    fn disabled_approval_never_dispatches() {
        let (identity, dispatched) = fixture(IdentityRole::Host);
        let mut disabled = config();
        disabled.allow_lan = false;
        assert_eq!(
            identity
                .server_tls(disabled, &[public(1)])
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::PermissionDenied)
        );
        assert!(!dispatched.load(Ordering::Acquire));
    }

    #[test]
    fn admitted_credential_handle_retains_only_approved_pins() -> Result<()> {
        let (identity, dispatched) = fixture(IdentityRole::Host);
        let tls = identity.server_tls(config(), &[public(1), public(2)])?;
        let expected =
            BTreeSet::from([public(1).fingerprint(), public(2).fingerprint()]);
        if !dispatched.load(Ordering::Acquire)
            || tls.approved.as_ref() != &expected
        {
            return Err(Error::other("admitted credential pins changed"));
        }
        Ok(())
    }
}
