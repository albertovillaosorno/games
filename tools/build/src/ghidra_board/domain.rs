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
// - Owns: Finite byte-register selector evaluation on decoded paths.
// - Must-Not: Assume an unknown memory value or remove ledger records.
// - Allows:
//   - Inputs: Verified fallthrough chains and selector register identity.
//   - Outputs: Conservative signed byte-offset domains.
//   - Side effects: None.
// - Split-When: Register-pair or memory analysis needs distinct semantics.
// - Merge-When: A shared abstract execution IR owns the same analysis.
// - Summary: Evaluate byte instructions and branch guards exhaustively.
// - Description: Unsupported effects widen the domain, never narrow it.
// - Usage: Internal computed dispatch analysis.
// - Defaults: All 256 input values; unknown comparisons retain every path.

//! Conservative finite byte evaluation of a known fallthrough path.

use std::collections::BTreeSet;

use super::bounds::{immediate, signed};

#[derive(Clone, Copy)]
enum Effect {
    Add(u8),
    And(u8),
    Compare(u8),
    Load(u8),
    Or(u8),
    Shift,
    Subtract(u8),
    Xor(u8),
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct State {
    comparison: Option<(u8, u8)>,
    value: u8,
}

fn all() -> BTreeSet<State> {
    (0..=u8::MAX)
        .map(|value| State { comparison: None, value })
        .collect()
}

fn test_branch(operation: &str, comparison: (u8, u8)) -> Option<bool> {
    let (left, right) = comparison;
    let result = match operation {
        "BEQ" => left == right,
        "BNE" => left != right,
        "BCS" => left < right,
        "BCC" => left >= right,
        "BHI" => left > right,
        "BLS" => left <= right,
        "BLT" => signed(left) < signed(right),
        "BGE" => signed(left) >= signed(right),
        "BGT" => signed(left) > signed(right),
        "BLE" => signed(left) <= signed(right),
        _ => return None,
    };
    Some(result)
}

fn branch(states: &mut BTreeSet<State>, operation: &str) {
    let short = operation.strip_prefix('L').unwrap_or(operation);
    states.retain(|state| {
        !state
            .comparison
            .and_then(|cmp| test_branch(short, cmp))
            .unwrap_or(false)
    });
}

const fn apply(state: State, effect: Effect) -> State {
    let old = state.value;
    let (value, comparison) = match effect {
        Effect::Add(rhs) => (old.wrapping_add(rhs), None),
        Effect::And(rhs) => (old & rhs, None),
        Effect::Compare(rhs) => (old, Some((old, rhs))),
        Effect::Load(rhs) => (rhs, None),
        Effect::Or(rhs) => (old | rhs, None),
        Effect::Shift => (old.wrapping_mul(2), None),
        Effect::Subtract(rhs) => (old.wrapping_sub(rhs), Some((old, rhs))),
        Effect::Xor(rhs) => (old ^ rhs, None),
    };
    State { comparison, value }
}

fn effect(line: &str, reg: &str) -> Option<Effect> {
    if line == format!("CLR{reg}") {
        return Some(Effect::Load(0));
    }
    if line == format!("ASL{reg}") {
        return Some(Effect::Shift);
    }
    let (operation, _) = line.strip_prefix("IMM ")?.split_once(' ')?;
    let instruction = operation.strip_suffix(reg)?;
    let rhs = u8::try_from(immediate(line, operation)?).ok()?;
    match instruction {
        "ADD" => Some(Effect::Add(rhs)),
        "AND" => Some(Effect::And(rhs)),
        "CMP" => Some(Effect::Compare(rhs)),
        "LD" => Some(Effect::Load(rhs)),
        "OR" => Some(Effect::Or(rhs)),
        "SUB" => Some(Effect::Subtract(rhs)),
        "EOR" => Some(Effect::Xor(rhs)),
        _ => None,
    }
}

fn keep_values(states: BTreeSet<State>) -> BTreeSet<State> {
    states
        .into_iter()
        .map(|state| State {
            comparison: None,
            value: state.value,
        })
        .collect()
}

fn is_guard(line: &str) -> bool {
    let op = line.split_once(' ').map_or(line, |(op, _)| op);
    test_branch(op.strip_prefix('L').unwrap_or(op), (0, 0)).is_some()
}

fn step(states: BTreeSet<State>, line: &str, reg: &str) -> BTreeSet<State> {
    if states.is_empty() {
        return states;
    }
    if let Some(transfer) = effect(line, reg) {
        return states
            .into_iter()
            .map(|state| apply(state, transfer))
            .collect();
    }
    if is_guard(line) {
        let mut next = states;
        branch(&mut next, line.split_once(' ').map_or(line, |(op, _)| op));
        return next;
    }
    // These operations may affect flags, but not the selector byte.
    if ["LDX", "LDY", "LDU", "LDS"]
        .iter()
        .any(|op| immediate(line, op).is_some())
        || line.starts_with("JOIN ")
    {
        return keep_values(states);
    }
    all()
}

pub(super) fn evaluate(chain: &[String], reg: &str) -> BTreeSet<i16> {
    if !matches!(reg, "A" | "B") {
        return BTreeSet::new();
    }
    chain
        .iter()
        .fold(all(), |states, line| step(states, line, reg))
        .into_iter()
        .map(|state| signed(state.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[&str]) -> Vec<String> {
        input.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn guard_and_wrapping_arithmetic() -> Result<(), String> {
        let chain = lines(&[
            "IMM CMPA 0xc0",
            "BCS 0x6000",
            "IMM SUBA 0xc7",
            "BCC 0x6000",
            "IMM ADDA 0x7",
            "ASLA",
        ]);
        if evaluate(&chain, "A") != BTreeSet::from([0, 2, 4, 6, 8, 10, 12]) {
            return Err(String::from("incorrect unsigned guard domain"));
        }
        Ok(())
    }

    #[test]
    fn unsupported_operations_widen() -> Result<(), String> {
        let chain = lines(&["IMM LDB 0x1", "LDB 0x3", "ASLB"]);
        if evaluate(&chain, "B").len() != 128 {
            return Err(String::from("unknown memory was narrowed"));
        }
        let join = lines(&["IMM CMPA 0x1", "JOIN BNE 0x6000", "ASLA"]);
        if evaluate(&join, "A").len() != 128 {
            return Err(String::from("branch join was narrowed"));
        }
        Ok(())
    }

    #[test]
    fn flags_clobbered_by_base_load() -> Result<(), String> {
        let chain =
            lines(&["IMM CMPB 0x7", "IMM LDX 0x7000", "BCC 0x6000", "ASLB"]);
        if evaluate(&chain, "B").len() != 128 {
            return Err(String::from("used stale comparison flags"));
        }
        Ok(())
    }
}
