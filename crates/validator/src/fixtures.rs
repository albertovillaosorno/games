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

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use std::{env, fs, process};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Create a unique directory for an isolated test.
///
/// # Errors
/// Returns an error if the clock or filesystem fails.
pub fn directory(label: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!(
        "games-{label}-{}-{stamp}-{sequence}",
        process::id()
    ));
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
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
