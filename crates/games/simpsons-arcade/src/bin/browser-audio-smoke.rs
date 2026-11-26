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
//   - Explicit browser audio diagnostic executable.
// - Must-Not:
//   - Open native APIs, interpret game data or modify user settings.
// - Allows:
//   - Inputs: Explicit invocation in an activated browser document.
//   - Outputs: Success or portable asynchronous audio diagnostic outcome.
//   - Side effects: Delegated isolated silent playback diagnostic.
// - Split-When:
//   - Other smoke scenarios need independent resources.
// - Merge-When:
//   - A shared executable covers the same acceptance checks.
// - Summary:
//   - Exercise browser audio translation without game assets.
// - Description:
//   - Delegates through the portable audio fixture contract.
// - Usage:
//   - Invoke in an isolated browser context.
// - Defaults:
//   - No original assets or gameplay state.

//! Isolated browser audio diagnostic.

use games_platform::ir::audio::PlaybackError;
use games_platform::ir::audio::fixtures::verify_playback;
use games_platform::ir::process::write_diagnostic;

fn main() -> Result<(), PlaybackError> {
    let result = verify_playback();
    if let Err(error) = &result {
        let _reported = write_diagnostic(format_args!(
            "Browser audio diagnostic failed: {error:?}"
        ));
    }
    result
}
