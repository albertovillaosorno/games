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
//   - Browser title entry and terminal status reporting.
// - Must-Not:
//   - Interpret source media or implement game behavior.
// - Allows:
//   - Inputs: An explicitly selected local canonical directory.
//   - Outputs: Live title presentation and a terminal diagnostic.
//   - Side effects: Delegated picker and host presentation.
// - Split-When:
//   - A different launch mode needs separate host composition.
// - Merge-When:
//   - An existing entry point provides the same selection.
// - Summary:
//   - Launch the local asset-backed browser title.
// - Description:
//   - Keep the touch panel enabled alongside keyboard input.
// - Usage:
//   - Load through the matching WebAssembly browser binding.
// - Defaults:
//   - No network discovery or implicit local paths.

//! Executable entry for the local browser title preview.

use games_platform::ir::frame::RenderError;
use games_platform::ir::process::{write_diagnostic, write_status};
use games_simpsons_arcade::runtime::browser_preview::start;

fn main() -> Result<(), RenderError> {
    let result = start(true, complete);
    match &result {
        Ok(()) => {
            let _reported = write_status(format_args!("Preview ready."));
        },
        Err(error) => report_failure(error),
    }
    result
}

fn complete(result: Result<(), RenderError>) {
    match result {
        Ok(()) => {
            let _reported =
                write_status(format_args!("browser-preview: ended"));
        },
        Err(error) => report_failure(&error),
    }
}

fn report_failure(error: &RenderError) {
    let _reported =
        write_diagnostic(format_args!("browser-preview: failed: {error:?}"));
}
