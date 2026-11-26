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

use std::num::NonZeroU64;
use std::path::{Component, Path, PathBuf};
use std::str::from_utf8;

use games_platform::ir::security::toolchain::validation::admit_paths;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::{PathKind, canonical_path, path_kind};
use games_platform::ir::toolchain::validation::{GitRequest, Request, run};

use crate::todo::roadmap_width;

// Authored source is not an asset store. Bound individual input buffers.
const MAX_AUTHORED_BYTES: NonZeroU64 =
    NonZeroU64::MIN.saturating_add(16_777_215);

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
    if repository_present(root)? {
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
        let kind = path_kind(current.as_os_str())
            .map_err(|error| error.to_string())?;
        if kind == PathKind::Missing {
            return Ok(false);
        }
        let expected = if components.peek().is_some() {
            PathKind::Directory
        } else {
            PathKind::RegularFile
        };
        let valid = kind == expected;
        if !valid {
            return Err(format!(
                "non-regular authored path: {}",
                path.display()
            ));
        }
    }
    Ok(true)
}

pub(crate) fn repository_present(root: &Path) -> Result<bool, String> {
    match path_kind(root.join(".git").as_os_str())
        .map_err(|error| error.to_string())?
    {
        PathKind::Directory | PathKind::RegularFile => Ok(true),
        PathKind::Missing => Ok(false),
        PathKind::Other | PathKind::SymbolicLink => {
            Err(String::from("invalid Git repository entry"))
        },
    }
}

/// Canonicalize a trusted local path for contributor identity checks.
pub(crate) fn canonical(path: &Path) -> Result<PathBuf, String> {
    canonical_path(path.as_os_str())
        .map(PathBuf::from)
        .map_err(|error| error.to_string())
}

/// Read an authored regular file with the shared bounded snapshot.
pub(crate) fn read_authored(
    root: &Path,
    path: &Path,
) -> Result<Vec<u8>, String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || private_configuration(path)
    {
        return Err(String::from("invalid authored read path"));
    }
    if !regular_path(root, path)? {
        return Err(format!("missing authored file: {}", path.display()));
    }
    snapshot(root.join(path).as_os_str(), MAX_AUTHORED_BYTES)
        .map_err(|error| format!("{}: {error:?}", path.display()))
}

/// Decode authored text after the same bounded regular-file admission.
pub(crate) fn read_text(root: &Path, path: &Path) -> Result<String, String> {
    String::from_utf8(read_authored(root, path)?)
        .map_err(|error| format!("{}: {error}", path.display()))
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
    let mut paths: Vec<_> = admit_paths(bytes)
        .map_err(|_error| "invalid authored inventory record")?
        .into_iter()
        .map(PathBuf::from)
        .collect();
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
    let ignore = read_text(root, Path::new(".gitignore"))?;
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
        if path_kind(path.as_os_str()).map_err(|error| error.to_string())?
            != PathKind::Missing
        {
            return Err(format!(
                "durable ledger belongs in .reference/ledger: {}",
                path.display(),
            ));
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
        let bytes = read_authored(root, &path)?;
        if python(&path, &bytes) {
            return Err(format!("Python is forbidden: {}", path.display()));
        }
        line_width(&path, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use games_platform::ir::storage::fixtures as platform_fixtures;
    use games_platform::ir::toolchain::validation::fixtures::{
        FixtureGit, git,
    };

    use super::{
        MAX_AUTHORED_BYTES, authored, line_width, local_storage_policy,
        private_configuration, python, read_authored, read_text,
    };
    use crate::fixtures::{directory, require};

    #[test]
    fn authored_reads_are_bounded_and_reject_links() -> Result<(), String> {
        let fixture = directory()?;
        let root = Path::new(fixture.location());
        fixture
            .write_file("source.rs", b"source")
            .map_err(|error| error.to_string())?;
        require(
            read_text(root, Path::new("source.rs"))? == "source",
            "exact snapshot",
        )?;
        fixture
            .link(root.join("source.rs").as_os_str(), "alias.rs")
            .map_err(|error| error.to_string())?;
        require(
            read_authored(root, Path::new("alias.rs")).is_err(),
            "reject linked source",
        )?;
        require(
            read_authored(root, Path::new("../source.rs")).is_err(),
            "reject traversal",
        )?;
        fixture
            .write_file(".env", b"private")
            .map_err(|error| error.to_string())?;
        require(
            read_authored(root, Path::new(".env")).is_err(),
            "reject private source",
        )?;
        fixture
            .create_sized_file(
                "large",
                MAX_AUTHORED_BYTES.get().saturating_add(1),
            )
            .map_err(|error| error.to_string())?;
        require(
            read_authored(root, Path::new("large")).is_err(),
            "reject oversized source",
        )
    }

    #[test]
    fn durable_ledgers_are_not_disposable() -> Result<(), String> {
        let root_owner = directory()?;
        let root = Path::new(root_owner.location());
        root_owner
            .write_file(".gitignore", b"/.reference/\n/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .create_directories(".reference/ledger")
            .map_err(|error| error.to_string())?;
        require(local_storage_policy(root).is_ok(), "persistent ledger")?;
        for name in [".cache/ledger", ".temp/ledger"] {
            root_owner
                .create_directories(name)
                .map_err(|error| error.to_string())?;
            require(local_storage_policy(root).is_err(), "disposable ledger")?;
            root_owner
                .remove_directory(name)
                .map_err(|error| error.to_string())?;
        }
        root_owner
            .write_file(".gitignore", b"/.cache/\n/.temp/\n")
            .map_err(|error| error.to_string())?;
        require(local_storage_policy(root).is_err(), "reference ignore")?;
        Ok(())
    }

    #[test]
    fn private_configuration_is_not_authored() -> Result<(), String> {
        let root_owner = directory()?;
        let root = Path::new(root_owner.location());
        root_owner
            .write_file(".gitignore", b"")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file(".env.example", b"EXAMPLE=1\n")
            .map_err(|error| error.to_string())?;
        require(authored(root).is_ok(), "example template is authored")?;
        root_owner
            .write_file(".env", b"LOCAL=1\n")
            .map_err(|error| error.to_string())?;
        require(authored(root).is_err(), "private configuration rejected")?;
        require(
            private_configuration(Path::new("local/.env.production")),
            "private variant rejected",
        )?;
        Ok(())
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
        let root_owner = directory()?;
        let root = Path::new(root_owner.location());
        root_owner
            .write_file(".gitignore", b"/.reference/\n/target/\n")
            .map_err(|error| error.to_string())?;
        for name in [".reference", "target"] {
            root_owner
                .create_directories(name)
                .map_err(|error| error.to_string())?;
            root_owner
                .write_file(&format!("{name}/hidden.py"), b"")
                .map_err(|error| error.to_string())?;
        }
        root_owner
            .write_file("lib.rs", b"// source\n")
            .map_err(|error| error.to_string())?;
        let files = authored(root)?;
        require(
            files == [Path::new(".gitignore"), Path::new("lib.rs")],
            "only authored source and configuration are visible",
        )?;
        Ok(())
    }

    #[test]
    fn rejects_git_paths_hidden_from_file_discovery() -> Result<(), String> {
        if !platform_fixtures::AVAILABLE {
            return Err(String::from("native fixture capability unavailable"));
        }
        let root_owner = directory()?;
        let root = Path::new(root_owner.location());
        git(&root_owner, FixtureGit::Initialize)
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file(".gitignore", b"/private/\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("source.rs", b"// source\n")
            .map_err(|error| error.to_string())?;
        git(&root_owner, FixtureGit::Stage {
            name: "source.rs",
            force: false,
        })
        .map_err(|error| error.to_string())?;
        require(authored(root).is_ok(), "regular tracked source")?;

        root_owner
            .link("source.rs".as_ref(), "link.rs")
            .map_err(|error| error.to_string())?;
        require(authored(root).is_err(), "untracked source symlink")?;
        git(&root_owner, FixtureGit::Stage {
            name: "link.rs",
            force: false,
        })
        .map_err(|error| error.to_string())?;
        require(authored(root).is_err(), "staged source symlink")?;
        root_owner
            .remove_file("link.rs")
            .map_err(|error| error.to_string())?;
        require(authored(root).is_ok(), "removed tracked link")?;

        root_owner
            .create_directories("private")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("private/asset", b"synthetic")
            .map_err(|error| error.to_string())?;
        require(authored(root).is_ok(), "ignored local media")?;
        git(&root_owner, FixtureGit::Stage {
            name: "private/asset",
            force: true,
        })
        .map_err(|error| error.to_string())?;
        require(authored(root).is_err(), "tracked ignored file")?;
        Ok(())
    }
}
