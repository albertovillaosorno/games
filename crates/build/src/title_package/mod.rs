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
//   - Local packaging of the selected authored native executable.
// - Must-Not:
//   - Package original media, search asset directories or bundle system libs.
// - Allows:
//   - Inputs: Selected title, trusted output directory and locked toolchain.
//   - Outputs: Bounded executable and licence documents with a receipt.
//   - Side effects: Builds the title and creates a new local directory.
// - Split-When:
//   - An independent target requires different binary or packaging formats.
// - Merge-When:
//   - Another tool duplicates authored native executable packaging.
// - Summary:
//   - Stage the release executable and dependency licence texts.
// - Description:
//   - Preflight bounded sources; never overwrite an existing output.
// - Usage:
//   - games-build package TITLE OUTPUT
// - Defaults:
//   - Local GNU/Linux x86-64 package, no assets or portability claim.

mod license_inventory;

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{ErrorKind, Write as _, stdout};
use std::num::NonZeroU64;
use std::os::unix::fs::{
    DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Component, Path, PathBuf};
use std::str;

use games_assets::ContentHash;
use games_platform_storage::file::snapshot;

use self::license_inventory::Inventory;
use crate::title_build::{NATIVE_TARGET, Title, workspace_root};

const MAX_DOCUMENT: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const MAX_EXECUTABLE: NonZeroU64 = NonZeroU64::MIN.saturating_add(33_554_431);
const MAX_FILES: usize = 256;
const MAX_PACKAGE: usize = 67_108_864;
const RECEIPT: &str = "PACKAGE.txt";

#[derive(Debug)]
struct PackageFile {
    bytes: Vec<u8>,
    executable: bool,
}

#[derive(Debug, Default)]
struct Prepared {
    bytes: usize,
    files: BTreeMap<PathBuf, PackageFile>,
}

#[derive(Debug)]
struct PreparedReceipt {
    prepared: Prepared,
    receipt: String,
}

impl Prepared {
    fn add(
        &mut self,
        path: PathBuf,
        bytes: Vec<u8>,
        executable: bool,
    ) -> Result<(), String> {
        if path.as_os_str().is_empty()
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || path == Path::new(RECEIPT)
            || self.files.contains_key(&path)
            || self.files.len() >= MAX_FILES
        {
            return Err(String::from("invalid or excessive package entries"));
        }
        let total = self
            .bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= MAX_PACKAGE)
            .ok_or_else(|| String::from("package byte budget exceeded"))?;
        let _previous =
            self.files.insert(path, PackageFile { bytes, executable });
        self.bytes = total;
        Ok(())
    }

    fn prepare(
        root: &Path,
        title: Title,
        executable: &Path,
        inventory: Inventory,
    ) -> Result<PreparedReceipt, String> {
        let bytes = snapshot(executable, MAX_EXECUTABLE)
            .map_err(|error| format!("authored executable: {error:?}"))?;
        if !native_elf(&bytes) {
            return Err(String::from("expected an x86-64 ELF executable"));
        }
        let receipt = format!(
            "Title: {}\nTarget: {NATIVE_TARGET}\nExecutable bytes: {}\n\
             Executable SHA-256: {}\n\
             Requires compatible GNU/Linux system libraries and SDL2.\n\
             No system libraries, original media or assets included.\n\
             Obtain and import lawful original assets locally.\n\
             This is a local prototype package, not a portable release.\n",
            title.binary(),
            bytes.len(),
            ContentHash::calculate(&bytes),
        );
        let mut prepared = Self::default();
        prepared.add(title.binary().into(), bytes, true)?;
        for name in ["LICENSE", "THIRD_PARTY_NOTICES.md"] {
            prepared.add(name.into(), document(&root.join(name))?, false)?;
        }
        for (destination, source) in inventory.documents {
            prepared.add(destination, document(&source)?, false)?;
        }
        prepared.add(
            "licenses/DEPENDENCIES.tsv".into(),
            inventory.report.into_bytes(),
            false,
        )?;
        Ok(PreparedReceipt { prepared, receipt })
    }

    fn publish(self, output: &Path, receipt: &str) -> Result<(), String> {
        // The caller selects a trusted, stable parent. No existing path is
        // replaced. The receipt is written last; an I/O failure may leave
        // a partial directory; the receipt is written only after all files.
        DirBuilder::new()
            .mode(0o700)
            .create(output)
            .map_err(|error| format!("new package directory: {error}"))?;
        for (path, file) in self.files {
            write_file(&output.join(path), &file.bytes, file.executable)?;
        }
        write_file(&output.join(RECEIPT), receipt.as_bytes(), false)
    }
}

/// Build and package one admitted native title in a new local directory.
///
/// # Errors
/// Rejects build, licence inventory, source admission and filesystem errors.
/// An I/O failure can leave a partial directory; discard it before retrying.
pub fn run(title: Title, output: &Path) -> Result<(), String> {
    match fs::symlink_metadata(output) {
        Ok(_) => {
            return Err(String::from("package destination already exists"));
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {},
        Err(error) => return Err(error.to_string()),
    }
    let root = workspace_root()?;
    let executable = title.compile(&root)?;
    let inventory = Inventory::load(&root, title)?;
    let PreparedReceipt { prepared, receipt } =
        Prepared::prepare(&root, title, &executable, inventory)?;
    prepared.publish(output, &receipt)?;
    writeln!(stdout().lock(), "Package: {}", output.display())
        .map_err(|error| error.to_string())
}

fn document(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = snapshot(path, MAX_DOCUMENT)
        .map_err(|error| format!("licence {}: {error:?}", path.display()))?;
    let text = str::from_utf8(&bytes)
        .map_err(|error| format!("licence {}: {error}", path.display()))?;
    if text.trim().is_empty() || text.contains('\0') {
        return Err(format!("empty or nontext licence: {}", path.display()));
    }
    Ok(bytes)
}

fn native_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 64
        && bytes.starts_with(b"\x7fELF\x02\x01\x01")
        && matches!(bytes.get(16..18), Some([2 | 3, 0]))
        && bytes.get(18..20) == Some(&[62, 0])
}

fn write_file(
    path: &Path,
    bytes: &[u8],
    executable: bool,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| String::from("package entry has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mode = if executable {
        0o700
    } else {
        0o600
    };
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(path)
        .map_err(|error| {
            format!("package entry {}: {error}", path.display())
        })?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.set_permissions(Permissions::from_mode(mode))
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, process};

    use super::*;

    #[derive(Debug)]
    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Result<Self, String> {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            let path = env::temp_dir()
                .join(format!("games-package-{}-{timestamp}", process::id()));
            DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|error| error.to_string())?;
            Ok(Self(path))
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn executable_header_rejects_wrong_target_or_truncation() {
        let mut elf = [0u8; 64];
        if let Some(header) = elf.get_mut(..7) {
            header.copy_from_slice(b"\x7fELF\x02\x01\x01");
        }
        if let Some(header) = elf.get_mut(16..20) {
            header.copy_from_slice(&[3, 0, 62, 0]);
        }
        assert!(native_elf(&elf));
        assert!(!native_elf(b"\x7fELF\x02\x01\x01"));
        if let Some(machine) = elf.get_mut(18) {
            *machine = 183;
        }
        assert!(!native_elf(&elf));
    }

    #[test]
    fn package_never_replaces_an_existing_directory() -> Result<(), String> {
        let root = TestRoot::new()?;
        let output = root.0.join("package");
        let mut prepared = Prepared::default();
        prepared.add("title".into(), vec![1, 2, 3], true)?;
        prepared.add("licenses/MIT".into(), b"text".to_vec(), false)?;
        require(prepared.add("../escape".into(), vec![1], false).is_err())?;
        require(prepared.add("title".into(), vec![4], false).is_err())?;
        prepared.publish(&output, "complete")?;
        let mut replacement = Prepared::default();
        replacement.add("title".into(), vec![4], true)?;
        require(replacement.publish(&output, "replacement").is_err())?;
        require(
            fs::read(output.join("title"))
                .map_err(|error| error.to_string())?
                == [1, 2, 3],
        )?;
        require(
            fs::read(output.join(RECEIPT))
                .map_err(|error| error.to_string())?
                == b"complete",
        )?;
        require(!root.0.join("escape").exists())
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("package contract failed"))
    }
}
