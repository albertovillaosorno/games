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
//   - Dormant contribution hook tests.
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
//   - Dormant contribution hook tests.
// - Description:
//   - Tests commit and staged-path policies without Git.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Reject invalid messages and distribution paths.

use std::fs;
use std::path::Path;
use std::process::Command;

use super::{check_commit, check_message_file, check_paths, root};
use crate::fixtures::{directory, require};

const SIGNED: &str = "Signed-off-by: Example <example@example.com>";

#[test]
fn accepts_conventional_subjects_and_final_trailers() {
    for subject in [
        "feat(engine): add capability",
        "fix(input/gamepad)!: change mapping",
        "docs: describe contracts",
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
        "Feat: changes",
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
        let message = format!("feat: change\n\nSigned-off-by: {identity}");
        assert!(check_commit(&message).is_err(), "{identity}");
    }
    for message in [
        String::from("feat: change"),
        format!("feat: change\n{SIGNED}"),
        format!("feat: change\n\n{SIGNED}\n\nNot a trailer."),
        format!("feat: change\n\n{SIGNED}\nSigned-off-by: Invalid"),
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

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[test]
fn exercises_isolated_hook_fixtures() -> Result<(), String> {
    let fixture = directory("hooks")?;
    let message = fixture.join("COMMIT_MSG");
    write(&message, format!("feat: change\n\n{SIGNED}").as_bytes())?;
    check_message_file(&message)?;
    write(&message, b"feat: change")?;
    require(
        check_message_file(&message).is_err(),
        "invalid commit message file",
    )?;

    let staged = fixture.join("staged-paths");
    write(&staged, b"README.md\0crates/engine/src/lib.rs\0")?;
    let valid = fs::read(&staged).map_err(|error| error.to_string())?;
    check_paths(&valid)?;
    write(&staged, b"README.md\0.reference/games/title/input\0")?;
    let blocked = fs::read(&staged).map_err(|error| error.to_string())?;
    require(check_paths(&blocked).is_err(), "proprietary staged file")?;

    for hook in ["pre-commit", "commit-msg"] {
        let status = Command::new("sh")
            .arg("-n")
            .arg(root().join(".githooks").join(hook))
            .status()
            .map_err(|error| error.to_string())?;
        require(status.success(), "hook dispatch shell syntax")?;
    }
    fs::remove_dir_all(fixture).map_err(|error| error.to_string())
}
