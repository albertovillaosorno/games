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
//   - Synchronisation and atomic publication of complete host output.
// - Must-Not:
//   - Interpret source media, execute payloads or replace existing output.
// - Allows:
//   - Inputs: Verified canonical staging or trusted authored output.
//   - Outputs: Private staging or a published directory receipt.
//   - Side effects: Sync files and atomically rename inside a trusted parent.
// - Split-When:
//   - A different platform needs a distinct publication mechanism.
// - Merge-When:
//   - A shared filesystem adapter provides equivalent publication semantics.
// - Summary:
//   - Publish an entire completed directory without overwriting data.
// - Description:
//   - Synchronise payloads and metadata before a no-replace directory rename.
// - Usage:
//   - Finish and synchronise staged output before publication.
// - Defaults:
//   - Keep the temporary directory private and reject existing destinations.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::ffi::OsStr;
use std::fs;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::io::Write as _;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::path::Path;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::{io, path::PathBuf};

use games_assets::ContentHash;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use games_assets::archive::validate_path;
use nix::fcntl::openat;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use nix::sys::stat::fchmod;
use nix::sys::stat::{FileStat, Mode, fstat};
use nix::unistd::fsync;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use nix::{
    errno::Errno,
    fcntl::{RenameFlags, renameat2},
};

use super::{ArchiveStaging, Directory, StagedArchive, StagingError};
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use super::{PrivateDirectory, staging_path};
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use crate::bundle::bundle_name;
use crate::file::{
    DIRECTORY_FLAGS, FileReadError, same_inode, sync_regular,
    trusted_directory_stat, verify_staged_regular,
};

/// A complete canonical bundle that has not yet been published.
///
/// The trusted host must not modify the staged files. Observable metadata
/// changes and unexpected directory entries are rejected before publication.
#[derive(Debug)]
pub struct StagedBundle {
    archive: StagedArchive,
    digest: ContentHash,
}

impl StagedBundle {
    /// The SHA-256 of the exact canonical manifest bytes.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        self.digest
    }

    /// The private, temporary path; dropping this handle removes it.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.archive.path()
    }

    /// Atomically publish to a manifest-addressed sibling directory.
    ///
    /// Requires a GNU/Linux filesystem supporting no-replace rename.
    /// The trusted parent and its ancestors must remain stable.
    /// # Errors
    /// Rejects collisions and pre-publication filesystem failures. A
    /// post-rename directory sync failure is reported in the receipt instead.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub fn publish(mut self) -> Result<PublishedBundle, StagingError> {
        self.archive.verify()?;
        let name = bundle_name(self.digest);
        publish_directory(&mut self.archive.temporary, OsStr::new(&name))
    }
}

/// Publication succeeded; the directory is visible and is not auto-deleted.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[derive(Debug)]
pub struct PublishedDirectory {
    /// The result of syncing the parent after publication. An error means
    /// the rename succeeded but its crash durability is uncertain.
    pub durability: Result<(), io::ErrorKind>,
    /// Persistent, published directory.
    pub path: PathBuf,
}

/// A published manifest-addressed canonical bundle.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub type PublishedBundle = PublishedDirectory;

/// Private staging for already-authored, trusted host output.
///
/// The caller supplies validated files and their byte and count budgets.
/// Writes use the same descriptor-relative namespace as archive staging,
/// but may set the owner execute bit for independently authored binaries.
/// Observable file and directory changes prevent publication.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[derive(Debug)]
pub struct AuthoredStaging {
    failed: bool,
    registry: Directory,
    temporary: PrivateDirectory,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
impl AuthoredStaging {
    /// Create a private sibling directory for trusted output.
    ///
    /// # Errors
    /// Rejects untrusted parents and filesystem failures.
    pub fn new(parent: &Path) -> Result<Self, StagingError> {
        Ok(Self {
            failed: false,
            registry: Directory::default(),
            temporary: PrivateDirectory::create_in(parent)?,
        })
    }

    /// Access the private directory, removed on drop unless published.
    ///
    /// Only files recorded through `write_file` may be published.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.temporary.path
    }

    /// Verify, synchronise and publish the exact recorded output tree.
    ///
    /// # Errors
    /// Rejects aborted writes, changed storage, non-component destinations,
    /// collisions and pre-publication failures. The receipt reports a
    /// post-publication parent sync failure separately.
    pub fn publish(
        mut self,
        name: &OsStr,
    ) -> Result<PublishedDirectory, StagingError> {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        self.temporary.verify_tree(&self.registry, true)?;
        self.temporary.verify_tree(&self.registry, false)?;
        publish_directory(&mut self.temporary, name)
    }

    /// Write an admitted, independently authored file without replacement.
    ///
    /// The caller must bound bytes and entry count and must never supply
    /// executable bytes from untrusted media. Paths use portable ASCII names.
    ///
    /// # Errors
    /// Rejects collisions, unsafe paths and observed directory substitution.
    /// Any write failure permanently aborts publication.
    pub fn write_file(
        &mut self,
        path: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), StagingError> {
        if self.failed {
            return Err(StagingError::Aborted);
        }
        let result = self.write_member(path, bytes, executable);
        self.failed = result.is_err();
        result
    }

    fn write_member(
        &mut self,
        path: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), StagingError> {
        validate_path(path)?;
        let (parents, leaf) = staging_path(path)?;
        let (directory, descriptor) =
            self.registry.descend(&self.temporary.root, parents, None)?;
        let parent = descriptor.as_ref().unwrap_or(&self.temporary.root);
        let mode = Mode::S_IRUSR
            | Mode::S_IWUSR
            | if executable {
                Mode::S_IXUSR
            } else {
                Mode::empty()
            };
        let (mut file, node) = directory.create_file(parent, leaf)?;
        file.write_all(bytes)?;
        fchmod(&file, mode)?;
        node.record_file(
            &file,
            u64::try_from(bytes.len())
                .map_err(|_error| StagingError::ChangedStorage)?,
        )
    }
}

// Verify the exact observed tree without rehashing or loading file contents.
// Directories are pinned and checked once at each descent; regular files
// must still have the metadata observed when their bounded writers finished.
pub(super) fn check_tree(
    directory: &Directory,
    fd: &OwnedFd,
    expected: Option<&FileStat>,
    sync: bool,
) -> Result<(), StagingError> {
    let current = fstat(fd)?;
    if !trusted_directory_stat(&current)
        || expected.is_some_and(|stamp| !same_inode(stamp, &current))
    {
        return Err(StagingError::ChangedStorage);
    }
    check_entry_count(fd, directory.children.len())?;
    let check = if sync {
        sync_regular
    } else {
        verify_staged_regular
    };
    for node in directory.children.values() {
        let stamp =
            node.stamp.as_deref().ok_or(StagingError::ChangedStorage)?;
        if let Some(child) = &node.directory {
            let child_fd =
                openat(fd, node.name.as_str(), DIRECTORY_FLAGS, Mode::empty())?;
            check_tree(child, &child_fd, Some(stamp), sync)?;
        } else {
            check(fd, &node.name, stamp).map_err(sync_error)?;
        }
    }
    if sync {
        fsync(fd)?;
    }
    Ok(())
}

pub(super) fn finish(
    stage: ArchiveStaging,
    digest: ContentHash,
) -> Result<StagedBundle, StagingError> {
    Ok(StagedBundle {
        archive: stage.finish_with(true)?,
        digest,
    })
}

fn check_entry_count(
    directory: &OwnedFd,
    expected: usize,
) -> Result<(), StagingError> {
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    let mut remaining = expected;
    for result in fs::read_dir(path)? {
        let _entry = result?;
        remaining = remaining
            .checked_sub(1)
            .ok_or(StagingError::ChangedStorage)?;
    }
    if remaining != 0 {
        return Err(StagingError::ChangedStorage);
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn publish_directory(
    temporary: &mut PrivateDirectory,
    name: &OsStr,
) -> Result<PublishedDirectory, StagingError> {
    if Path::new(name).file_name() != Some(name) {
        return Err(StagingError::UnsupportedPath);
    }
    let path = &temporary.path;
    let parent = path.parent().ok_or(StagingError::UnsupportedPath)?;
    let source = path.file_name().ok_or(StagingError::UnsupportedPath)?;
    temporary.verify_location()?;
    let fd = &temporary.parent;
    let destination = parent.join(name);
    match renameat2(fd, source, fd, name, RenameFlags::RENAME_NOREPLACE) {
        Ok(()) => {},
        Err(Errno::EEXIST) => return Err(StagingError::Collision),
        Err(error) => return Err(error.into()),
    }
    temporary.cleanup = false;
    let durability = fsync(fd).map_err(|error| io::Error::from(error).kind());
    Ok(PublishedDirectory {
        durability,
        path: destination,
    })
}

const fn sync_error(error: FileReadError) -> StagingError {
    match error {
        FileReadError::Io(kind) => StagingError::Io(kind),
        FileReadError::UnsafeFile | FileReadError::TooLarge => {
            StagingError::ChangedStorage
        },
    }
}
