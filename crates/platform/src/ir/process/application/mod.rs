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
//   - Deterministic selection of a registered application entry.
// - Must-Not:
//   - Interpret launch options or select a concrete host.
// - Allows:
//   - Inputs: Owned launch arguments and registered Rust callbacks.
//   - Outputs: A unique callback result or an explicit selection error.
//   - Side effects: The selected callback may launch the application.
// - Split-When:
//   - Another launch policy requires a separate contract.
// - Merge-When:
//   - A shared entry registry owns identical selection semantics.
// - Summary:
//   - One application entry per independently packaged artifact.
// - Description:
//   - The order of registration never selects an application.
// - Usage:
//   - Register one typed callback and dispatch with owned arguments.
// - Defaults:
//   - Reject absent or ambiguous entries before invoking any callback.

//! Application entry registration and selection.

use std::ffi::OsString;

/// Register one typed application callback in the linked artifact.
pub use inventory::submit as register;

/// Callback accepting the application launch arguments.
pub type ApplicationCallback = fn(Vec<OsString>) -> Result<(), String>;

/// An independently packaged application's launch callback.
#[derive(Clone, Copy, Debug)]
pub struct Application {
    run: ApplicationCallback,
}

impl Application {
    /// Register a callback that consumes the application's launch arguments.
    #[must_use]
    pub const fn new(run: ApplicationCallback) -> Self {
        Self { run }
    }
}

/// Result of dispatching a registered application.
#[derive(Debug, Eq, PartialEq)]
pub enum ApplicationError {
    /// More than one callback was registered in this artifact.
    Ambiguous,
    /// No callback was registered in this artifact.
    Missing,
    /// The selected callback reported failure.
    Rejected(String),
}

inventory::collect!(Application);

/// Run exactly one registered application with explicit owned arguments.
///
/// # Errors
/// Rejects missing or ambiguous registrations and propagates launch failure.
pub fn launch_registered(
    arguments: Vec<OsString>,
) -> Result<(), ApplicationError> {
    launch_with(inventory::iter::<Application>.into_iter(), arguments)
}

fn launch_with<'entry, Entries>(
    mut entries: Entries,
    arguments: Vec<OsString>,
) -> Result<(), ApplicationError>
where
    Entries: Iterator<Item = &'entry Application>,
{
    let entry = entries.next().ok_or(ApplicationError::Missing)?;
    if entries.next().is_some() {
        return Err(ApplicationError::Ambiguous);
    }
    (entry.run)(arguments).map_err(ApplicationError::Rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts_only_explicit_arguments(
        arguments: Vec<OsString>,
    ) -> Result<(), String> {
        if arguments
            .into_iter()
            .eq([OsString::from("--local"), OsString::from("a b")])
        {
            Ok(())
        } else {
            Err(String::from("unexpected launch arguments"))
        }
    }

    #[test]
    fn missing_and_ambiguous_entries_never_invoke_callbacks() {
        let first = Application::new(accepts_only_explicit_arguments);
        let second = Application::new(accepts_only_explicit_arguments);
        assert_eq!(
            launch_with([].iter(), Vec::new()),
            Err(ApplicationError::Missing),
        );
        assert_eq!(
            launch_with([first, second].iter(), Vec::new()),
            Err(ApplicationError::Ambiguous),
        );
    }

    #[test]
    fn one_entry_preserves_arguments_and_callback_error() {
        let entry = Application::new(accepts_only_explicit_arguments);
        assert_eq!(
            launch_with([entry].iter(), vec![
                OsString::from("--local"),
                OsString::from("a b")
            ],),
            Ok(()),
        );
        assert_eq!(
            launch_with([entry].iter(), Vec::new()),
            Err(ApplicationError::Rejected(String::from(
                "unexpected launch arguments",
            ))),
        );
    }
}
