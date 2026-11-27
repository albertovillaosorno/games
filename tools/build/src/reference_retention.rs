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
//   - Explicit contributor retention command arguments and receipts.
// - Must-Not:
//   - Access host storage, choose a deletion root or remove files directly.
// - Allows:
//   - Inputs: Retention operation, resource bounds and reviewed plan identity.
//   - Outputs: Readable plan or applied-plan receipt through the IR.
//   - Side effects: Explicit platform capability dispatch and output.
// - Split-When:
//   - Another independent research lifecycle command needs composition.
// - Merge-When:
//   - Existing contributor commands own the same retention workflow.
// - Summary:
//   - Expose the bounded, explicit private archive retention capability.
// - Description:
//   - Require all policy limits and a matching identity for application.
// - Usage:
//   - Preview retention with explicit age, quota, entry and byte bounds.
// - Defaults:
//   - No implicit application or user-selected deletion path.

//! Command composition for research retention.

use std::ffi::OsString;
use std::num::NonZeroUsize;

use games_platform::ir::toolchain::reference::retention::{
    Operation, Policy, retain,
};
use games_platform::ir::toolchain::write_line;

use super::{USAGE, parse_limit, text};

#[derive(Clone, Copy, Debug)]
/// Explicit retention command and its admitted resource bounds.
pub struct Arguments {
    operation: Operation,
    policy: Policy,
}

impl Arguments {
    /// Admit the exact plan or apply command arguments.
    /// # Errors
    /// Reports missing or malformed arguments and bounds.
    pub fn parse(action: &str, args: &[OsString]) -> Result<Self, String> {
        let (operation, bounds) = match (action, args) {
            ("plan", [age, quota, entries, bytes]) => {
                (Operation::Plan, [age, quota, entries, bytes])
            },
            ("apply", [age, quota, entries, bytes, identity]) => {
                let digest = text(identity)?.parse().map_err(|_error| {
                    String::from("invalid retention plan hash")
                })?;
                (Operation::Apply(digest), [age, quota, entries, bytes])
            },
            _ => return Err(String::from(USAGE)),
        };
        let [age, quota, entries, bytes] = bounds;
        let policy = Policy {
            maximum_bytes: parse_limit(bytes)?,
            maximum_entries: text(entries)?.parse::<NonZeroUsize>().map_err(
                |_error| String::from("invalid retention entry bound"),
            )?,
            minimum_age: parse_limit(age)?,
            quota: text(quota)?
                .parse()
                .map_err(|_error| String::from("invalid retention quota"))?,
        };
        Ok(Self { operation, policy })
    }

    /// Dispatch the admitted operation and print its receipt.
    /// # Errors
    /// Reports storage admission or output failure.
    pub fn run(self) -> Result<(), String> {
        let report = retain(self.policy, self.operation)
            .map_err(|error| error.to_string())?;
        let label = match self.operation {
            Operation::Plan => "Preview (read-only)",
            Operation::Apply(_) => "Applied reviewed plan",
        };
        output(&format!("{label}: {}", report.identity))?;
        for removal in report.removals {
            output(&format!(
                "  {}: {} logical bytes; retained games/{}",
                removal.name, removal.bytes, removal.evidence
            ))?;
            output(&format!(
                "    retained record SHA-256: {}",
                removal.evidence_digest
            ))?;
        }
        for name in report.protected {
            output(&format!("  protected unsealed run: {name}"))?;
        }
        output(&format!(
            "Sealed logical bytes: {} -> {}; quota {}",
            report.bytes_before, report.bytes_after, self.policy.quota
        ))?;
        if report.bytes_after > self.policy.quota {
            output(
                "Quota unmet: remaining runs are protected by minimum age.",
            )?;
        }
        Ok(())
    }
}

fn output(message: &str) -> Result<(), String> {
    write_line(message).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_apply_requires_exact_identity_and_all_bounds() {
        let valid = ["60", "0", "100", "1000"].map(OsString::from);
        assert!(Arguments::parse("plan", &valid).is_ok());
        assert!(Arguments::parse("apply", &valid).is_err());
        let mut apply = valid.to_vec();
        apply.push(OsString::from("0".repeat(64)));
        assert!(Arguments::parse("apply", &apply).is_ok());
        assert!(Arguments::parse("plan", &apply).is_err());
        assert!(
            Arguments::parse(
                "plan",
                &["0", "0", "100", "1000"].map(OsString::from)
            )
            .is_err()
        );
    }
}
