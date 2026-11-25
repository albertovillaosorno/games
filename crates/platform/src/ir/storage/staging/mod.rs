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

use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::io::{Error, ErrorKind, Read};

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryKind, validate_path,
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
    fn open_archive(parent: &OsStr, budget: ArchiveBudget) -> OpenArchive;
}

pub(crate) trait ArchivePort: Debug {
    fn append_member(
        &mut self,
        metadata: StagingEntry<'_>,
        input: &mut dyn Read,
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
            transaction: Transaction::new(
                <() as ArchiveFactory>::open_archive(parent.as_ref(), budget)?,
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
        self.transaction
            .apply(|backend| backend.append_member(metadata, input))
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
    fn copy_bundle(
        parent: &OsStr,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        limits: ManifestLimits,
    ) -> CompletedBundle;

    fn open_canonical(parent: &OsStr, budget: ArchiveBudget) -> OpenCanonical;
}

pub(crate) trait CanonicalPort: Debug {
    fn complete(
        self: Box<Self>,
        manifest: &AssetManifest,
        limits: ManifestLimits,
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
        Ok(StagedBundle {
            backend: <() as CanonicalFactory>::copy_bundle(
                parent.as_ref(),
                budget,
                source,
                limits,
            )?,
        })
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
        Ok(StagedBundle {
            backend: self.transaction.finish()?.complete(manifest, limits)?,
        })
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
}

impl StagedBundle {
    /// Digest of the exact canonical manifest bytes.
    #[must_use]
    pub fn digest(&self) -> ContentHash {
        self.backend.identity()
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
    use super::*;

    #[derive(Debug)]
    struct ArchiveFake;

    impl ArchivePort for ArchiveFake {
        fn append_member(
            &mut self,
            metadata: StagingEntry<'_>,
            input: &mut dyn Read,
        ) -> Result<(), StagingError> {
            let mut bytes = [0; 4];
            if metadata.path != "asset" || metadata.size != 4 {
                return Err(StagingError::UnsupportedPath);
            }
            input.read_exact(&mut bytes)?;
            (bytes == *b"tile")
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
    struct CanonicalFake;

    impl CanonicalPort for CanonicalFake {
        fn complete(
            self: Box<Self>,
            manifest: &AssetManifest,
            limits: ManifestLimits,
        ) -> CompletedBundle {
            let bytes = manifest.encode(limits)?;
            Ok(Box::new(BundleFake(ContentHash::calculate(&bytes))))
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

    #[test]
    fn archive_stream_and_recheck_use_the_selected_port()
    -> Result<(), StagingError> {
        let mut stage = ArchiveStaging {
            transaction: Transaction::new(Box::new(ArchiveFake)),
        };
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

    #[test]
    fn canonical_completion_and_publication_preserve_identity()
    -> Result<(), StagingError> {
        use std::num::{NonZeroU64, NonZeroUsize};

        use games_assets::SourceIdentity;

        let limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(15),
            records: NonZeroUsize::MIN.saturating_add(3),
            total_bytes: NonZeroU64::MIN.saturating_add(63),
        };
        let manifest = AssetManifest::new(
            SourceIdentity {
                revision: String::from("fixture"),
                title: String::from("test"),
            },
            Vec::new(),
            limits,
        )?;
        let mut rejected = CanonicalStaging {
            transaction: Transaction::new(Box::new(CanonicalFake)),
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
            transaction: Transaction::new(Box::new(CanonicalFake)),
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
