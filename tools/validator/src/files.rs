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
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::from_utf8;

use games_platform::ir::toolchain::validation::{GitRequest, Request, run};

use crate::todo::roadmap_width;

/// Discover authored files using the shared ignore policy.
///
/// # Errors
///
/// Returns an error if discovery fails.
pub fn authored(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = run(root.as_os_str(), Request::AuthoredFiles)
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err("authored file discovery failed".into());
    }
    let paths = discover_bytes(&output.stdout)?;
    if let Some(path) = paths.iter().find(|path| private_configuration(path)) {
        return Err(format!(
            "private configuration cannot be authored: {}",
            path.display()
        ));
    }
    if root.join(".git").exists() {
        verify_git_inventory(root, &paths)?;
    }
    Ok(paths)
}

/// Check every component of an authored path without following symlinks.
pub(crate) fn regular_path(root: &Path, path: &Path) -> Result<bool, String> {
    let mut current = root.to_path_buf();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(value) => value,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(false);
            },
            Err(error) => return Err(error.to_string()),
        };
        let valid = if components.peek().is_some() {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
        if !valid {
            return Err(format!(
                "non-regular authored path: {}",
                path.display()
            ));
        }
    }
    Ok(true)
}

fn verify_git_inventory(root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let output = run(root.as_os_str(), Request::Git(GitRequest::Inventory))
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err("Git inventory discovery failed".into());
    }
    for path in discover_bytes(&output.stdout)? {
        if regular_path(root, &path)? && paths.binary_search(&path).is_err() {
            return Err(format!(
                "Git path omitted from authored inventory: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn discover_bytes(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| from_utf8(part).map(PathBuf::from))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    paths.sort();
    Ok(paths)
}

fn private_configuration(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == ".env"
                || (name.starts_with(".env.") && name != ".env.example")
        })
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

fn local_storage_policy(root: &Path) -> Result<(), String> {
    let ignore = fs::read_to_string(root.join(".gitignore"))
        .map_err(|error| error.to_string())?;
    for directory in [".reference", ".cache", ".temp"] {
        let pattern = format!("/{directory}/");
        if !ignore.lines().any(|line| line == pattern) {
            return Err(format!("missing local storage ignore: {pattern}"));
        }
    }
    ledger_location(root)
}

/// Reject a durable ledger in disposable storage even before cache admission.
pub(crate) fn ledger_location(root: &Path) -> Result<(), String> {
    for directory in [".cache", ".temp"] {
        let path = root.join(directory).join("ledger");
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(format!(
                    "durable ledger belongs in .reference/ledger: {}",
                    path.display(),
                ));
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Check authored languages and line width using shared file discovery.
///
/// # Errors
/// Returns an error for prohibited source, malformed text or excessive width.
pub fn validate(root: &Path) -> Result<(), String> {
    local_storage_policy(root)?;
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
    use std::process::Command;

    use games_platform::ir::storage::fixtures as platform_fixtures;

    use super::{
        authored, line_width, local_storage_policy, private_configuration,
        python,
    };
    use crate::fixtures::{directory, require};

    #[test]
    fn durable_ledgers_are_not_disposable() -> Result<(), String> {
        let root = directory("ledger-policy")?;
        fs::write(root.join(".gitignore"), "/.reference/\n/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        fs::create_dir_all(root.join(".reference/ledger"))
            .map_err(|error| error.to_string())?;
        require(local_storage_policy(&root).is_ok(), "persistent ledger")?;
        for name in [".cache/ledger", ".temp/ledger"] {
            let path = root.join(name);
            fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            require(local_storage_policy(&root).is_err(), "disposable ledger")?;
            fs::remove_dir(&path).map_err(|error| error.to_string())?;
        }
        fs::write(root.join(".gitignore"), "/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        require(local_storage_policy(&root).is_err(), "reference ignore")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn private_configuration_is_not_authored() -> Result<(), String> {
        let root = directory("private-configuration")?;
        fs::write(root.join(".gitignore"), "")
            .map_err(|error| error.to_string())?;
        fs::write(root.join(".env.example"), "EXAMPLE=1\n")
            .map_err(|error| error.to_string())?;
        require(authored(&root).is_ok(), "example template is authored")?;
        fs::write(root.join(".env"), "LOCAL=1\n")
            .map_err(|error| error.to_string())?;
        require(authored(&root).is_err(), "private configuration rejected")?;
        require(
            private_configuration(Path::new("local/.env.production")),
            "private variant rejected",
        )?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

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

    fn git(root: &Path, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .map_err(|error| error.to_string())?;
        require(output.status.success(), "fixture Git command")
    }

    #[test]
    fn rejects_git_paths_hidden_from_file_discovery() -> Result<(), String> {
        if !platform_fixtures::AVAILABLE {
            return Err(String::from("native fixture capability unavailable"));
        }
        let root = directory("inventory")?;
        git(&root, &["init", "--quiet"])?;
        fs::write(root.join(".gitignore"), b"/private/\n")
            .map_err(|error| error.to_string())?;
        fs::write(root.join("source.rs"), b"// source\n")
            .map_err(|error| error.to_string())?;
        git(&root, &["add", "source.rs"])?;
        require(authored(&root).is_ok(), "regular tracked source")?;

        let link = root.join("link.rs");
        platform_fixtures::file_link(Path::new("source.rs"), &link)
            .map_err(|error| error.to_string())?;
        require(authored(&root).is_err(), "untracked source symlink")?;
        git(&root, &["add", "link.rs"])?;
        require(authored(&root).is_err(), "staged source symlink")?;
        fs::remove_file(&link).map_err(|error| error.to_string())?;
        require(authored(&root).is_ok(), "removed tracked link")?;

        let hidden = root.join("private");
        fs::create_dir_all(&hidden).map_err(|error| error.to_string())?;
        fs::write(hidden.join("asset"), b"synthetic")
            .map_err(|error| error.to_string())?;
        require(authored(&root).is_ok(), "ignored local media")?;
        git(&root, &["add", "-f", "private/asset"])?;
        require(authored(&root).is_err(), "tracked ignored file")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }
}
