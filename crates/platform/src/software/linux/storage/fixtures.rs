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
//   - Private, disposable native storage fixtures for workspace tests.
// - Must-Not:
//   - Own test assertions, production saves or original media.
// - Allows:
//   - Inputs: Explicit test requests for a temporary directory.
//   - Outputs: Unique owner-only directory and its lifetime handle.
//   - Side effects: Creates and removes the temporary test directory.
// - Split-When:
//   - A distinct fixture lifecycle requires another storage capability.
// - Merge-When:
//   - Another test adapter duplicates private temporary directories.
// - Summary:
//   - Shared storage test fixtures.
// - Description:
//   - Uses the native atomic private-directory creation mechanism.
// - Usage:
//   - Hold the root for the duration of an isolated storage test.
// - Defaults:
//   - Test-only feature, automatic best-effort cleanup.

//! Isolated native storage fixtures for workspace integration tests.

use std::ffi::OsStr;
use std::io::Write as _;
use std::os::fd::OwnedFd;
#[cfg(test)]
use std::os::unix::fs::{PermissionsExt as _, symlink};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::{env, fs, io};

use games_assets::archive::validate_path;
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{Mode, fchmod, fstat, fstatat, mkdirat};
use nix::unistd::{UnlinkatFlags, mkdtemp, symlinkat, unlinkat};

use super::file::{
    DIRECTORY_FLAGS, private_regular, same_inode, trusted_directory,
    trusted_directory_stat,
};
use super::pinned_directory_entries;
use crate::ir::storage::fixtures::{FixtureFactory, FixtureStorage};

/// Unique private test directory, removed on drop if still at its location.
#[derive(Debug)]
pub(in crate::software::linux) struct TestRoot {
    pub(super) path: PathBuf,
    root: OwnedFd,
}

impl TestRoot {
    /// Atomically create an owner-only directory under the temporary root.
    /// # Errors
    /// Reports a native temporary-directory creation failure.
    pub(in crate::software::linux) fn new() -> io::Result<Self> {
        let template = env::temp_dir().join("games-fixture-XXXXXX");
        let path = mkdtemp(&template)?;
        let root = open(&path, DIRECTORY_FLAGS, Mode::empty())?;
        Ok(Self { path, root })
    }

    fn open_directory(&self, name: &str, create: bool) -> io::Result<OwnedFd> {
        let mut directory = self.verify_location()?;
        for component in name.split('/').filter(|part| !part.is_empty()) {
            if create {
                ensure_directory(&directory, component)?;
            }
            directory =
                openat(&directory, component, DIRECTORY_FLAGS, Mode::empty())?;
            if !trusted_directory(&directory)? {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        Ok(directory)
    }

    fn open_parent<'name>(
        &self,
        name: &'name str,
    ) -> io::Result<(OwnedFd, &'name str)> {
        validate_path(name).map_err(|_error| io::ErrorKind::InvalidInput)?;
        let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
        Ok((self.open_directory(parent, false)?, leaf))
    }

    /// Access the fixture directory while retaining its cleanup handle.
    #[must_use]
    #[cfg(test)]
    pub(in crate::software::linux) fn path(&self) -> &Path {
        &self.path
    }

    fn replacement(&self, name: &str) -> io::Result<fs::File> {
        let (parent, leaf) = self.open_parent(name)?;
        match fstatat(&parent, leaf, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) if private_regular(&stat) => {
                unlinkat(&parent, leaf, UnlinkatFlags::NoRemoveDir)?;
            },
            Ok(_) => return Err(io::ErrorKind::InvalidInput.into()),
            Err(Errno::ENOENT) => {},
            Err(error) => return Err(error.into()),
        }
        let file = openat(
            &parent,
            leaf,
            OFlag::O_WRONLY
                | OFlag::O_CREAT
                | OFlag::O_EXCL
                | OFlag::O_NOFOLLOW
                | OFlag::O_CLOEXEC,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )?;
        Ok(fs::File::from(file))
    }

    fn verify_location(&self) -> io::Result<OwnedFd> {
        let current = open(&self.path, DIRECTORY_FLAGS, Mode::empty())?;
        let expected = fstat(&self.root)?;
        let observed = fstat(&current)?;
        if !trusted_directory_stat(&expected)
            || !trusted_directory_stat(&observed)
            || !same_inode(&expected, &observed)
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(current)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if self.verify_location().is_ok() {
            let _result = fs::remove_dir_all(&self.path);
        }
    }
}

impl FixtureFactory for () {
    const AVAILABLE: bool = true;

    fn open_fixture() -> io::Result<Box<dyn FixtureStorage>> {
        Ok(Box::new(TestRoot::new()?))
    }
}

impl FixtureStorage for TestRoot {
    /// Create all directories in a relative fixture path.
    /// # Errors
    /// Rejects unsafe names, linked ancestors and native creation failures.
    fn create_directories(&self, name: &str) -> io::Result<()> {
        validate_path(name).map_err(|_error| io::ErrorKind::InvalidInput)?;
        let _directory = self.open_directory(name, true)?;
        Ok(())
    }

    /// Create or replace a sparse file of the requested logical length.
    /// # Errors
    /// Rejects unsafe names, linked ancestors and native sizing failures.
    fn create_sized_file(&self, name: &str, length: u64) -> io::Result<()> {
        self.replacement(name)?.set_len(length)
    }

    /// Count directory entries, including unpublished staging remnants.
    /// # Errors
    /// Reports an enumeration or counter overflow failure.
    fn entry_count(&self) -> io::Result<usize> {
        let current = self.verify_location()?;
        pinned_directory_entries(&current)?.try_fold(0usize, |count, entry| {
            let _entry = entry?;
            count
                .checked_add(1)
                .ok_or_else(|| io::ErrorKind::InvalidData.into())
        })
    }

    /// Create a link at a local fixture leaf for rejection tests.
    /// # Errors
    /// Rejects unsafe link locations and native link creation failures.
    fn link(&self, target: &OsStr, name: &str) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(name)?;
        symlinkat(target, &parent, leaf)?;
        Ok(())
    }

    /// Access the opaque fixture location without a native path handle.
    fn location(&self) -> &OsStr {
        self.path.as_os_str()
    }

    /// Mark a local ordinary fixture file executable by its owner.
    /// # Errors
    /// Rejects unsafe paths, non-ordinary files and native failures.
    fn make_executable(&self, name: &str) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(name)?;
        let stat = fstatat(&parent, leaf, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        if !private_regular(&stat) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let file = openat(
            &parent,
            leaf,
            OFlag::O_RDONLY
                | OFlag::O_NOFOLLOW
                | OFlag::O_NONBLOCK
                | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        if !private_regular(&fstat(&file)?) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        fchmod(&file, Mode::S_IRWXU)?;
        Ok(())
    }

    /// Remove an empty local directory.
    /// # Errors
    /// Rejects invalid paths, linked directories and native removal failures.
    fn remove_directory(&self, name: &str) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(name)?;
        unlinkat(&parent, leaf, UnlinkatFlags::RemoveDir)?;
        Ok(())
    }

    /// Remove a single local entry without following its final link.
    /// # Errors
    /// Rejects invalid paths and native removal failures.
    fn remove_file(&self, name: &str) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(name)?;
        unlinkat(&parent, leaf, UnlinkatFlags::NoRemoveDir)?;
        Ok(())
    }

    /// Write or replace a fixture at an existing local parent.
    /// # Errors
    /// Rejects invalid relative paths and native write failures.
    fn write_file(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.replacement(name)?.write_all(bytes)
    }
}

fn ensure_directory(parent: &OwnedFd, name: &str) -> io::Result<()> {
    match mkdirat(parent, name, Mode::S_IRWXU) {
        Ok(()) | Err(Errno::EEXIST) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Create a file link for a disposable rejection test.
/// # Errors
/// Reports the host filesystem error.
#[cfg(test)]
pub(in crate::software::linux) fn file_link(
    target: &Path,
    link: &Path,
) -> io::Result<()> {
    symlink(target, link)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn fixture_admits_only_local_paths() -> io::Result<()> {
        let root = TestRoot::new()?;
        root.write_file("local", b"test")?;
        root.write_file("local", b"replacement")?;
        file_link(&root.path().join("local"), &root.path().join("link"))?;
        let valid = root.entry_count()? == 2
            && root.write_file("link", b"bad").is_err()
            && root.write_file("nested/leaf", b"bad").is_err()
            && fs::read(root.path().join("local"))? == b"replacement"
            && root.write_file("../escape", b"bad").is_err()
            && root.write_file("/absolute", b"bad").is_err();
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[test]
    fn nested_fixture_writes_never_follow_links() -> io::Result<()> {
        let root = TestRoot::new()?;
        let other = TestRoot::new()?;
        other.write_file("protected", b"untouched")?;
        root.create_directories("local/nested")?;
        root.write_file("local/nested/ordinary", b"original")?;
        root.create_sized_file("local/nested/ordinary", 8192)?;
        root.make_executable("local/nested/ordinary")?;
        root.link(other.location(), "local/linked")?;
        let valid = root.create_directories("local/linked/escape").is_err()
            && root
                .write_file("local/linked/protected", b"changed")
                .is_err()
            && root.remove_file("local/linked/protected").is_err()
            && root.make_executable("local/linked/protected").is_err()
            && root.create_sized_file("local/linked/protected", 1).is_err()
            && root.create_directories("../escape").is_err()
            && fs::read(other.path().join("protected"))? == b"untouched"
            && fs::metadata(root.path().join("local/nested/ordinary"))?.len()
                == 8192;
        root.remove_file("local/linked")?;
        root.remove_file("local/nested/ordinary")?;
        root.remove_directory("local/nested")?;
        valid
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[test]
    fn replaced_fixture_root_is_not_modified_or_removed() -> io::Result<()> {
        let outer = TestRoot::new()?;
        let path = outer.path().join("root");
        let moved = outer.path().join("moved");
        mkdirat(&outer.root, "root", Mode::S_IRWXU)?;
        let pinned = open(&path, DIRECTORY_FLAGS, Mode::empty())?;
        let root = TestRoot {
            path: path.clone(),
            root: pinned,
        };
        root.write_file("original", b"data")?;
        fs::rename(&path, &moved)?;
        mkdirat(&outer.root, "root", Mode::S_IRWXU)?;
        fs::write(path.join("sentinel"), b"untouched")?;
        let rejected = root.write_file("sentinel", b"changed").is_err()
            && root.entry_count().is_err();
        drop(root);
        (rejected
            && fs::read(path.join("sentinel"))? == b"untouched"
            && fs::read(moved.join("original"))? == b"data")
            .then_some(())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    #[test]
    fn distinct_private_roots_are_removed() -> Result<(), String> {
        let first = TestRoot::new().map_err(|error| error.to_string())?;
        let second = TestRoot::new().map_err(|error| error.to_string())?;
        let first_path = first.path().to_owned();
        let second_path = second.path().to_owned();
        if first_path == second_path {
            return Err(String::from("fixture directories must be distinct"));
        }
        for path in [&first_path, &second_path] {
            let metadata =
                fs::metadata(path).map_err(|error| error.to_string())?;
            if !metadata.is_dir()
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(String::from("fixture directory must be private"));
            }
            fs::write(path.join("local"), b"test")
                .map_err(|error| error.to_string())?;
        }
        drop(first);
        if first_path.exists() || !second_path.join("local").exists() {
            return Err(String::from("fixture cleanup affected another root"));
        }
        drop(second);
        if second_path.exists() {
            return Err(String::from("fixture cleanup left a directory"));
        }
        Ok(())
    }
}
