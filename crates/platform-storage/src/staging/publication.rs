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
use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::Path;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::{io, path::PathBuf};

use games_assets::ContentHash;
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::Mode;
use nix::unistd::fsync;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use nix::{
    errno::Errno,
    fcntl::{RenameFlags, renameat2},
};

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use super::PrivateDirectory;
use super::{ArchiveStaging, StagedArchive, StagingError};
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use crate::bundle::bundle_name;
use crate::bundle::{MANIFEST_NAME, PAYLOADS_DIRECTORY};
use crate::file::DIRECTORY_FLAGS;

/// A complete canonical bundle that has not yet been published.
///
/// The trusted host must not modify the staged files.
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
        let name = bundle_name(self.digest);
        publish_directory(&mut self.archive.0, OsStr::new(&name))
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
/// This does not admit untrusted archives or enforce archive budgets. The
/// caller must write only validated files, synchronise their contents and
/// nested directories, and keep the parent and ancestors trusted and stable.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[derive(Debug)]
pub struct AuthoredStaging(PrivateDirectory);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
impl AuthoredStaging {
    /// Create a private sibling directory for trusted output.
    ///
    /// # Errors
    /// Rejects untrusted parents and filesystem failures.
    pub fn new(parent: &Path) -> Result<Self, StagingError> {
        PrivateDirectory::create_in(parent).map(Self)
    }

    /// Access the private output directory, removed on drop unless published.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    /// Synchronise the root and publish the complete directory without replace.
    ///
    /// # Errors
    /// Rejects non-component names, collisions and pre-publication failures.
    /// The receipt reports a post-publication parent sync failure separately.
    pub fn publish(
        mut self,
        name: &OsStr,
    ) -> Result<PublishedDirectory, StagingError> {
        fsync(&self.0.root)?;
        publish_directory(&mut self.0, name)
    }
}

pub(super) fn finish<'digest>(
    stage: ArchiveStaging,
    digest: ContentHash,
    payloads: impl Iterator<Item = &'digest ContentHash>,
) -> Result<StagedBundle, StagingError> {
    let mut hashes = payloads.peekable();
    if hashes.peek().is_some() {
        let directory = openat(
            &stage.temporary.root,
            PAYLOADS_DIRECTORY,
            DIRECTORY_FLAGS,
            Mode::empty(),
        )?;
        for hash in hashes {
            sync_file(&directory, &hash.to_string())?;
        }
        File::from(directory).sync_all()?;
    }
    sync_file(&stage.temporary.root, MANIFEST_NAME)?;
    fsync(&stage.temporary.root)?;
    Ok(StagedBundle {
        archive: stage.finish()?,
        digest,
    })
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

fn sync_file(parent: &OwnedFd, name: &str) -> Result<(), StagingError> {
    let fd = openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    File::from(fd).sync_all()?;
    Ok(())
}
