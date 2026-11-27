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
//   - Bounded retention requests, deterministic selection and plan identity.
// - Must-Not:
//   - Access storage, clocks or native operating-system resources.
// - Allows:
//   - Inputs: Explicit limits and positively classified archive inventory.
//   - Outputs: Reviewable removal plan and typed dispatch results.
//   - Side effects: Explicit native backend dispatch only.
// - Split-When:
//   - A separate retention lifecycle needs independent policy.
// - Merge-When:
//   - Another contract owns identical bounded retention selection.
// - Summary:
//   - Retain fresh archives and remove old archives only above quota.
// - Description:
//   - Bind the reviewed plan to its complete admitted inventory and policy.
// - Usage:
//   - Preview a plan, then explicitly submit its identity for application.
// - Defaults:
//   - Read-only planning, no automatic or unclassified removal.

//! Contributor research retention policy and portable dispatch.

use std::fmt::{Error as FormatError, Write as _};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::ContentHash;

use super::ToolchainError;

/// Explicit age, quota and inventory admission bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    /// Maximum aggregate admitted logical bytes, including evidence reads.
    pub maximum_bytes: NonZeroU64,
    /// Maximum number of observed directory entries, including protected runs.
    pub maximum_entries: NonZeroUsize,
    /// Minimum age in whole seconds since the newest run modification.
    pub minimum_age: NonZeroU64,
    /// Maximum desired logical bytes in sealed runs; zero is explicit.
    pub quota: u64,
}

/// Explicit retention operation; no mutation without a reviewed identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Apply only if this exact plan is still current.
    Apply(ContentHash),
    /// Preview without modifying storage.
    Plan,
}

/// A selected archived investigation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Removal {
    /// Logical bytes to be removed, not physical disk blocks.
    pub bytes: u64,
    /// Retained evidence path relative to the private games root.
    pub evidence: String,
    /// SHA-256 of the exact retained JSON Lines record, excluding LF.
    pub evidence_digest: ContentHash,
    /// The direct run directory name.
    pub name: String,
}

/// A complete bounded preview or receipt for the same applied plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    /// Logical bytes remaining after the planned removals.
    pub bytes_after: u64,
    /// Logical bytes in sealed runs before removal.
    pub bytes_before: u64,
    /// Identity binding policy, admitted inventory and selected removals.
    pub identity: ContentHash,
    /// Unsealed run names, never selected or traversed for removal.
    pub protected: Vec<String>,
    /// Old eligible runs selected in deterministic oldest-first order.
    pub removals: Vec<Removal>,
}

/// A positively classified run and its observed metadata identity.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Identity of the observed run and verified seal.
    pub fingerprint: ContentHash,
    /// Latest observed modification time in epoch seconds.
    pub newest: u64,
    /// The possible removal receipt.
    pub removal: Removal,
}

/// Bounded inventory produced by a native admission adapter.
#[derive(Debug)]
pub struct Inventory {
    /// Positively classified sealed archives.
    pub candidates: Vec<Candidate>,
    /// Names of unsealed and protected runs.
    pub protected: Vec<String>,
    /// Observed identity of the fixed archive namespace.
    pub root: ContentHash,
}

pub(crate) trait RetentionBackend {
    fn retain(
        policy: Policy,
        operation: Operation,
    ) -> Result<Plan, ToolchainError>;
}

/// Plan or explicitly apply bounded retention in the fixed private namespace.
/// # Errors
/// Reports unsupported hosts, unsafe inventory, exceeded bounds, stale plan
/// identities or native failures. An interrupted apply may remove only a
/// subset of its reviewed entries; inspect storage again before retrying.
pub fn retain(
    policy: Policy,
    operation: Operation,
) -> Result<Plan, ToolchainError> {
    <() as RetentionBackend>::retain(policy, operation)
}

/// Select old archives above quota and bind a deterministic plan.
/// # Errors
/// Reports inventory arithmetic or identity construction failure.
pub fn select(
    policy: Policy,
    now: u64,
    inventory: Inventory,
) -> Result<Plan, ToolchainError> {
    let Inventory {
        mut candidates,
        mut protected,
        root,
    } = inventory;
    candidates.sort_by(|left, right| {
        (left.newest, &left.removal.name)
            .cmp(&(right.newest, &right.removal.name))
    });
    protected.sort_unstable();
    let bytes_before = candidates.iter().try_fold(0u64, |sum, run| {
        sum.checked_add(run.removal.bytes)
            .ok_or_else(invalid_inventory)
    })?;
    let mut bytes_after = bytes_before;
    let mut removals = Vec::new();
    let mut binding = format!(
        "retention-v1 {root} {} {} {} {}\n",
        policy.minimum_age,
        policy.quota,
        policy.maximum_entries,
        policy.maximum_bytes,
    );
    for name in &protected {
        writeln!(&mut binding, "protected {name}").map_err(format_error)?;
    }
    for run in candidates {
        let old = now
            .checked_sub(run.newest)
            .is_some_and(|age| age >= policy.minimum_age.get());
        let remove = old && bytes_after > policy.quota;
        writeln!(
            &mut binding,
            "{} {} {remove} {} {} {}",
            run.removal.name,
            run.removal.bytes,
            run.fingerprint,
            run.removal.evidence,
            run.removal.evidence_digest,
        )
        .map_err(format_error)?;
        if remove {
            bytes_after = bytes_after
                .checked_sub(run.removal.bytes)
                .ok_or_else(invalid_inventory)?;
            removals.push(run.removal);
        }
    }
    Ok(Plan {
        bytes_after,
        bytes_before,
        identity: ContentHash::calculate(binding.as_bytes()),
        protected,
        removals,
    })
}

fn invalid_inventory() -> ToolchainError {
    ToolchainError::Failed {
        operation: "research retention",
        diagnostic: String::from("inventory size overflow"),
    }
}

fn format_error(error: FormatError) -> ToolchainError {
    ToolchainError::Failed {
        operation: "research retention",
        diagnostic: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(quota: u64) -> Result<Policy, &'static str> {
        Ok(Policy {
            maximum_bytes: NonZeroU64::new(1000).ok_or("bytes")?,
            maximum_entries: NonZeroUsize::new(20).ok_or("entries")?,
            minimum_age: NonZeroU64::new(10).ok_or("age")?,
            quota,
        })
    }

    fn candidate(name: &str, newest: u64) -> Candidate {
        Candidate {
            fingerprint: ContentHash::calculate(name.as_bytes()),
            newest,
            removal: Removal {
                bytes: 10,
                evidence: String::from("title/ledger.jsonl"),
                evidence_digest: ContentHash::calculate(b"record"),
                name: name.to_owned(),
            },
        }
    }

    fn require(condition: bool, message: &str) -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(message.to_owned())
        }
    }

    #[test]
    fn quota_and_age_are_both_required() -> Result<(), String> {
        let root = ContentHash::calculate(b"root");
        let plan = select(policy(5)?, 20, Inventory {
            candidates: vec![
                candidate("fresh", 19),
                candidate("old-b", 1),
                candidate("old-a", 1),
            ],
            protected: vec![String::from("active")],
            root,
        })
        .map_err(|error| error.to_string())?;
        let names: Vec<_> =
            plan.removals.iter().map(|run| run.name.as_str()).collect();
        require(
            names == ["old-a", "old-b"]
                && plan.bytes_before == 30
                && plan.bytes_after == 10
                && plan.protected == ["active"],
            "oldest-first selection or fresh protection failed",
        )?;
        let within = select(policy(10)?, 20, Inventory {
            candidates: vec![candidate("old-a", 1)],
            protected: Vec::new(),
            root,
        })
        .map_err(|error| error.to_string())?;
        require(within.removals.is_empty(), "age alone must not remove")
    }

    #[test]
    fn plan_identity_binds_policy_and_selection() -> Result<(), String> {
        let root = ContentHash::calculate(b"root");
        let build = |quota, now| {
            select(policy(quota)?, now, Inventory {
                candidates: vec![candidate("run", 2)],
                protected: Vec::new(),
                root,
            })
            .map_err(|error| error.to_string())
        };
        let old = build(1, 12)?;
        require(
            old.identity == build(1, 13)?.identity
                && old.identity != build(1, 11)?.identity
                && old.identity != build(2, 12)?.identity,
            "plan must bind policy and age eligibility",
        )
    }
}
