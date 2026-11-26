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
//   - Native reference acquisition and revision-addressed publication.
// - Must-Not:
//   - Admit arbitrary repositories, download games or execute references.
// - Allows:
//   - Inputs: Shared admitted reference requests and revisions.
//   - Outputs: Published paths or typed operation failures.
//   - Side effects: Explicit upstream requests and local reference writes.
// - Split-When:
//   - A separate protocol needs different publication guarantees.
// - Merge-When:
//   - A shared adapter already owns the same acquisition lifecycle.
// - Summary:
//   - Resolve, acquire and publish official contributor references.
// - Description:
//   - Serialize acquisition and publish only after successful retrieval.
// - Usage:
//   - Dispatch through the portable toolchain reference contract.
// - Defaults:
//   - Keep retrieved material in ignored local reference storage.

//! Linux reference acquisition and publication.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ir::security::toolchain::reference::{
    ReferenceKind, ReferenceRevision, ReferenceTool,
};
use crate::ir::toolchain::ToolchainError;

type Outcome<T> = Result<T, ToolchainError>;

pub(super) fn download(
    workspace: &Path,
    tool: ReferenceTool,
) -> Outcome<OsString> {
    let directory = workspace.join(".reference/tools").join(tool.name());
    let _guard = lock(&directory)?;
    let path = match tool.kind() {
        ReferenceKind::Dependencies => {
            dependencies(workspace, &directory)?;
            directory
        },
        kind @ (ReferenceKind::Source(_) | ReferenceKind::Release { .. }) => {
            acquire(&directory, tool, kind)?
        },
    };
    Ok(path.into_os_string())
}

fn acquire(
    directory: &Path,
    tool: ReferenceTool,
    kind: ReferenceKind,
) -> Outcome<PathBuf> {
    let observed = resolve(kind)?;
    let revision = tool
        .revision(observed.trim())
        .map_err(|_error| ToolchainError::InvalidReferenceRevision)?;
    acquire_revision(directory, revision, |stage| match kind {
        ReferenceKind::Source(repo) => extract(repo, revision, stage),
        ReferenceKind::Release { repo, pattern } => {
            release(repo, pattern, revision, stage)
        },
        ReferenceKind::Dependencies => {
            Err(ToolchainError::InvalidReferenceRevision)
        },
    })
}

fn acquire_revision<Fetch>(
    directory: &Path,
    revision: ReferenceRevision<'_>,
    fetch: Fetch,
) -> Outcome<PathBuf>
where
    Fetch: FnOnce(&Path) -> Outcome<()>,
{
    let destination = directory.join(revision.value());
    if !directory_exists(&destination)? {
        let stage = directory.join(".pending");
        if directory_exists(&stage)? {
            fs::remove_dir_all(&stage)
                .map_err(io_failure("remove pending reference"))?;
        }
        fs::create_dir_all(&stage)
            .map_err(io_failure("create pending reference"))?;
        fetch(&stage)?;
        fs::rename(stage, &destination)
            .map_err(io_failure("publish reference directory"))?;
    }
    publish(directory, revision)?;
    Ok(destination)
}

fn directory_exists(path: &Path) -> Outcome<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(ToolchainError::Failed {
            operation: "inspect reference directory",
            diagnostic: "reference path is not a directory".into(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_failure("inspect reference directory")(error)),
    }
}

fn result(command: &mut Command) -> Outcome<()> {
    let status = command.status().map_err(io_failure("launch download"))?;
    if status.success() {
        Ok(())
    } else {
        Err(ToolchainError::Failed {
            operation: "download subprocess",
            diagnostic: status.to_string(),
        })
    }
}

fn lock(directory: &Path) -> Outcome<File> {
    fs::create_dir_all(directory).map_err(io_failure("create reference"))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".download.lock"))
        .map_err(io_failure("open download lock"))?;
    file.lock().map_err(io_failure("lock download"))?;
    Ok(file)
}

fn resolve(kind: ReferenceKind) -> Outcome<String> {
    let mut command = Command::new("gh");
    match kind {
        ReferenceKind::Source(repo) => {
            let _: &mut Command = command.args([
                "api",
                &format!("repos/{repo}/commits/HEAD"),
                "--jq",
                ".sha",
            ]);
        },
        ReferenceKind::Release { repo, .. } => {
            let _: &mut Command = command.args([
                "release", "view", "--repo", repo, "--json", "tagName", "--jq",
                ".tagName",
            ]);
        },
        ReferenceKind::Dependencies => {
            return Err(ToolchainError::InvalidReferenceRevision);
        },
    }
    let output = command.output().map_err(io_failure("resolve reference"))?;
    if !output.status.success() {
        return Err(ToolchainError::Failed {
            operation: "resolve upstream revision",
            diagnostic: output.status.to_string(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|_error| ToolchainError::InvalidReferenceRevision)
}

fn extract(
    repo: &str,
    revision: ReferenceRevision<'_>,
    stage: &Path,
) -> Outcome<()> {
    let archive = stage.join("source.tar.gz");
    let file = File::create(&archive).map_err(io_failure("create archive"))?;
    let url = format!("repos/{repo}/tarball/{}", revision.value());
    result(
        Command::new("gh")
            .args(["api", &url])
            .stdout(Stdio::from(file)),
    )?;
    result(
        Command::new("tar")
            .args([
                "--extract",
                "--gzip",
                "--no-same-owner",
                "--no-same-permissions",
                "--strip-components=1",
                "--file",
            ])
            .arg(&archive)
            .arg("--directory")
            .arg(stage),
    )?;
    fs::remove_file(archive).map_err(io_failure("remove archive"))
}

fn release(
    repo: &str,
    pattern: &str,
    revision: ReferenceRevision<'_>,
    stage: &Path,
) -> Outcome<()> {
    result(
        Command::new("gh")
            .args([
                "release",
                "download",
                revision.value(),
                "--repo",
                repo,
                "--pattern",
                pattern,
                "--dir",
            ])
            .arg(stage),
    )
}

fn publish(directory: &Path, revision: ReferenceRevision<'_>) -> Outcome<()> {
    let pending = directory.join("LATEST.pending");
    match fs::symlink_metadata(&pending) {
        Ok(metadata) if metadata.is_file() => {
            fs::remove_file(&pending)
                .map_err(io_failure("remove stale revision marker"))?;
        },
        Ok(_) => {
            return Err(ToolchainError::Failed {
                operation: "prepare revision marker",
                diagnostic: "pending marker is not a regular file".into(),
            });
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => return Err(io_failure("inspect pending marker")(error)),
    }
    let mut marker = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending)
        .map_err(io_failure("create revision marker"))?;
    writeln!(marker, "{}", revision.value())
        .map_err(io_failure("write reference revision"))?;
    fs::rename(pending, directory.join("LATEST"))
        .map_err(io_failure("publish reference revision"))
}

fn dependencies(workspace: &Path, directory: &Path) -> Outcome<()> {
    let manifest = workspace.join("Cargo.toml");
    result(
        Command::new("cargo")
            .current_dir(workspace)
            .args(["fetch", "--locked", "--manifest-path"])
            .arg(&manifest),
    )?;
    result(
        Command::new("cargo")
            .current_dir(workspace)
            .args([
                "vendor",
                "--locked",
                "--offline",
                "--versioned-dirs",
                "--no-delete",
                "--quiet",
                "--manifest-path",
            ])
            .arg(manifest)
            .arg(directory.join("sources"))
            .stdout(Stdio::null()),
    )
}

fn io_failure(
    operation: &'static str,
) -> impl FnOnce(io::Error) -> ToolchainError {
    move |source| ToolchainError::Io { operation, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::storage::fixtures::FixtureStorage as _;
    use crate::software::linux::storage::fixtures::{TestRoot, file_link};

    fn revision(value: &str) -> Result<ReferenceRevision<'_>, String> {
        ReferenceTool::parse("ghidra-release")
            .ok_or("missing release catalogue entry")?
            .revision(value)
            .map_err(|error| format!("{error:?}"))
    }

    #[test]
    fn failed_acquisition_preserves_current_revision() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let directory = root.path();
        let current = revision("v1")?;
        let next = revision("v2")?;
        let _published = acquire_revision(directory, current, |stage| {
            fs::write(stage.join("source"), b"complete")
                .map_err(io_failure("write fixture"))
        })
        .map_err(|error| error.to_string())?;
        let failure = acquire_revision(directory, next, |stage| {
            fs::write(stage.join("partial"), b"partial")
                .map_err(io_failure("write fixture"))?;
            Err(ToolchainError::InvalidReferenceRevision)
        });
        let marker = fs::read_to_string(directory.join("LATEST"))
            .map_err(|error| error.to_string())?;
        if failure.is_ok() || marker != "v1\n" || directory.join("v2").exists()
        {
            return Err("failed acquisition changed publication".into());
        }
        let published = acquire_revision(directory, next, |stage| {
            if stage.join("partial").exists() {
                return Err(ToolchainError::InvalidReferenceRevision);
            }
            fs::write(stage.join("source"), b"complete")
                .map_err(io_failure("write fixture"))
        })
        .map_err(|error| error.to_string())?;
        if published != directory.join("v2") {
            return Err("incorrect revision directory".into());
        }
        Ok(())
    }

    #[test]
    fn existing_revision_is_reused_without_acquisition() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let directory = root.path();
        let selected = revision("v1")?;
        let published = acquire_revision(directory, selected, |_| Ok(()))
            .map_err(|error| error.to_string())?;
        let reused = acquire_revision(directory, selected, |_| {
            Err(ToolchainError::InvalidReferenceRevision)
        })
        .map_err(|error| error.to_string())?;
        if published != reused {
            return Err("existing revision was not reused".into());
        }
        Ok(())
    }

    #[test]
    fn links_are_not_accepted_as_revision_or_pending_marker()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let directory = root.path();
        let selected = revision("v1")?;
        file_link(directory, &directory.join("v1"))
            .map_err(|error| error.to_string())?;
        if acquire_revision(directory, selected, |_| Ok(())).is_ok() {
            return Err("linked revision was accepted".into());
        }
        root.write_file("sentinel", b"unchanged")
            .map_err(|error| error.to_string())?;
        file_link(
            &directory.join("sentinel"),
            &directory.join("LATEST.pending"),
        )
        .map_err(|error| error.to_string())?;
        if publish(directory, selected).is_ok()
            || fs::read(directory.join("sentinel"))
                .map_err(|error| error.to_string())?
                != b"unchanged"
        {
            return Err("linked marker was followed".into());
        }
        Ok(())
    }
}
