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
#[cfg(test)]
use std::fs::DirBuilder;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{ErrorKind, Write as _, stdout};
use std::num::NonZeroU64;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};
use std::str;

use games_assets::ContentHash;
use games_platform_storage::file::snapshot;
use games_platform_storage::staging::AuthoredStaging;

use self::license_inventory::Inventory;
use crate::title_build::{NATIVE_TARGET, Title, workspace_root};

const ELF_HEADER: usize = 64;
const ELF_PROGRAM_HEADER: usize = 56;
const MAX_DOCUMENT: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const MAX_EXECUTABLE: NonZeroU64 = NonZeroU64::MIN.saturating_add(33_554_431);
const MAX_FILES: usize = 256;
const MAX_PACKAGE: usize = 67_108_864;
const RECEIPT: &str = "PACKAGE.txt";
type ProgramHeaders<'file> = (u64, &'file [[u8; ELF_PROGRAM_HEADER]]);

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
        let name = output
            .file_name()
            .ok_or("package needs a destination name")?;
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let stage = AuthoredStaging::new(parent)
            .map_err(|error| format!("package stage: {error:?}"))?;
        for (path, file) in self.files {
            let target = stage.path().join(path);
            write_file(&target, &file.bytes, file.executable)?;
            for directory in target
                .ancestors()
                .skip(1)
                .take_while(|directory| *directory != stage.path())
            {
                File::open(directory)
                    .and_then(|handle| handle.sync_all())
                    .map_err(|error| error.to_string())?;
            }
        }
        write_file(&stage.path().join(RECEIPT), receipt.as_bytes(), false)?;
        let published = stage
            .publish(name)
            .map_err(|error| format!("package publish: {error:?}"))?;
        published.durability.map_err(|error| {
            format!(
                "package published at {}, but directory sync failed: {error:?}",
                published.path.display()
            )
        })
    }
}

/// Build and package one admitted native title in a new local directory.
///
/// # Errors
/// Rejects build, licence inventory, source admission and filesystem errors.
/// Publication is atomic; a post-rename sync failure is reported separately.
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

fn elf_field<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

fn elf_program_table(bytes: &[u8]) -> Option<ProgramHeaders<'_>> {
    if !bytes.starts_with(b"\x7fELF\x02\x01\x01")
        || !matches!(bytes.get(16..18), Some([2 | 3, 0]))
        || bytes.get(18..24) != Some(&[62, 0, 1, 0, 0, 0])
        || usize::from(u16::from_le_bytes(elf_field(bytes, 52)?)) != ELF_HEADER
        || usize::from(u16::from_le_bytes(elf_field(bytes, 54)?))
            != ELF_PROGRAM_HEADER
    {
        return None;
    }
    let entry = u64::from_le_bytes(elf_field(bytes, 24)?);
    let offset =
        usize::try_from(u64::from_le_bytes(elf_field(bytes, 32)?)).ok()?;
    let count = u16::from_le_bytes(elf_field(bytes, 56)?);
    if entry == 0 || offset < ELF_HEADER || matches!(count, 0 | u16::MAX) {
        return None;
    }
    let end = offset
        .checked_add(usize::from(count).checked_mul(ELF_PROGRAM_HEADER)?)?;
    let programs = bytes.get(offset..end)?.as_chunks::<ELF_PROGRAM_HEADER>().0;
    Some((entry, programs))
}

fn load_contains_entry(
    program: &[u8],
    entry: u64,
    length: u64,
) -> Option<bool> {
    let kind = u32::from_le_bytes(elf_field(program, 0)?);
    if kind != 1 {
        return Some(false);
    }
    let flags = u32::from_le_bytes(elf_field(program, 4)?);
    let file = u64::from_le_bytes(elf_field(program, 8)?);
    let address = u64::from_le_bytes(elf_field(program, 16)?);
    let size = u64::from_le_bytes(elf_field(program, 32)?);
    let memory = u64::from_le_bytes(elf_field(program, 40)?);
    let alignment = u64::from_le_bytes(elf_field(program, 48)?);
    if size > memory
        || file.checked_add(size)? > length
        || address.checked_add(memory).is_none()
        || (alignment > 1
            && (!alignment.is_power_of_two()
                || file.checked_rem(alignment)
                    != address.checked_rem(alignment)))
    {
        return None;
    }
    Some(
        flags & 1 != 0
            && entry >= address
            && entry < address.checked_add(size)?,
    )
}

fn native_elf(bytes: &[u8]) -> bool {
    native_entry(bytes).is_some()
}

// This is a structural build-output check, not an ELF loader or ABI audit.
fn native_entry(bytes: &[u8]) -> Option<()> {
    let (entry, programs) = elf_program_table(bytes)?;
    let length = u64::try_from(bytes.len()).ok()?;
    let mut executable_entry = false;
    for program in programs {
        executable_entry |= load_contains_entry(program, entry, length)?;
    }
    executable_entry.then_some(())
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
    use std::os::unix::fs::DirBuilderExt as _;
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

    fn write_field<const N: usize>(
        bytes: &mut [u8],
        offset: usize,
        value: [u8; N],
    ) -> Result<(), String> {
        bytes
            .get_mut(offset..offset.saturating_add(N))
            .ok_or("invalid ELF fixture field")?
            .copy_from_slice(&value);
        Ok(())
    }

    fn executable_fixture() -> Result<[u8; 128], String> {
        let mut elf = [0u8; 128];
        write_field(&mut elf, 0, *b"\x7fELF\x02\x01\x01")?;
        write_field(&mut elf, 16, 3u16.to_le_bytes())?;
        write_field(&mut elf, 18, 62u16.to_le_bytes())?;
        write_field(&mut elf, 20, 1u32.to_le_bytes())?;
        write_field(&mut elf, 24, 0x1078u64.to_le_bytes())?;
        write_field(&mut elf, 32, 64u64.to_le_bytes())?;
        write_field(&mut elf, 52, 64u16.to_le_bytes())?;
        write_field(&mut elf, 54, 56u16.to_le_bytes())?;
        write_field(&mut elf, 56, 1u16.to_le_bytes())?;
        write_field(&mut elf, 64, 1u32.to_le_bytes())?;
        write_field(&mut elf, 68, 5u32.to_le_bytes())?;
        write_field(&mut elf, 72, 120u64.to_le_bytes())?;
        write_field(&mut elf, 80, 0x1078u64.to_le_bytes())?;
        write_field(&mut elf, 96, 8u64.to_le_bytes())?;
        write_field(&mut elf, 104, 16u64.to_le_bytes())?;
        write_field(&mut elf, 112, 4096u64.to_le_bytes())?;
        Ok(elf)
    }

    #[test]
    fn executable_admission_requires_a_file_backed_entry() -> Result<(), String>
    {
        let elf = executable_fixture()?;
        require(native_elf(&elf))?;
        for length in 0..elf.len() {
            require(!native_elf(elf.get(..length).ok_or("fixture prefix")?))?;
        }
        let mut static_elf = elf;
        write_field(&mut static_elf, 16, 2u16.to_le_bytes())?;
        require(native_elf(&static_elf))?;
        for (offset, value) in [
            (24, 0u64),
            (24, 0x1080), // Entry in the zero-filled tail.
            (32, 63),
            (32, u64::MAX),
            (72, 121), // Payload extends beyond the file.
            (80, u64::MAX),
            (96, 17),
            (104, 7),
            (112, 3),
            (112, 8192), // File offset and virtual address disagree.
        ] {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        for (offset, value) in
            [(18, 183u16), (52, 0), (54, 55), (56, 0), (56, u16::MAX)]
        {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        for (offset, value) in [(20, 2u32), (64, 0), (68, 4)] {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        Ok(())
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
        require(!root.0.join("escape").exists())?;
        require(
            fs::read_dir(&root.0)
                .map_err(|error| error.to_string())?
                .count()
                == 1,
        )
    }

    #[test]
    fn failed_write_does_not_publish_partial_package() -> Result<(), String> {
        let root = TestRoot::new()?;
        let output = root.0.join("package");
        let mut prepared = Prepared::default();
        prepared.add("licenses".into(), vec![1], false)?;
        prepared.add("licenses/MIT".into(), vec![2], false)?;
        require(prepared.publish(&output, "incomplete").is_err())?;
        require(!output.exists())?;
        require(
            fs::read_dir(&root.0)
                .map_err(|error| error.to_string())?
                .count()
                == 0,
        )
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("package contract failed"))
    }
}
