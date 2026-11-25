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
//   - Private native staging of bounded archive and canonical asset data.
// - Must-Not:
//   - Parse source media or execute imported data.
// - Allows:
//   - Inputs: Trusted parent, bounded streams, canonical payloads and
//     manifests.
//   - Outputs: Completed temporary archive or canonical bundle, or rejection.
//   - Side effects: Owner-only files and directories under the staging root.
// - Split-When:
//   - Publication or source-format decoding requires a separate adapter.
// - Merge-When:
//   - A shared contained filesystem provides the same staging guarantees.
// - Summary:
//   - Non-overwriting, descriptor-relative staging for native importers.
// - Description:
//   - Applies shared archive budgets and a portable case-insensitive namespace.
// - Usage:
//   - Stage decoded entries or canonical assets, then finish before use.
// - Defaults:
//   - Reject links, executables, collisions and partial imports.

mod publication;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use games_assets::archive::{
    ArchiveBudget, ArchiveError, EntryBudget, EntryKind,
};
use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};
use nix::errno::Errno;
use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::{FileStat, Mode, fstat, mkdirat};
pub use publication::StagedBundle;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub use publication::{AuthoredStaging, PublishedBundle, PublishedDirectory};

use super::bundle::{
    BundleError, CanonicalBundle, MANIFEST_NAME, PAYLOADS_DIRECTORY,
};
use super::file::{
    DIRECTORY_FLAGS, same_inode, trusted_directory, trusted_directory_stat,
};
const MAX_DEPTH: usize = 64;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    Io(io::ErrorKind),
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

impl From<Errno> for StagingError {
    fn from(error: Errno) -> Self {
        Self::Io(io::Error::from(error).kind())
    }
}

impl From<io::Error> for StagingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<ArchiveError> for StagingError {
    fn from(error: ArchiveError) -> Self {
        Self::Safety(error)
    }
}

#[derive(Debug, Default)]
struct Directory {
    children: BTreeMap<String, Node>,
    explicit: bool,
}

#[derive(Debug)]
struct Node {
    directory: Option<Directory>,
    name: String,
    // Directory identity or completed file metadata for verification.
    stamp: Option<Box<FileStat>>,
}

impl Node {
    fn record_file(
        &mut self,
        file: &File,
        size: u64,
    ) -> Result<(), StagingError> {
        let stamp = fstat(file)?;
        if u64::try_from(stamp.st_size).ok() != Some(size) {
            return Err(StagingError::ChangedStorage);
        }
        self.stamp = Some(Box::new(stamp));
        Ok(())
    }
}

type CreatedFile<'directory> = (File, &'directory mut Node);
type DescendedDirectory<'directory> =
    (&'directory mut Directory, Option<OwnedFd>);
type PathParts<'path> = (&'path str, &'path str);

#[derive(Debug)]
enum DirectoryAdmission<'budget> {
    Explicit,
    Implicit(Option<&'budget mut ArchiveBudget>),
}

impl Directory {
    fn create_file(
        &mut self,
        parent: &OwnedFd,
        name: &str,
    ) -> Result<CreatedFile<'_>, StagingError> {
        let Entry::Vacant(entry) =
            self.children.entry(name.to_ascii_lowercase())
        else {
            return Err(StagingError::Collision);
        };
        let fd = openat(
            parent,
            name,
            OFlag::O_WRONLY
                | OFlag::O_CREAT
                | OFlag::O_EXCL
                | OFlag::O_NOFOLLOW
                | OFlag::O_CLOEXEC,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )?;
        let node = entry.insert(Node {
            directory: None,
            name: name.to_owned(),
            stamp: None,
        });
        Ok((File::from(fd), node))
    }

    fn descend(
        &mut self,
        root: &OwnedFd,
        parents: &str,
        mut budget: Option<&mut ArchiveBudget>,
    ) -> Result<DescendedDirectory<'_>, StagingError> {
        let mut directory = self;
        let mut descriptor = None;
        if !parents.is_empty() {
            for part in parents.split('/') {
                let parent = descriptor.as_ref().unwrap_or(root);
                let OpenedDirectory { child, fd } = directory.open_child(
                    parent,
                    part,
                    DirectoryAdmission::Implicit(budget.as_deref_mut()),
                )?;
                directory = child;
                descriptor = Some(fd);
            }
        }
        Ok((directory, descriptor))
    }

    fn open_child(
        &mut self,
        parent: &OwnedFd,
        name: &str,
        admission: DirectoryAdmission<'_>,
    ) -> Result<OpenedDirectory<'_>, StagingError> {
        let explicit = matches!(admission, DirectoryAdmission::Explicit);
        let node = match self.children.entry(name.to_ascii_lowercase()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if let DirectoryAdmission::Implicit(Some(limits)) = admission {
                    limits.admit(name, EntryKind::Directory, 0)?.finish()?;
                }
                mkdirat(parent, name, Mode::S_IRWXU)?;
                entry.insert(Node {
                    directory: Some(Self::default()),
                    name: name.to_owned(),
                    stamp: None,
                })
            },
        };
        if node.name != name {
            return Err(StagingError::Collision);
        }
        let child = node.directory.as_mut().ok_or(StagingError::Collision)?;
        if explicit && child.explicit {
            return Err(StagingError::Collision);
        }
        let fd = openat(parent, name, DIRECTORY_FLAGS, Mode::empty())?;
        let current = fstat(&fd)?;
        if !trusted_directory_stat(&current)
            || node
                .stamp
                .as_deref()
                .is_some_and(|stamp| !same_inode(stamp, &current))
        {
            return Err(StagingError::ChangedStorage);
        }
        if node.stamp.is_none() {
            node.stamp = Some(Box::new(current));
        }
        child.explicit |= explicit;
        Ok(OpenedDirectory { child, fd })
    }
}

#[derive(Debug)]
struct OpenedDirectory<'directory> {
    child: &'directory mut Directory,
    fd: OwnedFd,
}

#[derive(Debug)]
struct PrivateDirectory {
    cleanup: bool,
    parent: OwnedFd,
    path: PathBuf,
    root: OwnedFd,
}

impl PrivateDirectory {
    fn create(parent: &Path, parent_fd: OwnedFd) -> Result<Self, StagingError> {
        for _ in 0u8..128 {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".games-import-{}-{sequence}", process::id());
            match mkdirat(&parent_fd, name.as_str(), Mode::S_IRWXU) {
                Ok(()) => {
                    let root = openat(
                        &parent_fd,
                        name.as_str(),
                        DIRECTORY_FLAGS,
                        Mode::empty(),
                    )?;
                    return Ok(Self {
                        cleanup: true,
                        parent: parent_fd,
                        path: parent.join(&name),
                        root,
                    });
                },
                Err(Errno::EEXIST) => {},
                Err(error) => return Err(error.into()),
            }
        }
        Err(StagingError::Io(io::ErrorKind::AlreadyExists))
    }

    fn create_in(parent: &Path) -> Result<Self, StagingError> {
        let parent_fd = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
        if !trusted_directory(&parent_fd)? {
            return Err(StagingError::UntrustedParent);
        }
        let absolute_parent = fs::canonicalize(parent)?;
        let canonical = open(&absolute_parent, DIRECTORY_FLAGS, Mode::empty())?;
        if !same_directory(&parent_fd, &canonical)? {
            return Err(StagingError::ChangedStorage);
        }
        let temporary = Self::create(&absolute_parent, parent_fd)?;
        temporary.verify_location()?;
        Ok(temporary)
    }

    // The public staging path must still name the directories we pinned.
    // A same-user replacement can leave an orphan but must not publish or
    // delete the unrelated directory now present at that path.
    fn verify_location(&self) -> Result<(), StagingError> {
        let path = self.path.parent().ok_or(StagingError::UnsupportedPath)?;
        let name =
            self.path.file_name().ok_or(StagingError::UnsupportedPath)?;
        let parent = open(path, DIRECTORY_FLAGS, Mode::empty())?;
        if !trusted_directory(&self.parent)? {
            return Err(StagingError::UntrustedParent);
        }
        if !same_directory(&self.parent, &parent)? {
            return Err(StagingError::ChangedStorage);
        }
        let root = openat(&self.parent, name, DIRECTORY_FLAGS, Mode::empty())?;
        if !trusted_directory(&self.root)?
            || !same_directory(&self.root, &root)?
        {
            return Err(StagingError::ChangedStorage);
        }
        Ok(())
    }

    fn verify_tree(
        &self,
        registry: &Directory,
        sync: bool,
    ) -> Result<(), StagingError> {
        self.verify_location()?;
        publication::check_tree(registry, &self.root, None, sync)?;
        self.verify_location()
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        if self.cleanup && self.verify_location().is_ok() {
            let _result = fs::remove_dir_all(&self.path);
        }
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

/// An incomplete private archive transaction.
///
/// The parent and its ancestors must be trusted host-controlled directories.
/// All archive-controlled components are opened relative to directory handles
/// without following symbolic links. Drop attempts cleanup of partial data.
#[derive(Debug)]
pub struct ArchiveStaging {
    budget: ArchiveBudget,
    failed: bool,
    registry: Directory,
    temporary: PrivateDirectory,
}

impl ArchiveStaging {
    /// Complete staging only if every attempted entry was accepted.
    ///
    /// # Errors
    /// Rejects an aborted transaction and discards its partial output.
    pub fn finish(self) -> Result<StagedArchive, StagingError> {
        self.finish_with(false)
    }

    fn finish_with(self, sync: bool) -> Result<StagedArchive, StagingError> {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        self.temporary.verify_tree(&self.registry, sync)?;
        Ok(StagedArchive {
            registry: self.registry,
            temporary: self.temporary,
        })
    }

    /// Create a private temporary directory under a trusted parent.
    ///
    /// The entry budget also charges each implicitly created directory.
    ///
    /// # Errors
    /// Rejects parents not owned by the current effective user,
    /// writable-by-others parents, symbolic-link parents and I/O failures.
    pub fn new(
        parent: &Path,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError> {
        Ok(Self {
            budget,
            failed: false,
            registry: Directory::default(),
            temporary: PrivateDirectory::create_in(parent)?,
        })
    }

    fn record_file(
        &mut self,
        path: &str,
        size: u64,
        bytes: &mut &[u8],
    ) -> Result<(), StagingError> {
        self.write_entry(
            StagingEntry {
                kind: EntryKind::File,
                path,
                size,
            },
            bytes,
        )
    }

    /// Validate one declared member and stream its exact payload into staging.
    ///
    /// Paths use portable ASCII components and at most 64 levels. Directory
    /// entries may follow implicitly created parents but cannot repeat.
    ///
    /// # Errors
    /// Rejects unsafe metadata, collisions, excess data and filesystem errors.
    /// Any failure aborts the whole staging transaction.
    pub fn write_entry<Source>(
        &mut self,
        metadata: StagingEntry<'_>,
        input: &mut Source,
    ) -> Result<(), StagingError>
    where
        Source: Read,
    {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        let result = self.write_member(metadata, input);
        self.failed = result.is_err();
        result
    }

    fn write_member<Source>(
        &mut self,
        metadata: StagingEntry<'_>,
        input: &mut Source,
    ) -> Result<(), StagingError>
    where
        Source: Read,
    {
        let StagingEntry { kind, path, size } = metadata;
        let entry = self.budget.admit(path, kind, size)?;
        let (parents, leaf) = staging_path(path)?;
        let (directory, descriptor) = self.registry.descend(
            &self.temporary.root,
            parents,
            Some(&mut self.budget),
        )?;
        let parent = descriptor.as_ref().unwrap_or(&self.temporary.root);
        match kind {
            EntryKind::Directory => {
                copy_bounded(input, &mut io::sink(), entry)?;
                let _directory = directory.open_child(
                    parent,
                    leaf,
                    DirectoryAdmission::Explicit,
                )?;
                Ok(())
            },
            EntryKind::File => {
                let (mut output, node) = directory.create_file(parent, leaf)?;
                copy_bounded(input, &mut output, entry)?;
                node.record_file(&output, size)
            },
            EntryKind::Hardlink | EntryKind::Other | EntryKind::Symlink => {
                Err(ArchiveError::LinkOrSpecial.into())
            },
        }
    }
}

type PayloadSizes = BTreeMap<ContentHash, u64>;

/// A private canonical bundle, built from content-addressed payloads.
///
/// Only hashes become paths; completion requires an exact manifest match.
/// The archive budget includes the manifest file and payload directory.
#[derive(Debug)]
pub struct CanonicalStaging {
    payloads: PayloadSizes,
    stage: ArchiveStaging,
}

impl CanonicalStaging {
    /// Copy a pinned bundle into a new complete canonical staging transaction.
    ///
    /// Each distinct payload is verified before copying. Shared payloads
    /// occupy one staged file; all declared lengths must agree. A legacy
    /// manifest is encoded using the current fingerprint-free source metadata.
    /// The source bundle is never modified.
    ///
    /// # Errors
    /// Rejects unsafe, corrupted or oversized input and any staging failure.
    pub fn copy_bundle(
        parent: &Path,
        budget: ArchiveBudget,
        source: &CanonicalBundle,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError> {
        let manifest = source.manifest();
        let bytes = manifest.encode(limits)?;
        let expected = manifest.payload_sizes();
        admit_copy(budget, &bytes, &expected)?;
        let mut stage = Self::new(parent, budget)?;
        for record in manifest.records() {
            if stage.payloads.contains_key(&record.digest) {
                continue;
            }
            let payload = source.read_payload(&record.key)?;
            stage.write_payload(record.digest, &payload)?;
        }
        stage.finish_encoded(&bytes, &expected)
    }

    /// Finish a bundle only if every staged payload is declared and present.
    ///
    /// # Errors
    /// Rejects failed staging, mismatches and invalid or oversized metadata.
    pub fn finish(
        self,
        manifest: &AssetManifest,
        limits: ManifestLimits,
    ) -> Result<StagedBundle, StagingError> {
        if self.stage.failed {
            return Err(StagingError::Aborted);
        }
        let bytes = manifest.encode(limits)?;
        let expected = manifest.payload_sizes();
        self.finish_encoded(&bytes, &expected)
    }

    fn finish_encoded(
        mut self,
        mut bytes: &[u8],
        expected: &PayloadSizes,
    ) -> Result<StagedBundle, StagingError> {
        if expected != &self.payloads {
            return Err(AssetError::PayloadMismatch.into());
        }
        let size = manifest_size(bytes)?;
        let mut budget = self.stage.budget;
        let _entry = budget.admit(MANIFEST_NAME, EntryKind::File, size)?;
        let digest = ContentHash::calculate(bytes);
        self.stage.record_file(MANIFEST_NAME, size, &mut bytes)?;
        publication::finish(self.stage, digest)
    }

    /// Create a private canonical bundle transaction.
    ///
    /// # Errors
    /// Rejects untrusted parents and filesystem failures.
    pub fn new(
        parent: &Path,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError> {
        Ok(Self {
            payloads: BTreeMap::new(),
            stage: ArchiveStaging::new(parent, budget)?,
        })
    }

    /// Verify and stage one unique canonical payload.
    ///
    /// # Errors
    /// Rejects hash mismatch, duplicate content, exhausted budgets or I/O.
    pub fn write_payload(
        &mut self,
        digest: ContentHash,
        mut bytes: &[u8],
    ) -> Result<(), StagingError> {
        if self.stage.failed {
            return Err(StagingError::Aborted);
        }
        if self.payloads.contains_key(&digest) {
            self.stage.failed = true;
            return Err(StagingError::Collision);
        }
        let Ok(size) = u64::try_from(bytes.len()) else {
            self.stage.failed = true;
            return Err(AssetError::TooLarge.into());
        };
        let path = payload_path(digest);
        // Preflight on a copy; write_entry consumes the actual budget.
        let mut budget = self.stage.budget;
        if let Err(error) =
            admit_payload(&mut budget, &path, size, self.payloads.is_empty())
        {
            self.stage.failed = true;
            return Err(error);
        }
        if ContentHash::calculate(bytes) != digest {
            self.stage.failed = true;
            return Err(AssetError::PayloadMismatch.into());
        }
        self.stage.record_file(&path, size, &mut bytes)?;
        let _previous = self.payloads.insert(digest, size);
        Ok(())
    }
}

/// A verified staging directory with best-effort cleanup on drop.
#[derive(Debug)]
pub struct StagedArchive {
    registry: Directory,
    temporary: PrivateDirectory,
}

impl StagedArchive {
    /// Access the completed staging directory for subsequent conversion.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.temporary.path
    }

    /// Recheck the exact staged tree before subsequent trusted use.
    ///
    /// # Errors
    /// Rejects observed storage changes or filesystem errors.
    pub fn verify(&self) -> Result<(), StagingError> {
        self.temporary.verify_tree(&self.registry, false)
    }
}

fn admit_copy(
    mut budget: ArchiveBudget,
    bytes: &[u8],
    payloads: &PayloadSizes,
) -> Result<(), StagingError> {
    let _manifest =
        budget.admit(MANIFEST_NAME, EntryKind::File, manifest_size(bytes)?)?;
    for (index, (&digest, &size)) in payloads.iter().enumerate() {
        admit_payload(&mut budget, &payload_path(digest), size, index == 0)?;
    }
    Ok(())
}

fn admit_payload(
    budget: &mut ArchiveBudget,
    path: &str,
    size: u64,
    first: bool,
) -> Result<(), StagingError> {
    let _payload = budget.admit(path, EntryKind::File, size)?;
    if first {
        budget
            .admit(PAYLOADS_DIRECTORY, EntryKind::Directory, 0)?
            .finish()?;
    }
    Ok(())
}

fn manifest_size(bytes: &[u8]) -> Result<u64, AssetError> {
    u64::try_from(bytes.len()).map_err(|_error| AssetError::TooLarge)
}

fn payload_path(digest: ContentHash) -> String {
    format!("{PAYLOADS_DIRECTORY}/{digest}")
}

fn copy_bounded<Source, Destination>(
    input: &mut Source,
    output: &mut Destination,
    mut budget: EntryBudget,
) -> Result<(), StagingError>
where
    Source: Read,
    Destination: Write,
{
    let mut buffer = [0; 8192];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => return Ok(budget.finish()?),
            Ok(count) => {
                let chunk =
                    buffer.get(..count).ok_or(ArchiveError::InvalidEntry)?;
                budget.record(chunk.len())?;
                output.write_all(chunk)?;
            },
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(error) => return Err(error.into()),
        }
    }
}

fn same_directory(left: &OwnedFd, right: &OwnedFd) -> nix::Result<bool> {
    let before = fstat(left)?;
    let after = fstat(right)?;
    Ok(same_inode(&before, &after))
}

fn staging_path(path: &str) -> Result<PathParts<'_>, StagingError> {
    if !path.is_ascii() || path.split('/').count() > MAX_DEPTH {
        return Err(StagingError::UnsupportedPath);
    }
    Ok(path.rsplit_once('/').unwrap_or(("", path)))
}

#[cfg(test)]
mod tests {
    use std::fs::FileTimes;
    use std::num::{NonZeroU64, NonZeroUsize};
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::time::UNIX_EPOCH;

    use games_assets::{AssetFormat, AssetRecord, SourceIdentity};
    use nix::unistd::mkfifo;

    use super::*;
    use crate::bundle::TestRoot;

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    type CopyFixture = (CanonicalBundle, PublishedBundle);

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_staging_rejects_non_component_destinations()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        for name in ["", ".", "..", "../escape", "nested/entry", "end/"] {
            let stage = AuthoredStaging::new(&parent.path)?;
            let temporary = stage.path().to_owned();
            require(matches!(
                stage.publish(OsStr::new(name)),
                Err(StagingError::UnsupportedPath)
            ))?;
            require(!temporary.exists())?;
        }
        require(fs::read_dir(&parent.path)?.count() == 0)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_tree_publishes_exact_bytes_and_modes()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        let mut stage = AuthoredStaging::new(&parent.path)?;
        stage.write_file("program", b"program", true)?;
        stage.write_file("licenses/NOTICE", b"notice", false)?;
        stage.write_file("licenses/nested/LICENSE", b"licence", false)?;
        stage.write_file("licenses/nested/OTHER", b"", false)?;
        let result = stage.publish(OsStr::new("published"))?;
        require(result.durability.is_ok())?;
        let path = result.path;
        for (name, bytes, mode) in [
            ("program", b"program".as_slice(), 0o700),
            ("licenses/NOTICE", b"notice".as_slice(), 0o600),
            ("licenses/nested/LICENSE", b"licence".as_slice(), 0o600),
            ("licenses/nested/OTHER", b"".as_slice(), 0o600),
        ] {
            require(fs::read(path.join(name))? == bytes)?;
            require(
                fs::metadata(path.join(name))?.permissions().mode() & 0o777
                    == mode,
            )?;
        }
        require(fs::read_dir(&path)?.count() == 2)?;
        require(fs::read_dir(path.join("licenses/nested"))?.count() == 2)?;
        require(fs::read_dir(&parent.path)?.count() == 1)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_tree_rejects_unsafe_or_colliding_writes()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        for name in [
            "../escape",
            "A/other",
            "a/file",
            "a/file/nested",
            "a",
            "a//other",
            "/absolute",
            "a/./other",
            "a/../other",
        ] {
            let mut stage = AuthoredStaging::new(&parent.path)?;
            stage.write_file("a/file", b"old", false)?;
            let path = stage.path().to_owned();
            require(stage.write_file(name, b"new", false).is_err())?;
            require(
                stage.write_file("next", b"next", false)
                    == Err(StagingError::Aborted),
            )?;
            require(matches!(
                stage.publish(OsStr::new("published")),
                Err(StagingError::Aborted)
            ))?;
            require(!path.exists())?;
        }
        require(fs::read_dir(&parent.path)?.count() == 0)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_tree_rejects_changed_files_and_unlisted_entries()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        let outside = fixture()?;
        let sentinel = outside.path.join("sentinel");
        fs::write(&sentinel, b"safe")?;
        for change in 0u8..6 {
            let mut stage = AuthoredStaging::new(&parent.path)?;
            stage.write_file("program", b"good", true)?;
            stage.write_file("licenses/LICENSE", b"good", false)?;
            let path = stage.path().to_owned();
            let target = path.join("licenses/LICENSE");
            match change {
                0 => {
                    fs::rename(&target, outside.path.join("prior"))?;
                    fs::write(&target, b"evil")?;
                },
                1 => {
                    fs::write(&target, b"evil")?;
                    File::open(&target)?
                        .set_times(FileTimes::new().set_modified(UNIX_EPOCH))?;
                },
                2 => fs::write(path.join("unlisted"), b"extra")?,
                3 => fs::create_dir_all(path.join("licenses/extra"))?,
                4 => {
                    fs::remove_file(&target)?;
                    symlink(&sentinel, &target)?;
                },
                _ => fs::set_permissions(
                    path.join("program"),
                    fs::Permissions::from_mode(0o600),
                )?,
            }
            require(stage.publish(OsStr::new("published")).is_err())?;
            require(!path.exists())?;
            require(fs::read(&sentinel)? == b"safe")?;
            require(fs::read_dir(&parent.path)?.count() == 0)?;
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_tree_rejects_substituted_directory_before_writing()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        let outside = fixture()?;
        let mut stage = AuthoredStaging::new(&parent.path)?;
        stage.write_file("licenses/first", b"first", false)?;
        let directory = stage.path().join("licenses");
        let original = outside.path.join("original");
        fs::rename(&directory, &original)?;
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("sentinel"), b"safe")?;
        require(
            stage.write_file("licenses/second", b"second", false)
                == Err(StagingError::ChangedStorage),
        )?;
        require(!directory.join("second").exists())?;
        require(fs::read(directory.join("sentinel"))? == b"safe")?;
        require(matches!(
            stage.publish(OsStr::new("published")),
            Err(StagingError::Aborted)
        ))?;
        require(fs::read(original.join("first"))? == b"first")?;
        require(fs::read_dir(&parent.path)?.count() == 0)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn authored_tree_rejects_substituted_directory_at_publication()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let parent = fixture()?;
        let outside = fixture()?;
        let mut stage = AuthoredStaging::new(&parent.path)?;
        stage.write_file("licenses/LICENSE", b"good", false)?;
        let directory = stage.path().join("licenses");
        let original = outside.path.join("original");
        fs::rename(&directory, &original)?;
        fs::create_dir_all(&directory)?;
        fs::rename(original.join("LICENSE"), directory.join("LICENSE"))?;
        require(matches!(
            stage.publish(OsStr::new("published")),
            Err(StagingError::ChangedStorage)
        ))?;
        require(fs::read_dir(&parent.path)?.count() == 0)
    }

    fn budget(entries: u64) -> ArchiveBudget {
        ArchiveBudget::new(
            NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(64).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(entries).unwrap_or(NonZeroU64::MIN),
        )
    }

    fn fixture() -> Result<TestRoot, StagingError> {
        Ok(TestRoot::new()?)
    }

    const fn member(
        path: &str,
        kind: EntryKind,
        size: u64,
    ) -> StagingEntry<'_> {
        StagingEntry { kind, path, size }
    }

    fn require(condition: bool) -> Result<(), StagingError> {
        condition
            .then_some(())
            .ok_or(StagingError::Io(io::ErrorKind::InvalidData))
    }

    #[test]
    fn completed_archive_has_private_files_and_cleans_up()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
        stage.write_entry(
            member("sprites/hero.bin", EntryKind::File, 4),
            &mut b"hero".as_slice(),
        )?;
        stage.write_entry(
            member("sprites", EntryKind::Directory, 0),
            &mut io::empty(),
        )?;
        stage.write_entry(
            member("sprites/villain.bin", EntryKind::File, 0),
            &mut io::empty(),
        )?;
        let completed = stage.finish()?;
        let root = completed.path().to_owned();
        let file = root.join("sprites/hero.bin");
        require(fs::read(&file)? == b"hero")?;
        require(
            fs::metadata(&file)?.permissions().mode().trailing_zeros() >= 7,
        )?;
        require(
            fs::metadata(&root)?.permissions().mode().trailing_zeros() >= 6,
        )?;
        require(fs::metadata(root.join("sprites/villain.bin"))?.len() == 0)?;
        drop(completed);
        require(!root.exists())
    }

    #[test]
    fn archive_completion_rejects_nested_tree_changes()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        for alteration in ["file", "extra", "empty-directory"] {
            let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
            stage.write_entry(
                member("sprites/hero.bin", EntryKind::File, 4),
                &mut b"hero".as_slice(),
            )?;
            stage.write_entry(
                member("sprites/empty", EntryKind::Directory, 0),
                &mut io::empty(),
            )?;
            let path = stage.temporary.path.clone();
            let original = parent.path.join("original");
            match alteration {
                "file" => {
                    let target = path.join("sprites/hero.bin");
                    fs::write(&target, b"evil")?;
                    File::open(&target)?
                        .set_times(FileTimes::new().set_modified(UNIX_EPOCH))?;
                },
                "extra" => {
                    fs::write(path.join("sprites/extra"), b"unlisted")?;
                },
                "empty-directory" => {
                    let target = path.join("sprites/empty");
                    fs::rename(&target, &original)?;
                    fs::create_dir_all(target)?;
                },
                _ => return Err(StagingError::UnsupportedPath),
            }
            require(matches!(
                stage.finish(),
                Err(StagingError::ChangedStorage)
            ))?;
            require(!path.exists())?;
        }
        Ok(())
    }

    #[test]
    fn completed_archive_can_recheck_before_conversion()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
        stage.write_entry(
            member("empty/nested", EntryKind::Directory, 0),
            &mut io::empty(),
        )?;
        let complete = stage.finish()?;
        complete.verify()?;
        let path = complete.path().to_owned();
        fs::write(path.join("empty/nested/unlisted"), b"extra")?;
        require(complete.verify() == Err(StagingError::ChangedStorage))?;
        drop(complete);
        require(!path.exists())
    }

    #[test]
    fn replaced_archive_root_is_not_completed_or_removed()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
        stage.write_entry(
            member("asset", EntryKind::File, 4),
            &mut b"tile".as_slice(),
        )?;
        let named = stage.temporary.path.clone();
        let original = parent.path.join("original");
        fs::rename(&named, &original)?;
        fs::create_dir_all(&named)?;
        fs::write(named.join("sentinel"), b"untouched")?;
        require(matches!(stage.finish(), Err(StagingError::ChangedStorage)))?;
        require(fs::read(named.join("sentinel"))? == b"untouched")?;
        require(fs::read(original.join("asset"))? == b"tile")
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn replaced_parent_cannot_redirect_authored_publication()
    -> Result<(), StagingError> {
        use std::ffi::OsStr;

        let fixture = fixture()?;
        let parent = fixture.path.join("parent");
        let original = fixture.path.join("original");
        fs::create_dir_all(&parent)?;
        let mut stage = AuthoredStaging::new(&parent)?;
        let named = stage.path().to_owned();
        let leaf = named.file_name().ok_or(StagingError::UnsupportedPath)?;
        stage.write_file("asset", b"tile", false)?;
        fs::rename(&parent, &original)?;
        fs::create_dir_all(&named)?;
        fs::write(named.join("sentinel"), b"untouched")?;
        require(matches!(
            stage.publish(OsStr::new("published")),
            Err(StagingError::ChangedStorage)
        ))?;
        require(fs::read(named.join("sentinel"))? == b"untouched")?;
        require(fs::read(original.join(leaf).join("asset"))? == b"tile")?;
        require(!parent.join("published").exists())?;
        require(!original.join("published").exists())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn canonical_publication_rechecks_completed_staging_identity()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let mut stage =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        stage.write_payload(digest, b"tile")?;
        let staged = stage.finish(&manifest, canonical_limits())?;
        let named = staged.path().to_owned();
        let original = parent.path.join("original");
        let destination =
            parent.path.join(format!("bundle-{}", staged.digest()));
        fs::rename(&named, &original)?;
        fs::create_dir_all(&named)?;
        fs::write(named.join("sentinel"), b"untouched")?;
        require(matches!(
            staged.publish(),
            Err(StagingError::ChangedStorage)
        ))?;
        require(fs::read(named.join("sentinel"))? == b"untouched")?;
        require(
            fs::read(original.join(MANIFEST_NAME))?
                == manifest.encode(canonical_limits())?,
        )?;
        require(!destination.exists())
    }

    #[test]
    fn rejects_duplicate_and_colliding_paths() -> Result<(), StagingError> {
        let parent = fixture()?;
        for (first, second) in [
            ("sprite.bin", "sprite.bin"),
            ("Sprite.bin", "sprite.bin"),
            ("Sprites/hero", "sprites/villain"),
            ("sprites", "sprites/hero"),
        ] {
            let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
            stage.write_entry(
                member(first, EntryKind::File, 0),
                &mut io::empty(),
            )?;
            let root = stage.temporary.path.clone();
            require(
                stage.write_entry(
                    member(second, EntryKind::File, 0),
                    &mut io::empty(),
                ) == Err(StagingError::Collision),
            )?;
            require(matches!(stage.finish(), Err(StagingError::Aborted)))?;
            require(!root.exists())?;
        }
        Ok(())
    }

    #[test]
    fn rejects_directory_symlink_and_existing_leaf() -> Result<(), StagingError>
    {
        let parent = fixture()?;
        let outside = fixture()?;
        let sentinel = outside.path.join("sentinel");
        fs::write(&sentinel, b"untouched")?;
        let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
        stage.write_entry(
            member("assets", EntryKind::Directory, 0),
            &mut io::empty(),
        )?;
        let directory = stage.temporary.path.join("assets");
        fs::remove_dir(&directory)?;
        symlink(&outside.path, &directory)?;
        require(
            stage
                .write_entry(
                    member("assets/escape", EntryKind::File, 0),
                    &mut io::empty(),
                )
                .is_err(),
        )?;
        require(!outside.path.join("escape").exists())?;
        require(matches!(stage.finish(), Err(StagingError::Aborted)))?;
        let mut other = ArchiveStaging::new(&parent.path, budget(8))?;
        symlink(&sentinel, other.temporary.path.join("output"))?;
        require(
            other
                .write_entry(
                    member("output", EntryKind::File, 0),
                    &mut io::empty(),
                )
                .is_err(),
        )?;
        require(matches!(other.finish(), Err(StagingError::Aborted)))?;
        require(fs::read(&sentinel)? == b"untouched")
    }

    #[test]
    fn rejects_wrong_lengths_and_implicit_dir_overflow()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        for size in [1, 3] {
            let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
            let root = stage.temporary.path.clone();
            require(
                stage
                    .write_entry(
                        member("file", EntryKind::File, size),
                        &mut b"ab".as_slice(),
                    )
                    .is_err(),
            )?;
            require(
                stage.write_entry(
                    member("next", EntryKind::File, 0),
                    &mut io::empty(),
                ) == Err(StagingError::Aborted),
            )?;
            require(matches!(stage.finish(), Err(StagingError::Aborted)))?;
            require(!root.exists())?;
        }
        let mut stage = ArchiveStaging::new(&parent.path, budget(2))?;
        require(
            stage.write_entry(
                member("a/b/file", EntryKind::File, 0),
                &mut io::empty(),
            ) == Err(StagingError::Safety(ArchiveError::TooLarge)),
        )?;
        require(matches!(stage.finish(), Err(StagingError::Aborted)))
    }

    #[test]
    fn rejects_unsafe_metadata_before_creating_files()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        for (path, kind) in [
            ("../escape", EntryKind::File),
            ("unsafe.exe", EntryKind::File),
            ("link", EntryKind::Symlink),
            ("link", EntryKind::Hardlink),
            ("\u{f1}", EntryKind::File),
        ] {
            let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
            require(
                stage
                    .write_entry(member(path, kind, 0), &mut io::empty())
                    .is_err(),
            )?;
            require(fs::read_dir(&stage.temporary.path)?.count() == 0)?;
            require(matches!(stage.finish(), Err(StagingError::Aborted)))?;
        }
        Ok(())
    }

    #[test]
    fn rejects_untrusted_staging_parent() -> Result<(), StagingError> {
        let parent = fixture()?;
        let alias = fixture()?;
        let link = alias.path.join("parent");
        symlink(&parent.path, &link)?;
        require(ArchiveStaging::new(&link, budget(8)).is_err())?;
        fs::set_permissions(&parent.path, fs::Permissions::from_mode(0o770))?;
        let rejected = matches!(
            ArchiveStaging::new(&parent.path, budget(8)),
            Err(StagingError::UntrustedParent)
        );
        fs::set_permissions(&parent.path, fs::Permissions::from_mode(0o700))?;
        require(rejected)
    }

    #[test]
    fn rejects_duplicate_dir_and_existing_hardlink() -> Result<(), StagingError>
    {
        let parent = fixture()?;
        let mut stage = ArchiveStaging::new(&parent.path, budget(8))?;
        stage.write_entry(
            member("assets", EntryKind::Directory, 0),
            &mut io::empty(),
        )?;
        require(
            stage.write_entry(
                member("assets", EntryKind::Directory, 0),
                &mut io::empty(),
            ) == Err(StagingError::Collision),
        )?;
        require(matches!(stage.finish(), Err(StagingError::Aborted)))?;

        let outside = parent.path.join("sentinel");
        fs::write(&outside, b"untouched")?;
        let mut other = ArchiveStaging::new(&parent.path, budget(8))?;
        fs::hard_link(&outside, other.temporary.path.join("file"))?;
        require(
            other
                .write_entry(
                    member("file", EntryKind::File, 2),
                    &mut b"ab".as_slice(),
                )
                .is_err(),
        )?;
        require(matches!(other.finish(), Err(StagingError::Aborted)))?;
        require(fs::read(&outside)? == b"untouched")
    }

    fn canonical_limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::new(4096)
                .unwrap_or(NonZeroUsize::MIN),
            payload_bytes: NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            records: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            total_bytes: NonZeroU64::new(64).unwrap_or(NonZeroU64::MIN),
        }
    }

    fn canonical_budget() -> ArchiveBudget {
        ArchiveBudget::new(
            NonZeroU64::new(4096).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(8192).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(8).unwrap_or(NonZeroU64::MIN),
        )
    }

    fn manifest(digest: ContentHash) -> Result<AssetManifest, StagingError> {
        Ok(AssetManifest::new(
            SourceIdentity {
                title: String::from("synthetic"),
                revision: String::from("fixture"),
            },
            vec![
                AssetRecord {
                    digest,
                    format: AssetFormat::Data,
                    key: String::from("tile"),
                    size: 4,
                },
                AssetRecord {
                    digest,
                    format: AssetFormat::Data,
                    key: String::from("tile-copy"),
                    size: 4,
                },
            ],
            canonical_limits(),
        )?)
    }

    #[test]
    fn canonical_bundle_contains_exact_manifest_and_content()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let mut stage =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        stage.write_payload(digest, b"tile")?;
        let bundle = stage.finish(&manifest, canonical_limits())?;
        let encoded = fs::read(bundle.path().join("manifest.json"))?;
        let decoded = AssetManifest::parse(&encoded, canonical_limits())?;
        require(decoded == manifest)?;
        require(fs::read_dir(bundle.path().join("payloads"))?.count() == 1)?;
        require(
            fs::read(bundle.path().join(format!("payloads/{digest}")))?
                == b"tile",
        )?;
        let path = bundle.path().to_owned();
        drop(bundle);
        require(!path.exists())
    }

    #[test]
    fn canonical_completion_rejects_replaced_payload_metadata()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let sentinel = parent.path.join("sentinel");
        fs::write(&sentinel, b"tile")?;
        for replacement in [
            "fifo",
            "directory",
            "symlink",
            "hardlink",
            "short",
            "long",
            "same-length",
            "writable",
        ] {
            let mut stage =
                CanonicalStaging::new(&parent.path, canonical_budget())?;
            stage.write_payload(digest, b"tile")?;
            let path = stage.stage.temporary.path.clone();
            let payload = path.join(payload_path(digest));
            let _original = File::open(&payload)?;
            fs::remove_file(&payload)?;
            match replacement {
                "fifo" => mkfifo(&payload, Mode::S_IRUSR)?,
                "directory" => fs::create_dir_all(&payload)?,
                "symlink" => symlink(&sentinel, &payload)?,
                "hardlink" => fs::hard_link(&sentinel, &payload)?,
                "short" => fs::write(&payload, b"x")?,
                "long" => fs::write(&payload, b"tiles")?,
                "same-length" => fs::write(&payload, b"evil")?,
                "writable" => {
                    fs::write(&payload, b"tile")?;
                    fs::set_permissions(
                        &payload,
                        fs::Permissions::from_mode(0o666),
                    )?;
                },
                _ => return Err(StagingError::UnsupportedPath),
            }
            require(matches!(
                stage.finish(&manifest, canonical_limits()),
                Err(StagingError::ChangedStorage)
            ))?;
            require(!path.exists())?;
            require(fs::read(&sentinel)? == b"tile")?;
        }
        Ok(())
    }

    #[test]
    fn canonical_completion_rejects_in_place_changes_and_extra_files()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        for extra in ["", "extra", "payloads/extra"] {
            let mut stage =
                CanonicalStaging::new(&parent.path, canonical_budget())?;
            stage.write_payload(digest, b"tile")?;
            let path = stage.stage.temporary.path.clone();
            if extra.is_empty() {
                let payload = path.join(payload_path(digest));
                fs::write(&payload, b"evil")?;
                File::open(&payload)?
                    .set_times(FileTimes::new().set_modified(UNIX_EPOCH))?;
            } else {
                fs::write(path.join(extra), b"unlisted")?;
            }
            require(matches!(
                stage.finish(&manifest, canonical_limits()),
                Err(StagingError::ChangedStorage)
            ))?;
            require(!path.exists())?;
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn canonical_publication_rejects_changes_after_completion()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let payload = payload_path(digest);
        for name in [MANIFEST_NAME, payload.as_str(), "payloads/extra", "extra"]
        {
            let mut stage =
                CanonicalStaging::new(&parent.path, canonical_budget())?;
            stage.write_payload(digest, b"tile")?;
            let staged = stage.finish(&manifest, canonical_limits())?;
            let path = staged.path().to_owned();
            let destination =
                parent.path.join(format!("bundle-{}", staged.digest()));
            let changed = path.join(name);
            if changed.exists() {
                let mut bytes = fs::read(&changed)?;
                bytes.fill(b'x');
                fs::write(&changed, bytes)?;
                File::open(&changed)?
                    .set_times(FileTimes::new().set_modified(UNIX_EPOCH))?;
            } else {
                fs::write(changed, b"unlisted")?;
            }
            require(matches!(
                staged.publish(),
                Err(StagingError::ChangedStorage)
            ))?;
            require(!destination.exists())?;
            require(!path.exists())?;
        }
        Ok(())
    }

    #[test]
    fn canonical_bundle_rejects_missing_extra_or_wrong_content()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let missing = CanonicalStaging::new(&parent.path, canonical_budget())?;
        require(matches!(
            missing.finish(&manifest, canonical_limits()),
            Err(StagingError::Asset(AssetError::PayloadMismatch))
        ))?;
        let mut extra =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        extra.write_payload(digest, b"tile")?;
        extra.write_payload(ContentHash::calculate(b"other"), b"other")?;
        require(matches!(
            extra.finish(&manifest, canonical_limits()),
            Err(StagingError::Asset(AssetError::PayloadMismatch))
        ))?;
        let mut wrong =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        require(
            wrong.write_payload(digest, b"evil")
                == Err(StagingError::Asset(AssetError::PayloadMismatch)),
        )?;
        require(matches!(
            wrong.finish(&manifest, canonical_limits()),
            Err(StagingError::Aborted)
        ))
    }

    #[test]
    fn canonical_budget_rejection_precedes_payload_hashing()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let wrong = ContentHash::calculate(b"wrong");
        for limits in [
            ArchiveBudget::new(
                NonZeroU64::MIN,
                NonZeroU64::MAX,
                NonZeroU64::MAX,
            ),
            ArchiveBudget::new(
                NonZeroU64::MAX,
                NonZeroU64::MIN,
                NonZeroU64::MAX,
            ),
            ArchiveBudget::new(
                NonZeroU64::MAX,
                NonZeroU64::MAX,
                NonZeroU64::MIN,
            ),
        ] {
            let mut stage = CanonicalStaging::new(&parent.path, limits)?;
            let path = stage.stage.temporary.path.clone();
            require(
                stage.write_payload(wrong, b"tile")
                    == Err(StagingError::Safety(ArchiveError::TooLarge)),
            )?;
            require(fs::read_dir(&path)?.count() == 0)?;
            require(
                stage.write_payload(wrong, b"wrong")
                    == Err(StagingError::Aborted),
            )?;
            drop(stage);
            require(!path.exists())?;
        }
        Ok(())
    }

    #[test]
    fn canonical_manifest_consumes_staging_budget() -> Result<(), StagingError>
    {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let limits = ArchiveBudget::new(
            NonZeroU64::new(4096).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(8192).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(2).unwrap_or(NonZeroU64::MIN),
        );
        let mut stage = CanonicalStaging::new(&parent.path, limits)?;
        stage.write_payload(digest, b"tile")?;
        let path = stage.stage.temporary.path.clone();
        require(matches!(
            stage.finish(&manifest, canonical_limits()),
            Err(StagingError::Safety(ArchiveError::TooLarge))
        ))?;
        require(!path.exists())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn publication_is_content_addressed_and_persistent()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let mut stage =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        stage.write_payload(digest, b"tile")?;
        let staged = stage.finish(&manifest, canonical_limits())?;
        let temporary = staged.path().to_owned();
        let encoded = manifest.encode(canonical_limits())?;
        let manifest_digest = ContentHash::calculate(&encoded);
        require(staged.digest() == manifest_digest)?;
        let published = staged.publish()?;
        require(published.durability.is_ok())?;
        let expected = parent.path.join(format!("bundle-{manifest_digest}"));
        require(published.path == expected)?;
        require(!temporary.exists())?;
        drop(published);
        require(fs::read(expected.join("manifest.json"))? == encoded)?;
        let reader = CanonicalBundle::open(
            &parent.path,
            manifest_digest,
            canonical_limits(),
        )
        .map_err(|_error| StagingError::Asset(AssetError::PayloadMismatch))?;
        require(reader.manifest() == &manifest)?;
        require(
            reader
                .read_payload("tile")
                .is_ok_and(|bytes| bytes == b"tile"),
        )?;
        require(
            fs::read(expected.join(format!("payloads/{digest}")))? == b"tile",
        )
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn copy_legacy_bundle_deduplicates_and_preserves_source()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let encoded = manifest.encode(canonical_limits())?;
        let text = String::from_utf8(encoded.clone())
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let fingerprint = ["0"; 32].join(",");
        let legacy = text
            .replace(
                "\"source\":{",
                &format!("\"source\":{{\"digest\":[{fingerprint}],"),
            )
            .replace("\"version\":2", "\"version\":1");
        let legacy_digest = ContentHash::calculate(legacy.as_bytes());
        let legacy_path = parent.path.join(format!("bundle-{legacy_digest}"));
        let payloads = legacy_path.join(PAYLOADS_DIRECTORY);
        fs::create_dir_all(&payloads)?;
        fs::write(legacy_path.join(MANIFEST_NAME), legacy.as_bytes())?;
        fs::write(payloads.join(digest.to_string()), b"tile")?;
        let source = CanonicalBundle::open(
            &parent.path,
            legacy_digest,
            canonical_limits(),
        )?;
        let staged = CanonicalStaging::copy_bundle(
            &parent.path,
            canonical_budget(),
            &source,
            canonical_limits(),
        )?;
        let current_digest = ContentHash::calculate(&encoded);
        require(staged.digest() == current_digest)?;
        let published = staged.publish()?;
        require(published.durability.is_ok())?;
        require(fs::read(published.path.join(MANIFEST_NAME))? == encoded)?;
        require(
            fs::read_dir(published.path.join(PAYLOADS_DIRECTORY))?.count() == 1,
        )?;
        require(
            fs::read(legacy_path.join(MANIFEST_NAME))? == legacy.as_bytes(),
        )?;
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn copy_bundle_rejects_corruption_without_partial_output()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let (source, published) = publish_copy_fixture(&parent, &manifest)?;
        fs::write(
            published
                .path
                .join(PAYLOADS_DIRECTORY)
                .join(digest.to_string()),
            b"evil",
        )?;
        let stricter = ManifestLimits {
            payload_bytes: NonZeroU64::MIN,
            ..canonical_limits()
        };
        require(matches!(
            CanonicalStaging::copy_bundle(
                &parent.path,
                canonical_budget(),
                &source,
                stricter,
            ),
            Err(StagingError::Asset(AssetError::TooLarge))
        ))?;
        require(matches!(
            CanonicalStaging::copy_bundle(
                &parent.path,
                canonical_budget(),
                &source,
                canonical_limits(),
            ),
            Err(StagingError::Bundle(BundleError::Asset(
                AssetError::PayloadMismatch
            )))
        ))?;
        require(fs::read_dir(&parent.path)?.count() == 1)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn publish_copy_fixture(
        parent: &TestRoot,
        manifest: &AssetManifest,
    ) -> Result<CopyFixture, StagingError> {
        let mut stage =
            CanonicalStaging::new(&parent.path, canonical_budget())?;
        stage.write_payload(ContentHash::calculate(b"tile"), b"tile")?;
        let staged = stage.finish(manifest, canonical_limits())?;
        let address = staged.digest();
        let published = staged.publish()?;
        require(published.durability.is_ok())?;
        let source =
            CanonicalBundle::open(&parent.path, address, canonical_limits())?;
        Ok((source, published))
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn copy_budget_rejects_before_reading_damaged_payload()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        let (source, published) = publish_copy_fixture(&parent, &manifest)?;
        let size = manifest_size(&manifest.encode(canonical_limits())?)?;
        let bound = |value| NonZeroU64::new(value).ok_or(AssetError::TooLarge);
        let exact = ArchiveBudget::new(
            bound(size)?,
            bound(size.saturating_add(4))?,
            bound(3)?,
        );
        let copied = CanonicalStaging::copy_bundle(
            &parent.path,
            exact,
            &source,
            canonical_limits(),
        )?;
        require(copied.digest() == source.digest())?;
        require(
            fs::read_dir(copied.path().join(PAYLOADS_DIRECTORY))?.count() == 1,
        )?;
        drop(copied);
        fs::write(
            published
                .path
                .join(PAYLOADS_DIRECTORY)
                .join(digest.to_string()),
            b"evil",
        )?;
        for deficient in [
            ArchiveBudget::new(
                bound(size.saturating_sub(1))?,
                bound(size.saturating_add(4))?,
                bound(3)?,
            ),
            ArchiveBudget::new(
                bound(size)?,
                bound(size.saturating_add(3))?,
                bound(3)?,
            ),
            ArchiveBudget::new(
                bound(size)?,
                bound(size.saturating_add(4))?,
                bound(2)?,
            ),
        ] {
            require(matches!(
                CanonicalStaging::copy_bundle(
                    &parent.path,
                    deficient,
                    &source,
                    canonical_limits(),
                ),
                Err(StagingError::Safety(ArchiveError::TooLarge))
            ))?;
        }
        require(fs::read_dir(&parent.path)?.count() == 1)
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn empty_bundle_copy_needs_only_the_manifest_entry()
    -> Result<(), StagingError> {
        let parent = fixture()?;
        let manifest = AssetManifest::new(
            SourceIdentity {
                title: String::from("synthetic"),
                revision: String::from("empty"),
            },
            Vec::new(),
            canonical_limits(),
        )?;
        let bytes = manifest.encode(canonical_limits())?;
        let size = NonZeroU64::new(manifest_size(&bytes)?)
            .ok_or(AssetError::TooLarge)?;
        let budget = ArchiveBudget::new(size, size, NonZeroU64::MIN);
        let stage = CanonicalStaging::new(&parent.path, budget)?;
        let published =
            stage.finish(&manifest, canonical_limits())?.publish()?;
        require(published.durability.is_ok())?;
        let source = CanonicalBundle::open(
            &parent.path,
            ContentHash::calculate(&bytes),
            canonical_limits(),
        )?;
        let copied = CanonicalStaging::copy_bundle(
            &parent.path,
            budget,
            &source,
            canonical_limits(),
        )?;
        require(fs::read(copied.path().join(MANIFEST_NAME))? == bytes)?;
        require(!copied.path().join(PAYLOADS_DIRECTORY).exists())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn publication_never_replaces_an_existing_path() -> Result<(), StagingError>
    {
        let parent = fixture()?;
        let outside = fixture()?;
        let digest = ContentHash::calculate(b"tile");
        let manifest = manifest(digest)?;
        for is_link in [false, true] {
            let mut stage =
                CanonicalStaging::new(&parent.path, canonical_budget())?;
            stage.write_payload(digest, b"tile")?;
            let staged = stage.finish(&manifest, canonical_limits())?;
            let temporary = staged.path().to_owned();
            let destination =
                parent.path.join(format!("bundle-{}", staged.digest()));
            if is_link {
                symlink(&outside.path, &destination)?;
            } else {
                fs::create_dir_all(&destination)?;
            }
            require(matches!(staged.publish(), Err(StagingError::Collision)))?;
            require(!temporary.exists())?;
            require(fs::read_dir(&destination)?.count() == 0)?;
            if is_link {
                require(fs::symlink_metadata(&destination)?.is_symlink())?;
                fs::remove_file(&destination)?;
            } else {
                fs::remove_dir(&destination)?;
            }
        }
        Ok(())
    }
}
