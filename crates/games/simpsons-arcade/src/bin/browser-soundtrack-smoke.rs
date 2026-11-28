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
//   - Explicit browser soundtrack diagnostic entry and terminal report.
// - Must-Not:
//   - Interpret source media or duplicate title presentation.
// - Allows:
//   - Inputs: A selected local canonical directory with finite scene music.
//   - Outputs: A bounded playback verification result.
//   - Side effects: Delegated directory selection and audio playback.
// - Split-When:
//   - Additional independent diagnostics need distinct selection.
// - Merge-When:
//   - An existing diagnostic provides the same asset-backed check.
// - Summary:
//   - Verify the admitted finite soundtrack through the browser host.
// - Description:
//   - Reuse the browser catalogue and title soundtrack playback to verify the
//     second-stage handoff, optional cemetery card, pause, resume and end.
// - Usage:
//   - Load with the matching WebAssembly browser binding.
// - Defaults:
//   - No implicit paths, network access or live gameplay loop.

//! Explicit asset-backed browser soundtrack playback diagnostic.

use games_platform::ir::frame::RenderError;
use games_platform::ir::process::{write_diagnostic, write_status};
use games_simpsons_arcade::runtime::browser_preview::soundtrack_smoke;

fn main() -> Result<(), RenderError> {
    soundtrack_smoke(complete)
}

fn complete(result: Result<(), RenderError>) {
    match result {
        Ok(()) => {
            let _reported =
                write_status(format_args!("browser-soundtrack-smoke: passed"));
        },
        Err(error) => {
            let _reported = write_diagnostic(format_args!(
                "browser-soundtrack-smoke: failed: {error:?}"
            ));
        },
    }
}
