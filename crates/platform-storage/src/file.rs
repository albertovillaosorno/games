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
//   - Read one snapshot before bounded source-format decoding.
// - Defaults:
//   - No final symlinks, no device access, no unchecked allocation.

use std::fs::File;
use std::io::{self, Read as _};
use std::num::NonZeroU64;
use std::os::fd::{AsFd, AsRawFd as _, OwnedFd};
use std::path::Path;

use nix::errno::Errno;
use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::{FileStat, Mode, SFlag, fstat};
use nix::unistd::geteuid;
use zeroize::Zeroizing;

pub(crate) const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

const PIN_FLAGS: OFlag = OFlag::O_PATH
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

/// Admission failure for a native file snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileReadError {
    /// Opening, inspecting or reading the pinned file failed.
    Io(io::ErrorKind),
    /// The file size or allocation exceeds the supplied limit.
    TooLarge,
    /// The file failed admission or its content metadata changed.
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

/// Owned credential bytes with zeroizing drop and redacted debug output.
pub type SecretSnapshot = Zeroizing<Vec<u8>>;
type OpenedRegular = (File, FileStat);

/// Read a bounded snapshot of a user-selected regular file.
///
/// A metadata-only descriptor pins the final component without opening a
/// device or following a symlink. A readable descriptor is obtained through
/// the trusted kernel procfs and its identity is rechecked. Ancestors and
/// procfs must be trusted; the snapshot does not authenticate its contents.
/// Treat externally supplied bytes as untrusted and validate their structure.
///
/// Observable length or modification-time changes are rejected. This is
/// not a lock against concurrent writers or an authenticity guarantee.
///
/// # Errors
/// Rejects non-regular files, final symlinks, unavailable procfs, oversized
/// files, allocation failures and observed content changes during the read.
pub fn snapshot(
    path: &Path,
    maximum: NonZeroU64,
) -> Result<Vec<u8>, FileReadError> {
    let pinned = open(path, PIN_FLAGS, Mode::empty())?;
    read_pinned(&pinned, maximum.get(), regular)
}

/// Read a bounded owner-only credential file without following its final link.
///
/// The same inode must remain an ordinary file owned by the effective user
/// with no group or other permissions or additional hard links. Trusted
/// ancestors and procfs are still required. The returned buffer is zeroized
/// when dropped and redacted in debug output. Callers should borrow it while
/// parsing rather than copy it.
///
/// # Errors
/// Rejects unsafe ownership, permissions, links, non-regular files,
/// oversize input and observable content changes during the read.
pub fn snapshot_secret(
    path: &Path,
    maximum: NonZeroU64,
) -> Result<SecretSnapshot, FileReadError> {
    let pinned = open(path, PIN_FLAGS, Mode::empty())?;
    let mut bytes = Zeroizing::new(Vec::new());
    read_pinned_into(&pinned, maximum.get(), secret_regular, &mut bytes)?;
    Ok(bytes)
}

fn read_pinned(
    pinned: &OwnedFd,
    maximum: u64,
    admitted: fn(&FileStat) -> bool,
) -> Result<Vec<u8>, FileReadError> {
    let mut bytes = Vec::new();
    read_pinned_into(pinned, maximum, admitted, &mut bytes)?;
    Ok(bytes)
}

fn read_pinned_into(
    pinned: &OwnedFd,
    maximum: u64,
    admitted: fn(&FileStat) -> bool,
    bytes: &mut Vec<u8>,
) -> Result<(), FileReadError> {
    let (file, stat) = reopen_pinned(pinned, maximum, admitted)?;
    read_consistent(file, &stat, admitted, bytes)
}

// A metadata-only pin is checked before ordinary content access or
// synchronisation, avoiding device or FIFO opens.
fn reopen_pinned(
    pinned: &OwnedFd,
    maximum: u64,
    admitted: fn(&FileStat) -> bool,
) -> Result<OpenedRegular, FileReadError> {
    let before = fstat(pinned)?;
    if !admitted(&before) {
        return Err(FileReadError::UnsafeFile);
    }
    let size = file_size(&before)?;
    if size > maximum {
        return Err(FileReadError::TooLarge);
    }
    let descriptor_path = format!("/proc/self/fd/{}", pinned.as_raw_fd());
    let fd = open(
        descriptor_path.as_str(),
        OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let after = fstat(&fd)?;
    if !admitted(&after) || !same_content_metadata(&before, &after) {
        return Err(FileReadError::UnsafeFile);
    }
    Ok((File::from(fd), after))
}

pub(crate) const fn same_inode(left: &FileStat, right: &FileStat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}

// Ignore ctime and link count: atomic replacement can legitimately unlink
// the old inode while a reader still holds it. Admission checks the latter.
const fn same_content_metadata(before: &FileStat, after: &FileStat) -> bool {
    same_inode(before, after)
        && before.st_size == after.st_size
        && before.st_mtime == after.st_mtime
        && before.st_mtime_nsec == after.st_mtime_nsec
}

fn read_consistent(
    mut file: File,
    before: &FileStat,
    admitted: fn(&FileStat) -> bool,
    bytes: &mut Vec<u8>,
) -> Result<(), FileReadError> {
    read_bounded_descriptor(&mut file, file_size(before)?, bytes)?;
    check_content_metadata(&file, before, admitted)
}

fn check_content_metadata(
    file: &File,
    before: &FileStat,
    admitted: fn(&FileStat) -> bool,
) -> Result<(), FileReadError> {
    let after = fstat(file)?;
    if !admitted(&after) || !same_content_metadata(before, &after) {
        return Err(FileReadError::UnsafeFile);
    }
    Ok(())
}

pub(crate) fn read_bounded_descriptor(
    file: &mut File,
    size: u64,
    bytes: &mut Vec<u8>,
) -> Result<(), FileReadError> {
    let length =
        usize::try_from(size).map_err(|_error| FileReadError::TooLarge)?;
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| FileReadError::TooLarge)?;
    bytes.resize(length, 0);
    file.read_exact(bytes)?;
    let mut extra = Zeroizing::new([0u8; 1]);
    match file.read_exact(&mut *extra) {
        Ok(()) => Err(FileReadError::TooLarge),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
        Err(error) => Err(error.into()),
    }
}

const fn regular(stat: &FileStat) -> bool {
    stat.st_mode & SFlag::S_IFMT.bits() == SFlag::S_IFREG.bits()
}

fn file_size(stat: &FileStat) -> Result<u64, FileReadError> {
    u64::try_from(stat.st_size).map_err(|_error| FileReadError::TooLarge)
}

pub(crate) fn trusted_directory(fd: impl AsFd) -> nix::Result<bool> {
    Ok(trusted_directory_stat(&fstat(fd)?))
}

pub(crate) fn trusted_directory_stat(stat: &FileStat) -> bool {
    stat.st_mode & SFlag::S_IFMT.bits() == SFlag::S_IFDIR.bits()
        && stat.st_mode & 0o022 == 0
        && stat.st_uid == geteuid().as_raw()
}

// A zero-link inode can be the complete previous file after an atomic rename.
// Multiple links remain inadmissible, as does any untrusted owner or mode.
pub(crate) fn private_regular(stat: &FileStat) -> bool {
    regular(stat)
        && stat.st_nlink <= 1
        && stat.st_uid == geteuid().as_raw()
        && stat.st_mode & 0o022 == 0
}

fn secret_regular(stat: &FileStat) -> bool {
    private_regular(stat) && stat.st_mode.trailing_zeros() >= 6
}

pub(crate) fn read_regular(
    parent: &impl AsFd,
    name: &str,
    maximum: u64,
) -> Result<Vec<u8>, FileReadError> {
    read_regular_with(parent, name, maximum, |file, size| {
        let mut bytes = Vec::new();
        read_bounded_descriptor(file, size, &mut bytes)?;
        Ok(bytes)
    })
}

// Read a pinned regular file through a caller-supplied bounded decoder.
pub(crate) fn read_regular_with<T, E>(
    parent: &impl AsFd,
    name: &str,
    maximum: u64,
    decode: impl FnOnce(&mut File, u64) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<FileReadError>,
{
    if !trusted_directory(parent).map_err(FileReadError::from)? {
        return Err(FileReadError::UnsafeFile.into());
    }
    let pinned = openat(parent, name, PIN_FLAGS, Mode::empty())
        .map_err(FileReadError::from)?;
    let (mut file, before) = reopen_pinned(&pinned, maximum, private_regular)?;
    let result = decode(&mut file, file_size(&before)?)?;
    check_content_metadata(&file, &before, private_regular)?;
    Ok(result)
}

fn staged_regular(stat: &FileStat) -> bool {
    private_regular(stat) && stat.st_nlink == 1
}

// A staged file cannot legitimately change identity, length, ownership,
// permissions or content timestamps after the writer closes it.
pub(crate) fn check_staged_metadata(
    expected: &FileStat,
    current: &FileStat,
) -> Result<(), FileReadError> {
    if !staged_regular(current)
        || !same_content_metadata(expected, current)
        || expected.st_mode != current.st_mode
        || expected.st_uid != current.st_uid
        || expected.st_gid != current.st_gid
        || expected.st_ctime != current.st_ctime
        || expected.st_ctime_nsec != current.st_ctime_nsec
    {
        return Err(FileReadError::UnsafeFile);
    }
    Ok(())
}

pub(crate) fn verify_staged_regular(
    parent: &impl AsFd,
    name: &str,
    expected: &FileStat,
) -> Result<(), FileReadError> {
    if !trusted_directory(parent)? {
        return Err(FileReadError::UnsafeFile);
    }
    let pinned = openat(parent, name, PIN_FLAGS, Mode::empty())?;
    check_staged_metadata(expected, &fstat(&pinned)?)
}

pub(crate) fn sync_regular(
    parent: &impl AsFd,
    name: &str,
    expected: &FileStat,
) -> Result<(), FileReadError> {
    if !trusted_directory(parent)? {
        return Err(FileReadError::UnsafeFile);
    }
    let pinned = openat(parent, name, PIN_FLAGS, Mode::empty())?;
    let (file, before) =
        reopen_pinned(&pinned, file_size(expected)?, staged_regular)?;
    check_staged_metadata(expected, &before)?;
    file.sync_all()?;
    check_staged_metadata(expected, &fstat(&file)?)?;
    verify_staged_regular(parent, name, expected)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::FileTimes;
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::time::{Duration, UNIX_EPOCH};

    use nix::unistd::mkfifo;

    use super::*;
    use crate::fixtures::TestRoot;

    fn require(condition: bool) -> Result<(), FileReadError> {
        condition
            .then_some(())
            .ok_or(FileReadError::Io(io::ErrorKind::InvalidData))
    }

    #[test]
    fn pinned_regular_survives_atomic_replacement() -> Result<(), FileReadError>
    {
        let root = TestRoot::new()?;
        let directory = open(&root.path, DIRECTORY_FLAGS, Mode::empty())?;
        let slot = root.path.join("slot");
        let next = root.path.join("next");
        fs::write(&slot, b"old")?;
        let pinned = openat(&directory, "slot", PIN_FLAGS, Mode::empty())?;
        fs::write(&next, b"new")?;
        fs::rename(next, slot)?;
        require(read_pinned(&pinned, 3, private_regular)? == b"old")?;
        require(read_regular(&directory, "slot", 3)? == b"new")
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
    fn secret_snapshot_requires_owner_only_file() -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("key");
        let link = root.path.join("link");
        fs::write(&path, b"key")?;
        let limit = NonZeroU64::new(3).unwrap_or(NonZeroU64::MIN);
        for mode in [0o644, 0o640, 0o604, 0o620, 0o602, 0o601] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
            require(
                snapshot_secret(&path, limit) == Err(FileReadError::UnsafeFile),
            )?;
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let secret = snapshot_secret(&path, limit)?;
        require(secret.as_slice() == b"key")?;
        require(format!("{secret:?}") == "Zeroizing { .. }")?;
        require(
            snapshot_secret(&path, NonZeroU64::MIN)
                == Err(FileReadError::TooLarge),
        )?;
        fs::hard_link(&path, &link)?;
        require(
            snapshot_secret(&path, limit) == Err(FileReadError::UnsafeFile),
        )?;
        fs::remove_file(&link)?;
        symlink(&path, &link)?;
        require(
            snapshot_secret(&link, limit) == Err(FileReadError::UnsafeFile),
        )?;
        require(snapshot_secret(&path, limit)?.as_slice() == b"key")
    }

    #[test]
    fn descriptor_same_length_modification_fails_closed()
    -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("file");
        fs::write(&path, b"old")?;
        let file = File::open(&path)?;
        let before = fstat(&file)?;
        fs::write(&path, b"new")?;
        let writer = fs::OpenOptions::new().write(true).open(&path)?;
        writer.set_times(
            FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)),
        )?;
        require(
            read_consistent(file, &before, private_regular, &mut Vec::new())
                == Err(FileReadError::UnsafeFile),
        )?;
        require(
            snapshot(&path, NonZeroU64::new(3).unwrap_or(NonZeroU64::MIN))?
                == b"new",
        )
    }

    #[test]
    fn descriptor_length_changes_fail_closed() -> Result<(), FileReadError> {
        let root = TestRoot::new()?;
        let path = root.path.join("file");
        fs::write(&path, b"four")?;
        let fd = open(&path, OFlag::O_RDONLY, Mode::empty())?;
        fs::write(&path, b"fives")?;
        require(
            read_bounded_descriptor(&mut File::from(fd), 4, &mut Vec::new())
                == Err(FileReadError::TooLarge),
        )?;
        let shortened = open(&path, OFlag::O_RDONLY, Mode::empty())?;
        fs::write(&path, b"x")?;
        require(matches!(
            read_bounded_descriptor(
                &mut File::from(shortened),
                5,
                &mut Vec::new(),
            ),
            Err(FileReadError::Io(io::ErrorKind::UnexpectedEof))
        ))
    }
}
