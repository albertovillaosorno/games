// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Exact member admission for authored Android ARM64 APKs.
// - Must-Not:
//   - Extract files, execute code or claim Android compatibility.
// - Allows:
//   - Inputs: An already bounded authored application archive.
//   - Outputs: Structural admission or rejection.
// - Side effects: None.
// - Split-When:
//   - Another Android package format requires a distinct contract.
// - Merge-When:
//   - Another package reader implements the same fixed archive grammar.
// - Summary:
//   - Check archive identity, native images and stored library alignment.
// - Description:
//   - Admit the exact five generated members; check decoded lengths and CRC.
// - Usage:
//   - Verify an authored APK before publishing or checking its receipt.
// - Defaults:
//   - No original assets, additional libraries or unrelated files.

use std::io::{Cursor, Read as _};

use zip::CompressionMethod;
use zip::read::{ArchiveOffset, Config, ZipArchive};
use zip::unstable::read::ZipIterable;

use super::android_shared;

const MAX_BYTES: u64 = 33_554_432;
const PAGE_SIZE: u64 = 16_384;
const EMPTY_RESOURCES: &[u8] = &[
    2, 0, 12, 0, 40, 0, 0, 0, 0, 0, 0, 0, 1, 0, 28, 0, 28, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 1, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0,
];
const MEMBERS: [&str; 5] = [
    "AndroidManifest.xml",
    "resources.arsc",
    "classes.dex",
    "lib/arm64-v8a/libSDL2.so",
    "lib/arm64-v8a/libgames_application.so",
];

type ApkArchive<'bytes> = ZipArchive<Cursor<&'bytes [u8]>>;

pub(super) fn apk(bytes: &[u8]) -> Option<()> {
    if u64::try_from(bytes.len()).ok()? > MAX_BYTES {
        return None;
    }
    let config = Config {
        archive_offset: ArchiveOffset::Known(0),
    };
    admit_metadata(bytes, config)?;
    let mut archive =
        ZipArchive::with_config(config, Cursor::new(bytes)).ok()?;
    if archive.len() != MEMBERS.len() || archive.has_duplicate_names() {
        return None;
    }
    archive.validate_header_consistency().ok()?;
    for name in MEMBERS {
        check_payload(&mut archive, name)?;
    }
    Some(())
}

fn admit_metadata(bytes: &[u8], config: Config) -> Option<()> {
    let mut iterable = ZipIterable::try_new(Cursor::new(bytes), config)
        .ok()?
        .enforce_header_consistency(true);
    if iterable.len() != MEMBERS.len() {
        return None;
    }
    let mut remaining = MEMBERS.to_vec();
    let mut budget = MAX_BYTES;
    for result in iterable.files().ok()? {
        let entry = result.ok()?;
        let name = entry.name().ok()?;
        let position =
            remaining.iter().position(|expected| *expected == name)?;
        let _removed = remaining.remove(position);
        if name.as_bytes() != entry.name_raw()
            || !entry.is_file()
            || entry.encrypted()
            || entry.size() == 0
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170_000, 0 | 0o100_000))
            || !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
        {
            return None;
        }
        budget = budget.checked_sub(entry.size())?;
    }
    remaining.is_empty().then_some(())
}

fn check_payload(archive: &mut ApkArchive<'_>, name: &str) -> Option<()> {
    let mut entry = archive.by_name(name).ok()?;
    if name.starts_with("lib/")
        && (entry.compression() != CompressionMethod::Stored
            || entry.data_start()?.checked_rem(PAGE_SIZE)? != 0)
    {
        return None;
    }
    let size = entry.size();
    let mut payload = Vec::new();
    let count = entry
        .by_ref()
        .take(size.checked_add(1)?)
        .read_to_end(&mut payload)
        .ok()?;
    if u64::try_from(count).ok()? != size || payload.len() != count {
        return None;
    }
    match name {
        "AndroidManifest.xml" if payload.starts_with(&[3, 0, 8, 0]) => {},
        "classes.dex" if payload.starts_with(b"dex\n037\0") => {},
        "resources.arsc" if payload == EMPTY_RESOURCES => {},
        _ if name.starts_with("lib/") => android_shared(&payload)?,
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::super::fixtures::{executable_fixture, write_field};
    use super::*;

    fn library() -> Result<[u8; 184], String> {
        let mut elf = [0; 184];
        write_field(&mut elf, 0, executable_fixture()?)?;
        write_field(&mut elf, 18, 183u16.to_le_bytes())?;
        write_field(&mut elf, 24, 0u64.to_le_bytes())?;
        write_field(&mut elf, 56, 2u16.to_le_bytes())?;
        write_field(&mut elf, 72, 0u64.to_le_bytes())?;
        write_field(&mut elf, 80, 0u64.to_le_bytes())?;
        write_field(&mut elf, 96, 184u64.to_le_bytes())?;
        write_field(&mut elf, 104, 184u64.to_le_bytes())?;
        write_field(&mut elf, 112, PAGE_SIZE.to_le_bytes())?;
        write_field(&mut elf, 120, 2u32.to_le_bytes())?;
        Ok(elf)
    }

    fn fixture(
        extra: bool,
        arm: bool,
        alignment: u16,
    ) -> Result<Vec<u8>, String> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let native = library()?;
        let wrong = executable_fixture()?;
        for (name, data) in [
            ("AndroidManifest.xml", &[3, 0, 8, 0][..]),
            ("resources.arsc", EMPTY_RESOURCES),
            ("classes.dex", b"dex\n037\0"),
            ("lib/arm64-v8a/libSDL2.so", native.as_slice()),
            (
                "lib/arm64-v8a/libgames_application.so",
                if arm {
                    native.as_slice()
                } else {
                    wrong.as_slice()
                },
            ),
        ] {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .with_alignment(alignment);
            archive
                .start_file(name, options)
                .map_err(|error| error.to_string())?;
            archive.write_all(data).map_err(|error| error.to_string())?;
        }
        if extra {
            archive
                .start_file("assets/rom", SimpleFileOptions::default())
                .map_err(|error| error.to_string())?;
            archive
                .write_all(b"fixture")
                .map_err(|error| error.to_string())?;
        }
        archive
            .finish()
            .map(Cursor::into_inner)
            .map_err(|error| error.to_string())
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| "APK admission failed".into())
    }

    #[test]
    fn exact_apk_requires_aligned_arm64_libraries() -> Result<(), String> {
        let aligned = fixture(false, true, 16_384)?;
        require(apk(&aligned).is_some())?;
        require(apk(&fixture(true, true, 16_384)?).is_none())?;
        require(apk(&fixture(false, false, 16_384)?).is_none())?;
        require(apk(&fixture(false, true, 4)?).is_none())?;
        require(apk(aligned.get(..32).ok_or("fixture prefix")?).is_none())?;
        Ok(())
    }
}
