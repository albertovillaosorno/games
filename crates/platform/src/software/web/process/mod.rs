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
//   - Browser dispatch for portable status and diagnostic messages.
// - Must-Not:
//   - Parse launch arguments or block the browser event loop.
// - Allows:
//   - Inputs: Borrowed formatted output messages.
//   - Outputs: Success or typed console availability and invocation errors.
//   - Side effects: Explicit console calls and opt-in page status updates.
// - Split-When:
//   - Other browser process capabilities need independent resources.
// - Merge-When:
//   - Another browser adapter owns equivalent console dispatch.
// - Summary:
//   - Browser process-facing status and diagnostic output.
// - Description:
//   - Preserves message text in the console and a marked status element.
// - Usage:
//   - Dispatch through the portable process capability.
// - Defaults:
//   - Missing or throwing console methods report errors.

//! Browser status and diagnostic output without blocking host work.

use std::ffi::OsString;
use std::fmt::Arguments;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::time::Duration;

use js_sys::{Function, Reflect, global};
use wasm_bindgen::{JsCast as _, JsValue};

use crate::ir::process::ProcessPort;

impl ProcessPort for () {
    fn arguments() -> IoResult<Vec<OsString>> {
        Err(ErrorKind::Unsupported.into())
    }

    fn wait_for_retry(_delay: Duration) -> IoResult<()> {
        Err(ErrorKind::Unsupported.into())
    }

    fn write_diagnostic(message: Arguments<'_>) -> IoResult<()> {
        write_output("error", message)
    }

    fn write_status(message: Arguments<'_>) -> IoResult<()> {
        write_output("log", message)
    }
}

fn write_output(method: &str, message: Arguments<'_>) -> IoResult<()> {
    let text = message.to_string();
    // Embedded titles need not supply a status region. Never overwrite
    // an unrelated element that happens to use the same identifier.
    if let Some(status) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("status"))
        .filter(|element| element.has_attribute("data-games-process-status"))
    {
        status.set_text_content(Some(&text));
    }
    let console = Reflect::get(&global(), &JsValue::from_str("console"))
        .map_err(|_error| Error::other("browser console lookup failed"))?;
    if !console.is_object() {
        return Err(ErrorKind::Unsupported.into());
    }
    let callable = Reflect::get(&console, &JsValue::from_str(method))
        .map_err(|_error| Error::other("browser console method lookup failed"))?
        .dyn_into::<Function>()
        .map_err(|_value| ErrorKind::Unsupported)?;
    let _result = callable
        .call1(&console, &JsValue::from_str(&text))
        .map_err(|_error| Error::other("browser console invocation failed"))?;
    Ok(())
}
