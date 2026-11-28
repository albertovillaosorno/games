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
//   - Read-only function ledger identity and flow consistency checks.
// - Must-Not:
//   - Infer semantics, claim parity or alter private research records.
// - Allows:
//   - Inputs: A local JSON Lines ledger and explicit byte and row bounds.
//   - Outputs: Aggregate function, status, graph and flow counts.
//   - Side effects: Bounded file admission and aggregate output only.
// - Split-When:
//   - Another ledger format requires independent schema admission.
// - Merge-When:
//   - Another contributor tool duplicates the ledger consistency check.
// - Summary:
//   - Score only verified implementations; distinguish pending and discarded.
// - Description:
//   - Check stable function identities and reciprocal graph references.
// - Usage:
//   - Supply a private ledger and positive resource limits.
// - Defaults:
//   - Reject malformed, duplicate or dangling function records.

//! Bounded private reference ledger consistency checks.

use std::cmp::Reverse;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::{NonZeroU64, NonZeroUsize};
use std::str::from_utf8;

use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain::reference::retention::ArchivedEvidence;
use games_platform::ir::toolchain::write_line;
use serde::Deserialize;
use serde_json::Value;

use crate::{USAGE, parse_limit, text};

/// Read-only function inventory check.
#[derive(Debug)]
pub(crate) struct Arguments {
    byte_limit: NonZeroU64,
    group_limit: Option<NonZeroUsize>,
    path: OsString,
    require_complete: bool,
    row_limit: NonZeroUsize,
}

impl Arguments {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut arguments = args.into_iter();
        let path = arguments.next().ok_or(USAGE)?;
        let bytes = arguments.next().ok_or(USAGE)?;
        let rows = arguments.next().ok_or(USAGE)?;
        let mut require_complete = false;
        let mut group_limit = None;
        for value in arguments {
            if value == "--complete" && !require_complete {
                require_complete = true;
            } else if group_limit.is_none() && !require_complete {
                group_limit =
                    Some(text(&value)?.parse::<NonZeroUsize>().map_err(
                        |_error| String::from("invalid ledger group limit"),
                    )?);
            } else {
                return Err(String::from(USAGE));
            }
        }
        Ok(Self {
            byte_limit: parse_limit(&bytes)?,
            group_limit,
            path,
            require_complete,
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
        let report = audit(input, self.row_limit, self.group_limit)?;
        write_line(&format!(
            "Functions: {}; bytes: {}; pairs: {}; auxiliary: {}",
            report.functions, report.bytes, report.pairs, report.auxiliary,
        ))
        .map_err(|error| error.to_string())?;
        write_line(&format!(
            "Flow records: {}; interior entries: {}; unresolved: {}",
            report.flows, report.interior, report.unresolved,
        ))
        .map_err(|error| error.to_string())?;
        write_line(&format!(
            "Listing reviews: {}; reviewed dependencies: {}; pending: {}",
            report.semantic_reviews,
            report.reviewed_dependencies,
            report
                .functions
                .saturating_sub(report.reviewed_dependencies),
        ))
        .map_err(|error| error.to_string())?;
        write_line(&format!(
            "Implementation 1: {}; implementation 0: {}",
            report.implemented,
            report.functions.saturating_sub(report.implemented),
        ))
        .map_err(|error| error.to_string())?;
        write_line(&format!(
            "Semantic complete: {}; ledger closed: {}",
            report.semantic_complete,
            report.closed(),
        ))
        .map_err(|error| error.to_string())?;
        for (status, count) in &report.statuses {
            write_line(&format!("{status}: {count}"))
                .map_err(|error| error.to_string())?;
        }
        if self.require_complete && !report.closed() {
            return Err(String::from("ledger acceptance incomplete"));
        }
        if let Some(groups) = report.groups {
            groups.write()?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Function {
    address_space: String,
    analysis_state: Option<String>,
    bank: String,
    behavior: Option<String>,
    body_bytes: NonZeroU64,
    callees: Vec<String>,
    callers: Vec<String>,
    confidence: String,
    cpu: String,
    data_dependencies: Option<Vec<String>>,
    edition: String,
    entry_address: String,
    flow_edges: Vec<FlowEdge>,
    id: String,
    inputs: Option<Vec<String>>,
    module: String,
    original_name: String,
    outputs: Option<Vec<String>>,
    rust_owner: Option<String>,
    rust_symbol: Option<String>,
    semantic_evidence: Option<String>,
    semantic_name: Option<String>,
    state_mutations: Option<Vec<String>>,
    status: String,
    test_evidence: Option<Value>,
    #[serde(default)]
    unresolved_flows: Vec<UnresolvedFlow>,
}

#[derive(Debug, Deserialize)]
struct FlowEdge {
    evidence: BTreeMap<String, Value>,
    interior_entry: bool,
    kind: String,
    site: String,
    target: String,
    target_address: String,
}

impl FlowEdge {
    fn validate(&self, target: &Function) -> Result<(), &'static str> {
        if address(&self.site).is_none()
            || address(&self.target_address).is_none()
        {
            return Err("invalid flow address");
        }
        if self.kind.trim().is_empty()
            || !nonempty(self.evidence.get("kind").and_then(Value::as_str))
        {
            return Err("missing flow provenance");
        }
        if self.interior_entry
            == (address(&self.target_address) == address(&target.entry_address))
        {
            return Err("inconsistent interior entry");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct UnresolvedFlow {
    kind: String,
    site: String,
    target: Option<String>,
}

impl UnresolvedFlow {
    fn validate(&self) -> Result<(), &'static str> {
        if address(&self.site).is_none()
            || self
                .target
                .as_deref()
                .is_some_and(|value| address(value).is_none())
            || self.kind.trim().is_empty()
        {
            return Err("invalid unresolved flow");
        }
        Ok(())
    }
}

impl Function {
    fn audit_flows(
        &self,
        functions: &BTreeMap<String, Self>,
        report: &mut Report,
    ) -> Result<(), String> {
        let failure =
            |message: &str| format!("function {}: {message}", self.id);
        let mut targets = BTreeSet::new();
        let mut records = BTreeSet::new();
        for flow in &self.flow_edges {
            let target = functions
                .get(&flow.target)
                .ok_or_else(|| failure("dangling detailed flow target"))?;
            flow.validate(target).map_err(failure)?;
            let evidence = serde_json::to_string(&flow.evidence)
                .map_err(|_error| failure("invalid flow provenance"))?;
            if !records.insert((
                address(&flow.site),
                &flow.kind,
                &flow.target,
                address(&flow.target_address),
                evidence,
            )) {
                return Err(failure("duplicate detailed flow record"));
            }
            let _inserted = targets.insert(flow.target.as_str());
            report.interior = report
                .interior
                .saturating_add(usize::from(flow.interior_entry));
        }
        if targets != self.callees.iter().map(String::as_str).collect() {
            return Err(failure("detailed flow targets differ from callees"));
        }
        report.flows = report.flows.saturating_add(self.flow_edges.len());

        let mut unresolved = BTreeSet::new();
        for flow in &self.unresolved_flows {
            flow.validate().map_err(failure)?;
            if !unresolved.insert((
                address(&flow.site),
                &flow.kind,
                flow.target.as_deref().and_then(address),
            )) {
                return Err(failure("duplicate unresolved flow record"));
            }
        }
        report.unresolved = report.unresolved.saturating_add(unresolved.len());
        Ok(())
    }

    fn validate(&self) -> Result<(), &'static str> {
        let address = self
            .entry_address
            .strip_prefix("0x")
            .ok_or("missing hexadecimal address prefix")?;
        if !hexadecimal(address) {
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
        self.validate_semantic_review()?;
        self.validate_disposition()?;
        self.validate_mapping()
    }

    fn validate_disposition(&self) -> Result<(), &'static str> {
        match self.status.as_str() {
            "pending" => return Ok(()),
            "implemented" | "discarded" => {},
            _ => return Err("unrecognized function disposition"),
        }
        if self.analysis_state.as_deref() != Some("semantic-complete") {
            return Err("resolved function lacks complete semantics");
        }
        if !self.unresolved_flows.is_empty() {
            return Err("resolved function has unresolved control flow");
        }
        if !review_list(self.data_dependencies.as_deref()) {
            return Err("resolved function lacks reviewed data dependencies");
        }
        if !self.test_evidence.as_ref().is_some_and(evidence) {
            return Err("resolved function lacks test evidence");
        }
        Ok(())
    }

    fn validate_mapping(&self) -> Result<(), &'static str> {
        if self.status != "implemented" {
            if self.rust_owner.is_some() || self.rust_symbol.is_some() {
                return Err("only implemented functions may have mappings");
            }
            return Ok(());
        }
        if !nonempty(self.rust_owner.as_deref())
            || !nonempty(self.rust_symbol.as_deref())
            || !self.test_evidence.as_ref().is_some_and(evidence)
        {
            return Err("Rust mapping lacks owner, symbol or test evidence");
        }
        Ok(())
    }

    fn validate_semantic_review(&self) -> Result<(), &'static str> {
        if !matches!(
            self.analysis_state.as_deref(),
            Some("listing-reviewed" | "semantic-complete")
        ) {
            return Ok(());
        }
        if self.analysis_state.as_deref() == Some("semantic-complete")
            && !review_list(self.data_dependencies.as_deref())
        {
            return Err("complete semantics lack reviewed dependencies");
        }
        if !nonempty(self.semantic_name.as_deref())
            || !nonempty(self.behavior.as_deref())
            || !nonempty(self.semantic_evidence.as_deref())
            || !review_list(self.inputs.as_deref())
            || !review_list(self.outputs.as_deref())
            || !review_list(self.state_mutations.as_deref())
            || self
                .data_dependencies
                .as_deref()
                .is_some_and(|entries| !review_list(Some(entries)))
        {
            return Err("incomplete semantic review");
        }
        Ok(())
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Report {
    auxiliary: usize,
    bytes: u64,
    flows: usize,
    functions: usize,
    groups: Option<GroupReport>,
    implemented: usize,
    interior: usize,
    pairs: usize,
    reviewed_dependencies: usize,
    semantic_complete: usize,
    semantic_reviews: usize,
    statuses: BTreeMap<String, usize>,
    unresolved: usize,
}

impl Report {
    fn closed(&self) -> bool {
        self.functions == self.semantic_complete
            && self.functions == self.reviewed_dependencies
            && self.statuses.get("pending").copied().unwrap_or_default() == 0
            && self.unresolved == 0
    }

    fn record_function(
        &mut self,
        function: &Function,
    ) -> Result<(), &'static str> {
        self.bytes = self
            .bytes
            .checked_add(function.body_bytes.get())
            .ok_or("owned byte count overflow")?;
        if matches!(
            function.analysis_state.as_deref(),
            Some("listing-reviewed" | "semantic-complete")
        ) {
            self.semantic_reviews = self.semantic_reviews.saturating_add(1);
            if function.data_dependencies.is_some() {
                self.reviewed_dependencies =
                    self.reviewed_dependencies.saturating_add(1);
            }
        }
        if function.analysis_state.as_deref() == Some("semantic-complete") {
            self.semantic_complete = self.semantic_complete.saturating_add(1);
        }
        if function.status == "implemented" {
            self.implemented = self.implemented.saturating_add(1);
        }
        let count = self.statuses.entry(function.status.clone()).or_default();
        *count = count.saturating_add(1);
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Group {
    callers: BTreeSet<usize>,
    dependencies: BTreeSet<usize>,
    members: Vec<String>,

    unknown_flows: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct GroupReport {
    ready: usize,
    selected: Vec<Group>,
    total: usize,
}

impl GroupReport {
    fn write(self) -> Result<(), String> {
        write_line(&format!(
            "Pending dependency groups: {}; graph-ready: {}; shown: {}",
            self.total,
            self.ready,
            self.selected.len(),
        ))
        .map_err(|error| error.to_string())?;
        for group in self.selected {
            write_line(&format!(
                concat!(
                    "Group: members={}; dependencies={}; callers={}; ",
                    "unknown-flows={}; ids={}"
                ),
                group.members.len(),
                group.dependencies.len(),
                group.callers.len(),
                group.unknown_flows,
                group.members.join(","),
            ))
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn audit(
    input: &str,
    limit: NonZeroUsize,
    group_limit: Option<NonZeroUsize>,
) -> Result<Report, String> {
    let mut functions = BTreeMap::new();
    let mut archived_runs = BTreeSet::new();
    let mut selected_edition = None;
    let mut report = Report::default();
    for (index, line) in input.lines().enumerate() {
        if index >= limit.get() {
            return Err(String::from("ledger row budget exceeded"));
        }
        let failure = |message: &str| {
            format!("ledger line {}: {message}", index.saturating_add(1))
        };
        if register_archived(line, &mut archived_runs).map_err(failure)? {
            report.auxiliary = report.auxiliary.saturating_add(1);
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|_error| failure("invalid JSON record"))?;
        let record = value
            .as_object()
            .ok_or_else(|| failure("expected object record"))?;
        if !["id", "module", "cpu", "entry_address"]
            .iter()
            .any(|key| record.contains_key(*key))
        {
            if !value.get("evidence").is_some_and(evidence)
                || !value.get("reproduce").is_some_and(evidence)
            {
                return Err(failure("unrecognized auxiliary record"));
            }
            report.auxiliary = report.auxiliary.saturating_add(1);
            continue;
        }
        let function: Function = serde_json::from_str(line)
            .map_err(|_error| failure("invalid function record"))?;
        function.validate().map_err(failure)?;
        let edition =
            selected_edition.get_or_insert_with(|| function.edition.clone());
        if edition.as_str() != function.edition.as_str() {
            return Err(failure("mixed selected editions"));
        }
        report.record_function(&function)?;
        if functions.insert(function.id.clone(), function).is_some() {
            return Err(failure("duplicate function identity"));
        }
    }
    if functions.is_empty() {
        return Err(String::from("ledger contains no functions"));
    }
    report.functions = functions.len();
    graph(&functions, &mut report)?;
    if let Some(maximum) = group_limit {
        report.groups = Some(dependency_groups(&functions, maximum));
    }
    Ok(report)
}

fn pending(function: &Function) -> bool {
    function.status == "pending"
}

// Iterative postorder avoids stack overflow on deep private call graphs.
fn finish_order(functions: &BTreeMap<String, Function>) -> Vec<&str> {
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    for function in functions.values().filter(|function| pending(function)) {
        let mut stack = vec![(function.id.as_str(), false)];
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            if let Some(node) = functions.get(id).filter(|node| pending(node))
                && visited.insert(id)
            {
                stack.push((id, true));
                stack.extend(
                    node.callees.iter().map(|callee| (callee.as_str(), false)),
                );
            }
        }
    }
    order
}

fn components(functions: &BTreeMap<String, Function>) -> BTreeMap<&str, usize> {
    let mut owners = BTreeMap::new();
    let mut next = 0usize;
    for seed in finish_order(functions).into_iter().rev() {
        if owners.contains_key(seed) {
            continue;
        }
        let mut stack = vec![seed];
        while let Some(id) = stack.pop() {
            if let Some(node) = functions.get(id).filter(|node| pending(node))
                && let Entry::Vacant(entry) = owners.entry(id)
            {
                let _owner = entry.insert(next);
                stack.extend(node.callers.iter().map(String::as_str));
            }
        }
        next = next.saturating_add(1);
    }
    owners
}

fn dependency_groups(
    functions: &BTreeMap<String, Function>,
    maximum: NonZeroUsize,
) -> GroupReport {
    let owners = components(functions);
    let mut groups = BTreeMap::new();
    for (id, owner) in &owners {
        let Some(function) = functions.get(*id) else {
            continue;
        };
        let group = groups.entry(*owner).or_insert_with(|| Group {
            callers: BTreeSet::new(),
            dependencies: BTreeSet::new(),
            members: Vec::new(),

            unknown_flows: 0,
        });
        group.members.push((*id).to_owned());
        group.unknown_flows = group
            .unknown_flows
            .saturating_add(function.unresolved_flows.len());
        group.dependencies.extend(
            function
                .callees
                .iter()
                .filter_map(|callee| owners.get(callee.as_str()))
                .copied()
                .filter(|target| target != owner),
        );
        group.callers.extend(
            function
                .callers
                .iter()
                .filter_map(|caller| owners.get(caller.as_str()))
                .copied()
                .filter(|source| source != owner),
        );
    }
    let mut selected: Vec<_> = groups.into_values().collect();
    let total = selected.len();
    let ready = selected
        .iter()
        .filter(|group| group.dependencies.is_empty())
        .count();
    selected.sort_by(|left, right| {
        (
            left.dependencies.len(),
            left.unknown_flows,
            Reverse(left.members.len()),
            Reverse(left.callers.len()),
            &left.members,
        )
            .cmp(&(
                right.dependencies.len(),
                right.unknown_flows,
                Reverse(right.members.len()),
                Reverse(right.callers.len()),
                &right.members,
            ))
    });
    selected.truncate(maximum.get());
    GroupReport { ready, selected, total }
}

fn register_archived(
    line: &str,
    archived_runs: &mut BTreeSet<String>,
) -> Result<bool, &'static str> {
    let Some(retained) = ArchivedEvidence::classify(line.as_bytes())
        .map_err(|_error| "invalid ledger archive classification")?
    else {
        return Ok(false);
    };
    if !archived_runs.insert(retained.run().to_owned()) {
        return Err("duplicate archived investigation run");
    }
    Ok(true)
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

fn graph(
    functions: &BTreeMap<String, Function>,
    report: &mut Report,
) -> Result<(), String> {
    let mut locations = BTreeSet::new();
    let mut inbound = BTreeSet::new();
    let mut outbound = BTreeSet::new();
    for (id, function) in functions {
        let (context, _) =
            id.rsplit_once(':').ok_or("invalid function identity")?;
        let entry =
            address(&function.entry_address).ok_or("invalid entry address")?;
        if !locations.insert((context, entry)) {
            return Err(format!("duplicate numeric function identity: {id}"));
        }
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
    report.pairs = outbound.len();
    for function in functions.values() {
        function.audit_flows(functions, report)?;
    }
    Ok(())
}

fn address(value: &str) -> Option<&str> {
    let digits = value.strip_prefix("0x")?;
    hexadecimal(digits).then_some(digits.trim_start_matches('0'))
}

fn hexadecimal(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
        })
}

fn review_list(value: Option<&[String]>) -> bool {
    value.is_some_and(|items| items.iter().all(|item| !item.trim().is_empty()))
}

fn nonempty(value: Option<&str>) -> bool {
    value.is_some_and(|item| !item.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::slice::from_ref;

    use games_platform::ir::toolchain::reference::retention::{
        ARCHIVED_EVIDENCE_KIND, ARCHIVED_EVIDENCE_STATUS,
    };

    use super::*;

    fn audit(input: &str, limit: NonZeroUsize) -> Result<Report, String> {
        super::audit(input, limit, None)
    }

    fn flow(site: &str, target: &str, entry: &str) -> Value {
        serde_json::json!({
            "evidence": {"kind": "fixture"},
            "interior_entry": false,
            "kind": "direct_call",
            "site": site,
            "target": target,
            "target_address": entry,
        })
    }

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
            "flow_edges": outbound.iter().map(|target| {
                let entry = target.rsplit(':').next().unwrap_or_default();
                flow(&format!("0x{address}"), target, &format!("0x{entry}"))
            }).collect::<Vec<_>>(),
            "entry_address": format!("0x{address}"),
            "id": format!("audio:z80:program:fixed:{address}"),
            "module": "audio",
            "original_name": "synthetic",
            "status": "pending",
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
            || report.flows != 1
            || report.interior != 0
            || report.unresolved != 0
            || report.auxiliary != 1
            || report.semantic_reviews != 0
            || report.reviewed_dependencies != 0
            || report.statuses.get("pending") != Some(&2)
            || audit(&input, NonZeroUsize::MIN).is_ok()
        {
            return Err(String::from("ledger counts or row budget differ"));
        }
        Ok(())
    }

    #[test]
    fn groups_cycles_and_prioritises_external_dependencies()
    -> Result<(), String> {
        let first_id = "audio:z80:program:fixed:0001";
        let second_id = "audio:z80:program:fixed:0002";
        let leaf_id = "audio:z80:program:fixed:0003";
        let records = [
            record("0001", &[second_id], &[second_id, leaf_id]),
            record("0002", &[first_id], &[first_id, leaf_id]),
            record("0003", &[first_id, second_id], &[]),
        ];
        let limit = NonZeroUsize::new(3).ok_or("fixture limit")?;
        let report = super::audit(&lines(&records), limit, Some(limit))?;
        let groups = report.groups.ok_or("missing groups")?;
        let [leaf, cycle] = groups.selected.as_slice() else {
            return Err(String::from("unexpected component count"));
        };
        if groups.total != 2
            || groups.ready != 1
            || leaf.members != [leaf_id]
            || !leaf.dependencies.is_empty()
            || leaf.callers.len() != 1
            || cycle.members != [first_id, second_id]
            || cycle.dependencies.len() != 1
        {
            return Err(String::from("incorrect dependency grouping"));
        }
        let [first, second, third] = records;
        let reordered =
            super::audit(&lines(&[third, second, first]), limit, Some(limit))?;
        if reordered.groups.as_ref() != Some(&groups) {
            return Err(String::from("group report is not deterministic"));
        }
        Ok(())
    }

    #[test]
    fn archive_evidence_requires_classification_and_unique_run()
    -> Result<(), String> {
        let [first, second] = pair();
        let archive = serde_json::json!({
            "kind": ARCHIVED_EVIDENCE_KIND,
            "run": "old-run",
            "status": ARCHIVED_EVIDENCE_STATUS,
            "evidence": "retained fixture",
            "reproduce": "fixture command and inputs"
        });
        let limit = NonZeroUsize::new(4).ok_or("fixture limit")?;
        let report = audit(
            &lines(&[first.clone(), second.clone(), archive.clone()]),
            limit,
        )?;
        if report.auxiliary != 1 || report.functions != 2 {
            return Err(String::from(
                "archive evidence changed function count",
            ));
        }
        let duplicate = lines(&[
            first.clone(),
            second.clone(),
            archive.clone(),
            archive.clone(),
        ]);
        if audit(&duplicate, limit).is_ok() {
            return Err(String::from("duplicate archived run accepted"));
        }
        let mut distinct = archive.clone();
        set_field(
            &mut distinct,
            "evidence",
            Value::from("different retained evidence"),
        )?;
        let conflict =
            lines(&[first.clone(), second.clone(), archive.clone(), distinct]);
        if audit(&conflict, limit).is_ok() {
            return Err(String::from("conflicting archived run accepted"));
        }
        for (field, value) in [
            ("kind", Value::from("other")),
            ("run", Value::from("../outside")),
            ("status", Value::from("active")),
            ("evidence", Value::from(" ")),
            ("reproduce", Value::from("command\nsecond line")),
        ] {
            let mut invalid = archive.clone();
            set_field(&mut invalid, field, value)?;
            let input = lines(&[first.clone(), second.clone(), invalid]);
            if audit(&input, limit).is_ok() {
                return Err(format!("invalid archive field accepted: {field}"));
            }
        }
        Ok(())
    }

    #[test]
    fn archive_classification_cannot_hide_or_repeat() -> Result<(), String> {
        let [first, second] = pair();
        let limit = NonZeroUsize::new(4).ok_or("fixture limit")?;
        let mut disguised = first.clone();
        set_field(&mut disguised, "kind", Value::from(ARCHIVED_EVIDENCE_KIND))?;
        if audit(&lines(&[disguised, second.clone()]), limit).is_ok() {
            return Err(String::from("archive declaration hid in function"));
        }
        let duplicate_kind = format!(
            "{}\n{{\"kind\":\"archived-investigation\",\"kind\":\"other\",\
             \"evidence\":\"fixture\",\"reproduce\":\"fixture\"}}",
            lines(&[first, second])
        );
        if audit(&duplicate_kind, limit).is_ok() {
            return Err(String::from("duplicate archive classification"));
        }
        Ok(())
    }

    fn set_field(
        value: &mut Value,
        field: &str,
        replacement: Value,
    ) -> Result<(), String> {
        let object = value.as_object_mut().ok_or("fixture object")?;
        let _previous = object.insert(field.to_owned(), replacement);
        Ok(())
    }

    fn edges(value: &mut Value) -> Result<&mut Vec<Value>, String> {
        value
            .get_mut("flow_edges")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| String::from("fixture edges"))
    }

    #[test]
    fn reconciles_aliased_targets_and_unresolved_observations()
    -> Result<(), String> {
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        let [mut first, second] = pair();
        let target = "audio:z80:program:fixed:0002";
        let mut alias = flow("0x1", target, "0x3");
        set_field(&mut alias, "interior_entry", Value::Bool(true))?;
        set_field(&mut alias, "kind", Value::from("possible_table_call"))?;
        let provenance = serde_json::json!({"kind": "table", "selector": 1u8});
        set_field(&mut alias, "evidence", provenance)?;
        let mut second_alias = alias.clone();
        let other = serde_json::json!({"kind": "table", "selector": 2u8});
        set_field(&mut second_alias, "evidence", other)?;
        edges(&mut first)?.extend([alias, second_alias]);
        let unresolved = serde_json::json!([
            {"kind": "alternate_context", "site": "0x0001"},
            {
                "kind": "unmapped_target",
                "site": "0x0001",
                "target": "0x0004"
            }
        ]);
        set_field(&mut first, "unresolved_flows", unresolved)?;
        let report = audit(&lines(&[first, second]), limit)?;
        if report.pairs != 1
            || report.flows != 3
            || report.interior != 2
            || report.unresolved != 2
        {
            return Err(String::from("detailed flow counts differ"));
        }
        Ok(())
    }

    #[test]
    fn rejects_missing_conflicting_and_duplicate_flow_records()
    -> Result<(), String> {
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        let [first, second] = pair();
        let mut missing = first.clone();
        let _removed = missing
            .as_object_mut()
            .ok_or("fixture")?
            .remove("flow_edges");
        let mut empty = first.clone();
        set_field(&mut empty, "flow_edges", serde_json::json!([]))?;
        let mut duplicate = first.clone();
        let original = edges(&mut duplicate)?.first().ok_or("edge")?.clone();
        edges(&mut duplicate)?.push(original);
        for invalid in [missing, empty, duplicate] {
            if audit(&lines(&[invalid, second.clone()]), limit).is_ok() {
                return Err(String::from("accepted missing or duplicate flow"));
            }
        }
        for (field, value) in [
            ("site", Value::from("0001")),
            ("target", Value::from("audio:z80:program:fixed:0009")),
            ("target_address", Value::from("0x0003")),
            ("interior_entry", Value::Bool(true)),
            ("evidence", serde_json::json!({})),
            ("kind", Value::from(" ")),
        ] {
            let mut invalid = first.clone();
            let edge = edges(&mut invalid)?.first_mut().ok_or("edge")?;
            set_field(edge, field, value)?;
            if audit(&lines(&[invalid, second.clone()]), limit).is_ok() {
                return Err(String::from("accepted conflicting flow field"));
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_and_duplicate_unresolved_records() -> Result<(), String>
    {
        let mut fixture = record("0001", &[], &[]);
        for invalid in [
            serde_json::json!([{"site": "bad", "kind": "computed"}]),
            serde_json::json!([{"site": "0x0001", "kind": ""}]),
            serde_json::json!([{
                "site": "0x0001", "kind": "computed", "target": "bad"
            }]),
            serde_json::json!([
                {"site": "0x0001", "kind": "computed"},
                {"site": "0x0001", "kind": "computed"}
            ]),
        ] {
            set_field(&mut fixture, "unresolved_flows", invalid)?;
            if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
                return Err(String::from("accepted invalid unresolved flow"));
            }
        }
        Ok(())
    }

    #[test]
    fn checks_explicit_semantic_review_fields() -> Result<(), String> {
        let mut reviewed = record("0001", &[], &[]);
        for (field, value) in [
            ("analysis_state", Value::from("listing-reviewed")),
            ("status", Value::from("pending")),
            ("semantic_name", Value::from("fixture")),
            ("behavior", Value::from("fixture")),
            ("semantic_evidence", Value::from("fixture")),
            ("inputs", serde_json::json!([])),
            ("outputs", serde_json::json!(["result"])),
            ("state_mutations", serde_json::json!([])),
            ("data_dependencies", Value::Null),
        ] {
            set_field(&mut reviewed, field, value)?;
        }
        let report = audit(&reviewed.to_string(), NonZeroUsize::MIN)?;
        if report.statuses.get("pending") != Some(&1)
            || report.semantic_reviews != 1
            || report.reviewed_dependencies != 0
        {
            return Err(String::from("missing unresolved review"));
        }
        for (field, value) in [
            ("semantic_name", Value::Null),
            ("behavior", Value::from(" ")),
            ("semantic_evidence", Value::Null),
            ("inputs", Value::Null),
            ("outputs", serde_json::json!([""])),
            ("state_mutations", serde_json::json!([1u8])),
            ("data_dependencies", serde_json::json!([" "])),
            ("status", Value::from("discovered")),
        ] {
            let mut invalid = reviewed.clone();
            set_field(&mut invalid, field, value)?;
            if audit(&invalid.to_string(), NonZeroUsize::MIN).is_ok() {
                return Err(format!("accepted invalid reviewed {field}"));
            }
        }
        set_field(&mut reviewed, "data_dependencies", serde_json::json!([]))?;
        let empty_review = audit(&reviewed.to_string(), NonZeroUsize::MIN)?;
        if empty_review.semantic_reviews != 1
            || empty_review.reviewed_dependencies != 1
        {
            return Err(String::from("explicit empty dependency review lost"));
        }
        let mut unreviewed = record("0002", &[], &[]);
        set_field(&mut unreviewed, "data_dependencies", serde_json::json!([]))?;
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        let combined = audit(&lines(&[reviewed, unreviewed]), limit)?;
        if combined.functions != 2
            || combined.semantic_reviews != 1
            || combined.reviewed_dependencies != 1
        {
            return Err(String::from("unreviewed dependencies counted"));
        }
        Ok(())
    }

    #[test]
    fn rejects_partial_mapping_and_requires_binary_completion()
    -> Result<(), String> {
        let mut fixture = record("0001", &[], &[]);
        for (field, value) in [
            ("status", Value::from("pending")),
            ("analysis_state", Value::from("listing-reviewed")),
            ("semantic_name", Value::from("fixture")),
            ("behavior", Value::from("fixture behavior")),
            ("semantic_evidence", Value::from("fixture")),
            ("inputs", serde_json::json!([])),
            ("outputs", serde_json::json!([])),
            ("state_mutations", serde_json::json!([])),
            ("data_dependencies", serde_json::json!([])),
            ("test_evidence", serde_json::json!(["fixture"])),
        ] {
            set_field(&mut fixture, field, value)?;
        }
        let report = audit(&fixture.to_string(), NonZeroUsize::MIN)?;
        if report.implemented != 0
            || report.closed()
            || report.semantic_complete != 0
        {
            return Err(String::from("pending counted as complete"));
        }
        for (field, value) in [
            ("rust_owner", Value::from("fixture.rs")),
            ("rust_symbol", Value::from("fixture")),
            ("status", Value::from("partial")),
            ("status", Value::from("implemented")),
            ("status", Value::from("discarded")),
        ] {
            let mut invalid = fixture.clone();
            set_field(&mut invalid, field, value)?;
            if audit(&invalid.to_string(), NonZeroUsize::MIN).is_ok() {
                return Err(format!("accepted invalid pending {field}"));
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_unsubstantiated_completion() -> Result<(), String> {
        let mut fixture = record("0001", &[], &[]);
        for (field, value) in [
            ("status", Value::from("implemented")),
            ("analysis_state", Value::from("semantic-complete")),
            ("semantic_name", Value::from("fixture")),
            ("behavior", Value::from("fixture")),
            ("semantic_evidence", Value::from("fixture")),
            ("inputs", serde_json::json!([])),
            ("outputs", serde_json::json!([])),
            ("state_mutations", serde_json::json!([])),
            ("data_dependencies", serde_json::json!([])),
            ("test_evidence", serde_json::json!(["fixture"])),
        ] {
            set_field(&mut fixture, field, value)?;
        }
        if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted unmapped implementation"));
        }
        set_field(&mut fixture, "rust_owner", Value::from("fixture.rs"))?;
        set_field(&mut fixture, "rust_symbol", Value::from("fixture"))?;
        let status = "implemented";
        {
            set_field(&mut fixture, "status", Value::from(status))?;
            let mut unreviewed = fixture.clone();
            set_field(&mut unreviewed, "analysis_state", Value::Null)?;
            if audit(&unreviewed.to_string(), NonZeroUsize::MIN).is_ok() {
                return Err(format!("accepted unreviewed {status}"));
            }
            let mut unknown = fixture.clone();
            set_field(&mut unknown, "data_dependencies", Value::Null)?;
            if audit(&unknown.to_string(), NonZeroUsize::MIN).is_ok() {
                return Err(format!(
                    "accepted unknown dependencies for {status}"
                ));
            }
            let report = audit(&fixture.to_string(), NonZeroUsize::MIN)?;
            if report.statuses.get(status) != Some(&1)
                || report.implemented != 1
                || !report.closed()
            {
                return Err(format!("missing resolved {status} count"));
            }
        }
        reject_unknown_flow(fixture.clone())?;
        let duplicate = fixture.to_string().replacen(
            "\"status\":",
            "\"status\":\"discovered\",\"status\":",
            1,
        );
        if audit(&duplicate, NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted duplicate status key"));
        }

        check_non_runtime(fixture)
    }

    fn reject_unknown_flow(mut fixture: Value) -> Result<(), String> {
        set_field(
            &mut fixture,
            "unresolved_flows",
            serde_json::json!([{
                "kind": "computed", "site": "0x0001", "target": null
            }]),
        )?;
        if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted final unknown flow"));
        }
        Ok(())
    }

    fn check_non_runtime(mut fixture: Value) -> Result<(), String> {
        set_field(&mut fixture, "status", Value::from("discarded"))?;
        set_field(&mut fixture, "rust_owner", Value::Null)?;
        set_field(&mut fixture, "rust_symbol", Value::Null)?;
        reject_unknown_flow(fixture.clone())?;
        let report = audit(&fixture.to_string(), NonZeroUsize::MIN)?;
        if report.statuses.get("discarded") != Some(&1)
            || report.implemented != 0
            || !report.closed()
        {
            return Err(String::from("incorrect non-runtime count"));
        }
        set_field(&mut fixture, "analysis_state", Value::Null)?;
        if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
            return Err(String::from("accepted unreviewed non-runtime"));
        }
        set_field(
            &mut fixture,
            "analysis_state",
            Value::from("semantic-complete"),
        )?;
        set_field(&mut fixture, "data_dependencies", Value::Null)?;
        if audit(&fixture.to_string(), NonZeroUsize::MIN).is_ok() {
            return Err(String::from(
                "accepted unknown non-runtime dependencies",
            ));
        }
        Ok(())
    }

    #[test]
    fn rejects_mixed_selected_editions() -> Result<(), String> {
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        let [first, mut second] = pair();
        set_field(&mut second, "edition", Value::from("another edition"))?;
        if audit(&lines(&[first, second]), limit).is_ok() {
            return Err(String::from("accepted mixed selected editions"));
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

    #[test]
    fn rejects_numeric_identity_aliases_but_preserves_banks()
    -> Result<(), String> {
        let limit = NonZeroUsize::new(2).ok_or("fixture limit")?;
        for (padded, short) in [("0001", "1"), ("0000", "0")] {
            let first = record(padded, &[], &[]);
            let alias = record(short, &[], &[]);
            if audit(&lines(&[first.clone(), alias.clone()]), limit).is_ok() {
                return Err(String::from("accepted numeric identity alias"));
            }
            let mut banked = alias;
            set_field(&mut banked, "bank", Value::from("bank-01"))?;
            set_field(
                &mut banked,
                "id",
                Value::from(format!("audio:z80:program:bank-01:{short}")),
            )?;
            if audit(&lines(&[first, banked]), limit)?.functions != 2 {
                return Err(String::from("lost distinct bank identity"));
            }
        }
        Ok(())
    }
}
