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
//   - Authored file discovery and text policy.
// - Must-Not:
//   - Implement game behaviour or modify source.
// - Allows:
//   - Inputs: Repository files and local tool configuration.
//   - Outputs: Validation data or diagnostic.
//   - Side effects: Read-only filesystem access.
// - Split-When:
//   - An independent validation responsibility emerges.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Authored file discovery and text policy.
// - Description:
//   - Checks languages and width using shared ignore rules.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Fail closed on invalid inputs.
//

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::from_utf8;

use crate::todo::roadmap_width;

/// Discover authored files using the shared ignore policy.
///
/// # Errors
///
/// Returns an error if discovery fails.
pub fn authored(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("rg")
        .current_dir(root)
        .args([
            "--files",
            "--hidden",
            "--no-ignore",
            "--ignore-file",
            ".gitignore",
            "-g",
            "!.git/**",
            "-0",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from("authored file discovery failed"));
    }
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| from_utf8(part).map(PathBuf::from))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    paths.sort();
    Ok(paths)
}

fn python(path: &Path, bytes: &[u8]) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    extension.is_some_and(|value| {
        ["py", "pyw", "pyi", "pyc", "pyo", "ipynb"]
            .iter()
            .any(|ext| value.eq_ignore_ascii_case(ext))
    }) || bytes
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| from_utf8(line).ok())
        .is_some_and(|line| line.starts_with("#!") && line.contains("python"))
}

fn requires_text(path: &Path) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    extension.is_some_and(|value| {
        [
            "rs", "toml", "md", "mdc", "txt", "json", "jsonc", "yaml", "yml",
            "sh", "wgsl", "glsl", "vert", "frag", "comp", "hlsl", "xml",
            "html", "css",
        ]
        .contains(&value)
    }) || path.starts_with(".githooks")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(name, "LICENSE" | ".gitignore" | ".editorconfig")
            })
}

fn authored_text<'bytes>(
    path: &Path,
    bytes: &'bytes [u8],
) -> Result<Option<&'bytes str>, String> {
    if path == Path::new("Cargo.lock") {
        return Ok(None);
    }
    if let Ok(text) = from_utf8(bytes)
        && !text.contains('\0')
    {
        return Ok(Some(text));
    }
    if requires_text(path) {
        return Err(format!("invalid UTF-8 text: {}", path.display()));
    }
    Ok(None)
}

fn schema_exception(path: &Path, line: &str, previous: &str) -> bool {
    path == Path::new(".markdownlint-cli2.jsonc")
        && previous
            == "  // jig-ignore-next-line: indivisible reviewed identifier"
        && line
            == concat!(
                "  \"$schema\": \"https://raw.githubusercontent.com/",
                "DavidAnson/",
                "markdownlint-cli2/main/schema/",
                "markdownlint-cli2-config-schema.json\",",
            )
}

fn line_width(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let Some(text) = authored_text(path, bytes)? else {
        return Ok(());
    };
    let mut previous = "";
    for (index, line) in text.lines().enumerate() {
        let valid = if path == Path::new("TODO.md") {
            roadmap_width(line, previous)
        } else {
            line.chars().count() <= 80 || schema_exception(path, line, previous)
        };
        if !valid {
            return Err(format!(
                "{}:{}: line exceeds 80 characters",
                path.display(),
                index.saturating_add(1)
            ));
        }
        previous = line;
    }
    Ok(())
}

/// Check authored languages and line width using shared file discovery.
///
/// # Errors
/// Returns an error for prohibited source, malformed text or excessive width.
pub fn validate(root: &Path) -> Result<(), String> {
    for path in authored(root)? {
        let bytes =
            fs::read(root.join(&path)).map_err(|error| error.to_string())?;
        if python(&path, &bytes) {
            return Err(format!("Python is forbidden: {}", path.display()));
        }
        line_width(&path, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/validator/files.rs"]
mod tests;
