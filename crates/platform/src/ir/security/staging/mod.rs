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
//   - Portable archive paths, payload budgets and publication admission.
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
//   - Bind staged members and destinations to shared admission policy.
// - Description:
//   - Check paths and inventory before dispatching storage operations.
// - Usage:
//   - Admit members and destinations before storage dispatch.
// - Defaults:
//   - Reject duplicates, exhausted budgets and integrity mismatches.

//! Shared private staging and publication admission.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::OsStr;

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryKind, validate_path,
};
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

/// An independently authored file with an admitted portable path and bytes.
#[derive(Debug)]
pub struct AdmittedAuthoredFile<'input> {
    bytes: &'input [u8],
    executable: bool,
    leaf: &'input str,
    parents: &'input str,
    plan: NamespacePlan<'input>,
    size: u64,
}

impl<'input> AdmittedAuthoredFile<'input> {
    /// Exact independently authored bytes.
    #[must_use]
    pub const fn bytes(&self) -> &'input [u8] {
        self.bytes
    }

    /// Whether the owner execute bit was explicitly requested.
    #[must_use]
    pub const fn executable(&self) -> bool {
        self.executable
    }

    pub(crate) fn new(
        path: &'input str,
        bytes: &'input [u8],
        executable: bool,
        namespace: &mut Namespace,
    ) -> Result<Self, StagingError> {
        validate_path(path)?;
        let (parents, leaf) = staging_path(path)?;
        let size = u64::try_from(bytes.len())
            .map_err(|_error| StagingError::ChangedStorage)?;
        let plan = namespace.admit((parents, leaf), EntryKind::File, None)?;
        Ok(Self {
            bytes,
            executable,
            leaf,
            parents,
            plan,
            size,
        })
    }

    /// Admitted parent components and final filename.
    #[must_use]
    pub const fn parts(&self) -> (&'input str, &'input str) {
        (self.parents, self.leaf)
    }

    /// Exact admitted traversal for these bytes.
    #[must_use]
    pub const fn plan(&self) -> &NamespacePlan<'input> {
        &self.plan
    }

    /// Exact admitted length.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// One portable component admitted as a new publication directory name.
#[derive(Clone, Copy, Debug)]
pub struct PublicationName<'name>(&'name OsStr);

impl<'name> PublicationName<'name> {
    pub(crate) fn new(name: &'name OsStr) -> Result<Self, StagingError> {
        let component = name.to_str().ok_or(StagingError::UnsupportedPath)?;
        if !component.is_ascii()
            || component.contains('/')
            || validate_path(component).is_err()
        {
            return Err(StagingError::UnsupportedPath);
        }
        Ok(Self(name))
    }

    /// The exact admitted destination component.
    #[must_use]
    pub const fn value(&self) -> &'name OsStr {
        self.0
    }
}

/// Portable directory namespace shared by all private staging transactions.
#[derive(Debug, Default)]
pub struct Namespace {
    next_id: usize,
    root: NamespaceDirectory,
}

impl Namespace {
    /// Admit a member before dispatch; discard this namespace on failure.
    pub(crate) fn admit<'path>(
        &mut self,
        parts: PathParts<'path>,
        kind: EntryKind,
        budget: Option<&mut ArchiveBudget>,
    ) -> Result<NamespacePlan<'path>, StagingError> {
        let (parents, leaf) = parts;
        let mut admission = NamespaceAdmission {
            budget,
            next_id: &mut self.next_id,
        };
        let mut directory = &mut self.root;
        let mut steps = Vec::new();
        if !parents.is_empty() {
            for part in parents.split('/') {
                let (step, node) = directory.admit_child(
                    &mut admission,
                    part,
                    NamespaceMember::Implicit,
                )?;
                directory =
                    node.directory.as_mut().ok_or(StagingError::Collision)?;
                steps.push(step);
            }
        }
        let member = if kind == EntryKind::Directory {
            NamespaceMember::Explicit
        } else {
            NamespaceMember::File
        };
        let (last, _) = directory.admit_child(&mut admission, leaf, member)?;
        Ok(NamespacePlan {
            leaf: last,
            parents: steps,
        })
    }

    /// Number of distinct admitted file and directory identities.
    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.next_id
    }

    /// Root directory of the exact admitted namespace.
    #[must_use]
    pub const fn root(&self) -> &NamespaceDirectory {
        &self.root
    }
}

#[derive(Debug)]
struct NamespaceAdmission<'budget> {
    budget: Option<&'budget mut ArchiveBudget>,
    next_id: &'budget mut usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NamespaceMember {
    Explicit,
    File,
    Implicit,
}

/// One admitted directory and its expected entries.
#[derive(Debug, Default)]
pub struct NamespaceDirectory {
    children: BTreeMap<String, NamespaceNode>,
    explicit: bool,
}

impl NamespaceDirectory {
    fn admit_child<'path>(
        &mut self,
        admission: &mut NamespaceAdmission<'_>,
        name: &'path str,
        member: NamespaceMember,
    ) -> Result<AdmittedChild<'_, 'path>, StagingError> {
        let is_directory = member != NamespaceMember::File;
        let explicit = member == NamespaceMember::Explicit;
        let (node, created) = match self
            .children
            .entry(name.to_ascii_lowercase())
        {
            Entry::Occupied(entry) => (entry.into_mut(), false),
            Entry::Vacant(entry) => {
                if member == NamespaceMember::Implicit
                    && let Some(limits) = admission.budget.as_deref_mut()
                {
                    limits.admit(name, EntryKind::Directory, 0)?.finish()?;
                }
                let id = *admission.next_id;
                *admission.next_id = admission
                    .next_id
                    .checked_add(1)
                    .ok_or(ArchiveError::TooLarge)?;
                let node = entry.insert(NamespaceNode {
                    directory: is_directory.then(Self::default),
                    id,
                    name: name.to_owned(),
                });
                (node, true)
            },
        };
        if node.name != name || (!created && !is_directory) {
            return Err(StagingError::Collision);
        }
        if is_directory {
            let directory =
                node.directory.as_mut().ok_or(StagingError::Collision)?;
            if explicit && directory.explicit {
                return Err(StagingError::Collision);
            }
            directory.explicit |= explicit;
        }
        Ok((
            NamespaceStep {
                created,
                id: node.id,
                name,
            },
            node,
        ))
    }

    /// Exact children in stable portable order.
    pub fn children(&self) -> impl ExactSizeIterator<Item = &NamespaceNode> {
        self.children.values()
    }
}

/// One file or directory in the portable staged namespace.
#[derive(Debug)]
pub struct NamespaceNode {
    directory: Option<NamespaceDirectory>,
    id: usize,
    name: String,
}

impl NamespaceNode {
    /// Directory contents, or absence for an ordinary file.
    #[must_use]
    pub const fn directory(&self) -> Option<&NamespaceDirectory> {
        self.directory.as_ref()
    }

    /// Stable transaction-local identity for associated native metadata.
    #[must_use]
    pub const fn id(&self) -> usize {
        self.id
    }

    /// Exact original portable component spelling.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The admitted parent traversal and leaf of a single staged member.
#[derive(Debug)]
pub struct NamespacePlan<'path> {
    leaf: NamespaceStep<'path>,
    parents: Vec<NamespaceStep<'path>>,
}

impl<'path> NamespacePlan<'path> {
    /// Exact final component and its transaction-local identity.
    #[must_use]
    pub const fn leaf(&self) -> NamespaceStep<'path> {
        self.leaf
    }

    /// Ordered parent components, including whether each must be created.
    #[must_use]
    pub fn parents(&self) -> &[NamespaceStep<'path>] {
        &self.parents
    }
}

/// One admitted component to create or reopen in a pinned directory.
#[derive(Clone, Copy, Debug)]
pub struct NamespaceStep<'path> {
    created: bool,
    id: usize,
    name: &'path str,
}

impl<'path> NamespaceStep<'path> {
    /// Whether the component is newly admitted and must not already exist.
    #[must_use]
    pub const fn created(self) -> bool {
        self.created
    }

    /// Stable transaction-local identity for associated native metadata.
    #[must_use]
    pub const fn id(self) -> usize {
        self.id
    }

    /// Original portable component spelling.
    #[must_use]
    pub const fn name(self) -> &'path str {
        self.name
    }
}

type AdmittedChild<'node, 'path> =
    (NamespaceStep<'path>, &'node mut NamespaceNode);

type PathParts<'path> = (&'path str, &'path str);

pub(crate) fn staging_path(path: &str) -> Result<PathParts<'_>, StagingError> {
    const MAX_DEPTH: usize = 64;
    if !path.is_ascii() || path.split('/').count() > MAX_DEPTH {
        return Err(StagingError::UnsupportedPath);
    }
    Ok(path.rsplit_once('/').unwrap_or(("", path)))
}

pub(crate) fn manifest_size(bytes: &[u8]) -> Result<u64, AssetError> {
    u64::try_from(bytes.len()).map_err(|_error| AssetError::TooLarge)
}

pub(crate) fn payload_path(digest: ContentHash) -> String {
    format!("{PAYLOADS_DIRECTORY}/{digest}")
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
