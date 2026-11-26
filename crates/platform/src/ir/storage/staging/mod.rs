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
//   - Portable private staging and atomic publication requests.
// - Must-Not:
//   - Open native descriptors, select filesystem APIs or decode media.
// - Allows:
//   - Inputs: Explicit parents, bounded decoded streams and manifests.
//   - Outputs: Complete staging handles or publication receipts.
//   - Side effects: Explicit selected backend operations only.
// - Split-When:
//   - A staging operation needs an independent lifecycle contract.
// - Merge-When:
//   - A shared staging transaction provides identical guarantees.
// - Summary:
//   - Keep native staging handles behind private selected ports.
// - Description:
//   - Preserve rejection, cleanup and post-publication durability semantics.
// - Usage:
//   - Create a transaction, write admitted entries and consume it to finish.
// - Defaults:
//   - Reject unsupported storage, collisions and incomplete output.

//! Portable private staging and publication.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::io::{Error, ErrorKind, Read, Write};

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryBudget, EntryKind, validate_path,
};
use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};

use super::bundle::{BundleError, CanonicalBundle};

/// A staging operation failed and the result must not be published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagingError {
    /// A previous entry failed; this transaction cannot be completed.
    Aborted,
    /// Canonical manifest or payload validation failed.
    Asset(AssetError),
    /// A pinned source canonical bundle failed verification.
    Bundle(BundleError),
    /// Staged file metadata or its directory identity changed.
    ChangedStorage,
    /// A duplicate path, case collision or file/directory conflict.
    Collision,
    /// The filesystem or decoded stream failed.
    Io(ErrorKind),
    /// Shared archive admission or decoded-byte validation failed.
    Safety(ArchiveError),
    /// Output path is not within the portable ASCII namespace or depth.
    UnsupportedPath,
    /// The parent is not owned by the effective user or permits other writers.
    UntrustedParent,
}

impl From<AssetError> for StagingError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

impl From<BundleError> for StagingError {
    fn from(error: BundleError) -> Self {
        Self::Bundle(error)
    }
}

impl From<Error> for StagingError {
    fn from(error: Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<ArchiveError> for StagingError {
    fn from(error: ArchiveError) -> Self {
        Self::Safety(error)
    }
}

/// Metadata for one decoded archive entry.
#[derive(Clone, Copy, Debug)]
pub struct StagingEntry<'path> {
    /// Original member kind; links and special files are not allowed.
    pub kind: EntryKind,
    /// Portable relative output path.
    pub path: &'path str,
    /// Exact decoded length.
    pub size: u64,
}

/// One member admitted against the portable archive budget and namespace.
#[derive(Debug)]
pub struct AdmittedArchiveEntry<'path> {
    budget: Option<EntryBudget>,
    leaf: &'path str,
    metadata: StagingEntry<'path>,
    parents: &'path str,
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
    pub fn new(
        metadata: StagingEntry<'path>,
        budget: &mut ArchiveBudget,
    ) -> Result<Self, StagingError> {
        let mut available = *budget;
        let entry =
            available.admit(metadata.path, metadata.kind, metadata.size)?;
        let (parents, leaf) = staging_path(metadata.path)?;
        *budget = available;
        Ok(Self {
            budget: Some(entry),
            leaf,
            metadata,
            parents,
            verified: false,
        })
    }

    /// The admitted parent path and final component.
    #[must_use]
    pub const fn parts(&self) -> (&'path str, &'path str) {
        (self.parents, self.leaf)
    }
}

/// Publication succeeded; the directory is visible and is not auto-deleted.
#[derive(Debug)]
pub struct PublishedDirectory {
    /// An error means the directory is visible but crash durability is
    /// uncertain. Publication must not be retried as though it had failed.
    pub durability: Result<(), ErrorKind>,
    /// Persistent published directory value.
    pub path: OsString,
}

/// A published manifest-addressed canonical bundle.
pub type PublishedBundle = PublishedDirectory;

pub(crate) type OpenArchive = Result<Box<dyn ArchivePort>, StagingError>;
pub(crate) type CompletedArchive =
    Result<Box<dyn CompletedArchivePort>, StagingError>;

pub(crate) trait ArchiveFactory {
    fn open_archive(parent: &OsStr) -> OpenArchive;
}

pub(crate) trait ArchivePort: Debug {
    fn append_member(
        &mut self,
        entry: &mut AdmittedArchiveEntry<'_>,
        input: &mut dyn Read,
        budget: &mut ArchiveBudget,
    ) -> Result<(), StagingError>;

    fn complete(self: Box<Self>) -> CompletedArchive;
}

pub(crate) trait CompletedArchivePort: Debug {
    fn location(&self) -> &OsStr;

    fn recheck(&self) -> Result<(), StagingError>;
}

#[derive(Debug)]
struct Transaction<Port>
where
    Port: ?Sized,
{
    backend: Box<Port>,
    failed: bool,
}

impl<Port> Transaction<Port>
where
    Port: ?Sized,
{
    fn apply<Operation>(
        &mut self,
        operation: Operation,
    ) -> Result<(), StagingError>
    where
        Operation: FnOnce(&mut Port) -> Result<(), StagingError>,
    {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        let result = operation(&mut self.backend);
        self.failed = result.is_err();
        result
    }

    fn finish(self) -> Result<Box<Port>, StagingError> {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        Ok(self.backend)
    }

    const fn new(backend: Box<Port>) -> Self {
        Self { backend, failed: false }
    }
}

/// Incomplete private archive transaction, removed on drop.
#[derive(Debug)]
pub struct ArchiveStaging {
    budget: ArchiveBudget,
    transaction: Transaction<dyn ArchivePort>,
}

impl ArchiveStaging {
    /// Complete only a transaction whose attempted entries all succeeded.
    ///
    /// # Errors
    /// Rejects an aborted transaction or changed storage.
    pub fn finish(self) -> Result<StagedArchive, StagingError> {
        Ok(StagedArchive {
            backend: self.transaction.finish()?.complete()?,
        })
    }

    /// Create private staging under an explicitly selected trusted parent.
    ///
    /// # Errors
    /// Rejects unavailable storage, untrusted parents and I/O failures.
    pub fn new<Parent>(
        parent: &Parent,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError>
    where
        Parent: AsRef<OsStr> + ?Sized,
    {
        Ok(Self {
            budget,
            transaction: Transaction::new(
                <() as ArchiveFactory>::open_archive(parent.as_ref())?,
            ),
        })
    }

    /// Stage one bounded member, including implicit directory charges.
    ///
    /// # Errors
    /// Rejects unsafe metadata, collisions, excess bytes and storage errors.
    /// Any failure aborts the entire transaction.
    pub fn write_entry<Source>(
        &mut self,
        metadata: StagingEntry<'_>,
        input: &mut Source,
    ) -> Result<(), StagingError>
    where
        Source: Read,
    {
        self.transaction.apply(|backend| {
            let mut entry =
                AdmittedArchiveEntry::new(metadata, &mut self.budget)?;
            backend.append_member(&mut entry, input, &mut self.budget)?;
            entry.finish()
        })
    }
}

/// Verified private staging directory, removed on drop.
#[derive(Debug)]
pub struct StagedArchive {
    backend: Box<dyn CompletedArchivePort>,
}

impl StagedArchive {
    /// Location of the complete archive for subsequent trusted conversion.
    #[must_use]
    pub fn path(&self) -> &OsStr {
        self.backend.location()
    }

    /// Recheck the exact staged tree before subsequent trusted use.
    ///
    /// # Errors
    /// Rejects observed storage changes or filesystem failures.
    pub fn verify(&self) -> Result<(), StagingError> {
        self.backend.recheck()
    }
}

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

pub(crate) type OpenCanonical = Result<Box<dyn CanonicalPort>, StagingError>;
pub(crate) type CompletedBundle =
    Result<Box<dyn CompletedBundlePort>, StagingError>;

pub(crate) trait CanonicalFactory {
    fn copy_bundle(
        parent: &OsStr,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        manifest: &PreparedManifest,
    ) -> CompletedBundle;

    fn open_canonical(parent: &OsStr, budget: ArchiveBudget) -> OpenCanonical;
}

pub(crate) trait CanonicalPort: Debug {
    fn complete(
        self: Box<Self>,
        manifest: &PreparedManifest,
    ) -> CompletedBundle;

    fn write_verified(
        &mut self,
        digest: ContentHash,
        bytes: &[u8],
    ) -> Result<(), StagingError>;
}

pub(crate) trait CompletedBundlePort: Debug {
    fn identity(&self) -> ContentHash;

    fn location(&self) -> &OsStr;

    fn publish_complete(
        self: Box<Self>,
    ) -> Result<PublishedBundle, StagingError>;
}

/// Incomplete manifest transaction with verified unique payloads.
#[derive(Debug)]
pub struct CanonicalStaging {
    transaction: Transaction<dyn CanonicalPort>,
}

impl CanonicalStaging {
    /// Verify and re-stage a pinned bundle in the shared content store.
    ///
    /// # Errors
    /// Rejects unavailable storage, invalid input and exceeded budgets.
    pub fn copy_bundle<Parent>(
        parent: &Parent,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError>
    where
        Parent: AsRef<OsStr> + ?Sized,
    {
        Self::copy_with::<()>(parent.as_ref(), budget, source, limits)
    }

    fn copy_with<Factory>(
        parent: &OsStr,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError>
    where
        Factory: CanonicalFactory,
    {
        let manifest = PreparedManifest::new(source.manifest(), limits)?;
        let backend = Factory::copy_bundle(parent, budget, source, &manifest)?;
        StagedBundle::admit(backend, manifest.digest())
    }

    /// Complete only a bundle with an exact match to its manifest.
    ///
    /// # Errors
    /// Rejects failed staging, mismatches or invalid metadata.
    pub fn finish(
        self,
        manifest: &AssetManifest,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError> {
        let port = self.transaction.finish()?;
        let prepared = PreparedManifest::new(manifest, limits)?;
        let backend = port.complete(&prepared)?;
        StagedBundle::admit(backend, prepared.digest())
    }

    /// Create a private canonical transaction for the shared object store.
    ///
    /// # Errors
    /// Rejects unsupported storage, mixed layouts and untrusted parents.
    pub fn new<Parent>(
        parent: &Parent,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError>
    where
        Parent: AsRef<OsStr> + ?Sized,
    {
        Ok(Self {
            transaction: Transaction::new(
                <() as CanonicalFactory>::open_canonical(
                    parent.as_ref(),
                    budget,
                )?,
            ),
        })
    }

    /// Verify and stage one unique canonical payload.
    ///
    /// # Errors
    /// Rejects hash mismatches, duplicates, excess sizes and I/O failures.
    pub fn write_payload(
        &mut self,
        digest: ContentHash,
        bytes: &[u8],
    ) -> Result<(), StagingError> {
        self.transaction
            .apply(|backend| backend.write_verified(digest, bytes))
    }
}

/// Complete verified canonical bundle awaiting atomic publication.
#[derive(Debug)]
pub struct StagedBundle {
    backend: Box<dyn CompletedBundlePort>,
    digest: ContentHash,
}

impl StagedBundle {
    fn admit(
        backend: Box<dyn CompletedBundlePort>,
        digest: ContentHash,
    ) -> Result<Self, StagingError> {
        if backend.identity() != digest {
            return Err(AssetError::PayloadMismatch.into());
        }
        Ok(Self { backend, digest })
    }

    /// Digest of the exact canonical manifest bytes.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.digest
    }

    /// Private temporary location, removed on drop unless published.
    #[must_use]
    pub fn path(&self) -> &OsStr {
        self.backend.location()
    }

    /// Atomically publish the complete bundle without replacing output.
    ///
    /// # Errors
    /// Rejects unsupported storage, collisions and pre-publication failures.
    pub fn publish(self) -> Result<PublishedBundle, StagingError> {
        self.backend.publish_complete()
    }
}

pub(crate) type OpenAuthored = Result<Box<dyn AuthoredPort>, StagingError>;

pub(crate) trait AuthoredFactory {
    fn open_authored(parent: &OsStr) -> OpenAuthored;
}

pub(crate) trait AuthoredPort: Debug {
    fn append_member(
        &mut self,
        path: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), StagingError>;

    fn location(&self) -> &OsStr;

    fn publish_complete(
        self: Box<Self>,
        name: &OsStr,
    ) -> Result<PublishedDirectory, StagingError>;
}

/// Private staging for already-authored, trusted host output.
///
/// Callers bound file sizes and counts and must never supply executable
/// bytes from untrusted media.
#[derive(Debug)]
pub struct AuthoredStaging {
    transaction: Transaction<dyn AuthoredPort>,
}

impl AuthoredStaging {
    /// Create a private sibling directory for trusted output.
    ///
    /// # Errors
    /// Rejects unsupported storage, untrusted parents and I/O failures.
    pub fn new<Parent>(parent: &Parent) -> Result<Self, StagingError>
    where
        Parent: AsRef<OsStr> + ?Sized,
    {
        Ok(Self {
            transaction: Transaction::new(
                <() as AuthoredFactory>::open_authored(parent.as_ref())?,
            ),
        })
    }

    /// Private temporary location, removed on drop unless published.
    #[must_use]
    pub fn path(&self) -> &OsStr {
        self.transaction.backend.location()
    }

    /// Publish exactly the recorded output tree, without replacement.
    ///
    /// # Errors
    /// Rejects aborted writes, unsafe names, collisions and changed storage.
    pub fn publish(
        self,
        name: &OsStr,
    ) -> Result<PublishedDirectory, StagingError> {
        self.transaction.finish()?.publish_complete(name)
    }

    /// Write one trusted file using the portable ASCII namespace.
    ///
    /// # Errors
    /// Rejects unsafe paths, collisions and filesystem changes.
    /// A failed write aborts the entire transaction.
    pub fn write_file(
        &mut self,
        path: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), StagingError> {
        self.transaction.apply(|backend| {
            validate_path(path)?;
            let _parts = staging_path(path)?;
            backend.append_member(path, bytes, executable)
        })
    }
}

type PathParts<'path> = (&'path str, &'path str);

pub(crate) fn staging_path(path: &str) -> Result<PathParts<'_>, StagingError> {
    const MAX_DEPTH: usize = 64;
    if !path.is_ascii() || path.split('/').count() > MAX_DEPTH {
        return Err(StagingError::UnsupportedPath);
    }
    Ok(path.rsplit_once('/').unwrap_or(("", path)))
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU64, NonZeroUsize};

    use games_assets::SourceIdentity;

    use super::*;

    #[derive(Debug)]
    struct ArchiveFake;

    impl ArchivePort for ArchiveFake {
        fn append_member(
            &mut self,
            entry: &mut AdmittedArchiveEntry<'_>,
            input: &mut dyn Read,
            budget: &mut ArchiveBudget,
        ) -> Result<(), StagingError> {
            let mut bytes = Vec::new();
            let metadata = entry.metadata();
            if metadata.path == "ignore" {
                return Ok(());
            }
            if metadata.size != 4 {
                return Err(StagingError::UnsupportedPath);
            }
            match entry.parts() {
                ("", "asset") => {},
                ("dir", "asset") => {
                    budget.admit("dir", EntryKind::Directory, 0)?.finish()?;
                },
                _ => return Err(StagingError::UnsupportedPath),
            }
            entry.copy_to(input, &mut bytes)?;
            (bytes == b"tile")
                .then_some(())
                .ok_or(StagingError::ChangedStorage)
        }

        fn complete(self: Box<Self>) -> CompletedArchive {
            Ok(self)
        }
    }

    impl CompletedArchivePort for ArchiveFake {
        fn location(&self) -> &OsStr {
            OsStr::new("archive")
        }

        fn recheck(&self) -> Result<(), StagingError> {
            Err(StagingError::ChangedStorage)
        }
    }

    #[derive(Debug)]
    struct CanonicalFake {
        wrong_identity: bool,
    }

    impl CanonicalFactory for CanonicalFake {
        fn copy_bundle(
            parent: &OsStr,
            _budget: ArchiveBudget,
            source: &CanonicalBundle,
            manifest: &PreparedManifest,
        ) -> CompletedBundle {
            if manifest.payload_sizes() != &source.manifest().payload_sizes() {
                return Err(StagingError::ChangedStorage);
            }
            if parent == OsStr::new("wrong") {
                Ok(Box::new(BundleFake(ContentHash::calculate(b"wrong"))))
            } else {
                Ok(Box::new(BundleFake(manifest.digest())))
            }
        }

        fn open_canonical(
            _parent: &OsStr,
            _budget: ArchiveBudget,
        ) -> OpenCanonical {
            Err(StagingError::Io(ErrorKind::Unsupported))
        }
    }

    impl CanonicalPort for CanonicalFake {
        fn complete(
            self: Box<Self>,
            manifest: &PreparedManifest,
        ) -> CompletedBundle {
            if self.wrong_identity {
                Ok(Box::new(BundleFake(ContentHash::calculate(b"wrong"))))
            } else {
                Ok(Box::new(BundleFake(manifest.digest())))
            }
        }

        fn write_verified(
            &mut self,
            digest: ContentHash,
            bytes: &[u8],
        ) -> Result<(), StagingError> {
            (digest == ContentHash::calculate(bytes))
                .then_some(())
                .ok_or(StagingError::Asset(AssetError::PayloadMismatch))
        }
    }

    #[derive(Debug)]
    struct BundleFake(ContentHash);

    impl CompletedBundlePort for BundleFake {
        fn identity(&self) -> ContentHash {
            self.0
        }

        fn location(&self) -> &OsStr {
            OsStr::new("bundle")
        }

        fn publish_complete(
            self: Box<Self>,
        ) -> Result<PublishedBundle, StagingError> {
            Ok(PublishedDirectory {
                durability: Ok(()),
                path: OsString::from("published"),
            })
        }
    }

    #[derive(Debug)]
    struct AuthoredFake;

    impl AuthoredPort for AuthoredFake {
        fn append_member(
            &mut self,
            path: &str,
            bytes: &[u8],
            executable: bool,
        ) -> Result<(), StagingError> {
            if path == "program" && bytes == b"binary" && executable {
                Ok(())
            } else {
                Err(StagingError::Collision)
            }
        }

        fn location(&self) -> &OsStr {
            OsStr::new("temporary")
        }

        fn publish_complete(
            self: Box<Self>,
            name: &OsStr,
        ) -> Result<PublishedDirectory, StagingError> {
            Ok(PublishedDirectory {
                durability: Err(ErrorKind::Other),
                path: name.to_os_string(),
            })
        }
    }

    fn archive_fixture() -> ArchiveStaging {
        let four = NonZeroU64::MIN.saturating_add(3);
        ArchiveStaging {
            budget: ArchiveBudget::new(four, four, NonZeroU64::MIN),
            transaction: Transaction::new(Box::new(ArchiveFake)),
        }
    }

    #[test]
    fn unsupported_archive_path_does_not_charge_the_portable_budget()
    -> Result<(), StagingError> {
        let four = NonZeroU64::MIN.saturating_add(3);
        let initial = ArchiveBudget::new(four, four, NonZeroU64::MIN);
        let mut budget = initial;
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "caf\u{e9}",
            size: 4,
        };
        if AdmittedArchiveEntry::new(metadata, &mut budget).err()
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
        )?;
        let mut output = Vec::new();
        entry.copy_to(&mut b"tile".as_slice(), &mut output)?;
        (entry.finish().is_ok() && output == b"tile")
            .then_some(())
            .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn portable_archive_rejects_metadata_before_backend_dispatch()
    -> Result<(), StagingError> {
        for (path, kind, size, error) in [
            ("../escape", EntryKind::File, 4, ArchiveError::UnsafePath),
            ("asset", EntryKind::Symlink, 4, ArchiveError::LinkOrSpecial),
            ("asset.exe", EntryKind::File, 4, ArchiveError::Executable),
            ("asset", EntryKind::File, 5, ArchiveError::TooLarge),
        ] {
            let mut stage = archive_fixture();
            let metadata = StagingEntry { kind, path, size };
            let mut input = b"tile".as_slice();
            if stage.write_entry(metadata, &mut input) != Err(error.into())
                || input != b"tile"
                || stage.finish().err() != Some(StagingError::Aborted)
            {
                return Err(StagingError::ChangedStorage);
            }
        }
        Ok(())
    }

    #[test]
    fn implicit_directories_charge_the_same_archive_budget()
    -> Result<(), StagingError> {
        let four = NonZeroU64::MIN.saturating_add(3);
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "dir/asset",
            size: 4,
        };
        let mut rejected = archive_fixture();
        let mut input = b"tile".as_slice();
        if rejected.write_entry(metadata, &mut input)
            != Err(ArchiveError::TooLarge.into())
            || input != b"tile"
        {
            return Err(StagingError::ChangedStorage);
        }

        let mut stage = ArchiveStaging {
            budget: ArchiveBudget::new(
                four,
                four,
                NonZeroU64::MIN.saturating_add(1),
            ),
            transaction: Transaction::new(Box::new(ArchiveFake)),
        };
        stage.write_entry(metadata, &mut input)?;
        let empty = StagingEntry {
            kind: EntryKind::File,
            path: "asset",
            size: 0,
        };
        (stage.write_entry(empty, &mut b"".as_slice())
            == Err(ArchiveError::TooLarge.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn portable_archive_rejects_a_skipped_backend_stream()
    -> Result<(), StagingError> {
        let mut stage = archive_fixture();
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "ignore",
            size: 4,
        };
        let mut input = b"tile".as_slice();
        (stage.write_entry(metadata, &mut input)
            == Err(ArchiveError::InvalidEntry.into())
            && input == b"tile"
            && stage.finish().err() == Some(StagingError::Aborted))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn portable_archive_streams_require_exact_size_and_available_budget()
    -> Result<(), StagingError> {
        let metadata = StagingEntry {
            kind: EntryKind::File,
            path: "asset",
            size: 4,
        };
        for (mut bytes, error) in [
            (b"til".as_slice(), ArchiveError::InvalidEntry),
            (b"tiles".as_slice(), ArchiveError::TooLarge),
        ] {
            let mut stage = archive_fixture();
            let result = stage.write_entry(metadata, &mut bytes);
            if result != Err(error.into())
                || stage.finish().err() != Some(StagingError::Aborted)
            {
                return Err(StagingError::ChangedStorage);
            }
        }
        let mut stage = archive_fixture();
        stage.write_entry(metadata, &mut b"tile".as_slice())?;
        let mut input = b"tile".as_slice();
        (stage.write_entry(metadata, &mut input)
            == Err(ArchiveError::TooLarge.into())
            && input == b"tile")
            .then_some(())
            .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn archive_stream_and_recheck_use_the_selected_port()
    -> Result<(), StagingError> {
        let mut stage = archive_fixture();
        let entry = StagingEntry {
            kind: EntryKind::File,
            path: "asset",
            size: 4,
        };
        stage.write_entry(entry, &mut b"tile".as_slice())?;
        let complete = stage.finish()?;
        (complete.path() == OsStr::new("archive")
            && complete.verify() == Err(StagingError::ChangedStorage))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    fn fixture_limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(15),
            records: NonZeroUsize::MIN.saturating_add(3),
            total_bytes: NonZeroU64::MIN.saturating_add(63),
        }
    }

    fn fixture_manifest(
        limits: ManifestLimits,
    ) -> Result<AssetManifest, StagingError> {
        Ok(AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            Vec::new(),
            limits,
        )?)
    }

    #[test]
    fn canonical_completion_and_publication_preserve_identity()
    -> Result<(), StagingError> {
        let limits = fixture_limits();
        let manifest = fixture_manifest(limits)?;
        let mut rejected = CanonicalStaging {
            transaction: Transaction::new(Box::new(CanonicalFake {
                wrong_identity: false,
            })),
        };
        let digest = ContentHash::calculate(b"tile");
        let invalid = rejected.write_payload(digest, b"wrong")
            == Err(StagingError::Asset(AssetError::PayloadMismatch))
            && rejected.write_payload(digest, b"tile")
                == Err(StagingError::Aborted)
            && matches!(
                rejected.finish(&manifest, limits),
                Err(StagingError::Aborted)
            );
        let stage = CanonicalStaging {
            transaction: Transaction::new(Box::new(CanonicalFake {
                wrong_identity: false,
            })),
        };
        let complete = stage.finish(&manifest, limits)?;
        let valid = complete.digest()
            == ContentHash::calculate(&manifest.encode(limits)?)
            && complete.path() == OsStr::new("bundle");
        let receipt = complete.publish()?;
        (invalid
            && valid
            && receipt.durability.is_ok()
            && receipt.path == OsStr::new("published"))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn canonical_staging_rejects_unadmitted_completion_identity()
    -> Result<(), StagingError> {
        let limits = fixture_limits();
        let manifest = fixture_manifest(limits)?;
        let stage = CanonicalStaging {
            transaction: Transaction::new(Box::new(CanonicalFake {
                wrong_identity: true,
            })),
        };
        if stage.finish(&manifest, limits).err()
            != Some(AssetError::PayloadMismatch.into())
        {
            return Err(StagingError::ChangedStorage);
        }
        let strict_stage = CanonicalStaging {
            transaction: Transaction::new(Box::new(CanonicalFake {
                wrong_identity: true,
            })),
        };
        let strict = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN,
            ..limits
        };
        (strict_stage.finish(&manifest, strict).err()
            == Some(AssetError::TooLarge.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn canonical_copy_rejects_unadmitted_completion_identity()
    -> Result<(), StagingError> {
        let limits = fixture_limits();
        let manifest = fixture_manifest(limits)?;
        let bytes = manifest.encode(limits)?;
        let digest = ContentHash::calculate(&bytes);
        let source = CanonicalBundle::from_memory(
            &bytes,
            digest,
            limits,
            BTreeMap::new(),
        )?;
        let budget = ArchiveBudget::new(
            limits.total_bytes,
            limits.total_bytes,
            NonZeroU64::MIN,
        );
        let accepted = CanonicalStaging::copy_with::<CanonicalFake>(
            OsStr::new("good"),
            budget,
            &source,
            limits,
        )?;
        let rejected = CanonicalStaging::copy_with::<CanonicalFake>(
            OsStr::new("wrong"),
            budget,
            &source,
            limits,
        );
        (accepted.digest() == digest
            && rejected.err() == Some(AssetError::PayloadMismatch.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    fn authored() -> AuthoredStaging {
        AuthoredStaging {
            transaction: Transaction::new(Box::new(AuthoredFake)),
        }
    }

    #[test]
    fn failed_write_permanently_aborts_the_publication() {
        let mut stage = authored();
        assert_eq!(stage.path(), OsStr::new("temporary"));
        assert_eq!(
            stage.write_file("program", b"binary", false),
            Err(StagingError::Collision)
        );
        assert_eq!(
            stage.write_file("program", b"binary", true),
            Err(StagingError::Aborted)
        );
        assert!(matches!(
            stage.publish(OsStr::new("published")),
            Err(StagingError::Aborted)
        ));
    }

    #[test]
    fn completed_publication_preserves_post_publish_failure()
    -> Result<(), StagingError> {
        let mut stage = authored();
        stage.write_file("program", b"binary", true)?;
        let receipt = stage.publish(OsStr::new("published"))?;
        (receipt.path == OsStr::new("published")
            && receipt.durability == Err(ErrorKind::Other))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }
}
