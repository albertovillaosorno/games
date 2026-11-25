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
//   - Explicit native LAN configuration and local credential selection.
// - Must-Not:
//   - Issue credentials, perform TLS verification or simulate gameplay.
// - Allows:
//   - Inputs: An explicitly selected bounded host catalogue and DER files.
//   - Outputs: A bound host with verified trust and assigned remote seats.
//   - Side effects: Bounded local file reads and explicit listener creation.
// - Split-When:
//   - Native remote controller presentation needs an independent module.
// - Merge-When:
//   - Multiple titles use the same explicit native credential catalogue.
// - Summary:
//   - Configure authenticated shared-screen LAN play without implicit trust.
// - Description:
//   - Anchor selected paths to the catalogue and protect private key reads.
// - Usage:
//   - Supply --lan-host FILE with an explicit address and approved peers.
// - Defaults:
//   - No network, credential generation, discovery or implicit approval.

use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::time::Duration;

use games_platform::lan::LanConfig;
use games_platform::lan::secure::identity::{LocalIdentity, PublicIdentity};
use games_platform::lan::secure::{PeerFingerprint, SecureFrameStream};
use games_platform_storage::file::{snapshot, snapshot_secret};
use rustls::pki_types::{PrivateKeyDer, ServerName};

use super::launch::{Fields, fields, required};
use crate::lan::{HostLan, session_config};

const MAX_CATALOGUE: NonZeroU64 = NonZeroU64::MIN.saturating_add(4095);
const MAX_CERTIFICATE: NonZeroU64 = NonZeroU64::MIN.saturating_add(4095);
const MAX_KEY: NonZeroU64 = NonZeroU64::MIN.saturating_add(8191);

#[derive(Debug, Eq, PartialEq)]
struct PublicFiles {
    leaf: PathBuf,
    root: PathBuf,
}

impl PublicFiles {
    fn load(&self) -> Result<PublicIdentity, String> {
        let root = certificate(&self.root)?;
        let leaf = certificate(&self.leaf)?;
        PublicIdentity::from_der(&root, PeerFingerprint::from_der(&leaf))
            .map_err(|error| format!("public identity: {error}"))
    }

    fn parse(
        base: &Path,
        values: &mut Fields<'_>,
        prefix: &str,
    ) -> Result<Option<Self>, String> {
        match (
            values.remove(format!("{prefix}leaf").as_str()),
            values.remove(format!("{prefix}root").as_str()),
        ) {
            (None, None) => Ok(None),
            (Some(leaf), Some(root)) => Ok(Some(Self {
                leaf: base.join(leaf),
                root: base.join(root),
            })),
            _ => Err(String::from("both public certificate paths required")),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct LocalFiles {
    key: PathBuf,
    public: PublicFiles,
}

impl LocalFiles {
    fn identity(&self, name: Option<&str>) -> Result<LocalIdentity, String> {
        let root = certificate(&self.public.root)?;
        let leaf = certificate(&self.public.leaf)?;
        let key = snapshot_secret(&self.key, MAX_KEY)
            .map_err(|error| format!("private key admission: {error:?}"))?;
        let private = PrivateKeyDer::try_from(key)
            .map_err(|_error| String::from("invalid DER private key"))?;
        match name {
            Some(server) => LocalIdentity::host(server, &root, &leaf, private),
            None => LocalIdentity::client(&root, &leaf, private),
        }
        .map_err(|error| format!("local identity: {error}"))
    }

    fn parse(base: &Path, values: &mut Fields<'_>) -> Result<Self, String> {
        let public = PublicFiles::parse(base, values, "")?
            .ok_or("missing local certificate paths")?;
        Ok(Self {
            key: base.join(required(values, "key")?),
            public,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Endpoint {
    address: SocketAddr,
    identity: LocalFiles,
    name: String,
}

impl Endpoint {
    fn parse(base: &Path, values: &mut Fields<'_>) -> Result<Self, String> {
        if values.remove("version") != Some("1") {
            return Err(String::from("expected LAN catalogue version=1"));
        }
        let address = required(values, "address")?
            .parse()
            .map_err(|_error| String::from("invalid private LAN address"))?;
        let name = required(values, "name")?.to_owned();
        let identity = LocalFiles::parse(base, values)?;
        Ok(Self { address, identity, name })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct HostSelection {
    address: SocketAddr,
    identity: LocalFiles,
    name: String,
    peers: [Option<PublicFiles>; 4],
}

impl HostSelection {
    fn bind(&self) -> Result<HostLan, String> {
        let identity = self.identity.identity(Some(&self.name))?;
        let mut approved = [None; 4];
        let mut public = Vec::new();
        for (peer, slot) in self.peers.iter().zip(&mut approved) {
            if let Some(files) = peer {
                let client = files.load()?;
                *slot = Some(client.fingerprint());
                public.push(client);
            }
        }
        let tls = identity
            .server_tls(session_config(), &public)
            .map_err(|error| format!("host trust: {error}"))?;
        HostLan::bind(self.address, tls, &approved)
            .map_err(|error| format!("LAN host: {error}"))
    }

    fn parse(path: &Path, text: &str) -> Result<Self, String> {
        let base = path.parent().ok_or("invalid LAN catalogue path")?;
        let mut values = fields(text)?;
        let Endpoint { address, identity, name } =
            Endpoint::parse(base, &mut values)?;
        let mut peers = [None, None, None, None];
        for (seat, peer) in (1u8..=4).zip(&mut peers) {
            *peer =
                PublicFiles::parse(base, &mut values, &format!("peer{seat}_"))?;
        }
        let remotes = peers.iter().filter(|peer| peer.is_some()).count();
        if !values.is_empty() || !(1..=3).contains(&remotes) {
            return Err(String::from("require one to three approved peers"));
        }
        Ok(Self {
            address,
            identity,
            name,
            peers,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ClientSelection {
    address: SocketAddr,
    host: PublicFiles,
    identity: LocalFiles,
    name: String,
}

impl ClientSelection {
    fn connect(&self) -> Result<SecureFrameStream, String> {
        let identity = self.identity.identity(None)?;
        let host = self.host.load()?;
        let tls = identity
            .client_tls(&host)
            .map_err(|error| format!("client trust: {error}"))?;
        let name = ServerName::try_from(self.name.clone())
            .map_err(|error| error.to_string())?;
        let config = LanConfig::new(session_config(), Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        SecureFrameStream::connect(self.address, config, &tls, name)
            .map_err(|error| format!("LAN controller: {error}"))
    }

    fn parse(path: &Path, text: &str) -> Result<Self, String> {
        let base = path.parent().ok_or("invalid LAN catalogue path")?;
        let mut values = fields(text)?;
        let Endpoint { address, identity, name } =
            Endpoint::parse(base, &mut values)?;
        let host = PublicFiles::parse(base, &mut values, "host_")?
            .ok_or("missing approved host certificate paths")?;
        if !values.is_empty() {
            return Err(String::from("unknown controller field"));
        }
        Ok(Self {
            address,
            host,
            identity,
            name,
        })
    }
}

pub(super) fn open_client(path: &Path) -> Result<SecureFrameStream, String> {
    ClientSelection::parse(path, &catalogue(path)?)?.connect()
}

fn certificate(path: &Path) -> Result<Vec<u8>, String> {
    snapshot(path, MAX_CERTIFICATE)
        .map_err(|error| format!("certificate {}: {error:?}", path.display()))
}

fn catalogue(path: &Path) -> Result<String, String> {
    let bytes = snapshot(path, MAX_CATALOGUE)
        .map_err(|error| format!("LAN catalogue: {error:?}"))?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

pub(super) fn open_host(path: &Path) -> Result<HostLan, String> {
    HostSelection::parse(path, &catalogue(path)?)?.bind()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = concat!(
        "version=1\n",
        "address=127.0.0.1:0\n",
        "name=games.test\n",
        "root=host-root.der\n",
        "leaf=host-leaf.der\n",
        "key=host-key.der\n",
    );

    #[test]
    fn explicit_catalogue_reserves_only_named_remote_seats()
    -> Result<(), String> {
        let path = Path::new("/private/lan/host.local");
        let text = format!(
            "{BASE}peer2_root=guest-root.der\n\
                            peer2_leaf=guest-leaf.der\n"
        );
        let selection = HostSelection::parse(path, &text)?;
        let expected = PublicFiles {
            leaf: PathBuf::from("/private/lan/guest-leaf.der"),
            root: PathBuf::from("/private/lan/guest-root.der"),
        };
        if selection.peers != [None, Some(expected), None, None]
            || selection.identity.key != Path::new("/private/lan/host-key.der")
            || selection.address != SocketAddr::from(([127, 0, 0, 1], 0))
        {
            return Err(String::from("incorrect LAN selection"));
        }
        Ok(())
    }

    #[test]
    fn catalogue_rejects_implicit_incomplete_or_unknown_approval() {
        let path = Path::new("host.local");
        for suffix in [
            "",
            "peer2_root=root.der\n",
            "peer2_leaf=leaf.der\n",
            "peer5_root=root.der\npeer5_leaf=leaf.der\n",
            "peer2_root=r\npeer2_leaf=l\nkey=override\n",
            "peer2_root=r\npeer2_leaf=l\nunknown=value\n",
        ] {
            assert!(
                HostSelection::parse(path, &format!("{BASE}{suffix}")).is_err()
            );
        }
    }

    #[test]
    fn controller_requires_explicit_host_and_local_identity()
    -> Result<(), String> {
        let path = Path::new("client/controller.local");
        let text = format!(
            "{BASE}host_root=approved-root.der\nhost_leaf=approved-leaf.der\n"
        );
        let selection = ClientSelection::parse(path, &text)?;
        if selection.host.root != Path::new("client/approved-root.der")
            || selection.host.leaf != Path::new("client/approved-leaf.der")
        {
            return Err(String::from("host paths not anchored"));
        }
        for invalid in [
            String::from(BASE),
            format!("{BASE}host_leaf=l\n"),
            format!("{text}peer2_leaf=l\n"),
            format!("{text}name=override\n"),
        ] {
            if ClientSelection::parse(path, &invalid).is_ok() {
                return Err(String::from("invalid controller trust accepted"));
            }
        }
        Ok(())
    }
}
