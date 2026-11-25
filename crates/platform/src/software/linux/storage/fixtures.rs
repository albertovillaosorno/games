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

use std::path::{Path, PathBuf};
use std::{env, fs, io};

use nix::unistd::mkdtemp;

/// Unique private test directory, removed when its handle is dropped.
#[derive(Debug)]
pub struct TestRoot {
    pub(crate) path: PathBuf,
}

impl TestRoot {
    /// Atomically create an owner-only directory under the temporary root.
    /// # Errors
    /// Reports a native temporary-directory creation failure.
    pub fn new() -> io::Result<Self> {
        let template = env::temp_dir().join("games-fixture-XXXXXX");
        Ok(Self {
            path: mkdtemp(&template)?,
        })
    }

    /// Access the fixture directory while retaining its cleanup handle.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

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
