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
//   - Deterministic assembly of explicit authored APK members.
// - Must-Not:
//   - Include source media, credentials or uncontrolled archive entries.
// - Allows:
//   - Inputs: Private compiler output and a compiled binary manifest.
//   - Outputs: Stored ZIP bytes with aligned native libraries.
//   - Side effects: Read-only bounded snapshots.
// - Split-When:
//   - Another application archive format needs independent semantics.
// - Merge-When:
//   - A shared archive assembler provides identical fixed-member admission.
// - Summary:
//   - Reuse the locked ZIP writer for Android archive assembly.
// - Description:
//   - Preserve the AAPT2 binary manifest, align libraries and omit assets.
// - Usage:
//   - Assemble only after the native and Java compilers succeed.
// - Defaults:
//   - Stable ZIP metadata, no original media and no signing.

use std::io::{Cursor, Read as _, Write as _};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::{
    APPLICATION_LIBRARY, MAX_ARTIFACT, MAX_SOURCE, Outcome, SDL_LIBRARY,
    failure,
};
use crate::ir::storage::file::snapshot;

type ApkWriter = ZipWriter<Cursor<Vec<u8>>>;

fn resource(directory: &Path, name: &str) -> Outcome<Vec<u8>> {
    let input = snapshot(directory.join("base.apk").as_os_str(), MAX_SOURCE)
        .map_err(|error| failure(format!("compiled manifest: {error:?}")))?;
    let mut archive = ZipArchive::new(Cursor::new(input))
        .map_err(|error| failure(format!("AAPT2 archive: {error}")))?;
    if archive.len() != 2 || archive.has_duplicate_names() {
        return Err(failure("unexpected AAPT2 members".into()));
    }
    let mut entry = archive
        .by_name(name)
        .map_err(|error| failure(format!("AAPT2 manifest: {error}")))?;
    if entry.size() > MAX_SOURCE.get() {
        return Err(failure("AAPT2 manifest too large".into()));
    }
    let mut bytes = Vec::new();
    let count = entry
        .by_ref()
        .take(MAX_SOURCE.get())
        .read_to_end(&mut bytes)
        .map_err(|error| failure(format!("AAPT2 manifest: {error}")))?;
    if u64::try_from(count).ok() != Some(entry.size()) {
        return Err(failure("AAPT2 manifest size mismatch".into()));
    }
    Ok(bytes)
}

pub(super) fn assemble(directory: &Path) -> Outcome<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let compiled_manifest = resource(directory, "AndroidManifest.xml")?;
    let resources = resource(directory, "resources.arsc")?;
    write_member(&mut writer, "AndroidManifest.xml", &compiled_manifest, 4)?;
    write_member(&mut writer, "resources.arsc", &resources, 4)?;
    let mut budget = MAX_ARTIFACT.get().saturating_sub(65_536);
    budget = budget
        .checked_sub(
            u64::try_from(
                compiled_manifest.len().saturating_add(resources.len()),
            )
            .map_err(|error| failure(error.to_string()))?,
        )
        .ok_or_else(|| failure("APK byte budget exceeded".into()))?;
    for (name, alignment) in [
        ("classes.dex", 4),
        (SDL_LIBRARY, 16_384),
        (APPLICATION_LIBRARY, 16_384),
    ] {
        let data = snapshot(
            directory.join("apk").join(name).as_os_str(),
            MAX_ARTIFACT,
        )
        .map_err(|error| failure(format!("authored APK member: {error:?}")))?;
        budget = budget
            .checked_sub(
                u64::try_from(data.len())
                    .map_err(|error| failure(error.to_string()))?,
            )
            .ok_or_else(|| failure("APK byte budget exceeded".into()))?;
        write_member(&mut writer, name, &data, alignment)?;
    }
    writer
        .finish()
        .map(Cursor::into_inner)
        .map_err(|error| failure(format!("assemble APK: {error}")))
}

fn write_member(
    writer: &mut ApkWriter,
    name: &str,
    data: &[u8],
    alignment: u16,
) -> Outcome<()> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .with_alignment(alignment);
    writer
        .start_file(name, options)
        .map_err(|error| failure(format!("APK member: {error}")))?;
    writer
        .write_all(data)
        .map_err(|error| failure(format!("write APK member: {error}")))
}
