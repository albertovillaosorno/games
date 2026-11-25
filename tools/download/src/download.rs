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
//   - Official reference selection and download reporting.
// - Must-Not:
//   - Download original games or execute downloaded tools.
// - Allows:
//   - Inputs: Allowlisted public repositories and release identifiers.
//   - Outputs: Local revision-addressed reference directories.
//   - Side effects: Explicit capability requests and status reporting.
// - Split-When:
//   - A separate download protocol needs independent policy.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Official reference selection and download reporting.
// - Description:
//   - Dispatches admitted upstream sources and official tool releases.
// - Usage:
//   - cargo download
// - Defaults:
//   - Fail closed on invalid inputs.
//

//! Select and report official reference acquisition requests.

use std::ffi::OsString;

use games_platform::ir::toolchain::{self, reference};
use reference::ReferenceTool;

fn selection(args: &[OsString]) -> Result<Vec<ReferenceTool>, String> {
    if args.is_empty() {
        return Ok(ReferenceTool::all().to_vec());
    }
    args.iter()
        .map(|argument| {
            argument
                .to_str()
                .and_then(ReferenceTool::parse)
                .ok_or_else(|| {
                    format!(
                        "unknown reference tool: {}",
                        argument.to_string_lossy()
                    )
                })
        })
        .collect()
}

fn main() -> Result<(), String> {
    let args = toolchain::arguments().map_err(|error| error.to_string())?;
    for tool in selection(&args)? {
        let path =
            reference::download(tool).map_err(|error| error.to_string())?;
        toolchain::write_line(&format!(
            "{}: {}",
            tool.name(),
            path.to_string_lossy()
        ))
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selection_uses_complete_catalogue() -> Result<(), String> {
        if selection(&[])? != ReferenceTool::all() {
            return Err("default reference order changed".into());
        }
        Ok(())
    }

    #[test]
    fn explicit_selection_is_ordered_and_fully_admitted() -> Result<(), String>
    {
        let args = [OsString::from("mame"), OsString::from("craft-libs")];
        let selected = selection(&args)?;
        let names: Vec<_> = selected.iter().map(|tool| tool.name()).collect();
        if names != ["mame", "craft-libs"]
            || selection(&[OsString::from("mame"), OsString::from("../bad")])
                .is_ok()
        {
            return Err("incorrect reference selection admission".into());
        }
        Ok(())
    }
}
