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
use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
use std::str;

use games_assets::ContentHash;
use games_assets::archive::validate_path;
use games_platform::ir::security::toolchain::artifact::admit_executable;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::staging::AuthoredStaging;
use games_platform::ir::storage::{
    PathKind, parent_path, path_is_absolute, path_kind, path_leaf,
};
use games_platform::ir::toolchain::write_line;

use self::license_inventory::Inventory;
use crate::title_build::{
    NATIVE_TARGET, Title, local_layout, local_path, workspace_root,
};

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
    files: BTreeMap<String, PackageFile>,
}

impl Prepared {
    fn add(
        &mut self,
        path: String,
        bytes: Vec<u8>,
        executable: bool,
    ) -> Result<(), String> {
        validate_path(&path)
            .map_err(|error| format!("package entry {path}: {error:?}"))?;
        if self.files.contains_key(&path) || self.files.len() >= MAX_FILES {
            return Err(String::from("invalid or excessive package entries"));
        }
        let total = self.admit_length(bytes.len())?;
        let _previous =
            self.files.insert(path, PackageFile { bytes, executable });
        self.bytes = total;
        Ok(())
    }

    fn admit_length(&self, length: usize) -> Result<usize, String> {
        self.bytes
            .checked_add(length)
            .filter(|total| *total <= MAX_PACKAGE)
            .ok_or_else(|| String::from("package byte budget exceeded"))
    }

    fn prepare(
        root: &OsStr,
        title: Title,
        executable: &OsStr,
        inventory: Inventory,
    ) -> Result<Self, String> {
        let bytes = snapshot(executable, MAX_EXECUTABLE)
            .map_err(|error| format!("authored executable: {error:?}"))?;
        admit_executable(NATIVE_TARGET, &bytes)
            .map_err(|error| error.to_string())?;
        let receipt = format!(
            "Title: {}\nTarget: {}\nExecutable bytes: {}\n\
             Executable SHA-256: {}\n\
             Requires compatible GNU/Linux system libraries and SDL2.\n\
             No system libraries, original media or assets included.\n\
             Obtain and import lawful original assets locally.\n\
             This is a local prototype package, not a portable release.\n",
            title.binary(),
            NATIVE_TARGET.triple(),
            bytes.len(),
            ContentHash::calculate(&bytes),
        );
        let mut prepared = Self::default();
        prepared.add(RECEIPT.into(), receipt.into_bytes(), false)?;
        prepared.add(title.binary().into(), bytes, true)?;
        for name in ["LICENSE", "THIRD_PARTY_NOTICES.md"] {
            prepared.add(
                name.into(),
                document(&local_path(root, name)?)?,
                false,
            )?;
        }
        for (destination, source) in inventory.documents {
            prepared.add(destination, document(&source)?, false)?;
        }
        prepared.add(
            "licenses/DEPENDENCIES.tsv".into(),
            inventory.report.into_bytes(),
            false,
        )?;
        Ok(prepared)
    }

    fn publish(self, output: &OsStr) -> Result<(), String> {
        if self.files.get(RECEIPT).is_none_or(|file| file.executable) {
            return Err(String::from("missing or executable package receipt"));
        }
        let name = path_leaf(output)
            .map_err(|error| format!("package destination name: {error}"))?;
        let parent = parent_path(output).map_err(|error| error.to_string())?;
        let destination_parent = if parent.is_empty() {
            OsStr::new(".")
        } else {
            parent
        };
        let mut stage = AuthoredStaging::new(destination_parent)
            .map_err(|error| format!("package stage: {error:?}"))?;
        for (path, file) in self.files {
            stage
                .write_file(&path, &file.bytes, file.executable)
                .map_err(|error| format!("package entry {path}: {error:?}"))?;
        }
        let published = stage
            .publish(name)
            .map_err(|error| format!("package publish: {error:?}"))?;
        published.durability.map_err(|error| {
            format!(
                "package published at {}, but directory sync failed: {error:?}",
                published.path.to_string_lossy()
            )
        })
    }
}

/// Package into the operator-selected persistent deployment directory.
///
/// Asset files remain external and are never included in the binary package.
/// # Errors
/// Rejects missing or invalid local settings and unavailable output parents.
pub fn configured(title: Title) -> Result<(), String> {
    let root = workspace_root()?;
    let layout = local_layout(&root)?;
    if layout.target() != NATIVE_TARGET {
        return Err(String::from("configured package target is unsupported"));
    }
    let output = OsString::from(layout.binaries(title.binary())?);
    let assets = OsString::from(layout.assets(title.binary())?);
    if !path_is_absolute(&output).map_err(|error| error.to_string())?
        || !path_is_absolute(&assets).map_err(|error| error.to_string())?
    {
        return Err(String::from("persistent roots must be absolute"));
    }
    let parent = parent_path(&output).map_err(|error| error.to_string())?;
    if path_kind(parent).map_err(|error| error.to_string())?
        != PathKind::Directory
    {
        return Err(String::from("create the configured bin directory first"));
    }
    run(title, &output)?;
    write_line(&format!("Shared assets: {}", assets.to_string_lossy()))
        .map_err(|error| error.to_string())
}

/// Build and package one admitted native title in a new local directory.
///
/// # Errors
/// Rejects build, licence inventory, source admission and filesystem errors.
/// Publication is atomic; a post-rename sync failure is reported separately.
pub fn run(title: Title, output: &OsStr) -> Result<(), String> {
    if path_kind(output).map_err(|error| error.to_string())?
        != PathKind::Missing
    {
        return Err(String::from("package destination already exists"));
    }
    let root = workspace_root()?;
    let executable = title.compile(&root)?;
    let inventory = Inventory::load(&root, title)?;
    let prepared = Prepared::prepare(&root, title, &executable, inventory)?;
    prepared.publish(output)?;
    write_line(&format!("Package: {}", output.to_string_lossy()))
        .map_err(|error| error.to_string())
}

fn document(path: &OsStr) -> Result<Vec<u8>, String> {
    let bytes = snapshot(path, MAX_DOCUMENT).map_err(|error| {
        format!("licence {}: {error:?}", path.to_string_lossy())
    })?;
    let text = str::from_utf8(&bytes).map_err(|error| {
        format!("licence {}: {error}", path.to_string_lossy())
    })?;
    if text.trim().is_empty() || text.contains('\0') {
        return Err(format!(
            "empty or nontext licence: {}",
            path.to_string_lossy()
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use games_platform::ir::storage::fixtures::TestRoot;

    use super::*;

    #[test]
    fn receipt_is_included_in_package_file_and_byte_budgets()
    -> Result<(), String> {
        let mut prepared = Prepared::default();
        prepared.add(RECEIPT.into(), b"receipt".to_vec(), false)?;
        require(prepared.admit_length(MAX_PACKAGE - 7) == Ok(MAX_PACKAGE))?;
        require(prepared.admit_length(MAX_PACKAGE - 6).is_err())?;
        require(prepared.admit_length(usize::MAX).is_err())?;
        require(prepared.add(RECEIPT.into(), vec![], false).is_err())?;
        for index in 1..MAX_FILES {
            prepared.add(format!("entry-{index}"), vec![], false)?;
        }
        require(prepared.add("overflow".into(), vec![], false).is_err())
    }

    #[test]
    fn package_requires_a_receipt_and_rejects_nonportable_paths()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let output = local_path(root.location(), "package")?;
        let mut prepared = Prepared::default();
        for path in [
            "licenses//MIT",
            "licenses/./MIT",
            "licenses/../MIT",
            "licenses/CON.txt",
            "licenses/invalid:name",
            "licenses/trailing.",
            "/absolute",
        ] {
            require(prepared.add(path.into(), vec![], false).is_err())?;
        }
        prepared.add("title".into(), vec![1], true)?;
        require(prepared.publish(&output).is_err())?;
        let mut executable_receipt = Prepared::default();
        executable_receipt.add(RECEIPT.into(), b"receipt".to_vec(), true)?;
        require(executable_receipt.publish(&output).is_err())?;
        require(root.entry_count().map_err(|error| error.to_string())? == 0)
    }

    #[test]
    fn package_never_replaces_an_existing_directory() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let output = local_path(root.location(), "package")?;
        let mut prepared = Prepared::default();
        prepared.add("title".into(), vec![1, 2, 3], true)?;
        prepared.add("licenses/MIT".into(), b"text".to_vec(), false)?;
        require(prepared.add("../escape".into(), vec![1], false).is_err())?;
        require(prepared.add("title".into(), vec![4], false).is_err())?;
        prepared.add(RECEIPT.into(), b"complete".to_vec(), false)?;
        prepared.publish(&output)?;
        let mut replacement = Prepared::default();
        replacement.add("title".into(), vec![4], true)?;
        replacement.add(RECEIPT.into(), b"replacement".to_vec(), false)?;
        require(replacement.publish(&output).is_err())?;
        require(
            snapshot(&local_path(&output, "title")?, MAX_DOCUMENT)
                .map_err(|error| format!("{error:?}"))?
                == [1, 2, 3],
        )?;
        require(
            snapshot(&local_path(&output, RECEIPT)?, MAX_DOCUMENT)
                .map_err(|error| format!("{error:?}"))?
                == b"complete",
        )?;
        require(
            path_kind(&local_path(root.location(), "escape")?)
                .map_err(|error| error.to_string())?
                == PathKind::Missing,
        )?;
        require(root.entry_count().map_err(|error| error.to_string())? == 1)
    }

    #[test]
    fn failed_write_does_not_publish_partial_package() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let output = local_path(root.location(), "package")?;
        let mut prepared = Prepared::default();
        prepared.add("licenses".into(), vec![1], false)?;
        prepared.add("licenses/MIT".into(), vec![2], false)?;
        prepared.add(RECEIPT.into(), b"incomplete".to_vec(), false)?;
        require(prepared.publish(&output).is_err())?;
        require(
            path_kind(&output).map_err(|error| error.to_string())?
                == PathKind::Missing,
        )?;
        require(root.entry_count().map_err(|error| error.to_string())? == 0)
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("package contract failed"))
    }
}
