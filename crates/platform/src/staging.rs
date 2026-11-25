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
//   - Private native staging of validated, bounded archive data.
// - Must-Not:
//   - Parse source media, execute imported data or publish runtime assets.
// - Allows:
//   - Inputs: Trusted parent directory, entry metadata and decoded streams.
//   - Outputs: Completed temporary directory or explicit rejection.
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
//   - Stage decoded entries, then finish before consuming the result.
// - Defaults:
//   - Reject links, executables, collisions and partial imports.

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
use nix::errno::Errno;
use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::{Mode, fstat, mkdirat};
use nix::unistd::geteuid;

const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);
const MAX_DEPTH: usize = 64;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A staging operation failed and the result must not be published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagingError {
    /// A previous entry failed; this transaction cannot be completed.
    Aborted,
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
}

impl Directory {
    fn create_file(
        &mut self,
        parent: &OwnedFd,
        name: &str,
    ) -> Result<File, StagingError> {
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
        let _node = entry.insert(Node {
            directory: None,
            name: name.to_owned(),
        });
        Ok(File::from(fd))
    }

    fn open_child(
        &mut self,
        parent: &OwnedFd,
        name: &str,
        budget: Option<&mut ArchiveBudget>,
    ) -> Result<OpenedDirectory<'_>, StagingError> {
        let explicit = budget.is_none();
        let node = match self.children.entry(name.to_ascii_lowercase()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if let Some(limits) = budget {
                    limits.admit(name, EntryKind::Directory, 0)?.finish()?;
                }
                mkdirat(parent, name, Mode::S_IRWXU)?;
                entry.insert(Node {
                    directory: Some(Self::default()),
                    name: name.to_owned(),
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
        child.explicit |= explicit;
        let fd = openat(parent, name, DIRECTORY_FLAGS, Mode::empty())?;
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
    path: PathBuf,
}

#[derive(Debug)]
struct CreatedDirectory {
    root: OwnedFd,
    temporary: PrivateDirectory,
}

impl PrivateDirectory {
    fn create(
        parent: &Path,
        parent_fd: &OwnedFd,
    ) -> Result<CreatedDirectory, StagingError> {
        for _ in 0u8..128 {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".games-import-{}-{sequence}", process::id());
            match mkdirat(parent_fd, name.as_str(), Mode::S_IRWXU) {
                Ok(()) => {
                    let temporary = Self { path: parent.join(&name) };
                    let root = openat(
                        parent_fd,
                        name.as_str(),
                        DIRECTORY_FLAGS,
                        Mode::empty(),
                    )?;
                    return Ok(CreatedDirectory { root, temporary });
                },
                Err(Errno::EEXIST) => {},
                Err(error) => return Err(error.into()),
            }
        }
        Err(StagingError::Io(io::ErrorKind::AlreadyExists))
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.path);
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
    root: OwnedFd,
    temporary: PrivateDirectory,
}

impl ArchiveStaging {
    /// Complete staging only if every attempted entry was accepted.
    ///
    /// # Errors
    /// Rejects an aborted transaction and discards its partial output.
    pub fn finish(self) -> Result<StagedArchive, StagingError> {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        Ok(StagedArchive(self.temporary))
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
        let parent_fd = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
        let stat = fstat(&parent_fd)?;
        if stat.st_mode & 0o022 != 0 || stat.st_uid != geteuid().as_raw() {
            return Err(StagingError::UntrustedParent);
        }
        let absolute_parent = fs::canonicalize(parent)?;
        let CreatedDirectory { temporary, root } =
            PrivateDirectory::create(&absolute_parent, &parent_fd)?;
        Ok(Self {
            budget,
            failed: false,
            registry: Directory::default(),
            root,
            temporary,
        })
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
        if !path.is_ascii() || path.split('/').count() > MAX_DEPTH {
            return Err(StagingError::UnsupportedPath);
        }
        let (parents, leaf) = path.rsplit_once('/').unwrap_or(("", path));
        let mut directory = &mut self.registry;
        let mut descriptor = None;
        if !parents.is_empty() {
            for part in parents.split('/') {
                let parent = descriptor.as_ref().unwrap_or(&self.root);
                let OpenedDirectory { child, fd } = directory.open_child(
                    parent,
                    part,
                    Some(&mut self.budget),
                )?;
                directory = child;
                descriptor = Some(fd);
            }
        }
        let parent = descriptor.as_ref().unwrap_or(&self.root);
        match kind {
            EntryKind::Directory => {
                copy_bounded(input, &mut io::sink(), entry)?;
                let _directory = directory.open_child(parent, leaf, None)?;
                Ok(())
            },
            EntryKind::File => {
                let mut output = directory.create_file(parent, leaf)?;
                copy_bounded(input, &mut output, entry)
            },
            EntryKind::Hardlink | EntryKind::Other | EntryKind::Symlink => {
                Err(ArchiveError::LinkOrSpecial.into())
            },
        }
    }
}

/// A verified staging directory with best-effort cleanup on drop.
#[derive(Debug)]
pub struct StagedArchive(PrivateDirectory);

impl StagedArchive {
    /// Access the completed staging directory for subsequent conversion.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0.path
    }
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::num::NonZeroU64;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    fn budget(entries: u64) -> ArchiveBudget {
        ArchiveBudget::new(
            NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(64).unwrap_or(NonZeroU64::MIN),
            NonZeroU64::new(entries).unwrap_or(NonZeroU64::MIN),
        )
    }

    fn fixture() -> Result<PrivateDirectory, StagingError> {
        let parent = env::temp_dir();
        let parent_fd = open(&parent, DIRECTORY_FLAGS, Mode::empty())?;
        let CreatedDirectory { temporary, root: _root } =
            PrivateDirectory::create(&parent, &parent_fd)?;
        Ok(temporary)
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
}
