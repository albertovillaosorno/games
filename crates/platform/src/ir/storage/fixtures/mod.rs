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
//   - Portable lifetime and admission of disposable test storage.
// - Must-Not:
//   - Expose native paths or mutate production storage.
// - Allows:
//   - Inputs: Relative fixture names and explicitly authored bytes.
//   - Outputs: Opaque fixture location and typed operation outcomes.
//   - Side effects: Explicit delegated operations inside a private root.
// - Split-When:
//   - Fixture services require an independent lifetime.
// - Merge-When:
//   - Another portable fixture contract provides equivalent ownership.
// - Summary:
//   - Isolate native fixture storage from contributor and title tests.
// - Description:
//   - Keep temporary root ownership until test cleanup.
// - Usage:
//   - Create a private root and operate on admitted relative names.
// - Defaults:
//   - Reject unsupported storage and invalid relative paths.

//! Private storage capabilities for explicitly enabled integration fixtures.

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{ErrorKind, Result as IoResult};

use games_assets::archive::validate_path;

/// Whether the selected backend implements private test storage.
pub const AVAILABLE: bool = <() as FixtureFactory>::AVAILABLE;

/// Owns an isolated private directory until dropped.
#[derive(Debug)]
pub struct TestRoot(Box<dyn FixtureStorage>);

pub(crate) trait FixtureFactory {
    const AVAILABLE: bool;
    fn open_fixture() -> IoResult<Box<dyn FixtureStorage>>;
}

pub(crate) trait FixtureStorage: Debug {
    fn create_directories(&self, name: &str) -> IoResult<()>;
    fn create_sized_file(&self, name: &str, length: u64) -> IoResult<()>;
    fn entry_count(&self) -> IoResult<usize>;
    fn hard_link(&self, source: &str, name: &str) -> IoResult<()>;
    fn link(&self, target: &OsStr, name: &str) -> IoResult<()>;
    fn location(&self) -> &OsStr;
    fn make_executable(&self, name: &str) -> IoResult<()>;
    fn remove_directory(&self, name: &str) -> IoResult<()>;
    fn remove_file(&self, name: &str) -> IoResult<()>;
    fn write_file(&self, name: &str, bytes: &[u8]) -> IoResult<()>;
}

impl TestRoot {
    /// Create all directories in an admitted relative path.
    /// # Errors
    /// Reports invalid names, linked ancestors or creation failure.
    pub fn create_directories(&self, name: &str) -> IoResult<()> {
        self.0.create_directories(admitted(name)?)
    }

    /// Create or replace a sparse fixture of a specified logical length.
    /// # Errors
    /// Reports invalid names, linked entries or sizing failure.
    pub fn create_sized_file(&self, name: &str, length: u64) -> IoResult<()> {
        self.0.create_sized_file(admitted(name)?, length)
    }

    /// Count direct fixture entries.
    /// # Errors
    /// Reports enumeration failure.
    pub fn entry_count(&self) -> IoResult<usize> {
        self.0.entry_count()
    }

    /// Hard-link a local ordinary file to a new local name.
    /// # Errors
    /// Reports unsafe paths, non-ordinary sources or link failures.
    pub fn hard_link(&self, source: &str, name: &str) -> IoResult<()> {
        self.0.hard_link(admitted(source)?, admitted(name)?)
    }

    /// Create a local link for a rejection fixture.
    ///
    /// The target is not opened or resolved by this operation.
    /// # Errors
    /// Reports invalid link names, linked ancestors or creation failure.
    pub fn link(&self, target: &OsStr, name: &str) -> IoResult<()> {
        self.0.link(target, admitted(name)?)
    }

    /// Access the opaque fixture location while retaining ownership.
    #[must_use]
    pub fn location(&self) -> &OsStr {
        self.0.location()
    }

    /// Mark a local ordinary fixture executable by its owner.
    /// # Errors
    /// Reports invalid names, linked entries or permission failure.
    pub fn make_executable(&self, name: &str) -> IoResult<()> {
        self.0.make_executable(admitted(name)?)
    }

    /// Create a unique private fixture directory.
    /// # Errors
    /// Reports unavailable storage or native creation failure.
    pub fn new() -> IoResult<Self> {
        <() as FixtureFactory>::open_fixture().map(Self)
    }

    /// Remove an empty local directory.
    /// # Errors
    /// Reports invalid names, linked ancestors or removal failure.
    pub fn remove_directory(&self, name: &str) -> IoResult<()> {
        self.0.remove_directory(admitted(name)?)
    }

    /// Remove a local non-directory entry without following its final link.
    /// # Errors
    /// Reports invalid names, linked ancestors or removal failure.
    pub fn remove_file(&self, name: &str) -> IoResult<()> {
        self.0.remove_file(admitted(name)?)
    }

    /// Create or replace an ordinary local fixture.
    /// # Errors
    /// Reports invalid names, linked entries or write failure.
    pub fn write_file(&self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.0.write_file(admitted(name)?, bytes)
    }
}

fn admitted(name: &str) -> IoResult<&str> {
    validate_path(name).map_err(|_error| ErrorKind::InvalidInput)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::admitted;

    #[test]
    fn fixture_names_are_admitted_before_dispatch() {
        assert!(admitted("nested/leaf").is_ok());
        for name in ["", "/", "..", "../escape", "a//b", "a/../b", "a\\b"] {
            assert!(admitted(name).is_err(), "{name}");
        }
    }
}
