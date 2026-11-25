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
    discover(&mut authored_command(root))
}

fn authored_command(root: &Path) -> Command {
    let mut command = Command::new("rg");
    let _configured = command.current_dir(root).args([
        "--no-config",
        "--files",
        "--hidden",
        "--no-ignore",
        "--ignore-file",
        ".gitignore",
        "-g",
        "!.git/**",
        "-0",
    ]);
    command
}

fn discover(command: &mut Command) -> Result<Vec<PathBuf>, String> {
    let output = command.output().map_err(|error| error.to_string())?;
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
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{authored, authored_command, discover, line_width, python};
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
        let config = root.join(".reference/ripgrep.conf");
        fs::write(&config, "--glob\n!*.rs\n")
            .map_err(|error| error.to_string())?;
        let mut command = authored_command(&root);
        let _configured = command.env("RIPGREP_CONFIG_PATH", config);
        require(discover(&mut command)? == files, "ignore local rg config")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }
}
