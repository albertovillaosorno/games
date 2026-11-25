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
//   - Synthetic exact, renamed and unrelated source-clone tests.
// - Must-Not:
//   - Treat superficial similarity as conclusive duplication.
// - Allows:
//   - Inputs: Synthetic Rust functions with no private game content.
//   - Outputs: Clone reports with both source locations.
//   - Side effects: None.
// - Split-When:
//   - Integration fixtures require another responsibility.
// - Merge-When:
//   - Shared architecture tests supersede lexical fixtures.
// - Summary:
//   - Duplicate detector regression fixtures.
// - Description:
//   - Differentiates exact bodies, renamed similarities and nonclones.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No generated content or original game media.
//

use std::fmt::Write as _;
use std::path::Path;

use super::{compare, excluded, functions, tokens};

fn fixture(name: &str, prefix: &str, operator: &str) -> String {
    let mut source = format!("fn {name}() {{\n");
    for index in 0u8..16u8 {
        let _result =
            writeln!(source, "let {prefix}_{index} = {index} {operator} 1;");
    }
    source.push_str("}\n");
    source
}

#[test]
fn exact_clones_are_rejected_with_both_locations() {
    let first = fixture("alpha", "name", "+");
    let second = fixture("beta", "name", "+");
    let mut records = functions("crates/a/src/lib.rs", &first);
    records.extend(functions("crates/b/src/lib.rs", &second));
    let findings = compare(&records);
    assert_eq!(findings.exact.len(), 1);
    assert_eq!(findings.near, Vec::<String>::new());
    assert!(findings.exact.iter().any(|message| {
        message.contains("a/src/lib.rs:1") && message.contains("b/src/lib.rs:1")
    }));
}

#[test]
fn renamed_clones_require_manual_review() {
    let first = fixture("alpha", "first", "+");
    let second = fixture("beta", "second", "+");
    let mut records = functions("crates/a/src/lib.rs", &first);
    records.extend(functions("crates/b/src/lib.rs", &second));
    let findings = compare(&records);
    assert_eq!(findings.exact, Vec::<String>::new());
    assert_eq!(findings.near.len(), 1);
}

#[test]
fn unrelated_bodies_and_trivial_glue_are_not_clones() {
    let first = fixture("alpha", "first", "+");
    let second = fixture("beta", "second", "*");
    let short = String::from("fn get() { let x = 1; x; }");
    let mut records = functions("crates/a/src/lib.rs", &first);
    records.extend(functions("crates/b/src/lib.rs", &second));
    assert!(functions("crates/c/src/lib.rs", &short).is_empty());
    let findings = compare(&records);
    assert_eq!(findings.exact, Vec::<String>::new());
    assert_eq!(findings.near, Vec::<String>::new());
}

#[test]
fn ignores_comments_and_generated_inputs() {
    let first =
        String::from("fn example() { /* nested /* comment */ text */ 1; }");
    let stream = tokens(&first);
    assert!(!stream.iter().any(|token| token.text == "comment"));
    let path = Path::new("crates/example/src/generated/file.rs");
    assert_eq!(excluded(path, &first), Some("generated path"));
    let marked = String::from("// @generated\nfn manual() {}\n");
    assert_eq!(
        excluded(Path::new("crates/example/src/lib.rs"), &marked),
        Some("explicit generated marker"),
    );
}

#[test]
fn normalises_whitespace_and_nested_comments() {
    let first = fixture("alpha", "entity", "+");
    let mut second = fixture("beta", "entity", "+");
    second = second.replace(" = ", "/* outer /* inner */ ok */=");
    let mut records = functions("crates/a/src/lib.rs", &first);
    records.extend(functions("crates/b/src/lib.rs", &second));
    let findings = compare(&records);
    assert_eq!(findings.exact.len(), 1);
    assert_eq!(findings.near, Vec::<String>::new());
}

#[test]
fn raw_literals_do_not_expose_fake_functions() {
    let raw = "const EXAMPLE: &str = r##\"fn nested() { \" hello }\"##;";
    let stream = tokens(raw);
    assert_eq!(stream.iter().filter(|token| token.text == "fn").count(), 0);
    assert_eq!(stream.iter().filter(|token| token.text == "{").count(), 0);
}
