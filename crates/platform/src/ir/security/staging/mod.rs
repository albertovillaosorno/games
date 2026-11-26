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
//   - Outputs: Admitted members, traversal plans and exact manifest values.
//   - Side effects: Portable accounting and caller-owned stream access.
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
use std::io::{ErrorKind, Read, Write};

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryBudget, EntryKind, validate_path,
};
use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};

use crate::ir::storage::bundle::{MANIFEST_NAME, PAYLOADS_DIRECTORY};
use crate::ir::storage::staging::{StagingEntry, StagingError};

/// Exact encoded metadata and distinct payload inventory for publication.
#[derive(Debug)]
pub struct PreparedManifest {
    bytes: Vec<u8>,
    digest: ContentHash,
    payload_sizes: BTreeMap<ContentHash, u64>,
}

impl PreparedManifest {
    /// The canonical bytes to write without re-encoding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Identity of those exact bytes.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.digest
    }

    /// Encode the manifest within caller-selected resource limits.
    ///
    /// # Errors
    /// Rejects invalid or oversized canonical metadata.
    pub fn new(
        manifest: &AssetManifest,
        limits: ManifestLimits,
    ) -> Result<Self, StagingError> {
        let bytes = manifest.encode(limits)?;
        let digest = ContentHash::calculate(&bytes);
        Ok(Self {
            bytes,
            digest,
            payload_sizes: manifest.payload_sizes(),
        })
    }

    /// The complete inventory of distinct expected payloads.
    #[must_use]
    pub const fn payload_sizes(&self) -> &BTreeMap<ContentHash, u64> {
        &self.payload_sizes
    }
}

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

/// One member admitted against the portable archive budget and namespace.
#[derive(Debug)]
pub struct AdmittedArchiveEntry<'path> {
    budget: Option<EntryBudget>,
    leaf: &'path str,
    metadata: StagingEntry<'path>,
    parents: &'path str,
    plan: NamespacePlan<'path>,
    verified: bool,
}

impl<'path> AdmittedArchiveEntry<'path> {
    /// Stream exactly the reserved byte count without buffering the member.
    ///
    /// # Errors
    /// Rejects truncation, excess decoded bytes and read or write errors.
    pub fn copy_to(
        &mut self,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), StagingError> {
        self.verified = false;
        let mut budget = self.budget.take().ok_or(StagingError::Aborted)?;
        let mut buffer = [0; 8192];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => {
                    budget.finish()?;
                    self.verified = true;
                    return Ok(());
                },
                Ok(count) => {
                    let chunk = buffer
                        .get(..count)
                        .ok_or(ArchiveError::InvalidEntry)?;
                    budget.record(chunk.len())?;
                    output.write_all(chunk)?;
                },
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Require successful exact streaming before accepting the backend result.
    ///
    /// # Errors
    /// Rejects a skipped or unsuccessfully consumed entry.
    pub fn finish(self) -> Result<(), StagingError> {
        self.verified
            .then_some(())
            .ok_or(StagingError::Safety(ArchiveError::InvalidEntry))
    }

    /// The admitted entry's original metadata.
    #[must_use]
    pub const fn metadata(&self) -> StagingEntry<'path> {
        self.metadata
    }

    /// Reserve declared resources before opening the member output.
    ///
    /// # Errors
    /// Rejects unsafe metadata, exhausted budgets and nonportable paths.
    /// Any rejection requires discarding the namespace and transaction.
    pub(crate) fn new(
        metadata: StagingEntry<'path>,
        budget: &mut ArchiveBudget,
        namespace: &mut Namespace,
    ) -> Result<Self, StagingError> {
        let mut available = *budget;
        let entry =
            available.admit(metadata.path, metadata.kind, metadata.size)?;
        let (parents, leaf) = staging_path(metadata.path)?;
        let plan = namespace.admit(
            (parents, leaf),
            metadata.kind,
            Some(&mut available),
        )?;
        *budget = available;
        Ok(Self {
            budget: Some(entry),
            leaf,
            metadata,
            parents,
            plan,
            verified: false,
        })
    }

    /// The admitted parent path and final component.
    #[must_use]
    pub const fn parts(&self) -> (&'path str, &'path str) {
        (self.parents, self.leaf)
    }

    /// Admitted exact traversal with stable node identities.
    #[must_use]
    pub const fn plan(&self) -> &NamespacePlan<'path> {
        &self.plan
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

fn staging_path(path: &str) -> Result<PathParts<'_>, StagingError> {
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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, sink};
    use std::num::{NonZeroU64, NonZeroUsize};

    use games_assets::{AssetFormat, AssetRecord, SourceIdentity};

    use super::*;

    #[test]
    fn prepared_manifest_preserves_exact_inventory_and_limit()
    -> Result<(), StagingError> {
        let limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(15),
            records: NonZeroUsize::MIN.saturating_add(3),
            total_bytes: NonZeroU64::MIN.saturating_add(63),
        };
        let payload = ContentHash::calculate(b"tile");
        let records = ["first", "second"]
            .into_iter()
            .map(|key| AssetRecord {
                digest: payload,
                format: AssetFormat::Data,
                key: String::from(key),
                size: 4,
            })
            .collect();
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            records,
            limits,
        )?;
        let bytes = manifest.encode(limits)?;
        let manifest_bytes = NonZeroUsize::new(bytes.len())
            .ok_or(StagingError::ChangedStorage)?;
        let exact = ManifestLimits { manifest_bytes, ..limits };
        let prepared = PreparedManifest::new(&manifest, exact)?;
        let strict = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN,
            ..limits
        };
        if prepared.bytes() != bytes
            || prepared.digest() != ContentHash::calculate(&bytes)
            || prepared.payload_sizes() != &BTreeMap::from([(payload, 4u64)])
            || PreparedManifest::new(&manifest, strict).err()
                != Some(AssetError::TooLarge.into())
        {
            return Err(StagingError::ChangedStorage);
        }
        Ok(())
    }

    #[test]
    fn unsupported_archive_path_does_not_charge_the_portable_budget()
    -> Result<(), StagingError> {
        let four = NonZeroU64::MIN.saturating_add(3);
        let initial = ArchiveBudget::new(four, four, NonZeroU64::MIN);
        let mut budget = initial;
        let mut namespace = Namespace::default();
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "caf\u{e9}",
            size: 4,
        };
        if AdmittedArchiveEntry::new(metadata, &mut budget, &mut namespace)
            .err()
            != Some(StagingError::UnsupportedPath)
            || budget != initial
        {
            return Err(StagingError::ChangedStorage);
        }
        let mut entry = AdmittedArchiveEntry::new(
            StagingEntry {
                path: "asset",
                ..metadata
            },
            &mut budget,
            &mut namespace,
        )?;
        let mut output = Vec::new();
        entry.copy_to(&mut b"tile".as_slice(), &mut output)?;
        (entry.finish().is_ok() && output == b"tile")
            .then_some(())
            .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn namespace_plans_reopen_implicit_directories_without_duplicate_nodes()
    -> Result<(), StagingError> {
        let mut namespace = Namespace::default();
        let maximum = NonZeroU64::MIN.saturating_add(15);
        let mut budget = ArchiveBudget::new(maximum, maximum, maximum);
        let plan = namespace.admit(
            ("dir/nested", "asset"),
            EntryKind::File,
            Some(&mut budget),
        )?;
        if budget.remaining_entries() != 14 {
            return Err(StagingError::ChangedStorage);
        }
        let mut parents = plan.parents().iter();
        let outer = parents.next().ok_or(StagingError::ChangedStorage)?;
        let inner = parents.next().ok_or(StagingError::ChangedStorage)?;
        if !outer.created()
            || !inner.created()
            || !plan.leaf().created()
            || outer.name() != "dir"
            || inner.name() != "nested"
            || outer.id() != 0
            || inner.id() != 1
            || plan.leaf().id() != 2
        {
            return Err(StagingError::ChangedStorage);
        }
        let explicit = namespace.admit(
            ("dir", "nested"),
            EntryKind::Directory,
            Some(&mut budget),
        )?;
        if explicit.leaf().created()
            || explicit.leaf().id() != inner.id()
            || budget.remaining_entries() != 14
        {
            return Err(StagingError::ChangedStorage);
        }
        let sibling = namespace.admit(
            ("dir/nested", "other"),
            EntryKind::File,
            Some(&mut budget),
        )?;
        (sibling.leaf().id() == 3
            && budget.remaining_entries() == 14
            && namespace.root().children().len() == 1
            && namespace.entry_count() == 4)
            .then_some(())
            .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn exact_archive_copy_cannot_reuse_failed_or_consumed_admission()
    -> Result<(), StagingError> {
        let four = NonZeroU64::MIN.saturating_add(3);
        let initial = ArchiveBudget::new(four, four, NonZeroU64::MIN);
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "asset",
            size: 4,
        };
        let mut budget = initial;
        let mut namespace = Namespace::default();
        let mut entry =
            AdmittedArchiveEntry::new(metadata, &mut budget, &mut namespace)?;
        let mut empty = [0u8; 0];
        let mut output = Cursor::new(empty.as_mut_slice());
        if entry.copy_to(&mut b"tile".as_slice(), &mut output)
            != Err(StagingError::Io(ErrorKind::WriteZero))
            || entry.copy_to(&mut b"tile".as_slice(), &mut sink())
                != Err(StagingError::Aborted)
            || entry.finish() != Err(ArchiveError::InvalidEntry.into())
        {
            return Err(StagingError::ChangedStorage);
        }

        let mut second_budget = initial;
        let mut second_namespace = Namespace::default();
        let mut consumed = AdmittedArchiveEntry::new(
            metadata,
            &mut second_budget,
            &mut second_namespace,
        )?;
        let mut copied = Vec::new();
        consumed.copy_to(&mut b"tile".as_slice(), &mut copied)?;
        (copied == b"tile"
            && consumed.copy_to(&mut b"tile".as_slice(), &mut sink())
                == Err(StagingError::Aborted)
            && consumed.finish() == Err(ArchiveError::InvalidEntry.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }
}
