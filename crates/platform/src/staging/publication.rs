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
//   - Synchronisation and atomic publication of completed canonical bundles.
// - Must-Not:
//   - Interpret source media, execute payloads or replace existing bundles.
// - Allows:
//   - Inputs: Verified canonical staging and its manifest content identity.
//   - Outputs: A private staged bundle or a published directory receipt.
//   - Side effects: Sync files and atomically rename inside a trusted parent.
// - Split-When:
//   - A different platform needs a distinct publication mechanism.
// - Merge-When:
//   - A shared filesystem adapter provides equivalent publication semantics.
// - Summary:
//   - Publish an entire verified bundle without overwriting existing data.
// - Description:
//   - Synchronise payloads and metadata before a no-replace directory rename.
// - Usage:
//   - Finish canonical staging before publication.
// - Defaults:
//   - Keep the temporary directory private and reject existing destinations.

use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::Path;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::{io, path::PathBuf};

use games_assets::ContentHash;
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::Mode;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use nix::{
    errno::Errno,
    fcntl::{RenameFlags, open, renameat2},
};

use super::{ArchiveStaging, StagedArchive, StagingError};
use crate::bundle::{DIRECTORY_FLAGS, MANIFEST_NAME, PAYLOADS_DIRECTORY};
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use crate::bundle::{bundle_name, trusted_directory};

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
        let path = self.archive.path();
        let parent = path.parent().ok_or(StagingError::UnsupportedPath)?;
        let source = path.file_name().ok_or(StagingError::UnsupportedPath)?;
        let fd = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
        if !trusted_directory(&fd)? {
            return Err(StagingError::UntrustedParent);
        }
        let name = bundle_name(self.digest);
        let destination = parent.join(&name);
        match renameat2(
            &fd,
            source,
            &fd,
            name.as_str(),
            RenameFlags::RENAME_NOREPLACE,
        ) {
            Ok(()) => {},
            Err(Errno::EEXIST) => return Err(StagingError::Collision),
            Err(error) => return Err(error.into()),
        }
        self.archive.0.cleanup = false;
        let durability =
            File::from(fd).sync_all().map_err(|error| error.kind());
        Ok(PublishedBundle {
            durability,
            path: destination,
        })
    }
}

/// Publication succeeded; the directory is visible and is not auto-deleted.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[derive(Debug)]
pub struct PublishedBundle {
    /// The result of syncing the parent after publication. An error means
    /// the rename succeeded but its crash durability is uncertain.
    pub durability: Result<(), io::ErrorKind>,
    /// Persistent, manifest-addressed bundle directory.
    pub path: PathBuf,
}

pub(super) fn finish<'digest>(
    stage: ArchiveStaging,
    digest: ContentHash,
    payloads: impl Iterator<Item = &'digest ContentHash>,
) -> Result<StagedBundle, StagingError> {
    let mut hashes = payloads.peekable();
    if hashes.peek().is_some() {
        let directory = openat(
            &stage.root,
            PAYLOADS_DIRECTORY,
            DIRECTORY_FLAGS,
            Mode::empty(),
        )?;
        for hash in hashes {
            sync_file(&directory, &hash.to_string())?;
        }
        File::from(directory).sync_all()?;
    }
    sync_file(&stage.root, MANIFEST_NAME)?;
    let root = openat(&stage.root, ".", DIRECTORY_FLAGS, Mode::empty())?;
    File::from(root).sync_all()?;
    Ok(StagedBundle {
        archive: stage.finish()?,
        digest,
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
