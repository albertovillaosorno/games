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
//   - Native validation cache lease, isolated logs and marker publication.
// - Must-Not:
//   - Compute fingerprints, validate source or decide cache reuse.
// - Allows:
//   - Inputs: Trusted workspace location, fixed keys and log writes.
//   - Outputs: Cache observations, log path and native I/O results.
//   - Side effects: Creates local directories, locks and writes local state.
// - Split-When:
//   - Another operation requires a distinct cache transaction lifecycle.
// - Merge-When:
//   - A native adapter owns the same lock and publication semantics.
// - Summary:
//   - Preserve validation state without exposing native file handles.
// - Description:
//   - Retain a workspace lock until the isolated run finishes.
// - Usage:
//   - Acquire through the portable contributor validation state.
// - Defaults:
//   - Reject unexpected entry types; publish complete markers atomically.

//! Native validation lease and log state.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Error, ErrorKind, Read as _, Result as IoResult, Write};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use nix::libc::{O_NOFOLLOW, O_NONBLOCK};

use crate::ir::toolchain::environment::validation_log_name;
use crate::ir::toolchain::validation::state::{
    StateBackend, StateFactory, StatePort, ValidationKey,
};

#[derive(Debug)]
struct State {
    _lease: File,
    log: File,
    log_path: PathBuf,
    marker: PathBuf,
    pending: PathBuf,
}

impl StateBackend for State {
    fn invalidate(&mut self) -> IoResult<()> {
        match fs::remove_file(&self.marker) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn log_path(&self) -> &OsStr {
        self.log_path.as_os_str()
    }

    fn log_writer(&mut self) -> &mut dyn Write {
        &mut self.log
    }

    fn matches(&self, key: &ValidationKey) -> IoResult<bool> {
        let Some(mut file) = existing_marker(&self.marker)? else {
            return Ok(false);
        };
        let mut observed = [0; 32];
        match file.read_exact(&mut observed) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                return Ok(false);
            },
            Err(error) => return Err(error),
        }
        let mut extra = [0];
        Ok(file.read(&mut extra)? == 0 && observed == *key)
    }

    fn publish(&mut self, key: &ValidationKey) -> IoResult<()> {
        let mut pending = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.pending)?;
        let result = pending
            .write_all(key)
            .and_then(|()| pending.sync_all())
            .and_then(|()| fs::rename(&self.pending, &self.marker));
        if result.is_err() {
            let _cleanup = fs::remove_file(&self.pending);
        }
        result
    }
}

impl StateFactory for () {
    fn open_state(root: &OsStr) -> IoResult<StatePort> {
        let workspace = Path::new(root);
        let cache = state_directory(workspace, ".cache")?;
        let logs = state_directory(workspace, ".temp")?;
        let lease = lock(&cache.join("lock"))?;
        let name = validation_log_name()
            .map_err(|error| Error::other(error.to_string()))?;
        let log_path = logs.join(&name);
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&log_path)?;
        Ok(Box::new(State {
            _lease: lease,
            log,
            log_path,
            marker: cache.join("success"),
            pending: cache.join(format!("pending-{name}")),
        }))
    }
}

fn directory(path: &Path) -> IoResult<()> {
    fs::create_dir_all(path)?;
    if fs::symlink_metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(ErrorKind::InvalidData.into())
    }
}

fn existing_marker(path: &Path) -> IoResult<Option<File>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {},
        Ok(_metadata) => return Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)?;
    if file.metadata()?.is_file() {
        Ok(Some(file))
    } else {
        Err(ErrorKind::InvalidData.into())
    }
}

fn lock(path: &Path) -> IoResult<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {},
        Ok(_metadata) => return Err(ErrorKind::InvalidData.into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ErrorKind::InvalidData.into());
    }
    file.lock()?;
    Ok(file)
}

fn state_directory(root: &Path, parent: &str) -> IoResult<PathBuf> {
    let local = root.join(parent);
    directory(&local)?;
    let state = local.join("validate");
    directory(&state)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::storage::fixtures::{TestRoot, file_link};
    use crate::ir::toolchain::validation::state::ValidationState;

    fn require(condition: bool) -> IoResult<()> {
        condition
            .then_some(())
            .ok_or_else(|| ErrorKind::InvalidData.into())
    }

    #[test]
    fn exact_markers_and_link_rejection_preserve_other_files() -> IoResult<()> {
        let root = TestRoot::new()?;
        let mut state = ValidationState::open(root.path().as_os_str())?;
        let marker = root.path().join(".cache/validate/success");
        let key = [7; 32];
        fs::write(&marker, [7; 31])?;
        require(!state.matches(&key)?)?;
        fs::write(&marker, [7; 33])?;
        require(!state.matches(&key)?)?;
        state.invalidate()?;
        root.write_file("unrelated", &key)?;
        let unrelated = root.path().join("unrelated");
        file_link(&unrelated, &marker)?;
        require(!state.matches(&key)?)?;
        state.invalidate()?;
        state.log_writer().write_all(b"PASS\n")?;
        state.publish(&key)?;
        require(state.matches(&key)?)?;
        require(fs::read(&unrelated)? == key)?;
        require(fs::read(&marker)? == key)?;
        require(fs::read(Path::new(state.log_path()))? == b"PASS\n")?;
        Ok(())
    }

    #[test]
    fn linked_state_directories_and_lock_are_not_opened() -> IoResult<()> {
        let root = TestRoot::new()?;
        let other = TestRoot::new()?;
        let linked = root.path().join(".cache");
        file_link(other.path(), &linked)?;
        require(
            ValidationState::open(root.path().as_os_str())
                .is_err_and(|error| error.kind() == ErrorKind::InvalidData),
        )?;
        require(other.entry_count()? == 0)?;
        fs::remove_file(linked)?;
        let cache = state_directory(root.path(), ".cache")?;
        other.write_file("protected", b"unchanged")?;
        let protected = other.path().join("protected");
        file_link(&protected, &cache.join("lock"))?;
        require(
            ValidationState::open(root.path().as_os_str())
                .is_err_and(|error| error.kind() == ErrorKind::InvalidData),
        )?;
        require(fs::read(protected)? == b"unchanged")?;
        Ok(())
    }
}
