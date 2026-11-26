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
/// Verified migration planning and private shared-store staging.
pub mod migration;

#[cfg(test)]
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::io::{Error, ErrorKind, Read};

use games_assets::archive::{ArchiveBudget, ArchiveError, EntryKind};
use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};

use super::bundle::{BundleError, CanonicalBundle};
pub use crate::ir::security::staging::{
    AdmittedArchiveEntry, PreparedManifest,
};
use crate::ir::security::staging::{
    AdmittedAuthoredFile, AdmittedManifest, AdmittedPayload,
    CanonicalAdmission, Namespace, PublicationName,
};

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
    ) -> Result<(), StagingError>;

    fn complete(self: Box<Self>, namespace: Namespace) -> CompletedArchive;
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
    namespace: Namespace,
    transaction: Transaction<dyn ArchivePort>,
}

impl ArchiveStaging {
    /// Complete only a transaction whose attempted entries all succeeded.
    ///
    /// # Errors
    /// Rejects an aborted transaction or changed storage.
    pub fn finish(self) -> Result<StagedArchive, StagingError> {
        Ok(StagedArchive {
            backend: self.transaction.finish()?.complete(self.namespace)?,
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
            namespace: Namespace::default(),
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
            let mut entry = AdmittedArchiveEntry::new(
                metadata,
                &mut self.budget,
                &mut self.namespace,
            )?;
            backend.append_member(&mut entry, input)?;
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

pub(crate) type OpenCanonical = Result<Box<dyn CanonicalPort>, StagingError>;
pub(crate) type CompletedBundle =
    Result<Box<dyn CompletedBundlePort>, StagingError>;

pub(crate) trait CanonicalFactory {
    fn open_canonical(parent: &OsStr, budget: ArchiveBudget) -> OpenCanonical;
}

pub(crate) trait CanonicalPort: Debug {
    fn complete(
        self: Box<Self>,
        manifest: &AdmittedManifest<'_>,
    ) -> CompletedBundle;

    fn write_verified(
        &mut self,
        payload: &AdmittedPayload<'_>,
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
    admission: CanonicalAdmission,
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

    pub(crate) fn copy_with<Factory>(
        parent: &OsStr,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError>
    where
        Factory: CanonicalFactory,
    {
        let manifest = PreparedManifest::new(source.manifest(), limits)?;
        CanonicalAdmission::preflight(budget, &manifest)?;
        let mut stage = Self::open_with::<Factory>(parent, budget)?;
        for record in source.manifest().records() {
            if stage.admission.contains(record.digest) {
                continue;
            }
            let payload = source.read_payload(&record.key)?;
            stage.write_payload(record.digest, &payload)?;
        }
        stage.finish_prepared(&manifest)
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
        if self.transaction.failed {
            return Err(StagingError::Aborted);
        }
        self.finish_prepared(&PreparedManifest::new(manifest, limits)?)
    }

    fn finish_prepared(
        self,
        prepared: &PreparedManifest,
    ) -> Result<StagedBundle, StagingError> {
        let port = self.transaction.finish()?;
        let manifest = self.admission.complete(prepared)?;
        let backend = port.complete(&manifest)?;
        StagedBundle::admit(backend, manifest.digest())
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
        Self::open_with::<()>(parent.as_ref(), budget)
    }

    fn open_with<Factory>(
        parent: &OsStr,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError>
    where
        Factory: CanonicalFactory,
    {
        Ok(Self {
            admission: CanonicalAdmission::new(budget),
            transaction: Transaction::new(Factory::open_canonical(
                parent, budget,
            )?),
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
        self.transaction.apply(|backend| {
            let payload = self.admission.payload(digest, bytes)?;
            backend.write_verified(&payload)
        })
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
        file: &AdmittedAuthoredFile<'_>,
    ) -> Result<(), StagingError>;

    fn location(&self) -> &OsStr;

    fn publish_complete(
        self: Box<Self>,
        name: PublicationName<'_>,
        namespace: &Namespace,
    ) -> Result<PublishedDirectory, StagingError>;
}

/// Private staging for already-authored, trusted host output.
///
/// Callers bound file sizes and counts and must never supply executable
/// bytes from untrusted media.
#[derive(Debug)]
pub struct AuthoredStaging {
    namespace: Namespace,
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
            namespace: Namespace::default(),
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
        let backend = self.transaction.finish()?;
        backend.publish_complete(PublicationName::new(name)?, &self.namespace)
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
            backend.append_member(&AdmittedAuthoredFile::new(
                path,
                bytes,
                executable,
                &mut self.namespace,
            )?)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::sink;
    use std::mem::replace;
    use std::num::{NonZeroU64, NonZeroUsize};

    use games_assets::{AssetFormat, AssetRecord, SourceIdentity};

    use super::*;

    #[derive(Debug)]
    struct ArchiveFake;

    impl ArchivePort for ArchiveFake {
        fn append_member(
            &mut self,
            entry: &mut AdmittedArchiveEntry<'_>,
            input: &mut dyn Read,
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
                ("" | "dir", "asset") => {},
                _ => return Err(StagingError::UnsupportedPath),
            }
            entry.copy_to(input, &mut bytes)?;
            (bytes == b"tile")
                .then_some(())
                .ok_or(StagingError::ChangedStorage)
        }

        fn complete(
            self: Box<Self>,
            _namespace: Namespace,
        ) -> CompletedArchive {
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
        payload_seen: bool,
        wrong_identity: bool,
    }

    impl CanonicalFactory for CanonicalFake {
        fn open_canonical(
            parent: &OsStr,
            _budget: ArchiveBudget,
        ) -> OpenCanonical {
            if parent == OsStr::new("unavailable") {
                return Err(StagingError::Io(ErrorKind::Unsupported));
            }
            Ok(Box::new(Self {
                payload_seen: false,
                wrong_identity: parent == OsStr::new("wrong"),
            }))
        }
    }

    impl CanonicalPort for CanonicalFake {
        fn complete(
            self: Box<Self>,
            manifest: &AdmittedManifest<'_>,
        ) -> CompletedBundle {
            if u64::try_from(manifest.bytes().len()).ok()
                != Some(manifest.size())
            {
                return Err(StagingError::ChangedStorage);
            }
            if self.wrong_identity {
                Ok(Box::new(BundleFake(ContentHash::calculate(b"wrong"))))
            } else {
                Ok(Box::new(BundleFake(manifest.digest())))
            }
        }

        fn write_verified(
            &mut self,
            payload: &AdmittedPayload<'_>,
        ) -> Result<(), StagingError> {
            let repeated = replace(&mut self.payload_seen, true);
            (!repeated
                && payload.bytes() == b"tile"
                && payload.digest() == ContentHash::calculate(b"tile")
                && payload.size() == 4)
                .then_some(())
                .ok_or(StagingError::ChangedStorage)
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
            file: &AdmittedAuthoredFile<'_>,
        ) -> Result<(), StagingError> {
            if file.parts() == ("", "program")
                && file.bytes() == b"binary"
                && file.executable()
                && file.size() == 6
            {
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
            name: PublicationName<'_>,
            _namespace: &Namespace,
        ) -> Result<PublishedDirectory, StagingError> {
            Ok(PublishedDirectory {
                durability: Err(ErrorKind::Other),
                path: name.value().to_os_string(),
            })
        }
    }

    #[derive(Debug)]
    struct PermissiveArchive;

    impl ArchivePort for PermissiveArchive {
        fn append_member(
            &mut self,
            entry: &mut AdmittedArchiveEntry<'_>,
            input: &mut dyn Read,
        ) -> Result<(), StagingError> {
            entry.copy_to(input, &mut sink())
        }

        fn complete(
            self: Box<Self>,
            _namespace: Namespace,
        ) -> CompletedArchive {
            Ok(Box::new(ArchiveFake))
        }
    }

    fn archive_fixture() -> ArchiveStaging {
        let four = NonZeroU64::MIN.saturating_add(3);
        ArchiveStaging {
            budget: ArchiveBudget::new(four, four, NonZeroU64::MIN),
            namespace: Namespace::default(),
            transaction: Transaction::new(Box::new(ArchiveFake)),
        }
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
    fn portable_namespace_rejects_collisions_before_streaming()
    -> Result<(), StagingError> {
        let maximum = NonZeroU64::MIN.saturating_add(63);
        for (first, first_kind, second, second_kind) in [
            ("a/file", EntryKind::File, "A/other", EntryKind::File),
            ("a/file", EntryKind::File, "a/FILE", EntryKind::File),
            ("a/file", EntryKind::File, "a/file", EntryKind::File),
            ("a/file", EntryKind::File, "a/file/sub", EntryKind::File),
            ("a/file", EntryKind::File, "a/file", EntryKind::Directory),
            ("a", EntryKind::Directory, "a", EntryKind::Directory),
            ("a", EntryKind::Directory, "a", EntryKind::File),
        ] {
            let mut stage = ArchiveStaging {
                budget: ArchiveBudget::new(maximum, maximum, maximum),
                namespace: Namespace::default(),
                transaction: Transaction::new(Box::new(PermissiveArchive)),
            };
            let mut bytes = if first_kind == EntryKind::File {
                b"tile".as_slice()
            } else {
                b"".as_slice()
            };
            let first_size = if first_kind == EntryKind::File {
                4
            } else {
                0
            };
            stage.write_entry(
                StagingEntry {
                    kind: first_kind,
                    path: first,
                    size: first_size,
                },
                &mut bytes,
            )?;
            let mut input = b"tile".as_slice();
            let second_size = if second_kind == EntryKind::File {
                4
            } else {
                0
            };
            if stage.write_entry(
                StagingEntry {
                    kind: second_kind,
                    path: second,
                    size: second_size,
                },
                &mut input,
            ) != Err(StagingError::Collision)
                || input != b"tile"
                || !matches!(stage.finish(), Err(StagingError::Aborted))
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
            namespace: Namespace::default(),
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

    fn canonical_budget() -> ArchiveBudget {
        let maximum = NonZeroU64::MIN.saturating_add(8191);
        ArchiveBudget::new(maximum, maximum, NonZeroU64::MIN.saturating_add(7))
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
            admission: CanonicalAdmission::new(canonical_budget()),
            transaction: Transaction::new(Box::new(CanonicalFake {
                payload_seen: false,
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
            admission: CanonicalAdmission::new(canonical_budget()),
            transaction: Transaction::new(Box::new(CanonicalFake {
                payload_seen: false,
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
            admission: CanonicalAdmission::new(canonical_budget()),
            transaction: Transaction::new(Box::new(CanonicalFake {
                payload_seen: false,
                wrong_identity: true,
            })),
        };
        if stage.finish(&manifest, limits).err()
            != Some(AssetError::PayloadMismatch.into())
        {
            return Err(StagingError::ChangedStorage);
        }
        let strict_stage = CanonicalStaging {
            admission: CanonicalAdmission::new(canonical_budget()),
            transaction: Transaction::new(Box::new(CanonicalFake {
                payload_seen: false,
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
        let size = NonZeroU64::new(
            u64::try_from(bytes.len())
                .map_err(|_error| AssetError::TooLarge)?,
        )
        .ok_or(AssetError::TooLarge)?;
        let budget = ArchiveBudget::new(size, size, NonZeroU64::MIN);
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

    fn payload_manifest(
        limits: ManifestLimits,
    ) -> Result<AssetManifest, StagingError> {
        let digest = ContentHash::calculate(b"tile");
        let records = ["first", "second"]
            .into_iter()
            .map(|key| AssetRecord {
                digest,
                format: AssetFormat::Data,
                key: String::from(key),
                size: 4,
            })
            .collect();
        Ok(AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            records,
            limits,
        )?)
    }

    #[test]
    fn canonical_copy_preflights_and_dispatches_each_distinct_payload_once()
    -> Result<(), StagingError> {
        let limits = fixture_limits();
        let manifest = payload_manifest(limits)?;
        let bytes = manifest.encode(limits)?;
        let digest = ContentHash::calculate(&bytes);
        let payload = ContentHash::calculate(b"tile");
        let source = CanonicalBundle::from_memory(
            &bytes,
            digest,
            limits,
            BTreeMap::from([(payload, b"tile".to_vec())]),
        )?;
        let maximum = NonZeroU64::new(
            u64::try_from(bytes.len())
                .map_err(|_error| AssetError::TooLarge)?,
        )
        .ok_or(AssetError::TooLarge)?;
        let total = maximum.saturating_add(4);
        let exact = ArchiveBudget::new(
            maximum,
            total,
            NonZeroU64::MIN.saturating_add(2),
        );
        let deficient = ArchiveBudget::new(
            maximum,
            total,
            NonZeroU64::MIN.saturating_add(1),
        );
        let accepted = CanonicalStaging::copy_with::<CanonicalFake>(
            OsStr::new("good"),
            exact,
            &source,
            limits,
        )?;
        let rejected = CanonicalStaging::copy_with::<CanonicalFake>(
            OsStr::new("unavailable"),
            deficient,
            &source,
            limits,
        );
        (accepted.digest() == digest
            && rejected.err() == Some(ArchiveError::TooLarge.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    #[test]
    fn canonical_staging_rejects_budget_duplicates_and_incomplete_inventory()
    -> Result<(), StagingError> {
        let limits = fixture_limits();
        let manifest = payload_manifest(limits)?;
        let empty = fixture_manifest(limits)?;
        let digest = ContentHash::calculate(b"tile");
        let four = NonZeroU64::MIN.saturating_add(3);
        let tight = ArchiveBudget::new(four, four, NonZeroU64::MIN);
        let mut stage = CanonicalStaging::open_with::<CanonicalFake>(
            OsStr::new("good"),
            tight,
        )?;
        if stage.write_payload(digest, b"wrong")
            != Err(ArchiveError::TooLarge.into())
            || stage.write_payload(digest, b"tile")
                != Err(StagingError::Aborted)
        {
            return Err(StagingError::ChangedStorage);
        }
        let budget = canonical_budget();
        let missing = CanonicalStaging::open_with::<CanonicalFake>(
            OsStr::new("good"),
            budget,
        )?;
        if missing.finish(&manifest, limits).err()
            != Some(AssetError::PayloadMismatch.into())
        {
            return Err(StagingError::ChangedStorage);
        }
        let mut duplicate = CanonicalStaging::open_with::<CanonicalFake>(
            OsStr::new("good"),
            budget,
        )?;
        duplicate.write_payload(digest, b"tile")?;
        if duplicate.write_payload(digest, b"tile")
            != Err(StagingError::Collision)
        {
            return Err(StagingError::ChangedStorage);
        }
        let mut extra = CanonicalStaging::open_with::<CanonicalFake>(
            OsStr::new("good"),
            budget,
        )?;
        extra.write_payload(digest, b"tile")?;
        (extra.finish(&empty, limits).err()
            == Some(AssetError::PayloadMismatch.into()))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
    }

    fn authored() -> AuthoredStaging {
        AuthoredStaging {
            namespace: Namespace::default(),
            transaction: Transaction::new(Box::new(AuthoredFake)),
        }
    }

    #[test]
    fn authored_publication_rejects_nonportable_destinations() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "nested/entry",
            "end/",
            "a\\b",
            "C:drive",
            "NUL",
            "trailing.",
            "trailing ",
            "caf\u{e9}",
            "a\0b",
        ] {
            assert!(matches!(
                authored().publish(OsStr::new(name)),
                Err(StagingError::UnsupportedPath)
            ));
        }
    }

    #[test]
    fn authored_file_admission_preserves_nested_bytes_and_mode()
    -> Result<(), StagingError> {
        let bytes = b"independently authored";
        let member = AdmittedAuthoredFile::new(
            "nested/bin/program.exe",
            bytes,
            true,
            &mut Namespace::default(),
        )?;
        let mut stage = authored();
        (member.parts() == ("nested/bin", "program.exe")
            && member.bytes() == bytes
            && member.size() == 22
            && member.executable()
            && stage.write_file("../bad", bytes, true)
                == Err(StagingError::Safety(ArchiveError::UnsafePath))
            && matches!(
                stage.publish(OsStr::new("invalid/name")),
                Err(StagingError::Aborted)
            ))
        .then_some(())
        .ok_or(StagingError::ChangedStorage)
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
