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
//   - Isolated validator test fixtures.
// - Must-Not:
//   - Implement game behaviour or modify source.
// - Allows:
//   - Inputs: Repository files and local tool configuration.
//   - Outputs: Validation data or diagnostic.
//   - Side effects: Temporary fixture directory writes.
// - Split-When:
//   - An independent validation responsibility emerges.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Isolated validator test fixtures.
// - Description:
//   - Creates independent fixture directories for tests.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Use unique local temporary directories.

use std::ffi::OsStr;
use std::num::NonZeroU64;

use games_platform::ir::security::toolchain::validation::admit_path;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::fixtures::TestRoot;
use games_platform::ir::storage::{PathKind, join_path, path_kind};
use games_platform::ir::toolchain::workspace_root;

/// Create a unique directory for an isolated test.
///
/// # Errors
/// Returns an error if the platform cannot create a private fixture.
pub fn directory() -> Result<TestRoot, String> {
    TestRoot::new().map_err(|error| error.to_string())
}

/// Report a failed test condition without a panic.
///
/// # Errors
/// Returns the supplied diagnostic when the condition is false.
pub fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(String::from(message))
    }
}

/// Read a small ordinary fixture through the bounded snapshot port.
///
/// # Errors
/// Returns a storage or allocation error.
pub fn read(path: &OsStr) -> Result<Vec<u8>, String> {
    snapshot(path, NonZeroU64::MIN.saturating_add(65535))
        .map_err(|error| format!("{error:?}"))
}

/// Observe whether a fixture entry exists without following its final link.
///
/// # Errors
/// Returns an observation error.
pub fn exists(path: &OsStr) -> Result<bool, String> {
    path_kind(path)
        .map(|kind| kind != PathKind::Missing)
        .map_err(|error| error.to_string())
}

/// Read a small authored workspace test fixture through bounded storage.
///
/// # Errors
/// Reports unavailable workspace identity, invalid names or failed reads.
pub fn repository_text(name: &str) -> Result<String, String> {
    let admitted = admit_path(name).map_err(|_error| "invalid fixture name")?;
    let root = workspace_root().map_err(|error| error.to_string())?;
    let location =
        join_path(&root, admitted).map_err(|error| error.to_string())?;
    String::from_utf8(read(&location)?).map_err(|error| error.to_string())
}
