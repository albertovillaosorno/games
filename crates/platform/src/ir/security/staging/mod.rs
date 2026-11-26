// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Portable budget, payload integrity and canonical manifest admission.
// - Must-Not:
//   - Open storage, choose native paths or publish assets.
// - Allows:
//   - Inputs: Explicit budgets, verified manifests and immutable bytes.
//   - Outputs: Admitted payload and exact manifest values.
//   - Side effects: Portable accounting only.
// - Split-When:
//   - Independent staging policies require different accounting.
// - Merge-When:
//   - Another shared policy owns equivalent canonical admission.
// - Summary:
//   - Bind each canonical payload to its verified identity and limits.
// - Description:
//   - Reserve resources before hashing and check the complete inventory.
// - Usage:
//   - Admit payloads before dispatch and verify manifests before finishing.
// - Defaults:
//   - Reject duplicates, exhausted budgets and integrity mismatches.

//! Shared canonical staging admission.

use std::collections::BTreeMap;

use games_assets::archive::{ArchiveBudget, EntryKind};
use games_assets::{AssetError, ContentHash};

use crate::ir::storage::bundle::{MANIFEST_NAME, PAYLOADS_DIRECTORY};
use crate::ir::storage::staging::{PreparedManifest, StagingError};

/// Portable accounting and integrity admission for canonical staging.
#[derive(Debug)]
pub(crate) struct CanonicalAdmission {
    budget: ArchiveBudget,
    payloads: BTreeMap<ContentHash, u64>,
}

impl CanonicalAdmission {
    pub(crate) fn complete<'manifest>(
        &self,
        manifest: &'manifest PreparedManifest,
    ) -> Result<AdmittedManifest<'manifest>, StagingError> {
        if manifest.payload_sizes() != &self.payloads {
            return Err(AssetError::PayloadMismatch.into());
        }
        let size = manifest_size(manifest.bytes())?;
        let mut budget = self.budget;
        let _entry = budget.admit(MANIFEST_NAME, EntryKind::File, size)?;
        Ok(AdmittedManifest { budget, manifest, size })
    }

    pub(crate) fn contains(&self, digest: ContentHash) -> bool {
        self.payloads.contains_key(&digest)
    }

    pub(crate) const fn new(budget: ArchiveBudget) -> Self {
        Self {
            budget,
            payloads: BTreeMap::new(),
        }
    }

    pub(crate) fn payload<'bytes>(
        &mut self,
        digest: ContentHash,
        bytes: &'bytes [u8],
    ) -> Result<AdmittedPayload<'bytes>, StagingError> {
        if self.contains(digest) {
            return Err(StagingError::Collision);
        }
        let size = manifest_size(bytes)?;
        let mut budget = self.budget;
        reserve_payload(&mut budget, digest, size, self.payloads.is_empty())?;
        if ContentHash::calculate(bytes) != digest {
            return Err(AssetError::PayloadMismatch.into());
        }
        self.budget = budget;
        let _previous = self.payloads.insert(digest, size);
        Ok(AdmittedPayload {
            budget,
            bytes,
            digest,
            size,
        })
    }

    pub(crate) fn preflight(
        mut budget: ArchiveBudget,
        manifest: &PreparedManifest,
    ) -> Result<(), StagingError> {
        let _entry = budget.admit(
            MANIFEST_NAME,
            EntryKind::File,
            manifest_size(manifest.bytes())?,
        )?;
        for (index, (&digest, &size)) in
            manifest.payload_sizes().iter().enumerate()
        {
            reserve_payload(&mut budget, digest, size, index == 0)?;
        }
        Ok(())
    }
}

/// One immutable payload bound to its verified digest and budget.
#[derive(Clone, Copy, Debug)]
pub struct AdmittedPayload<'bytes> {
    budget: ArchiveBudget,
    bytes: &'bytes [u8],
    digest: ContentHash,
    size: u64,
}

impl<'bytes> AdmittedPayload<'bytes> {
    /// The remaining transaction budget after this payload.
    #[must_use]
    pub const fn budget(&self) -> ArchiveBudget {
        self.budget
    }

    /// The exact verified bytes.
    #[must_use]
    pub const fn bytes(&self) -> &'bytes [u8] {
        self.bytes
    }

    /// The admitted content identity.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.digest
    }

    /// The exact admitted byte length.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// Exact encoded metadata matching the entire admitted payload inventory.
#[derive(Clone, Copy, Debug)]
pub struct AdmittedManifest<'manifest> {
    budget: ArchiveBudget,
    manifest: &'manifest PreparedManifest,
    size: u64,
}

impl AdmittedManifest<'_> {
    /// The remaining transaction budget after metadata admission.
    #[must_use]
    pub const fn budget(&self) -> ArchiveBudget {
        self.budget
    }

    /// The exact encoded manifest bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.manifest.bytes()
    }

    /// The admitted content identity.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.manifest.digest()
    }

    /// The exact admitted byte length.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

fn reserve_payload(
    budget: &mut ArchiveBudget,
    digest: ContentHash,
    size: u64,
    first: bool,
) -> Result<(), StagingError> {
    let path = payload_path(digest);
    let _payload = budget.admit(&path, EntryKind::File, size)?;
    if first {
        budget
            .admit(PAYLOADS_DIRECTORY, EntryKind::Directory, 0)?
            .finish()?;
    }
    Ok(())
}

pub(crate) fn manifest_size(bytes: &[u8]) -> Result<u64, AssetError> {
    u64::try_from(bytes.len()).map_err(|_error| AssetError::TooLarge)
}

pub(crate) fn payload_path(digest: ContentHash) -> String {
    format!("{PAYLOADS_DIRECTORY}/{digest}")
}
