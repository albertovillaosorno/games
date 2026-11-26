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
//   - Structural framing admission of authored WebAssembly modules.
// - Must-Not:
//   - Execute modules or claim instruction or import compatibility.
// - Allows:
//   - Inputs: A bounded version-one WebAssembly module snapshot.
//   - Outputs: Structural admission or rejection.
//   - Side effects: None.
// - Split-When:
//   - A new Wasm format requires different framing.
// - Merge-When:
//   - Another shared contract admits identical module framing.
// - Summary:
//   - Require ordered bounded sections and defined function bodies.
// - Description:
//   - Reject truncated lengths, repeated sections and incomplete code.
// - Usage:
//   - Admit the output of the trusted authored browser toolchain.
// - Defaults:
//   - No general Wasm validation or runtime support claim.

const HEADER: &[u8] = b"\0asm\x01\0\0\0";

fn integer(bytes: &mut &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for shift in [0u32, 7, 14, 21, 28] {
        let (byte, rest) = bytes.split_first()?;
        *bytes = rest;
        if shift == 28 && *byte > 15 {
            return None;
        }
        value |= u32::from(*byte & 127).checked_shl(shift)?;
        if *byte & 128 == 0 {
            return Some(value);
        }
    }
    None
}

fn payload<'bytes>(bytes: &mut &'bytes [u8]) -> Option<&'bytes [u8]> {
    let size = usize::try_from(integer(bytes)?).ok()?;
    let (section, rest) = bytes.split_at_checked(size)?;
    *bytes = rest;
    Some(section)
}

const fn rank(id: u8) -> Option<u8> {
    match id {
        1..=5 => Some(id),
        13 => Some(6),
        6..=9 => Some(id.saturating_add(1)),
        12 => Some(11),
        10 => Some(12),
        11 => Some(13),
        _ => None,
    }
}

fn functions(mut section: &[u8]) -> Option<u32> {
    let count = integer(&mut section)?;
    for _ in 0..count {
        let _index = integer(&mut section)?;
    }
    section.is_empty().then_some(count)
}

fn code(mut section: &[u8], expected: u32) -> Option<()> {
    let count = integer(&mut section)?;
    if count == 0 || count != expected {
        return None;
    }
    for _ in 0..count {
        let body = payload(&mut section)?;
        // Each body needs at least the locals vector and terminal end.
        if body.len() < 2 || body.last() != Some(&0x0b) {
            return None;
        }
    }
    section.is_empty().then_some(())
}

pub(super) fn module(bytes: &[u8]) -> Option<()> {
    let mut remaining = bytes.strip_prefix(HEADER)?;
    let mut previous = 0u8;
    let mut count = None;
    let mut defined_code = false;
    while let Some((id, rest)) = remaining.split_first() {
        remaining = rest;
        let section = payload(&mut remaining)?;
        if *id == 0 {
            continue;
        }
        let next = rank(*id)?;
        if next <= previous || section.is_empty() {
            return None;
        }
        previous = next;
        match *id {
            3 => count = Some(functions(section)?),
            10 => {
                code(section, count?)?;
                defined_code = true;
            },
            _ => {},
        }
    }
    defined_code.then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &[u8] = super::super::fixtures::browser_module_fixture();

    #[test]
    fn version_one_module_requires_complete_ordered_code() {
        assert_eq!(module(MODULE), Some(()));
        for length in 0..MODULE.len() {
            assert_eq!(module(MODULE.get(..length).unwrap_or_default()), None);
        }
        for suffix in [
            b"\x0a\x04\x01\x02\0\x0b".as_slice(),
            b"\x00\x80\x80\x80\x80\x10",
            b"\x00\x04x",
            b"\x7f\x01x",
        ] {
            let mut invalid = MODULE.to_vec();
            invalid.extend_from_slice(suffix);
            assert_eq!(module(&invalid), None);
        }
        let mut valid = MODULE.to_vec();
        valid.extend_from_slice(b"\0\x02\x01x");
        assert_eq!(module(&valid), Some(()));
        let mut invalid = MODULE.to_vec();
        if let Some(last) = invalid.last_mut() {
            *last = 0;
        }
        assert_eq!(module(&invalid), None);
    }
}
