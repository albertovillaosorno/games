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
//   - Portable launch arguments and explicit status output.
// - Must-Not:
//   - Read host arguments, write native streams or interpret title options.
// - Allows:
//   - Inputs: Explicit requests and borrowed formatted status messages.
//   - Outputs: Owned portable arguments or typed I/O errors.
//   - Side effects: Only explicitly requested adapter dispatch.
// - Split-When:
//   - Additional process capabilities require independent admission.
// - Merge-When:
//   - Another contract owns the same process observations.
// - Summary:
//   - Shared process-facing observations for titles and tools.
// - Description:
//   - Preserves non-Unicode launch arguments and output failures.
// - Usage:
//   - Observe launch arguments and explicitly emit a status line.
// - Defaults:
//   - Missing host implementations return unsupported errors.

//! Portable process arguments and status output.

use std::ffi::OsString;
use std::fmt::Arguments;
use std::io::Result as IoResult;

pub(crate) trait ProcessPort {
    fn arguments() -> IoResult<Vec<OsString>>;

    fn write_status(message: Arguments<'_>) -> IoResult<()>;
}

/// Observe launch arguments excluding the executable name.
///
/// # Errors
/// Returns an unavailable capability or host observation error.
pub fn arguments() -> IoResult<Vec<OsString>> {
    arguments_with::<()>()
}

/// Write a formatted status message followed by a newline.
///
/// # Errors
/// Returns an unavailable capability or output failure.
pub fn write_status(message: Arguments<'_>) -> IoResult<()> {
    write_status_with::<()>(message)
}

fn arguments_with<Port>() -> IoResult<Vec<OsString>>
where
    Port: ProcessPort,
{
    Port::arguments()
}

fn write_status_with<Port>(message: Arguments<'_>) -> IoResult<()>
where
    Port: ProcessPort,
{
    Port::write_status(message)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    struct Recording;

    impl ProcessPort for Recording {
        fn arguments() -> IoResult<Vec<OsString>> {
            Ok(vec![OsString::from("--local"), OsString::from("a b")])
        }

        fn write_status(message: Arguments<'_>) -> IoResult<()> {
            if message.to_string() == "ready 4" {
                Ok(())
            } else {
                Err(ErrorKind::BrokenPipe.into())
            }
        }
    }

    #[test]
    fn process_port_preserves_arguments_messages_and_errors() -> IoResult<()> {
        let arguments = arguments_with::<Recording>()?;
        if arguments != [OsString::from("--local"), OsString::from("a b")] {
            return Err(ErrorKind::InvalidData.into());
        }
        write_status_with::<Recording>(format_args!("ready {}", 4u8))?;
        if write_status_with::<Recording>(format_args!("unavailable"))
            .is_err_and(|error| error.kind() == ErrorKind::BrokenPipe)
        {
            Ok(())
        } else {
            Err(ErrorKind::InvalidData.into())
        }
    }
}
