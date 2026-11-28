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
// - Owns: Proof-backed reconciliation of already admitted local table flows.
// - Must-Not: Invent targets, change function status or overwrite inputs.
// - Allows:
//   - Inputs: Verified fact/ROM pair and structurally valid ledger.
//   - Outputs: Reconciled candidate ledger, verified before output.
//   - Side effects: None beyond contributor output.
// - Split-When: Bank or stack proof requires a distinct verifier.
// - Merge-When: A common reference evidence merger subsumes this verifier.
// - Summary: Close finite local table obligations only when graph agrees.
// - Description: Preserve all prior observations and unrelated unknowns.
// - Usage: games-build ghidra-board simpsons reconcile FACTS ROM LEDGER BYTES
// - Defaults: Refuse conflicting or incomplete graph evidence.

//! Reconcile local dispatch proofs against an already admitted ledger.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use serde_json::{Value, json};

use super::bounds::{Facts, proofs};
use crate::reference_ledger::verify;

fn field<'value>(item: &'value Value, name: &str) -> Option<&'value str> {
    item.get(name)?.as_str()
}

fn key(bank: &str, address: &str) -> Option<String> {
    Some(format!("{bank}:{}", address.strip_prefix("0x")?))
}

fn target(edge: &Value) -> Option<String> {
    let id = field(edge, "target")?;
    let (prefix, _) = id.rsplit_once(':')?;
    let (_, bank) = prefix.rsplit_once(':')?;
    key(bank, field(edge, "target_address")?)
}

fn entries(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .flat_map(|row| {
            row.get("flow_edges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(target)
        })
        .collect()
}

fn row_key(row: &Value) -> Option<String> {
    key(field(row, "bank")?, field(row, "entry_address")?)
}

fn eligible(
    row: &Value,
    proof: &Value,
    incoming: &BTreeSet<String>,
) -> Option<String> {
    if field(row, "status")? != "pending"
        || field(row, "module")? != "maincpu"
        || field(row, "cpu")? != "Konami053248"
        || row_key(row)?.as_str() != field(proof, "source_owner")?
    {
        return None;
    }
    let path = proof.get("path")?.as_array()?;
    if path.iter().skip(1).any(|item| {
        item.as_str()
            .is_none_or(|address| incoming.contains(address))
    }) {
        return None;
    }
    let site = format!("0x{}", field(proof, "site")?);
    let unresolved = row.get("unresolved_flows")?.as_array()?;
    if !unresolved.iter().any(|item| {
        field(item, "site") == Some(&site)
            && field(item, "kind") == Some("computed")
    }) || unresolved.iter().any(|item| {
        field(item, "site") == Some(&site)
            && field(item, "kind") != Some("computed")
    }) {
        return None;
    }
    Some(site)
}

fn edges_agree(row: &Value, proof: &Value, site: &str) -> Option<()> {
    let edges = row.get("flow_edges")?.as_array()?;
    let actual: BTreeSet<_> = edges
        .iter()
        .filter(|edge| field(edge, "site") == Some(site))
        .map(target)
        .collect::<Option<_>>()?;
    let expected: BTreeSet<_> = proof
        .get("slots")?
        .as_array()?
        .iter()
        .map(|slot| field(slot, "target").map(str::to_owned))
        .collect::<Option<_>>()?;
    if actual.is_empty() || actual != expected {
        return None;
    }
    // Require the exact existing ledger owner, including interior targets.
    for slot in proof.get("slots")?.as_array()? {
        let expected_owner = field(slot, "owner")?;
        let expected_target = field(slot, "target")?;
        if !edges.iter().any(|edge| {
            field(edge, "site") == Some(site)
                && target(edge).as_deref() == Some(expected_target)
                && field(edge, "target").is_some_and(|id| {
                    id.strip_prefix("maincpu:konami053248:program:")
                        == Some(expected_owner)
                })
        }) {
            return None;
        }
    }
    Some(())
}

fn apply(row: &mut Value, proof: &Value, site: &str) -> Option<()> {
    edges_agree(row, proof, site)?;
    let edges = row.get_mut("flow_edges")?.as_array_mut()?;
    for edge in edges
        .iter_mut()
        .filter(|edge| field(edge, "site") == Some(site))
    {
        let evidence = edge.get_mut("evidence")?.as_object_mut()?;
        let _previous = evidence.insert(
            "automated_local_proof".to_owned(),
            json!({
                "rule":proof.get("rule")?,
                "rom_sha256":proof.get("rom_sha256")?,
                "path":proof.get("path")?,
                "slots":proof.get("slots")?,
                "scope":"decoded fallthrough and existing physical graph",
            }),
        );
    }
    let unresolved = row.get_mut("unresolved_flows")?.as_array_mut()?;
    unresolved.retain(|item| {
        field(item, "site") != Some(site)
            || field(item, "kind") != Some("computed")
    });
    Some(())
}

pub(super) fn candidate(
    input: &str,
    facts: &Facts,
    rom: &[u8],
) -> Result<String, String> {
    let limit = NonZeroUsize::new(65_536).ok_or("row limit")?;
    verify(input, limit)?;
    let mut rows: Vec<Value> = input
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    let incoming = entries(&rows);
    let proven = proofs(facts, rom);
    for row in &mut rows {
        for proof in &proven {
            let Some(site) = eligible(row, proof, &incoming) else {
                continue;
            };
            // Any incomplete target or contradictory edge leaves the
            // entire record unchanged.
            let mut staged = row.clone();
            if apply(&mut staged, proof, &site).is_some() {
                *row = staged;
            }
        }
    }
    let output = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    verify(&output, limit)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, Value) {
        let row = json!({
            "status":"pending","module":"maincpu","cpu":"Konami053248",
            "bank":"bank-01","entry_address":"0x6000",
            "unresolved_flows":[
                {"site":"0x6004","kind":"computed"},
                {"site":"0x6006","kind":"computed"}
            ],
            "flow_edges":[{
                "site":"0x6004",
                "target":"maincpu:konami053248:program:bank-01:6020",
                "target_address":"0x6020","evidence":{"kind":"fixture"}
            }]
        });
        let proof = json!({
            "source_owner":"bank-01:6000","site":"6004",
            "path":["bank-01:6000","bank-01:6002","bank-01:6004"],
            "slots":[{"target":"bank-01:6020","owner":"bank-01:6020"}],
            "rule":"fixture","rom_sha256":"fixture"
        });
        (row, proof)
    }

    #[test]
    fn closes_only_matching_observation() -> Result<(), String> {
        let (mut row, proof) = fixture();
        let site = eligible(&row, &proof, &BTreeSet::new())
            .ok_or("eligible proof rejected")?;
        apply(&mut row, &proof, &site).ok_or("proof rejected")?;
        let remaining = row
            .get("unresolved_flows")
            .and_then(Value::as_array)
            .ok_or("missing obligations")?;
        if remaining.len() != 1
            || remaining.first().and_then(|item| field(item, "site"))
                != Some("0x6006")
            || field(&row, "status") != Some("pending")
        {
            return Err(String::from("unrelated obligation changed"));
        }
        Ok(())
    }

    #[test]
    fn refuses_interior_entries_and_alternate_contexts() -> Result<(), String> {
        let (mut row, proof) = fixture();
        let incoming = BTreeSet::from([String::from("bank-01:6002")]);
        if eligible(&row, &proof, &incoming).is_some() {
            return Err(String::from("interior entry accepted"));
        }
        row.get_mut("unresolved_flows")
            .and_then(Value::as_array_mut)
            .ok_or("missing obligations")?
            .push(json!({"site":"0x6004","kind":"alternate-bank"}));
        if eligible(&row, &proof, &BTreeSet::new()).is_some() {
            return Err(String::from("unknown alternate context accepted"));
        }
        Ok(())
    }

    #[test]
    fn refuses_conflicting_owners_or_missing_edges() -> Result<(), String> {
        let (row, mut proof) = fixture();
        let slots = proof
            .get_mut("slots")
            .and_then(Value::as_array_mut)
            .ok_or("missing slots")?;
        slots.push(json!({"target":"bank-01:6030","owner":"bank-01:6030"}));
        if edges_agree(&row, &proof, "0x6004").is_some() {
            return Err(String::from("incomplete graph accepted"));
        }
        Ok(())
    }
}
