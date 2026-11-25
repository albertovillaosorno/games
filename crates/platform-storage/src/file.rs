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
//   - Bounded snapshots and shared native regular-file admission.
// - Must-Not:
//   - Interpret source formats, asset semantics or execute files.
// - Allows:
//   - Inputs: Explicit local paths and byte limits.
//   - Outputs: Bounded bytes or typed file admission failures.
//   - Side effects: Read-only descriptor access, without following final links.
// - Split-When:
//   - A platform needs a different safe file-opening primitive.
// - Merge-When:
//   - A shared native file service provides identical admission guarantees.
// - Summary:
//   - Pin ordinary files before accessing their contents.
// - Description:
//   - Reject special files and bound allocation using the pinned descriptor.
// - Usage:
//   - Read one snapshot before verifying a trusted source identity.
// - Defaults:
//   - No final symlinks, no device access, no unchecked allocation.

use std::fs::File;
use std::io::{self, Read as _};
use std::num::NonZeroU64;
use std::os::fd::{AsFd, AsRawFd as _, OwnedFd};
use std::path::Path;

use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{FileStat, Mode, SFlag, fstat, fstatat};
use nix::unistd::geteuid;

pub(crate) const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

/// Admission failure for a native file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileReadError {
    /// Opening, inspecting or reading the pinned file failed.
    Io(io::ErrorKind),
    /// The file size or allocation exceeds the supplied limit.
    TooLarge,
    /// The descriptor is not a regular file or its identity changed.
    UnsafeFile,
}

impl From<Errno> for FileReadError {
    fn from(error: Errno) -> Self {
        Self::Io(io::Error::from(error).kind())
    }
}

impl From<io::Error> for FileReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Read a bounded snapshot of a user-selected regular file.
///
/// A metadata-only descriptor pins the final component without opening a
/// device or following a symlink. A readable descriptor is obtained through
/// the trusted kernel procfs and its identity is rechecked. Ancestors and
/// procfs must be trusted; the snapshot does not authenticate its contents.
/// Verify a trusted content hash before interpreting externally supplied bytes.
///
/// # Errors
/// Rejects non-regular files, final symlinks, unavailable procfs, oversized
/// files, allocation failures and file length changes during the read.
pub fn snapshot(
    path: &Path,
    maximum: NonZeroU64,
) -> Result<Vec<u8>, FileReadError> {
    let pinned = open(
        path,
        OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let before = fstat(&pinned)?;
    if !regular(&before) {
        return Err(FileReadError::UnsafeFile);
    }
    let size = file_size(&before)?;
    if size > maximum.get() {
        return Err(FileReadError::TooLarge);
    }
    let descriptor_path = format!("/proc/self/fd/{}", pinned.as_raw_fd());
    let fd = open(
        descriptor_path.as_str(),
        OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let after = fstat(&fd)?;
    if !regular(&after)
        || before.st_dev != after.st_dev
        || before.st_ino != after.st_ino
        || before.st_size != after.st_size
    {
        return Err(FileReadError::UnsafeFile);
    }
    read_bounded_descriptor(fd, size, maximum.get())
}

pub(crate) fn read_bounded_descriptor(
    fd: OwnedFd,
    size: u64,
    maximum: u64,
) -> Result<Vec<u8>, FileReadError> {
    if size > maximum {
        return Err(FileReadError::TooLarge);
    }
    let length =
        usize::try_from(size).map_err(|_error| FileReadError::TooLarge)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| FileReadError::TooLarge)?;
    bytes.resize(length, 0);
    let mut file = File::from(fd);
    file.read_exact(&mut bytes)?;
    let mut extra = [0u8; 1];
    match file.read_exact(&mut extra) {
        Ok(()) => Err(FileReadError::TooLarge),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(bytes),
        Err(error) => Err(error.into()),
    }
}

pub(crate) const fn regular(stat: &FileStat) -> bool {
    stat.st_mode & SFlag::S_IFMT.bits() == SFlag::S_IFREG.bits()
}

pub(crate) fn file_size(stat: &FileStat) -> Result<u64, FileReadError> {
    u64::try_from(stat.st_size).map_err(|_error| FileReadError::TooLarge)
}

pub(crate) fn trusted_directory(fd: impl AsFd) -> nix::Result<bool> {
    let stat = fstat(fd)?;
    Ok(stat.st_mode & 0o022 == 0 && stat.st_uid == geteuid().as_raw())
}

pub(crate) fn private_regular(stat: &FileStat) -> bool {
    regular(stat)
        && stat.st_nlink == 1
        && stat.st_uid == geteuid().as_raw()
        && stat.st_mode & 0o022 == 0
}

pub(crate) fn read_regular(
    parent: &impl AsFd,
    name: &str,
    maximum: u64,
) -> Result<Vec<u8>, FileReadError> {
    if !trusted_directory(parent)? {
        return Err(FileReadError::UnsafeFile);
    }
    let before = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if !private_regular(&before) {
        return Err(FileReadError::UnsafeFile);
    }
    let fd = openat(
        parent,
        name,
        OFlag::O_RDONLY
            | OFlag::O_NOFOLLOW
            | OFlag::O_NONBLOCK
            | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let stat = fstat(&fd)?;
    if !private_regular(&stat)
        || stat.st_dev != before.st_dev
        || stat.st_ino != before.st_ino
    {
        return Err(FileReadError::UnsafeFile);
    }
    read_bounded_descriptor(fd, file_size(&stat)?, maximum)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use nix::unistd::mkfifo;

    use super::*;
    use crate::bundle::TestRoot;

    fn require(condition: bool) -> Result<(), FileReadError> {
        condition
            .then_some(())
            .ok_or(FileReadError::Io(io::ErrorKind::InvalidData))
    }

    #[test]
    fn snapshots_are_exact_and_bounded() -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("file");
        let max = NonZeroU64::new(4).unwrap_or(NonZeroU64::MIN);
        fs::write(&path, b"four")?;
        require(snapshot(&path, max)? == b"four")?;
        fs::write(&path, b"fives")?;
        require(snapshot(&path, max) == Err(FileReadError::TooLarge))?;
        fs::write(&path, b"")?;
        require(snapshot(&path, max)?.is_empty())
    }

    #[test]
    fn links_and_special_files_are_not_opened() -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("file");
        let fifo = root.path.join("fifo");
        let link = root.path.join("link");
        fs::write(&path, b"x")?;
        symlink(&path, &link)?;
        mkfifo(&fifo, Mode::S_IRUSR | Mode::S_IWUSR)?;
        for rejected in [&link, &fifo, &root.path] {
            require(
                snapshot(rejected, NonZeroU64::MIN)
                    == Err(FileReadError::UnsafeFile),
            )?;
        }
        Ok(())
    }

    #[test]
    fn descriptor_length_changes_fail_closed() -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("file");
        fs::write(&path, b"four")?;
        let fd = open(&path, OFlag::O_RDONLY, Mode::empty())?;
        fs::write(&path, b"fives")?;
        require(
            read_bounded_descriptor(fd, 4, 4) == Err(FileReadError::TooLarge),
        )?;
        let shortened = open(&path, OFlag::O_RDONLY, Mode::empty())?;
        fs::write(&path, b"x")?;
        require(matches!(
            read_bounded_descriptor(shortened, 5, 5),
            Err(FileReadError::Io(io::ErrorKind::UnexpectedEof))
        ))
    }
}
