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
//   - Portable exclusive validation state and success-marker requests.
// - Must-Not:
//   - Open host files, compute fingerprints or choose validation checks.
// - Allows:
//   - Inputs: Trusted workspace identity, fixed fingerprints and log bytes.
//   - Outputs: Cache observations, log location and typed I/O failures.
//   - Side effects: Explicit delegation to the private state adapter.
// - Split-When:
//   - Another contributor operation requires independent transaction rules.
// - Merge-When:
//   - A shared validation port owns the same lifecycle and observations.
// - Summary:
//   - Keep native locks, files and atomic publication behind one port.
// - Description:
//   - Hold the exclusive validation lease through log and marker operations.
// - Usage:
//   - Invalidate stale success before checks and publish only after success.
// - Defaults:
//   - Reject unavailable state; no cache success is invented.

//! Exclusive validation state and isolated log output.

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{Result as IoResult, Write};

/// Exact SHA-256 identity of one validated workspace observation.
pub type ValidationKey = [u8; 32];

pub(crate) type StatePort = Box<dyn StateBackend>;

/// Private adapter holding an exclusive run lease and log stream.
pub(crate) trait StateBackend: Debug {
    fn invalidate(&mut self) -> IoResult<()>;
    fn log_path(&self) -> &OsStr;
    fn log_writer(&mut self) -> &mut dyn Write;
    fn matches(&self, key: &ValidationKey) -> IoResult<bool>;
    fn publish(&mut self, key: &ValidationKey) -> IoResult<()>;
}

pub(crate) trait StateFactory {
    fn open_state(root: &OsStr) -> IoResult<StatePort>;
}

/// A validation lease with a dedicated log and atomic success marker.
///
/// Dropping the session releases the backend's exclusive lease. Cache
/// policy belongs to the caller; publication does not run any checks.
#[derive(Debug)]
pub struct ValidationState {
    backend: StatePort,
}

impl ValidationState {
    /// Remove an existing success marker, if present.
    ///
    /// # Errors
    /// Reports a failed invalidation.
    pub fn invalidate(&mut self) -> IoResult<()> {
        self.backend.invalidate()
    }

    /// Location of this run's isolated log.
    #[must_use]
    pub fn log_path(&self) -> &OsStr {
        self.backend.log_path()
    }

    /// Borrow the isolated log stream.
    pub fn log_writer(&mut self) -> &mut dyn Write {
        self.backend.log_writer()
    }

    /// Whether the exact admitted fingerprint has a success marker.
    ///
    /// # Errors
    /// Reports a failed cache observation.
    pub fn matches(&self, key: &ValidationKey) -> IoResult<bool> {
        self.backend.matches(key)
    }

    /// Acquire the workspace validation lease and create a fresh log.
    ///
    /// # Errors
    /// Reports an unsupported or failed native state operation.
    pub fn open(root: &OsStr) -> IoResult<Self> {
        open_with::<()>(root)
    }

    /// Publish the caller's verified fingerprint after flushing the log.
    ///
    /// # Errors
    /// Reports failed log or atomic marker publication.
    pub fn publish(&mut self, key: &ValidationKey) -> IoResult<()> {
        self.log_writer().flush()?;
        self.backend.publish(key)
    }
}

fn open_with<Backend>(root: &OsStr) -> IoResult<ValidationState>
where
    Backend: StateFactory,
{
    Ok(ValidationState {
        backend: Backend::open_state(root)?,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::io::{ErrorKind, Result as IoResult, Write};

    use super::{
        StateBackend, StateFactory, StatePort, ValidationKey, open_with,
    };

    #[derive(Debug, Default)]
    struct MemoryState {
        log: Vec<u8>,
        marker: Option<ValidationKey>,
    }

    impl StateBackend for MemoryState {
        fn invalidate(&mut self) -> IoResult<()> {
            self.marker = None;
            Ok(())
        }

        fn log_path(&self) -> &OsStr {
            OsStr::new("run.log")
        }

        fn log_writer(&mut self) -> &mut dyn Write {
            &mut self.log
        }

        fn matches(&self, key: &ValidationKey) -> IoResult<bool> {
            Ok(self.marker.as_ref() == Some(key))
        }

        fn publish(&mut self, key: &ValidationKey) -> IoResult<()> {
            if self.log.is_empty() {
                return Err(ErrorKind::InvalidData.into());
            }
            self.marker = Some(*key);
            Ok(())
        }
    }

    impl StateFactory for MemoryState {
        fn open_state(root: &OsStr) -> IoResult<StatePort> {
            if root == OsStr::new("unsupported") {
                return Err(ErrorKind::Unsupported.into());
            }
            Ok(Box::new(Self::default()))
        }
    }

    fn require(condition: bool) -> IoResult<()> {
        condition
            .then_some(())
            .ok_or_else(|| ErrorKind::InvalidData.into())
    }

    #[test]
    fn portable_state_preserves_exact_key_and_log_before_publication()
    -> IoResult<()> {
        let mut state = open_with::<MemoryState>(OsStr::new("workspace"))?;
        let first = [1; 32];
        let second = [2; 32];
        require(state.log_path() == OsStr::new("run.log"))?;
        require(!state.matches(&first)?)?;
        require(state.publish(&first).is_err())?;
        state.log_writer().write_all(b"checked")?;
        state.publish(&first)?;
        require(state.matches(&first)?)?;
        require(!state.matches(&second)?)?;
        state.invalidate()?;
        require(!state.matches(&first)?)?;
        require(
            open_with::<MemoryState>(OsStr::new("unsupported"))
                .is_err_and(|error| error.kind() == ErrorKind::Unsupported),
        )?;
        Ok(())
    }
}
