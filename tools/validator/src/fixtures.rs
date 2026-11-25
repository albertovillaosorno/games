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

use games_platform::ir::storage::fixtures::TestRoot;

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
