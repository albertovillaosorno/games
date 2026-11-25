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
//   - Official reference downloads and revision selection.
// - Must-Not:
//   - Download original games or execute downloaded tools.
// - Allows:
//   - Inputs: Allowlisted public repositories and release identifiers.
//   - Outputs: Local revision-addressed reference directories.
//   - Side effects: Network requests and ignored reference writes.
// - Split-When:
//   - A separate download protocol needs independent policy.
// - Merge-When:
//   - A shared mechanism covers equivalent responsibilities.
// - Summary:
//   - Official reference downloads and revision selection.
// - Description:
//   - Downloads upstream sources and official tool releases.
// - Usage:
//   - cargo download
// - Defaults:
//   - Fail closed on invalid inputs.
//

//! Fetch official upstream reference snapshots without a Git checkout.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SOURCES: &[(&str, &str)] = &[
    ("craft-libs", "storytold/craft-libs"),
    ("ghidra", "NationalSecurityAgency/ghidra"),
    ("jadx", "skylot/jadx"),
    ("apktool", "iBotPeaches/Apktool"),
    ("mame", "mamedev/mame"),
    ("pcsx2", "PCSX2/pcsx2"),
    ("rpcs3", "RPCS3/rpcs3"),
    ("dolphin", "dolphin-emu/dolphin"),
    ("xemu", "xemu-project/xemu"),
    ("xenia", "xenia-project/xenia"),
    ("melonds", "melonDS-emu/melonDS"),
];

const RELEASES: &[Release] = &[
    (
        "ghidra-release",
        "NationalSecurityAgency/ghidra",
        "ghidra_*_PUBLIC_*.zip",
    ),
    ("jadx-release", "skylot/jadx", "jadx-[0-9]*.zip"),
    ("apktool-release", "iBotPeaches/Apktool", "apktool_*.jar"),
];

type Release = (&'static str, &'static str, &'static str);

fn result(command: &mut Command) -> Result<(), String> {
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("download subprocess failed: {status}"))
    }
}

fn lock(directory: &Path) -> Result<File, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".download.lock"))
        .map_err(|error| error.to_string())?;
    file.lock().map_err(|error| error.to_string())?;
    Ok(file)
}

fn revision(repo: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["api", &format!("repos/{repo}/commits/HEAD"), "--jq", ".sha"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("cannot resolve upstream revision: {repo}"));
    }
    let hash = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_owned();
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(String::from("invalid upstream commit hash"));
    }
    Ok(hash)
}

fn extract(repo: &str, hash: &str, stage: &Path) -> Result<(), String> {
    fs::create_dir_all(stage).map_err(|error| error.to_string())?;
    let archive = stage.join("source.tar.gz");
    let file = File::create(&archive).map_err(|error| error.to_string())?;
    let url = format!("repos/{repo}/tarball/{hash}");
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
    fs::remove_file(archive).map_err(|error| error.to_string())
}

fn publish(directory: &Path, revision: &str) -> Result<(), String> {
    let pending = directory.join("LATEST.pending");
    fs::write(&pending, format!("{revision}\n"))
        .map_err(|error| error.to_string())?;
    fs::rename(pending, directory.join("LATEST"))
        .map_err(|error| error.to_string())
}

fn source(root: &Path, name: &str, repo: &str) -> Result<(), String> {
    let directory = root.join(name);
    let _guard = lock(&directory)?;
    let hash = revision(repo)?;
    let destination = directory.join(&hash);
    if !destination.exists() {
        let stage = directory.join(".pending");
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|error| error.to_string())?;
        }
        extract(repo, &hash, &stage)?;
        fs::rename(stage, &destination).map_err(|error| error.to_string())?;
    }
    publish(&directory, &hash)?;
    writeln!(stdout().lock(), "{name}: {}", destination.display())
        .map_err(|error| error.to_string())
}

fn release(
    root: &Path,
    name: &str,
    repo: &str,
    pattern: &str,
) -> Result<(), String> {
    let directory = root.join(name);
    let _guard = lock(&directory)?;
    let output = Command::new("gh")
        .args([
            "release", "view", "--repo", repo, "--json", "tagName", "--jq",
            ".tagName",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("cannot resolve release: {repo}"));
    }
    let tag = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_owned();
    if tag.is_empty()
        || tag.starts_with('.')
        || !tag.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(String::from("invalid release tag"));
    }
    let destination = directory.join(&tag);
    if !destination.exists() {
        let stage = directory.join(".pending");
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|error| error.to_string())?;
        }
        fs::create_dir_all(&stage).map_err(|error| error.to_string())?;
        result(
            Command::new("gh")
                .args([
                    "release",
                    "download",
                    &tag,
                    "--repo",
                    repo,
                    "--pattern",
                    pattern,
                    "--dir",
                ])
                .arg(&stage),
        )?;
        fs::rename(stage, &destination).map_err(|error| error.to_string())?;
    }
    publish(&directory, &tag)?;
    writeln!(stdout().lock(), "{name}: {}", destination.display())
        .map_err(|error| error.to_string())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.reference/tools")
}

fn main() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    let directory = root();
    if args.is_empty() {
        for (name, repo) in SOURCES {
            source(&directory, name, repo)?;
        }
        for (name, repo, pattern) in RELEASES {
            release(&directory, name, repo, pattern)?;
        }
        return Ok(());
    }
    for argument in args {
        if let Some((name, repo)) =
            SOURCES.iter().find(|(name, _)| *name == argument)
        {
            source(&directory, name, repo)?;
        } else if let Some((name, repo, pattern)) =
            RELEASES.iter().find(|(name, _, _)| *name == argument)
        {
            release(&directory, name, repo, pattern)?;
        } else {
            return Err(format!("unknown reference tool: {argument}"));
        }
    }
    Ok(())
}
