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
//   - Explicit browser scenery diagnostic entry and terminal report.
// - Must-Not:
//   - Interpret source media or duplicate title presentation.
// - Allows:
//   - Inputs: A selected local canonical directory with scene tracks.
//   - Outputs: A bounded pixel verification result.
//   - Side effects: Delegated directory selection and scene presentation.
// - Split-When:
//   - Additional independent diagnostics need distinct selection.
// - Merge-When:
//   - An existing diagnostic provides the same asset-backed check.
// - Summary:
//   - Verify the admitted scenery in the ordinary browser renderer.
// - Description:
//   - Reuse the browser catalogue and title's native scene-track and optional
//     second-stage handoff smoke.
// - Usage:
//   - Load with the matching WebAssembly browser binding.
// - Defaults:
//   - No implicit paths, network access or live gameplay loop.

//! Explicit asset-backed browser scenery readback diagnostic.

use games_platform::ir::frame::RenderError;
use games_platform::ir::process::{write_diagnostic, write_status};
use games_simpsons_arcade::runtime::browser_preview::scenery_smoke;

fn main() -> Result<(), RenderError> {
    scenery_smoke(true, complete)
}

fn complete(result: Result<(), RenderError>) {
    match result {
        Ok(()) => {
            let _reported =
                write_status(format_args!("browser-scenery-smoke: passed"));
        },
        Err(error) => {
            let _reported = write_diagnostic(format_args!(
                "browser-scenery-smoke: failed: {error:?}"
            ));
        },
    }
}
