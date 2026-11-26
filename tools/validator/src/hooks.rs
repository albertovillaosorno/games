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
//   - Staged-path, commit-message and history checks.
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
//   - Contribution and commit history checks.
// - Description:
//   - Checks staged paths, Conventional Commits and DCO trailers.
// - Usage:
//   - Called by cargo validate and Git hooks.
// - Defaults:
//   - Fail if any required check fails.
//

//! Validate prospective commit messages and committed history.

/// Isolated test fixture helpers.
#[cfg(test)]
pub mod fixtures;

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::str::from_utf8;

use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::{PathKind, path_kind};
use games_platform::ir::toolchain::validation::{
    GitRequest, Request, repository_head, run,
};
use games_platform::ir::toolchain::{arguments, workspace_root};

type DiffPaths<'path> = Vec<&'path [u8]>;

fn root() -> Result<PathBuf, String> {
    workspace_root()
        .map(PathBuf::from)
        .map_err(|error| error.to_string())
}

fn scope_valid(scope: &str) -> bool {
    !scope.is_empty()
        && scope.split('-').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit()
                })
        })
}

fn type_valid(kind: &str) -> bool {
    matches!(
        kind,
        "build"
            | "chore"
            | "ci"
            | "docs"
            | "feat"
            | "fix"
            | "perf"
            | "refactor"
            | "release"
            | "revert"
            | "style"
            | "test"
    )
}

fn subject_valid(subject: &str) -> Result<(), String> {
    let (raw_kind, description) = subject
        .split_once(": ")
        .ok_or("expected type(scope): description")?;
    let kind = raw_kind.strip_suffix('!').unwrap_or(raw_kind);
    let (commit_type, scope) =
        kind.split_once('(').ok_or("commit scope is required")?;
    if !scope.strip_suffix(')').is_some_and(scope_valid) {
        return Err("invalid Conventional Commit scope".into());
    }
    if !type_valid(commit_type)
        || subject.chars().count() > 72
        || description.is_empty()
        || description.trim() != description
        || description.ends_with('.')
        || description.contains("  ")
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
    let bytes =
        snapshot(path.as_os_str(), NonZeroU64::MIN.saturating_add(1_048_575))
            .map_err(|error| format!("commit message: {error:?}"))?;
    let message = from_utf8(&bytes).map_err(|error| error.to_string())?;
    check_commit(message)
}

fn title_data(parts: &[&str]) -> bool {
    let index = match parts {
        ["crates", "games" | "apps", _, ..] => 3,
        ["games" | "apps", _, ..] => 2,
        ["crates", name, ..]
            if name.starts_with("game-") || name.starts_with("app-") =>
        {
            2
        },
        _ => return false,
    };
    matches!(parts.get(index), Some(&("assets" | "saves")))
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
    ) || title_data(&parts)
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

fn path_list(bytes: &[u8]) -> Result<DiffPaths<'_>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<_> = bytes
        .strip_suffix(&[0])
        .ok_or("path list lacks final NUL")?
        .split(|byte| *byte == 0)
        .collect();
    if paths.iter().any(|path| path.is_empty()) {
        return Err("empty path in diff".into());
    }
    Ok(paths)
}

fn check_paths(bytes: &[u8]) -> Result<(), String> {
    for raw in path_list(bytes)? {
        let path = from_utf8(raw).map_err(|error| error.to_string())?;
        if prohibited_path(path) {
            return Err(format!("prohibited staged path: {path}"));
        }
    }
    Ok(())
}

fn changed_paths(root: &Path, request: GitRequest) -> Result<Vec<u8>, String> {
    let output = run(root.as_os_str(), Request::Git(request))
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err("cannot inspect changed paths".into());
    }
    Ok(output.stdout)
}

fn check_worktree(staged: &[u8], unstaged: &[u8]) -> Result<(), String> {
    let dirty: BTreeSet<_> = path_list(unstaged)?.into_iter().collect();
    for path in path_list(staged)? {
        if dirty.contains(path) {
            return Err(format!(
                "staged file has unvalidated worktree changes: {}",
                String::from_utf8_lossy(path)
            ));
        }
    }
    Ok(())
}

fn check_deleted(root: &Path, deleted: &[u8]) -> Result<(), String> {
    for raw in path_list(deleted)? {
        let path = from_utf8(raw).map_err(|error| error.to_string())?;
        if !relative_path(path) {
            return Err("invalid deleted path".into());
        }
        match path_kind(root.join(path).as_os_str()) {
            Ok(PathKind::Directory | PathKind::Missing) => {},
            Ok(_) => {
                return Err(format!("staged deletion still exists: {path}"));
            },
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::NotADirectory
                ) => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn check_staged() -> Result<(), String> {
    let root = root()?;
    let staged = changed_paths(&root, GitRequest::ChangedStaged)?;
    check_paths(&staged)?;
    check_worktree(&staged, &changed_paths(&root, GitRequest::Unstaged)?)?;
    check_deleted(&root, &changed_paths(&root, GitRequest::DeletedStaged)?)
}

fn check_history_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let records = bytes.strip_suffix(&[0]).ok_or("invalid history framing")?;
    for record in records.split(|byte| *byte == 0) {
        let text = from_utf8(record).map_err(|error| error.to_string())?;
        let (id, message) = text.split_once('\n').ok_or("invalid history")?;
        if id.len() != 40 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid commit identity".into());
        }
        check_commit(message).map_err(|error| format!("{id}: {error}"))?;
    }
    Ok(())
}

fn check_history() -> Result<(), String> {
    let root = root()?;
    match path_kind(root.join(".git").as_os_str())
        .map_err(|error| error.to_string())?
    {
        PathKind::Missing => return Ok(()),
        PathKind::Directory | PathKind::RegularFile => {},
        PathKind::Other | PathKind::SymbolicLink => {
            return Err(String::from("invalid Git repository entry"));
        },
    }
    if repository_head(root.as_os_str())
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Ok(());
    }
    let history = run(root.as_os_str(), Request::Git(GitRequest::History))
        .map_err(|error| error.to_string())?;
    if !history.success {
        return Err("cannot read commit history".into());
    }
    check_history_bytes(&history.stdout)
}

fn main() -> Result<(), String> {
    let args = arguments().map_err(|error| error.to_string())?;
    match args.as_slice() {
        [mode] if mode == "staged" => check_staged(),
        [mode] if mode == "history" => check_history(),
        [mode, path] if mode == "commit-message" => {
            check_message_file(Path::new(path))
        },
        _ => Err(String::from("invalid hook invocation")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use games_platform::ir::storage::fixtures::{
        self as platform_fixtures, TestRoot,
    };
    use games_platform::ir::toolchain::validation::fixtures::{
        FixtureDelegate, FixtureHook, hook_syntax, run_hook,
    };

    use super::{
        check_commit, check_deleted, check_history_bytes, check_message_file,
        check_paths, check_worktree,
    };
    use crate::fixtures::{directory, read, require};

    const SIGNED: &str = "Signed-off-by: Example <example@example.com>";

    #[test]
    fn accepts_conventional_subjects_and_final_trailers() {
        for subject in [
            "feat(engine): add capability",
            "fix(input-gamepad)!: change mapping",
            "docs(contracts): describe contracts",
        ] {
            let message = format!(
                "{subject}\n\nBody text.\n\nRefs: #42\n{SIGNED}\n\n# status"
            );
            assert!(check_commit(&message).is_ok(), "{subject}");
        }
    }

    #[test]
    fn rejects_malformed_subjects() {
        for subject in [
            "changes",
            "feat: changes",
            "Feat: changes",
            "feat(Engine): changes",
            "feat(input/gamepad): changes",
            "feat(engine--input): changes",
            "unknown(engine): changes",
            "feat(engine): changes.",
            "feat(engine): two  spaces",
            "feat(): changes",
            "feat(a(b)): changes",
            "feat( ): changes",
            "feat!!: changes",
            "feat: ",
            "feat:  changes",
        ] {
            let message = format!("{subject}\n\n{SIGNED}");
            assert!(check_commit(&message).is_err(), "{subject}");
        }
    }

    #[test]
    fn history_uses_the_same_required_scope() {
        let id = "a".repeat(40);
        let valid = format!("{id}\nfeat(engine): change\n\n{SIGNED}\n\0");
        assert!(check_history_bytes(valid.as_bytes()).is_ok());
        let invalid = valid.replace("feat(engine):", "feat:");
        assert!(check_history_bytes(invalid.as_bytes()).is_err());
        assert!(
            check_history_bytes(valid.trim_end_matches('\0').as_bytes())
                .is_err()
        );
    }

    #[test]
    fn rejects_overlong_subjects() {
        let subject = format!("feat(engine): {}", "a".repeat(60));
        let message = format!("{subject}\n\n{SIGNED}");
        assert!(check_commit(&message).is_err());
    }

    #[test]
    fn rejects_missing_or_invalid_dco_trailers() {
        for identity in [
            "Example <@example.com>",
            "Example <example@>",
            "Example <example@@example.com>",
            "Example <example @example.com>",
            "Example <example@example.com>>",
            " <example@example.com>",
            "Example <example@example.com> trailing",
        ] {
            let message =
                format!("feat(engine): change\n\nSigned-off-by: {identity}");
            assert!(check_commit(&message).is_err(), "{identity}");
        }
        for message in [
            String::from("feat(engine): change"),
            format!("feat(engine): change\n{SIGNED}"),
            format!("feat(engine): change\n\n{SIGNED}\n\nNot a trailer."),
            format!("feat(engine): change\n\n{SIGNED}\nSigned-off-by: Invalid"),
        ] {
            assert!(check_commit(&message).is_err(), "{message}");
        }
    }

    #[test]
    fn distinguishes_staged_local_data_from_source() {
        for path in [
            ".local/originals/input",
            ".reference/games/test/input",
            ".cache/validate/success",
            ".temp/validate/run.log",
            ".git/config",
            ".env",
            "crates/test/.env.production",
            "crates/test/.reference/original",
            "credentials.pem",
            "credentials.key",
            "credentials.p12",
            "credentials.pfx",
            "games/test/assets/image.png",
            "crates/game-test/assets/image.png",
            "crates/games/test/assets/image.png",
            "crates/apps/sush/saves/state",
            "crates/app-sush/saves/state",
            "apps/sush/saves/state",
            "game.APK",
            "games.md",
            "game.rep/data",
            "../originals/file",
            "/absolute/path",
            "games/../README.md",
            "games//file",
            "C:/file",
            "games\\file",
        ] {
            let bytes = format!("{path}\0");
            assert!(check_paths(bytes.as_bytes()).is_err(), "{path}");
        }
        let accepted = concat!(
            "crates/assets/src/lib.rs\0",
            "crates/persistence/src/saves/mod.rs\0",
            "games/test/src/lib.rs\0",
            "docs/file with spaces.md\0",
            ".env.example\0",
        );
        assert!(
            check_paths(accepted.as_bytes()).is_ok(),
            "authored paths are distributable"
        );
    }

    #[test]
    fn rejects_malformed_staged_path_lists() {
        for bytes in [
            b"no-final-nul".as_slice(),
            b"\0".as_slice(),
            b"README.md\0\0".as_slice(),
            b"\xff\0".as_slice(),
        ] {
            assert!(check_paths(bytes).is_err(), "invalid framing");
        }
        assert!(check_paths(b"").is_ok(), "empty staged list");
    }

    #[test]
    fn rejects_partial_staging_without_blocking_unrelated_edits() {
        let staged = b"docs/a file.md\0crates/engine/src/lib.rs\0";
        assert!(check_worktree(staged, b"docs/other.md\0").is_ok());
        assert!(check_worktree(staged, b"docs/a file.md\0").is_err());
        assert!(check_worktree(staged, b"crates/engine/src/lib.rs\0").is_err());
        assert!(check_worktree(staged, b"invalid framing").is_err());
    }

    #[test]
    fn rejects_restored_staged_deletions() -> Result<(), String> {
        let fixture_owner = directory()?;
        let fixture = Path::new(fixture_owner.location());
        let deleted = b"old.md\0";
        check_deleted(fixture, deleted)?;
        write(&fixture_owner, "old.md", b"restored")?;
        require(
            check_deleted(fixture, deleted).is_err(),
            "restored deletion",
        )?;
        Ok(())
    }

    fn write(root: &TestRoot, name: &str, bytes: &[u8]) -> Result<(), String> {
        root.write_file(name, bytes)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn exercises_isolated_hook_fixtures() -> Result<(), String> {
        let fixture_owner = directory()?;
        let fixture = Path::new(fixture_owner.location());
        let message = fixture.join("COMMIT_MSG");
        write(
            &fixture_owner,
            "COMMIT_MSG",
            format!("feat(engine): change\n\n{SIGNED}").as_bytes(),
        )?;
        check_message_file(&message)?;
        write(&fixture_owner, "COMMIT_MSG", b"feat(engine): change")?;
        require(
            check_message_file(&message).is_err(),
            "invalid commit message file",
        )?;

        let staged = fixture.join("staged-paths");
        write(
            &fixture_owner,
            "staged-paths",
            b"README.md\0crates/engine/src/lib.rs\0",
        )?;
        let valid = read(&staged)?;
        check_paths(&valid)?;
        write(
            &fixture_owner,
            "staged-paths",
            b"README.md\0.reference/games/title/input\0",
        )?;
        let blocked = read(&staged)?;
        require(check_paths(&blocked).is_err(), "proprietary staged file")?;

        for hook in [FixtureHook::PreCommit, FixtureHook::CommitMessage] {
            require(
                hook_syntax(hook).map_err(|error| error.to_string())?,
                "hook dispatch shell syntax",
            )?;
        }
        Ok(())
    }

    fn executable(
        root: &TestRoot,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        write(root, name, bytes)?;
        root.make_executable(name)
            .map_err(|error| error.to_string())
    }

    fn hook_dispatch(
        fixture: &TestRoot,
        hook: FixtureHook,
        failure: FixtureDelegate,
        expected: &str,
    ) -> Result<(), String> {
        let calls = Path::new(fixture.location()).join("calls");
        write(fixture, "calls", b"")?;
        let status = run_hook(fixture, hook, failure)
            .map_err(|error| error.to_string())?;
        require(
            status == (failure == FixtureDelegate::None),
            "hook must propagate delegate failure",
        )?;
        let actual = read(&calls)?;
        require(
            actual == expected.as_bytes(),
            "hook delegate order or arguments",
        )
    }

    #[test]
    fn hooks_execute_delegates_and_fail_closed() -> Result<(), String> {
        if !platform_fixtures::AVAILABLE {
            return Err(String::from("native fixture capability unavailable"));
        }
        let fixture_owner = directory()?;
        let fixture = Path::new(fixture_owner.location());
        executable(
            &fixture_owner,
            "git",
            b"#!/bin/sh\nprintf '%s\\n' \"$HOOK_FIXTURE\"\n",
        )?;
        executable(
            &fixture_owner,
            "cargo",
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' \"$*\" >> \"$HOOK_FIXTURE/calls\"\n",
                "for arg do\n",
                "  if [ \"$arg\" = \"$HOOK_FAIL\" ]; then exit 1; fi\n",
                "done\n",
            )
            .as_bytes(),
        )?;
        let staged = "run --locked -q -p games-validator --bin games-hooks";
        let first = format!("{staged} -- staged\n");
        let both = format!("{first}validate\n");
        hook_dispatch(
            &fixture_owner,
            FixtureHook::PreCommit,
            FixtureDelegate::None,
            &both,
        )?;
        hook_dispatch(
            &fixture_owner,
            FixtureHook::PreCommit,
            FixtureDelegate::Staged,
            &first,
        )?;
        hook_dispatch(
            &fixture_owner,
            FixtureHook::PreCommit,
            FixtureDelegate::Validate,
            &both,
        )?;
        let message = format!(
            "{staged} -- commit-message {}/COMMIT_MSG\n",
            fixture.display()
        );
        hook_dispatch(
            &fixture_owner,
            FixtureHook::CommitMessage,
            FixtureDelegate::None,
            &message,
        )?;
        hook_dispatch(
            &fixture_owner,
            FixtureHook::CommitMessage,
            FixtureDelegate::CommitMessage,
            &message,
        )?;
        Ok(())
    }
}
