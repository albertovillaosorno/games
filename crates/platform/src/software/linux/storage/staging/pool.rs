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
//   - Verified content-addressed object publication in one title store.
// - Must-Not:
//   - Replace existing objects, follow links or copy verified payloads.
// - Allows:
//   - Inputs: Admitted digest, exact bytes and a trusted title directory.
//   - Outputs: A verified reusable object or an explicit error.
//   - Side effects: Atomic publication only for previously absent bytes.
// - Split-When:
//   - An independent object store backend is required.
// - Merge-When:
//   - Another shared storage service provides identical object semantics.
// - Summary:
//   - Deduplicate canonical payloads across manifests.
// - Description:
//   - Verify existing objects by streaming; publish absent ones once.
// - Usage:
//   - Write only verified canonical content to a shared title store.
// - Defaults:
//   - Fail on corruption, unsafe storage and mixed legacy layout.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read as _;
use std::num::NonZeroU64;
use std::os::fd::OwnedFd;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use games_assets::archive::ArchiveBudget;
use games_assets::{AssetError, ContentHash};
use nix::errno::Errno;
use nix::fcntl::{open, openat};
use nix::sys::stat::{FileStat, Mode, fstat, mkdirat};
use nix::unistd::fsync;

use super::super::bundle::{
    BundleError, OBJECT_PAYLOAD, OBJECTS_DIRECTORY, PAYLOADS_DIRECTORY,
};
use super::super::file::{
    DIRECTORY_FLAGS, read_regular_with, trusted_directory,
    verify_staged_regular,
};
use super::super::pinned_directory_entries;
use super::publication::publish_directory;
use super::{ArchiveStaging, PrivateDirectory, StagingError, same_directory};

/// A private, pinned, title-wide object directory.
#[derive(Debug)]
pub(super) struct SharedPool {
    path: PathBuf,
    root: OwnedFd,
    verified: BTreeMap<String, FileStat>,
}

impl SharedPool {
    pub(super) fn open(stage: &PrivateDirectory) -> Result<Self, StagingError> {
        stage.verify_location()?;
        let parent =
            stage.path.parent().ok_or(StagingError::UnsupportedPath)?;
        reject_legacy(stage)?;
        match openat(
            &stage.parent,
            OBJECTS_DIRECTORY,
            DIRECTORY_FLAGS,
            Mode::empty(),
        ) {
            Ok(_) => {},
            Err(Errno::ENOENT) => {
                match mkdirat(&stage.parent, OBJECTS_DIRECTORY, Mode::S_IRWXU) {
                    Ok(()) | Err(Errno::EEXIST) => {},
                    Err(error) => return Err(error.into()),
                }
                fsync(&stage.parent)?;
            },
            Err(error) => return Err(error.into()),
        }
        let root = openat(
            &stage.parent,
            OBJECTS_DIRECTORY,
            DIRECTORY_FLAGS,
            Mode::empty(),
        )?;
        if !trusted_directory(&root)? {
            return Err(StagingError::UntrustedParent);
        }
        stage.verify_location()?;
        Ok(Self {
            path: parent.join(OBJECTS_DIRECTORY),
            root,
            verified: BTreeMap::new(),
        })
    }

    fn publish(&self, name: &str, bytes: &[u8]) -> Result<(), StagingError> {
        let size = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        let maximum = NonZeroU64::new(size).unwrap_or(NonZeroU64::MIN);
        let budget = ArchiveBudget::new(maximum, maximum, NonZeroU64::MIN);
        let mut stage = ArchiveStaging::new(&self.path, budget)?;
        if !same_directory(&stage.tree.temporary.parent, &self.root)? {
            return Err(StagingError::ChangedStorage);
        }
        let mut input = bytes;
        stage.record_file(OBJECT_PAYLOAD, size, &mut input)?;
        let mut completed = stage.finish_with(true)?;
        match publish_directory(&mut completed.temporary, OsStr::new(name)) {
            Ok(receipt) => receipt.durability.map_err(StagingError::Io),
            // A concurrent publisher won; the caller verifies its bytes.
            Err(StagingError::Collision) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn put(
        &mut self,
        digest: ContentHash,
        bytes: &[u8],
    ) -> Result<(), StagingError> {
        let name = digest.to_string();
        let stat = if let Some(stat) = self.verify(&name, bytes)? {
            stat
        } else {
            self.publish(&name, bytes)?;
            self.verify(&name, bytes)?
                .ok_or(StagingError::ChangedStorage)?
        };
        let _previous = self.verified.insert(name, stat);
        Ok(())
    }

    pub(super) fn recheck(
        &self,
        stage: &PrivateDirectory,
    ) -> Result<(), StagingError> {
        stage.verify_location()?;
        reject_legacy(stage)?;
        let current = open(&self.path, DIRECTORY_FLAGS, Mode::empty())?;
        if !same_directory(&self.root, &current)?
            || !trusted_directory(&self.root)?
        {
            return Err(StagingError::ChangedStorage);
        }
        for (name, stat) in &self.verified {
            let object = openat(
                &self.root,
                name.as_str(),
                DIRECTORY_FLAGS,
                Mode::empty(),
            )?;
            verify_staged_regular(&object, OBJECT_PAYLOAD, stat)
                .map_err(BundleError::from)?;
        }
        Ok(())
    }

    fn verify(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> Result<Option<FileStat>, StagingError> {
        let object =
            match openat(&self.root, name, DIRECTORY_FLAGS, Mode::empty()) {
                Ok(directory) => directory,
                Err(Errno::ENOENT) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
        let expected = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        let stat = read_regular_with::<_, BundleError>(
            &object,
            OBJECT_PAYLOAD,
            expected,
            |file, actual| {
                if actual != expected {
                    return Err(AssetError::PayloadMismatch.into());
                }
                compare_payload(file, bytes)?;
                Ok(fstat(file)?)
            },
        )?;
        Ok(Some(stat))
    }
}

fn compare_payload(
    file: &mut fs::File,
    bytes: &[u8],
) -> Result<(), BundleError> {
    let mut buffer = [0u8; 4096];
    for chunk in bytes.chunks(buffer.len()) {
        let target =
            buffer.get_mut(..chunk.len()).ok_or(AssetError::TooLarge)?;
        file.read_exact(target)?;
        if target != chunk {
            return Err(AssetError::PayloadMismatch.into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn reject_shared(parent: &Path) -> Result<(), StagingError> {
    let directory = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
    match openat(
        &directory,
        OBJECTS_DIRECTORY,
        DIRECTORY_FLAGS,
        Mode::empty(),
    ) {
        Err(Errno::ENOENT) => Ok(()),
        Ok(_) => Err(StagingError::Collision),
        Err(error) => Err(error.into()),
    }
}

fn reject_legacy(stage: &PrivateDirectory) -> Result<(), StagingError> {
    // Check metadata, not payload bytes. Even an existing shared store
    // must reject a legacy payload directory inserted alongside it.
    for entry in pinned_directory_entries(&stage.parent)? {
        let name = entry?.file_name();
        if !name
            .to_str()
            .is_some_and(|value| value.starts_with("bundle-"))
        {
            continue;
        }
        let directory = openat(
            &stage.parent,
            name.as_os_str(),
            DIRECTORY_FLAGS,
            Mode::empty(),
        )?;
        if !trusted_directory(&directory)? {
            return Err(StagingError::UntrustedParent);
        }
        match openat(
            &directory,
            PAYLOADS_DIRECTORY,
            DIRECTORY_FLAGS,
            Mode::empty(),
        ) {
            Err(Errno::ENOENT) => {},
            Ok(_) => return Err(StagingError::Collision),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
