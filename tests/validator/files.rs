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
//   - Authored-file policy regression tests.
// - Must-Not:
//   - Implement game behaviour or modify source.
// - Allows:
//   - Inputs: Repository files and local tool configuration.
//   - Outputs: Validation data or diagnostic.
//   - Side effects: Temporary fixture directory writes.
// - Split-When:
//   - An independent validation responsibility emerges.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Authored-file policy regression tests.
// - Description:
//   - Tests width, source exclusions and the language policy.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Reject prohibited and overlong authored source.

use std::fs;
use std::path::Path;

use super::{authored, line_width, python};
use crate::fixtures::{directory, require};

#[test]
fn prohibits_python_source_and_scripts() {
    assert!(python(Path::new("test.PY"), b""), "Python source");
    assert!(
        python(Path::new("run"), b"#!/usr/bin/env python3\n"),
        "Python script"
    );
    assert!(!python(Path::new("lib.rs"), b"// python"), "Rust source");
}

#[test]
fn checks_unicode_width_and_excludes_generated_or_binary_data() {
    let exact = "\u{e9}".repeat(80);
    let wide = "\u{e9}".repeat(81);
    assert!(
        line_width(Path::new("notes.txt"), exact.as_bytes()).is_ok(),
        "80 Unicode characters fit"
    );
    assert!(
        line_width(Path::new("notes.txt"), wide.as_bytes()).is_err(),
        "81 Unicode characters exceed the contract"
    );
    assert!(
        line_width(Path::new("Cargo.lock"), wide.as_bytes()).is_ok(),
        "generated lockfile is outside authored width checks"
    );
    assert!(
        line_width(Path::new("image.png"), b"\x89PNG\0").is_ok(),
        "binary resources are not authored text"
    );
    assert!(
        line_width(Path::new("lib.rs"), b"\xff").is_err(),
        "invalid UTF-8 in source is not a binary exemption"
    );
}

#[test]
fn restricts_width_exceptions_to_reviewed_surfaces() {
    let link = format!(
        "<!-- MarkdownLint-disable-next-line MD013 MD044 -->\n[{}](a.mdc)",
        "a".repeat(80)
    );
    assert!(
        line_width(Path::new("TODO.md"), link.as_bytes()).is_ok(),
        "reviewed direct roadmap link"
    );
    assert!(
        line_width(Path::new("README.md"), link.as_bytes()).is_err(),
        "other documents cannot use the roadmap exception"
    );
    let prose = format!(
        "<!-- MarkdownLint-disable-next-line MD013 MD044 -->\n{}",
        "a".repeat(81)
    );
    assert!(
        line_width(Path::new("TODO.md"), prose.as_bytes()).is_err(),
        "exception cannot suppress an overlong prose line"
    );
}

#[test]
fn discovers_authored_files_without_local_media() -> Result<(), String> {
    let root = directory("files")?;
    fs::write(root.join(".gitignore"), "/.reference/\n/target/\n")
        .map_err(|error| error.to_string())?;
    for name in [".reference", "target"] {
        let path = root.join(name);
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        fs::write(path.join("hidden.py"), b"")
            .map_err(|error| error.to_string())?;
    }
    fs::write(root.join("lib.rs"), b"// source\n")
        .map_err(|error| error.to_string())?;
    let files = authored(&root)?;
    require(
        files == [Path::new(".gitignore"), Path::new("lib.rs")],
        "only authored source and configuration are visible",
    )?;
    fs::remove_dir_all(root).map_err(|error| error.to_string())
}
