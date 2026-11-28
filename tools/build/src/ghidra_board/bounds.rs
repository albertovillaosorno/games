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
// - Owns: Finite selector bounds and immutable table candidate analysis.
// - Must-Not: Infer unknown control flow or write the canonical ledger.
// - Allows:
//   - Inputs: Ghidra facts, physical program image and explicit byte limit.
//   - Outputs: Deterministic candidate slot and ownership reports.
//   - Side effects: Read-only bounded analysis.
// - Split-When: Additional selector architectures require other semantics.
// - Merge-When: A shared processor analysis IR supersedes local rules.
// - Summary: Enumerate all inputs to recognised selector expressions.
// - Description: Unknown incoming contexts remain explicit.
// - Usage: games-build ghidra-board simpsons bounds FACTS ROM BYTES
// - Defaults: Reject inconsistent facts and retain unsupported sites.

//! Finite selector analysis for immutable Konami dispatch tables.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::str::from_utf8;

use games_assets::ContentHash;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain::write_line;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(super) struct Facts {
    code: BTreeSet<String>,
    owners: BTreeMap<String, String>,
    sha256: String,
    sites: Vec<Dispatch>,
}

#[derive(Deserialize)]
struct Dispatch {
    #[serde(default)]
    addresses: Vec<String>,
    bank: String,
    chain: Vec<String>,
    #[serde(default)]
    owner: String,
    site: String,
}

type Loaded = (Facts, Vec<u8>);
type Selector = (BTreeSet<i16>, &'static str);

struct Bound {
    base: u16,
    offsets: BTreeSet<i16>,
    rule: &'static str,
}

fn number(value: &str) -> Option<u16> {
    u16::from_str_radix(value.strip_prefix("0x")?, 16).ok()
}

pub(super) fn immediate(line: &str, operation: &str) -> Option<u16> {
    let (name, value) = line.strip_prefix("IMM ")?.split_once(' ')?;
    (name == operation).then_some(())?;
    number(value)
}

pub(super) fn signed(value: u8) -> i16 {
    i16::from(i8::from_be_bytes([value]))
}

fn register(instruction: &str) -> Option<(&str, &str)> {
    let (op, rest) = instruction.split_once(' ')?;
    if !matches!(op, "JMP" | "JSR") {
        return None;
    }
    let (index, base) =
        rest.strip_prefix('[')?.strip_suffix(']')?.split_once(',')?;
    if !matches!(index, "A" | "B" | "D") || !matches!(base, "X" | "Y" | "U") {
        return None;
    }
    Some((index, base))
}

fn table(chain: &[String], base: &str) -> Option<(usize, u16)> {
    let load = format!("LD{base}");
    chain.iter().enumerate().rev().find_map(|(index, line)| {
        immediate(line, &load).map(|value| (index, value))
    })
}

fn signed_guard(chain: &[&str]) -> Option<BTreeSet<i16>> {
    let [load, zero, low, below, high, above, "ASLB", "SEX"] = chain else {
        return None;
    };
    if !load.starts_with("LDB ")
        || !zero.starts_with("LBEQ ")
        || !below.starts_with("LBLT ")
        || !above.starts_with("LBGE ")
    {
        return None;
    }
    let minimum = signed(u8::try_from(immediate(low, "CMPB")?).ok()?);
    let maximum = signed(u8::try_from(immediate(high, "CMPB")?).ok()?);
    if minimum < -64 || maximum > 64 || minimum >= maximum {
        return None;
    }
    Some(
        (minimum..maximum)
            .filter(|value| *value != 0)
            .map(|value| value.saturating_mul(2))
            .collect(),
    )
}

fn masked_word(chain: &[&str]) -> Option<BTreeSet<i16>> {
    let ["CLRA", load, mask_op, shift_op, "ASLB"] = chain else {
        return None;
    };
    if !load.starts_with("LDB ") {
        return None;
    }
    let mask = u8::try_from(immediate(mask_op, "ANDB")?).ok()?;
    let shift = u32::from(immediate(shift_op, "LSRDI")? & 15);
    Some(
        (0..=u8::MAX)
            .map(|value| {
                signed(
                    u8::try_from(u16::from(value & mask) >> shift)
                        .unwrap_or_default()
                        .wrapping_mul(2),
                )
            })
            .collect(),
    )
}

fn masked_byte(chain: &[&str], index: &str) -> Option<BTreeSet<i16>> {
    let [mask_op, scale] = chain else {
        return None;
    };
    if *scale != format!("ASL{index}") {
        return None;
    }
    let mask =
        u8::try_from(immediate(mask_op, &format!("AND{index}"))?).ok()?;
    Some(
        (0..=u8::MAX)
            .map(|value| signed((value & mask).wrapping_mul(2)))
            .collect(),
    )
}

fn bound(site: &Dispatch) -> Option<Bound> {
    let (dispatch, prior) = site.chain.split_last()?;
    let (index, base_reg) = register(dispatch)?;
    let (position, base) = table(prior, base_reg)?;
    // Only allow the base load among the exact selector operations.
    // Removing an intervening load is sound for A/B arithmetic, but not
    // between a comparison and its consuming branch (it sets flags).
    let mut chain: Vec<&str> = prior.iter().map(String::as_str).collect();
    let _base_load = chain.remove(position);
    let suffix = prior.len().saturating_sub(position.saturating_add(1));
    let (offsets, rule) = selector(index, &chain, suffix)?;
    Some(Bound { base, offsets, rule })
}

fn selector(index: &str, chain: &[&str], suffix: usize) -> Option<Selector> {
    match index {
        "D" if suffix == 0 => {
            Some((signed_guard(tail(chain, 8)?)?, "signed-guard"))
        },
        "B" if suffix == 1 => {
            if let Some(offsets) = tail(chain, 5).and_then(masked_word) {
                return Some((offsets, "masked-word"));
            }
            Some((masked_byte(tail(chain, 2)?, index)?, "masked-byte"))
        },
        "A" | "B" if suffix <= 1 => {
            Some((masked_byte(tail(chain, 2)?, index)?, "masked-byte"))
        },
        _ => None,
    }
}

fn tail<'chain>(
    chain: &'chain [&'chain str],
    count: usize,
) -> Option<&'chain [&'chain str]> {
    chain.get(chain.len().checked_sub(count)?..)
}

fn physical(bank: &str, address: u16) -> Option<usize> {
    match address {
        0x8000..=0xffff => Some(
            0x78000usize
                .saturating_add(usize::from(address.saturating_sub(0x8000))),
        ),
        0x6000..=0x7fff => {
            let bank_index =
                bank.strip_prefix("bank-")?.parse::<usize>().ok()?;
            (bank_index < 64).then_some(
                bank_index.saturating_mul(8192).saturating_add(usize::from(
                    address.saturating_sub(0x6000),
                )),
            )
        },
        _ => None,
    }
}

fn location(bank: &str, address: u16) -> Option<String> {
    match address {
        0x8000..=0xfff1 => Some(format!("fixed:{address:04x}")),
        0x6000..=0x7fff if bank != "fixed" => {
            Some(format!("{bank}:{address:04x}"))
        },
        _ => None,
    }
}

fn domain_bound(site: &Dispatch) -> Option<Bound> {
    let (dispatch, prior) = site.chain.split_last()?;
    let (index, base_reg) = register(dispatch)?;
    let (position, base) = table(prior, base_reg)?;
    let after = prior.get(position.saturating_add(1)..)?;
    if !after.iter().all(|line| base_preserved(line, index)) {
        return None;
    }
    let offsets = super::domain::evaluate(prior, index);
    if offsets.is_empty() || offsets.len() >= 128 {
        return None;
    }
    Some(Bound {
        base,
        offsets,
        rule: "finite-byte-path",
    })
}

fn base_preserved(line: &str, index: &str) -> bool {
    if line == format!("ASL{index}") || line == format!("CLR{index}") {
        return true;
    }
    if let Some((operation, _)) = line
        .strip_prefix("IMM ")
        .and_then(|text| text.split_once(' '))
    {
        return ["ADD", "AND", "CMP", "EOR", "LD", "OR", "SUB"]
            .iter()
            .any(|op| operation == format!("{op}{index}"));
    }
    if line.starts_with(&format!("LD{index} ")) && !line.contains(['+', '-']) {
        return true;
    }
    let op = line.split_once(' ').map_or(line, |(op, _)| op);
    matches!(
        op.strip_prefix('L').unwrap_or(op),
        "BEQ"
            | "BNE"
            | "BCS"
            | "BCC"
            | "BHI"
            | "BLS"
            | "BLT"
            | "BGE"
            | "BGT"
            | "BLE"
    )
}

fn scaled(site: &Dispatch) -> Option<Bound> {
    let (dispatch, prior) = site.chain.split_last()?;
    let (index, base_reg) = register(dispatch)?;
    let (position, base) = table(prior, base_reg)?;
    let mut chain: Vec<&str> = prior.iter().map(String::as_str).collect();
    let _load = chain.remove(position);
    let suffix = prior.len().saturating_sub(position.saturating_add(1));
    let offsets = scaled_offsets(index, &chain, suffix)?;
    Some(Bound {
        base,
        offsets,
        rule: "unconstrained-scale",
    })
}

fn scaled_offsets(
    index: &str,
    chain: &[&str],
    suffix: usize,
) -> Option<BTreeSet<i16>> {
    if index == "D" && suffix == 0 && tail(chain, 2)? == ["CLRA", "ASLB"] {
        return Some(
            (0i16..=127).map(|value| value.saturating_mul(2)).collect(),
        );
    }
    if !matches!(index, "A" | "B") || *chain.last()? != format!("ASL{index}") {
        return None;
    }
    if suffix == 2 {
        let load = *tail(chain, 2)?.first()?;
        if !load.starts_with(&format!("LD{index} "))
            || load.contains(['+', '-'])
        {
            return None;
        }
    } else if suffix > 2 {
        return None;
    }
    Some(
        (0..=u8::MAX)
            .map(|value| signed(value.wrapping_mul(2)))
            .collect(),
    )
}

fn slot(
    facts: &Facts,
    rom: &[u8],
    bank: &str,
    table: (u16, i16),
) -> Option<serde_json::Value> {
    let (base, offset) = table;
    let address = base.checked_add_signed(offset)?;
    let next = address.checked_add(1)?;
    let start = physical(bank, address)?;
    if physical(bank, next)? != start.saturating_add(1)
        || facts.code.contains(&location(bank, address)?)
        || facts.code.contains(&location(bank, next)?)
    {
        return None;
    }
    let word = rom.get(start..start.saturating_add(2))?;
    let target = u16::from_be_bytes([*word.first()?, *word.get(1)?]);
    let key = location(bank, target)?;
    Some(json!({
        "slot": format!("{address:04x}"), "offset": offset,
        "target": key, "owner": facts.owners.get(&key),
    }))
}

fn resolve(facts: &Facts, rom: &[u8], site: &Dispatch) -> serde_json::Value {
    let Some(bound) = domain_bound(site)
        .or_else(|| bound(site))
        .or_else(|| scaled(site))
    else {
        return json!({
            "bank":site.bank,"site":site.site,"status":"unsupported"
        });
    };
    let slots: Vec<_> = bound
        .offsets
        .iter()
        .map(|offset| {
            slot(facts,rom,&site.bank,(bound.base,*offset)).unwrap_or_else(||
            json!({"offset":offset,"problem":"not an admitted ROM pointer"}))
        })
        .collect();
    let missing = slots
        .iter()
        .filter(|item| item.get("owner").is_none_or(serde_json::Value::is_null))
        .count();
    let status = if bound.rule == "unconstrained-scale" {
        "needs-selector"
    } else if missing > 0 {
        "needs-targets"
    } else {
        "bounded-candidate"
    };
    json!({
        "bank":site.bank, "site":site.site, "rule":bound.rule,
        "status":status,"missing":missing,"slots":slots,
        "conditional":"known predecessor chain and bank",
    })
}

pub(super) fn proofs(facts: &Facts, rom: &[u8]) -> Vec<serde_json::Value> {
    facts
        .sites
        .iter()
        .filter_map(|site| {
            if site.addresses.len() != site.chain.len()
                || site.owner.is_empty()
                || !site.addresses.iter().all(|address| {
                    facts.owners.get(address) == Some(&site.owner)
                })
                || site.addresses.last()
                    != Some(&format!("{}:{}", site.bank, site.site))
            {
                return None;
            }
            let mut report = resolve(facts, rom, site);
            if report.get("status")?.as_str()? != "bounded-candidate" {
                return None;
            }
            let _previous_path = report
                .as_object_mut()?
                .insert("path".to_owned(), json!(site.addresses));
            let _previous_owner = report
                .as_object_mut()?
                .insert("source_owner".to_owned(), json!(site.owner));
            let _previous_digest = report
                .as_object_mut()?
                .insert("rom_sha256".to_owned(), json!(facts.sha256));
            Some(report)
        })
        .collect()
}

fn load(
    facts_path: &OsStr,
    rom_path: &OsStr,
    limit: NonZeroU64,
) -> Result<Loaded, String> {
    let bytes =
        snapshot(facts_path, limit).map_err(|error| format!("{error:?}"))?;
    let facts: Facts =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let rom = snapshot(
        rom_path,
        NonZeroU64::new(524_288).ok_or("invalid ROM size")?,
    )
    .map_err(|error| format!("{error:?}"))?;
    if rom.len() != 524_288
        || facts.sha256 != ContentHash::calculate(&rom).to_string()
    {
        return Err(String::from("fact and ROM digest mismatch"));
    }
    if facts.sites.len() > 65_536 || facts.code.len() > 524_288 {
        return Err(String::from("dispatch fact budget exceeded"));
    }
    Ok((facts, rom))
}

pub(super) fn reconcile(
    facts_path: &OsStr,
    rom_path: &OsStr,
    ledger_path: &OsStr,
    limit: NonZeroU64,
) -> Result<(), String> {
    let (facts, rom) = load(facts_path, rom_path, limit)?;
    let bytes =
        snapshot(ledger_path, limit).map_err(|error| format!("{error:?}"))?;
    let input = from_utf8(&bytes).map_err(|error| error.to_string())?;
    let output = super::reconcile::candidate(input, &facts, &rom)?;
    write_line(&output).map_err(|error| error.to_string())
}

pub(super) fn run(
    facts_path: &OsStr,
    rom_path: &OsStr,
    limit: NonZeroU64,
) -> Result<(), String> {
    let (facts, rom) = load(facts_path, rom_path, limit)?;
    let mut counts = BTreeMap::<String, usize>::new();
    for site in &facts.sites {
        let report = resolve(&facts, &rom, site);
        let status = report
            .get("status")
            .and_then(serde_json::Value::as_str)
            .ok_or("missing status")?;
        let count = counts.entry(status.to_owned()).or_default();
        *count = count.saturating_add(1);
        write_line(&report.to_string()).map_err(|error| error.to_string())?;
    }
    write_line(&format!(
        "DISPATCH_BOUNDS sites={} statuses={}",
        facts.sites.len(),
        json!(counts)
    ))
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(chain: &[&str]) -> Dispatch {
        Dispatch {
            addresses: Vec::new(),
            bank: String::from("bank-01"),
            chain: chain.iter().map(|line| (*line).to_owned()).collect(),
            owner: String::new(),
            site: String::from("6000"),
        }
    }

    #[test]
    fn finite_selector_families() -> Result<(), String> {
        let masked = dispatch(&[
            "CLRA",
            "LDB 0x10",
            "IMM ANDB 0x60",
            "IMM LSRDI 0x5",
            "IMM LDX 0x7000",
            "ASLB",
            "JMP [B,X]",
        ]);
        let bounded = bound(&masked).ok_or("missing masked bound")?;
        if bounded.offsets != BTreeSet::from([0, 2, 4, 6]) {
            return Err(String::from("incorrect mask domain"));
        }
        let guarded = dispatch(&[
            "LDB 0x10,U",
            "LBEQ 0x6000",
            "IMM CMPB 0xf0",
            "LBLT 0x6000",
            "IMM CMPB 0x10",
            "LBGE 0x6000",
            "ASLB",
            "SEX",
            "IMM LDY 0x7000",
            "JMP [D,Y]",
        ]);
        let bounded_guard = bound(&guarded).ok_or("missing signed bound")?;
        if bounded_guard.offsets.len() != 31
            || bounded_guard.offsets.first() != Some(&-32)
            || bounded_guard.offsets.last() != Some(&30)
            || bounded_guard.offsets.contains(&0)
        {
            return Err(String::from("incorrect signed guard domain"));
        }
        Ok(())
    }

    #[test]
    fn refuses_ambiguous_load_and_preserves_unbounded_domain()
    -> Result<(), String> {
        let ambiguous = dispatch(&["LDX 0x7000", "ASLA", "JMP [A,X]"]);
        if bound(&ambiguous).is_some() || scaled(&ambiguous).is_some() {
            return Err(String::from("accepted non-immediate base"));
        }
        let scaled_byte =
            dispatch(&["IMM LDX 0x7000", "LDA 0x10,X", "ASLA", "JMP [A,X]"]);
        let domain = scaled(&scaled_byte).ok_or("missing diagnostic")?;
        if bound(&scaled_byte).is_some()
            || domain.offsets.len() != 128
            || domain.offsets.first() != Some(&-128)
            || domain.offsets.last() != Some(&126)
        {
            return Err(String::from("invalid unbounded selector domain"));
        }
        let post_increment =
            dispatch(&["IMM LDX 0x7000", "LDA X+", "ASLA", "JMP [A,X]"]);
        if scaled(&post_increment).is_some() {
            return Err(String::from("accepted changed table base"));
        }
        Ok(())
    }

    #[test]
    fn missing_owners_and_code_conflicts_do_not_close() -> Result<(), String> {
        let selector_site =
            dispatch(&["IMM ANDA 0x0", "IMM LDX 0x7000", "ASLA", "JMP [A,X]"]);
        let mut image = vec![0u8; 524_288];
        image
            .get_mut(0x3000..0x3002)
            .ok_or("fixture")?
            .copy_from_slice(&[0x60, 0x20]);
        let mut facts = Facts {
            code: BTreeSet::new(),
            owners: BTreeMap::new(),
            sha256: String::new(),
            sites: Vec::new(),
        };
        if resolve(&facts, &image, &selector_site)
            .get("status")
            .and_then(serde_json::Value::as_str)
            != Some("needs-targets")
        {
            return Err(String::from("missing owner not reported"));
        }
        let _old = facts
            .owners
            .insert(String::from("bank-01:6020"), String::from("bank-01:6020"));
        if resolve(&facts, &image, &selector_site)
            .get("status")
            .and_then(serde_json::Value::as_str)
            != Some("bounded-candidate")
        {
            return Err(String::from("owned target not recognised"));
        }
        let _added = facts.code.insert(String::from("bank-01:7001"));
        if resolve(&facts, &image, &selector_site)
            .get("status")
            .and_then(serde_json::Value::as_str)
            != Some("needs-targets")
        {
            return Err(String::from("overlapping code accepted"));
        }
        Ok(())
    }
}
