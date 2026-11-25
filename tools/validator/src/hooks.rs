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
//   - Dormant staged-path and commit-message checks.
// - Must-Not:
//   - Approve untested behaviour or modify authored source.
// - Allows:
//   - Inputs: Command arguments and repository files.
//   - Outputs: Diagnostics and process exit status.
//   - Side effects: Normal tool caches and subprocess execution.
// - Split-When:
//   - A check requires an independent policy.
// - Merge-When:
//   - A shared runner supersedes the entry point.
// - Summary:
//   - Future publication hook checks.
// - Description:
//   - Checks staged paths, Conventional Commits and DCO trailers.
// - Usage:
//   - Called only by future authorised Git hooks.
// - Defaults:
//   - Fail if any required check fails.
//

//! Dormant Git hook checks for future authorised publication.

/// Isolated test fixture helpers.
#[cfg(test)]
#[path = "../../../tests/validator/fixtures.rs"]
pub mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::from_utf8;
use std::{env, fs};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scope_valid(scope: &str) -> bool {
    !scope.is_empty()
        && scope
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/-".contains(&byte))
}

fn subject_valid(subject: &str) -> Result<(), String> {
    let (raw_kind, description) = subject
        .split_once(": ")
        .ok_or("expected Conventional Commit")?;
    let kind = raw_kind.strip_suffix('!').unwrap_or(raw_kind);
    let commit_type = if let Some((prefix, scope)) = kind.split_once('(') {
        if !scope.strip_suffix(')').is_some_and(scope_valid) {
            return Err("invalid Conventional Commit scope".into());
        }
        prefix
    } else {
        kind
    };
    if commit_type.is_empty()
        || !commit_type.bytes().all(|byte| byte.is_ascii_lowercase())
        || description.is_empty()
        || description.trim() != description
        || description.chars().any(char::is_control)
    {
        return Err("invalid Conventional Commit subject".into());
    }
    Ok(())
}

fn email_valid(email: &str) -> bool {
    email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.is_empty()
            && !domain.contains('@')
            && !email.chars().any(|character| {
                character.is_whitespace()
                    || character.is_control()
                    || matches!(character, '<' | '>')
            })
    })
}

fn identity_valid(identity: &str) -> bool {
    identity.rsplit_once(" <").is_some_and(|(name, address)| {
        !name.is_empty()
            && name.trim() == name
            && !name.contains(['<', '>'])
            && !name.chars().any(char::is_control)
            && address.strip_suffix('>').is_some_and(email_valid)
    })
}

fn trailer_valid(line: &str) -> bool {
    line.split_once(": ")
        .or_else(|| line.split_once(" #"))
        .is_some_and(|(token, value)| {
            !value.trim().is_empty()
                && (token == "BREAKING CHANGE"
                    || (!token.is_empty()
                        && token.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || byte == b'-'
                        })))
        })
}

fn check_trailers(message: &str) -> Result<(), String> {
    let lines = message
        .lines()
        .rev()
        .skip_while(|line| line.trim().is_empty() || line.starts_with('#'));
    let mut signed = false;
    for line in lines {
        if line.trim().is_empty() {
            if signed {
                return Ok(());
            }
            break;
        }
        if let Some(identity) = line.strip_prefix("Signed-off-by: ") {
            if !identity_valid(identity) {
                return Err("invalid DCO Signed-off-by identity".into());
            }
            signed = true;
        } else if line.starts_with("Signed-off-by:") || !trailer_valid(line) {
            return Err("invalid final commit trailer block".into());
        }
    }
    Err("missing DCO Signed-off-by trailer; use git commit -s".into())
}

fn check_commit(message: &str) -> Result<(), String> {
    subject_valid(message.lines().next().unwrap_or_default())?;
    check_trailers(message)
}

fn check_message_file(path: &Path) -> Result<(), String> {
    fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|message| check_commit(&message))
}

fn prohibited_path(path: &str) -> bool {
    if !relative_path(path) {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    let parts: Vec<_> = lower.split('/').collect();
    matches!(
        parts.first(),
        Some(
            &(".local"
                | ".git"
                | "assets"
                | "originals"
                | "research"
                | "dist"
                | "target"
                | "games.md"
                | ".cache"
                | ".temp"
                | ".reference")
        )
    ) || (matches!(parts.first(), Some(&("games" | "apps")))
        && matches!(parts.get(2), Some(&("assets" | "saves"))))
        || parts.iter().any(|part| {
            (part.starts_with(".env") && *part != ".env.example")
                || *part == ".reference"
                || Path::new(part)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rep"))
        })
        || [
            ".iso",
            ".chd",
            ".rom",
            ".apk",
            ".xapk",
            ".apks",
            ".obb",
            ".exe",
            ".dll",
            ".sqlite",
            ".sqlite-wal",
            ".sqlite-shm",
            ".gpr",
            ".pem",
            ".key",
            ".p12",
            ".pfx",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn relative_path(path: &str) -> bool {
    !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

fn check_paths(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let paths = bytes
        .strip_suffix(&[0])
        .ok_or("staged path list lacks final NUL")?;
    for raw in paths.split(|byte| *byte == 0) {
        let path = from_utf8(raw).map_err(|error| error.to_string())?;
        if prohibited_path(path) {
            return Err(format!("prohibited staged path: {path}"));
        }
    }
    Ok(())
}

fn check_staged() -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(root())
        .args([
            "diff",
            "--cached",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("cannot inspect staged paths".into());
    }
    check_paths(&output.stdout)
}

fn main() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "staged" => check_staged(),
        [mode, path] if mode == "commit-message" => {
            check_message_file(Path::new(path))
        },
        _ => Err(String::from("invalid hook invocation")),
    }
}

#[cfg(test)]
#[path = "../../../tests/validator/hooks.rs"]
mod tests;
