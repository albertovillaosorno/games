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
//   - Authored processor description generation for contributor analysis.
// - Must-Not:
//   - Access original binaries, install tools or execute analysis.
// - Allows:
//   - Inputs: Admitted processor and output component identifiers.
//   - Outputs: Deterministic Ghidra processor description text.
//   - Side effects: Explicit contributor output through the toolchain IR.
// - Split-When:
//   - Additional processor families need independent definitions.
// - Merge-When:
//   - Shared generation machinery replaces duplicate emitters.
// - Summary:
//   - Generate contributor-only Ghidra processor components.
// - Description:
//   - Unsupported instructions remain undecodable.
// - Usage:
//   - games-build ghidra konami slaspec
// - Defaults:
//   - Reject unknown processors and component identifiers.

//! Deterministic contributor-only Ghidra processor descriptions.

mod konami;

use std::ffi::OsString;

use games_platform::ir::toolchain::write_line;

use super::{USAGE, text};

/// Selected processor description component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Component {
    /// Compiler specification.
    Compiler,
    /// Language registration.
    Language,
    /// Processor specification.
    Processor,
    /// SLEIGH specification.
    Sleigh,
}

impl Component {
    /// Admit a processor and exactly one component.
    /// # Errors
    /// Rejects unknown processors and output identifiers.
    pub fn parse(processor: &str, args: &[OsString]) -> Result<Self, String> {
        let [component] = args else {
            return Err(String::from(USAGE));
        };
        match (processor, text(component)?) {
            ("konami", "cspec") => Ok(Self::Compiler),
            ("konami", "ldefs") => Ok(Self::Language),
            ("konami", "pspec") => Ok(Self::Processor),
            ("konami", "slaspec") => Ok(Self::Sleigh),
            _ => Err(String::from(USAGE)),
        }
    }

    /// Emit one component through the contributor output IR.
    /// # Errors
    /// Reports output or generation failure.
    pub fn run(self) -> Result<(), String> {
        let output = match self {
            Self::Compiler => String::from(konami::COMPILER),
            Self::Language => String::from(konami::LANGUAGE),
            Self::Processor => String::from(konami::PROCESSOR),
            Self::Sleigh => konami::sleigh()?,
        };
        write_line(&output).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_selection_is_closed() -> Result<(), String> {
        for (name, component) in [
            ("cspec", Component::Compiler),
            ("ldefs", Component::Language),
            ("pspec", Component::Processor),
            ("slaspec", Component::Sleigh),
        ] {
            if Component::parse("konami", &[OsString::from(name)])? != component
            {
                return Err(String::from("incorrect component"));
            }
        }
        if Component::parse("6809", &[OsString::from("slaspec")]).is_ok()
            || Component::parse("konami", &[OsString::from("../file")]).is_ok()
        {
            return Err(String::from("unknown component admitted"));
        }
        Ok(())
    }
}
