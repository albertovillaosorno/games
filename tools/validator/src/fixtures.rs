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

use std::num::NonZeroU64;
use std::path::Path;

use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::fixtures::TestRoot;
use games_platform::ir::storage::{PathKind, path_kind};

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
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    snapshot(path.as_os_str(), NonZeroU64::MIN.saturating_add(65535))
        .map_err(|error| format!("{error:?}"))
}

/// Observe whether a fixture entry exists without following its final link.
///
/// # Errors
/// Returns an observation error.
pub fn exists(path: &Path) -> Result<bool, String> {
    path_kind(path.as_os_str())
        .map(|kind| kind != PathKind::Missing)
        .map_err(|error| error.to_string())
}
