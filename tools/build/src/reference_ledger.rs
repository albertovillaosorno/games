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
//   - Read-only function ledger identity and graph consistency checks.
// - Must-Not:
//   - Infer semantics, claim parity or alter private research records.
// - Allows:
//   - Inputs: A local JSON Lines ledger and explicit byte and row bounds.
//   - Outputs: Aggregate function, status and graph counts.
//   - Side effects: Bounded file admission and aggregate output only.
// - Split-When:
//   - Another ledger format requires independent schema admission.
// - Merge-When:
//   - Another contributor tool duplicates the ledger consistency check.
// - Summary:
//   - Distinguish discovered, unresolved and resolved function records.
// - Description:
//   - Check stable function identities and reciprocal graph references.
// - Usage:
//   - Supply a private ledger and positive resource limits.
// - Defaults:
//   - Reject malformed, duplicate or dangling function records.

//! Bounded private reference ledger consistency checks.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::{NonZeroU64, NonZeroUsize};
use std::str::from_utf8;

use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain::write_line;
use serde::Deserialize;
use serde_json::Value;

use crate::{USAGE, parse_limit, text};

/// Read-only function inventory check.
#[derive(Debug)]
pub(crate) struct Arguments {
    byte_limit: NonZeroU64,
    path: OsString,
    row_limit: NonZeroUsize,
}

impl Arguments {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [path, bytes, rows] = <[OsString; 3]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        Ok(Self {
            byte_limit: parse_limit(&bytes)?,
            path,
            row_limit: text(&rows)?
                .parse()
                .map_err(|_error| String::from("invalid ledger row limit"))?,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let bytes = snapshot(&self.path, self.byte_limit)
            .map_err(|error| format!("ledger snapshot: {error:?}"))?;
        let input = from_utf8(&bytes)
            .map_err(|_error| String::from("ledger is not UTF-8"))?;
        let report = audit(input, self.row_limit)?;
        write_line(&format!(
            "Functions: {}; bytes: {}; pairs: {}; auxiliary: {}",
            report.functions, report.bytes, report.pairs, report.auxiliary,
        ))
        .map_err(|error| error.to_string())?;
        for (status, count) in report.statuses {
            write_line(&format!("{status}: {count}"))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Function {
    address_space: String,
    bank: String,
    behavior: Option<String>,
    body_bytes: NonZeroU64,
    callees: Vec<String>,
    callers: Vec<String>,
    confidence: String,
    cpu: String,
    edition: String,
    entry_address: String,
    id: String,
    module: String,
    original_name: String,
    rust_owner: Option<String>,
    rust_symbol: Option<String>,
    semantic_name: Option<String>,
    status: String,
    test_evidence: Option<Value>,
}

impl Function {
    fn validate(&self) -> Result<(), &'static str> {
        let address = self
            .entry_address
            .strip_prefix("0x")
            .ok_or("missing hexadecimal address prefix")?;
        if address.is_empty()
            || !address.bytes().all(|byte| {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            })
        {
            return Err("invalid entry address");
        }
        let components = [
            self.module.as_str(),
            self.cpu.as_str(),
            self.address_space.as_str(),
            self.bank.as_str(),
        ];
        if !components.iter().all(|part| identifier(part))
            || self.id
                != format!(
                    "{}:{}:{}:{}:{address}",
                    self.module,
                    self.cpu.to_ascii_lowercase(),
                    self.address_space,
                    self.bank,
                )
        {
            return Err("inconsistent stable function identity");
        }
        if self.edition.trim().is_empty()
            || self.original_name.trim().is_empty()
            || self.confidence.trim().is_empty()
        {
            return Err("missing function provenance");
        }
        self.validate_disposition()
    }

    fn validate_disposition(&self) -> Result<(), &'static str> {
        match self.status.as_str() {
            "discovered" | "unresolved" => return Ok(()),
            "implemented" | "shared-mechanism" | "intentional-replacement" => {
                if !nonempty(self.rust_owner.as_deref())
                    || !nonempty(self.rust_symbol.as_deref())
                {
                    return Err("resolved runtime function lacks Rust mapping");
                }
            },
            "non-runtime" => {},
            _ => return Err("unrecognized function disposition"),
        }
        if !nonempty(self.semantic_name.as_deref())
            || !nonempty(self.behavior.as_deref())
            || !self.test_evidence.as_ref().is_some_and(evidence)
        {
            return Err("resolved function lacks behavior or evidence");
        }
        Ok(())
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Report {
    auxiliary: usize,
    bytes: u64,
    functions: usize,
    pairs: usize,
    statuses: BTreeMap<String, usize>,
}

fn audit(input: &str, limit: NonZeroUsize) -> Result<Report, String> {
    let mut functions = BTreeMap::new();
    let mut report = Report::default();
    for (index, line) in input.lines().enumerate() {
        if index >= limit.get() {
            return Err(String::from("ledger row budget exceeded"));
        }
        let failure = |message: &str| {
            format!("ledger line {}: {message}", index.saturating_add(1))
        };
        let value: Value = serde_json::from_str(line)
            .map_err(|_error| failure("invalid JSON record"))?;
        let record = value
            .as_object()
            .ok_or_else(|| failure("expected object record"))?;
        if !["id", "module", "cpu", "entry_address"]
            .iter()
            .any(|key| record.contains_key(*key))
        {
            if !record.contains_key("evidence")
                || !record.contains_key("reproduce")
            {
                return Err(failure("unrecognized auxiliary record"));
            }
            report.auxiliary = report.auxiliary.saturating_add(1);
            continue;
        }
        let function: Function = serde_json::from_str(line)
            .map_err(|_error| failure("invalid function record"))?;
        function.validate().map_err(failure)?;
        report.bytes = report
            .bytes
            .checked_add(function.body_bytes.get())
            .ok_or("owned byte count overflow")?;
        let count = report.statuses.entry(function.status.clone()).or_default();
        *count = count.saturating_add(1);
        if functions.insert(function.id.clone(), function).is_some() {
            return Err(failure("duplicate function identity"));
        }
    }
    if functions.is_empty() {
        return Err(String::from("ledger contains no functions"));
    }
    report.functions = functions.len();
    report.pairs = graph(&functions)?;
    Ok(report)
}

fn evidence(value: &Value) -> bool {
    match value {
        Value::String(item) => !item.trim().is_empty(),
        Value::Array(items) => {
            !items.is_empty()
                && items.iter().all(|item| {
                    item.as_str().is_some_and(|entry| !entry.trim().is_empty())
                })
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            false
        },
    }
}

fn graph(functions: &BTreeMap<String, Function>) -> Result<usize, String> {
    let mut inbound = BTreeSet::new();
    let mut outbound = BTreeSet::new();
    for (id, function) in functions {
        for caller in &function.callers {
            if !functions.contains_key(caller)
                || !inbound.insert((caller.as_str(), id.as_str()))
            {
                return Err(String::from("dangling or duplicate caller"));
            }
        }
        for callee in &function.callees {
            if !functions.contains_key(callee)
                || !outbound.insert((id.as_str(), callee.as_str()))
            {
                return Err(String::from("dangling or duplicate callee"));
            }
        }
    }
    if inbound != outbound {
        return Err(String::from("nonreciprocal function graph"));
    }
    Ok(outbound.len())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
        })
}

fn nonempty(value: Option<&str>) -> bool {
    value.is_some_and(|item| !item.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::slice::from_ref;

    use super::*;

    fn record(address: &str, inbound: &[&str], outbound: &[&str]) -> Value {
        serde_json::json!({
            "address_space": "program",
            "bank": "fixed",
            "body_bytes": 3,
            "callees": outbound,
            "callers": inbound,
            "confidence": "fixture",
            "cpu": "Z80",
            "edition": "fixture",
            "entry_address": format!("0x{address}"),
            "id": format!("audio:z80:program:fixed:{address}"),
            "module": "audio",
            "original_name": "synthetic",
            "status": "discovered",
        })
    }

    fn pair() -> [Value; 2] {
        [
            record("0001", &[], &["audio:z80:program:fixed:0002"]),
            record("0002", &["audio:z80:program:fixed:0001"], &[]),
        ]
    }

    fn lines(records: &[Value]) -> String {
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn counts_reciprocal_graph_and_auxiliary_record() -> Result<(), String> {
        let [first, second] = pair();
        let input = lines(&[
            first,
            second,
            serde_json::json!({"evidence": "fixture", "reproduce": "fixture"}),
        ]);
        let limit = NonZeroUsize::new(3).ok_or("fixture limit")?;
        let report = audit(&input, limit)?;
        if report.functions != 2
            || report.bytes != 6
            || report.pairs != 1
            || report.auxiliary != 1
            || report.statuses.get("discovered") != Some(&2)
            || audit(&input, NonZeroUsize::MIN).is_ok()
        {
            return Err(String::from("ledger counts or row budget differ"));
        }
        Ok(())
    }

    #[test]
    fn rejects_unsubstantiated_completion() -> Result<(), String> {
        let mut fixture = record("0001", &[], &[]);
        let row = fixture.as_object_mut().ok_or("fixture object")?;
        let _status = row.insert("status".into(), Value::from("implemented"));
        let _semantic =
            row.insert("semantic_name".into(), Value::from("fixture"));
        let _behavior = row.insert("behavior".into(), Value::from("fixture"));
        let _evidence =
            row.insert("test_evidence".into(), serde_json::json!(["fixture"]));
        if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted unmapped implementation"));
        }
        let mapped = fixture.as_object_mut().ok_or("fixture object")?;
        let _owner =
            mapped.insert("rust_owner".into(), Value::from("fixture.rs"));
        let _symbol =
            mapped.insert("rust_symbol".into(), Value::from("fixture"));
        let report = audit(&fixture.to_string(), NonZeroUsize::MIN)?;
        if report.statuses.get("implemented") != Some(&1) {
            return Err(String::from("missing implemented count"));
        }
        let duplicate = fixture.to_string().replacen(
            "\"status\":",
            "\"status\":\"discovered\",\"status\":",
            1,
        );
        if audit(&duplicate, NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted duplicate status key"));
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_identity_and_graph() -> Result<(), String> {
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        let [first, second] = pair();
        if audit(&lines(from_ref(&first)), limit).is_ok() {
            return Err(String::from("accepted dangling graph"));
        }
        if audit(&lines(&[first.clone(), first.clone()]), limit).is_ok() {
            return Err(String::from("accepted duplicate identity"));
        }
        let unmatched = record("0002", &[], &[]);
        if audit(&lines(&[first.clone(), unmatched]), limit).is_ok() {
            return Err(String::from("accepted nonreciprocal graph"));
        }
        let mut invalid = second;
        let object = invalid.as_object_mut().ok_or("fixture object")?;
        let _previous = object.insert("id".into(), Value::from("incorrect"));
        if audit(&lines(&[first, invalid]), limit).is_ok() {
            return Err(String::from("accepted inconsistent identity"));
        }
        if audit("{}", limit).is_ok() {
            return Err(String::from("accepted unrecognized record"));
        }
        Ok(())
    }
}
