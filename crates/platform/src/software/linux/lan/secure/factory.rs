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
//   - Bind the portable LAN contract to existing authenticated TCP operations.
// - Must-Not:
//   - Select gameplay policy or duplicate credential or framing checks.
// - Allows:
//   - Inputs: Portable credential, endpoint and host scheduling requests.
//   - Outputs: Admitted private native handles and bounded observations.
//   - Side effects: Delegated local credential checks and socket operations.
// - Split-When:
//   - Another native protocol needs independent translation.
// - Merge-When:
//   - Another adapter already supplies equivalent authenticated transport.
// - Summary:
//   - Private adapter implementation of the shared LAN capability.
// - Description:
//   - Routes existing mutual TLS transport through neutral public handles.
// - Usage:
//   - Use only through the portable platform IR.
// - Defaults:
//   - Preserve the existing explicit approval and private endpoint policy.

//! Authenticated native LAN factory and handle implementations.

use std::io::{ErrorKind, Result};
use std::net::SocketAddr;
use std::sync::Arc;

use games::network::MessageSchema;
use rustls::{ClientConfig, ServerConfig};

use super::identity::{LocalIdentity, validate_root};
use super::pump::HostInputPump;
use super::{
    ClientTls as NativeClient, SecureFrameStream, ServerTls as NativeServer,
};
use crate::ir::lan::identity::{IdentityBackend, IdentityPort, PublicIdentity};
use crate::ir::lan::pump::PumpPort;
use crate::ir::lan::{
    ClientTls, CredentialPort, FramePort, LanAvailability, LanConfig,
    LanFactory, ServerTls,
};
use crate::ir::security::lan::host::{ClientApprovals, HostBindRequest};

impl LanAvailability for () {
    const SUPPORTED: bool = true;
}

impl LanFactory for () {
    fn bind<Message, Codec>(
        config: HostBindRequest,
        tls: ServerTls,
        codec: Codec,
    ) -> Result<PumpPort<Message>>
    where
        Message: Send + 'static,
        Codec: MessageSchema<Message = Message> + Send + 'static,
    {
        let native = NativeServer(selected::<ServerConfig>(tls.backend)?);
        Ok(Box::new(HostInputPump::bind(config, native, codec)?))
    }

    fn client_identity(
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<IdentityPort> {
        Ok(Box::new(LocalIdentity::client(
            root_der, leaf_der, key_der,
        )?))
    }

    fn connect(
        address: SocketAddr,
        config: LanConfig,
        tls: &ClientTls,
        name: &str,
    ) -> Result<FramePort> {
        let native =
            NativeClient(selected::<ClientConfig>(Arc::clone(&tls.0))?);
        Ok(Box::new(SecureFrameStream::connect(
            address, config, &native, name,
        )?))
    }

    fn host_identity(
        name: &str,
        root_der: &[u8],
        leaf_der: &[u8],
        key_der: &[u8],
    ) -> Result<IdentityPort> {
        Ok(Box::new(LocalIdentity::host(
            name, root_der, leaf_der, key_der,
        )?))
    }

    fn validate_root(root_der: &[u8]) -> Result<()> {
        validate_root(root_der)
    }
}

impl IdentityBackend for LocalIdentity {
    fn client_credentials(&self, host: &PublicIdentity) -> Result<ClientTls> {
        let native = self.client_tls(host)?;
        Ok(ClientTls(native.0))
    }

    fn public_material(&self) -> &PublicIdentity {
        self.public()
    }

    fn server_credentials(
        &self,
        approvals: &ClientApprovals<'_>,
    ) -> Result<CredentialPort> {
        let native = self.server_tls(approvals)?;
        Ok(native.0)
    }
}

fn selected<Config>(port: CredentialPort) -> Result<Arc<Config>>
where
    Config: Send + Sync + 'static,
{
    port.downcast::<Config>()
        .map_err(|_other| ErrorKind::InvalidInput.into())
}
