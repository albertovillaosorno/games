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
//   - Local packaging of the selected authored title distribution.
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
//   - Stage authored release files and dependency licence texts.
// - Description:
//   - Preflight bounded sources; never overwrite an existing output.
// - Usage:
//   - games-build package TITLE OUTPUT [TARGET]
// - Defaults:
//   - Local GNU/Linux x86-64 package, no assets or portability claim.

mod license_inventory;
mod receipt;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
use std::str;

use games_assets::archive::validate_path;
use games_platform::ir::security::toolchain::artifact::admit_executable;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::staging::AuthoredStaging;
use games_platform::ir::storage::{
    PathKind, canonical_future_directory, canonical_path, join_path,
    parent_path, path_is_absolute, path_kind, path_leaf, paths_overlap,
};
use games_platform::ir::toolchain::write_line;

use self::license_inventory::Inventory;
use self::receipt::Receipt;
use crate::title_build::{
    BROWSER_INDEX, BROWSER_MODULE, BROWSER_SCRIPT, Distribution, Title,
    local_layout, local_path, workspace_root,
};

const MAX_DOCUMENT: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const MAX_EXECUTABLE: NonZeroU64 = NonZeroU64::MIN.saturating_add(33_554_431);
const MAX_FILES: usize = 256;
const MAX_PACKAGE: usize = 67_108_864;
const RECEIPT: &str = "PACKAGE.txt";
const INDEX_HTML: &str = concat!(
    "<!doctype html>\n<html lang=\"en\"><head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n",
    "<title>Games title preview</title>\n</head><body>\n",
    "<p>Select your locally imported canonical assets to start.</p>\n",
    "<p id=\"status\" role=\"status\" data-games-process-status>",
    "Loading preview...</p>\n",
    "<script type=\"module\">\n",
    "const status = document.getElementById('status');\n",
    "try { const {default: init} = await import('./game.js');\n",
    "await init(); }\n",
    "catch (error) { status.textContent = 'Preview unavailable.'; ",
    "console.error(error); }\n",
    "</script>\n</body></html>\n",
);

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

    fn finish_receipt(&mut self, title: Distribution) -> Result<(), String> {
        let mut receipt = Receipt::default();
        for (path, file) in &self.files {
            receipt.add(title, path, &file.bytes)?;
        }
        self.add(RECEIPT.into(), receipt.encode(title)?.into_bytes(), false)
    }

    fn prepare(
        root: &OsStr,
        title: Distribution,
        parent: &OsStr,
        inventory: Inventory,
    ) -> Result<Self, String> {
        let request = title.request()?;
        let mut prepared = Self::default();
        if title.is_browser() {
            let bindings = request
                .bind_browser(root, parent)
                .map_err(|error| error.to_string())?;
            prepared.add(BROWSER_MODULE.into(), bindings.module, false)?;
            prepared.add(BROWSER_SCRIPT.into(), bindings.javascript, false)?;
            prepared.add(
                BROWSER_INDEX.into(),
                INDEX_HTML.as_bytes().to_vec(),
                false,
            )?;
        } else {
            let executable =
                request.compile(root).map_err(|error| error.to_string())?;
            let bytes = snapshot(&executable, MAX_EXECUTABLE)
                .map_err(|error| format!("authored executable: {error:?}"))?;
            admit_executable(title.target(), &bytes)
                .map_err(|error| error.to_string())?;
            prepared.add(title.binary().into(), bytes, true)?;
        }
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
        prepared.finish_receipt(title)?;
        Ok(prepared)
    }

    fn publish(self, output: &OsStr) -> Result<(), String> {
        if self.files.get(RECEIPT).is_none_or(|file| file.executable) {
            return Err(String::from("missing or executable package receipt"));
        }
        let name = path_leaf(output)
            .map_err(|error| format!("package destination name: {error}"))?;
        let destination_parent = package_parent(output)?;
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
    let assets = configured_with(&root, title, |selection, output| {
        publish(&root, selection, output)
    })?;
    write_line(&format!("Shared assets: {}", assets.to_string_lossy()))
        .map_err(|error| error.to_string())
}

fn configured_with<Publish>(
    root: &OsStr,
    title: Title,
    publish: Publish,
) -> Result<OsString, String>
where
    Publish: FnOnce(Distribution, &OsStr) -> Result<(), String>,
{
    let layout = local_layout(root)?;
    let selection = Distribution::new(title, layout.target())?;
    let output = OsString::from(layout.binaries(title.binary())?);
    let assets = OsString::from(layout.assets(title.binary())?);
    if !path_is_absolute(&output).map_err(|error| error.to_string())?
        || !path_is_absolute(&assets).map_err(|error| error.to_string())?
    {
        return Err(String::from("persistent roots must be absolute"));
    }
    reject_overlap(&output, &assets)?;
    let parent = parent_path(&output).map_err(|error| error.to_string())?;
    if path_kind(parent).map_err(|error| error.to_string())?
        != PathKind::Directory
    {
        return Err(String::from("create the configured bin directory first"));
    }
    let canonical_assets = canonical_future_directory(&assets)
        .map_err(|error| format!("configured asset directory: {error}"))?;
    let canonical_output = canonical_future_directory(&output)
        .map_err(|error| format!("configured package directory: {error}"))?;
    reject_overlap(&canonical_output, &canonical_assets)?;
    publish(selection, &canonical_output)?;
    Ok(canonical_assets)
}

fn reject_overlap(output: &OsStr, assets: &OsStr) -> Result<(), String> {
    if paths_overlap(output, assets).map_err(|error| error.to_string())? {
        return Err(String::from("package and asset paths overlap"));
    }
    Ok(())
}

fn package_parent(output: &OsStr) -> Result<&OsStr, String> {
    let parent = parent_path(output).map_err(|error| error.to_string())?;
    Ok(if parent.is_empty() {
        OsStr::new(".")
    } else {
        parent
    })
}

fn explicit_with<Publish>(
    root: &OsStr,
    title: Distribution,
    output: &OsStr,
    publish: Publish,
) -> Result<(), String>
where
    Publish: FnOnce(Distribution, &OsStr) -> Result<(), String>,
{
    let config = local_path(root, ".env")?;
    if path_kind(&config).map_err(|error| error.to_string())?
        != PathKind::Missing
    {
        let layout = local_layout(root)?;
        let assets = OsString::from(layout.assets(title.title().binary())?);
        let canonical_assets = canonical_future_directory(&assets)
            .map_err(|error| format!("configured asset directory: {error}"))?;
        let parent = canonical_path(package_parent(output)?)
            .map_err(|error| format!("package parent: {error}"))?;
        let name = path_leaf(output).map_err(|error| error.to_string())?;
        let canonical_output =
            join_path(&parent, name).map_err(|error| error.to_string())?;
        reject_overlap(&canonical_output, &canonical_assets)?;
    }
    publish(title, output)
}

/// Build and package one admitted title in a new local directory.
///
/// # Errors
/// Rejects build, licence inventory, source admission and filesystem errors.
/// Also rejects overlap with any configured asset directory.
/// Publication is atomic; a post-rename sync failure is reported separately.
pub fn run(title: Distribution, output: &OsStr) -> Result<(), String> {
    let root = workspace_root()?;
    explicit_with(&root, title, output, |selection, destination| {
        publish(&root, selection, destination)
    })
}

fn publish(
    root: &OsStr,
    title: Distribution,
    output: &OsStr,
) -> Result<(), String> {
    if path_kind(output).map_err(|error| error.to_string())?
        != PathKind::Missing
    {
        return Err(String::from("package destination already exists"));
    }
    let inventory = Inventory::load(root, title)?;
    let stage_parent = package_parent(output)?;
    let prepared = Prepared::prepare(root, title, stage_parent, inventory)?;
    prepared.publish(output)?;
    write_line(&format!("Package: {}", output.to_string_lossy()))
        .map_err(|error| error.to_string())?;
    check(title, output).map_err(|error| {
        format!("package published, but readback failed: {error}")
    })
}

/// Verify an untouched package without executing or modifying any file.
///
/// # Errors
/// Rejects unsupported receipts, inconsistent identities and extra entries.
pub fn check(title: Distribution, output: &OsStr) -> Result<(), String> {
    let report = Receipt::verify(title, output)?;
    write_line(&report).map_err(|error| error.to_string())
}

fn document(path: &OsStr) -> Result<Vec<u8>, String> {
    let bytes = snapshot(path, MAX_DOCUMENT).map_err(|error| {
        format!("licence {}: {error:?}", path.to_string_lossy())
    })?;
    admit_document(&bytes).map_err(|error| {
        format!("licence {}: {error}", path.to_string_lossy())
    })?;
    Ok(bytes)
}

fn admit_document(bytes: &[u8]) -> Result<(), String> {
    let text = str::from_utf8(bytes).map_err(|error| error.to_string())?;
    if text.trim().is_empty() || text.contains('\0') {
        return Err(String::from("empty or nontext licence"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use games_platform::ir::storage::directory_entries;
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

    fn fixture_configuration(
        root: &TestRoot,
        assets: &str,
        output: &str,
        target: &str,
    ) -> Result<(), String> {
        let base = root.location().to_str().ok_or("fixture path encoding")?;
        let config = format!(
            "GAMES_ASSET_ROOT={base}/{assets}\n\
             GAMES_OUTPUT_ROOT={base}/{output}\nGAMES_TARGET={target}\n"
        );
        root.write_file(".env", config.as_bytes())
            .map_err(|error| error.to_string())
    }

    fn fixture_package(
        title: Distribution,
        output: &OsStr,
    ) -> Result<(), String> {
        use games_platform::ir::security::toolchain::artifact::fixtures;

        let mut prepared = Prepared::default();
        if title.is_browser() {
            prepared.add(
                title.binary().into(),
                fixtures::browser_module_fixture().to_vec(),
                false,
            )?;
            prepared.add(
                BROWSER_INDEX.into(),
                INDEX_HTML.as_bytes().to_vec(),
                false,
            )?;
            prepared.add(
                BROWSER_SCRIPT.into(),
                b"export default function init() {}".to_vec(),
                false,
            )?;
        } else {
            prepared.add(
                title.binary().into(),
                fixtures::executable_fixture()?.to_vec(),
                true,
            )?;
        }
        for name in [
            "LICENSE",
            "THIRD_PARTY_NOTICES.md",
            "licenses/DEPENDENCIES.tsv",
        ] {
            prepared.add(name.into(), b"fixture".to_vec(), false)?;
        }
        prepared.finish_receipt(title)?;
        prepared.publish(output)
    }

    #[test]
    fn explicit_publication_rejects_asset_overlap_before_build()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let asset = "deploy/simpsons-arcade/assets";
        root.create_directories(asset)
            .map_err(|error| error.to_string())?;
        root.write_file(&format!("{asset}/payload"), b"asset")
            .map_err(|error| error.to_string())?;
        let assets = local_path(root.location(), asset)?;
        root.link(&assets, "asset-alias")
            .map_err(|error| error.to_string())?;
        fixture_configuration(&root, "deploy", "deploy", "linux-x86-64")?;
        let unexpected =
            |_, _: &OsStr| Err(String::from("unexpected publication"));
        for target in ["linux-x86-64", "webassembly"] {
            let selection =
                Distribution::parse("simpsons-arcade", &[target.into()])?;
            for relative in [
                format!("{asset}/preview"),
                String::from("asset-alias/preview"),
            ] {
                let output = local_path(root.location(), &relative)?;
                require(
                    explicit_with(
                        root.location(),
                        selection,
                        &output,
                        unexpected,
                    ) == Err(String::from("package and asset paths overlap")),
                )?;
            }
            fixture_configuration(&root, "future", "deploy", "linux-x86-64")?;
            let output = local_path(root.location(), "future")?;
            require(
                explicit_with(root.location(), selection, &output, unexpected)
                    == Err(String::from("package and asset paths overlap")),
            )?;
            fixture_configuration(&root, "deploy", "deploy", "linux-x86-64")?;
        }
        require(
            snapshot(&local_path(&assets, "payload")?, MAX_DOCUMENT)
                .map_err(|error| format!("{error:?}"))?
                == b"asset",
        )?;
        require(
            directory_entries(&assets, NonZeroUsize::MIN)
                .map_err(|error| error.to_string())?
                .len()
                == 1,
        )
    }

    #[test]
    fn explicit_publication_keeps_its_target_and_rejects_invalid_config()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let selection =
            Distribution::parse("simpsons-arcade", &["webassembly".into()])?;
        let no_config = local_path(root.location(), "without-config")?;
        explicit_with(root.location(), selection, &no_config, fixture_package)?;
        let _report = Receipt::verify(selection, &no_config)?;

        fixture_configuration(&root, "deploy", "deploy", "linux-x86-64")?;
        let independent = local_path(root.location(), "independent")?;
        explicit_with(
            root.location(),
            selection,
            &independent,
            fixture_package,
        )?;
        let _independent_report = Receipt::verify(selection, &independent)?;

        root.write_file(".env", b"invalid configuration")
            .map_err(|error| error.to_string())?;
        let invalid = local_path(root.location(), "invalid")?;
        require(
            explicit_with(
                root.location(),
                selection,
                &invalid,
                fixture_package,
            )
            .is_err(),
        )?;
        require(
            path_kind(&invalid).map_err(|error| error.to_string())?
                == PathKind::Missing,
        )
    }

    #[test]
    fn configured_publication_preserves_the_shared_asset_namespace()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let title = Title::SimpsonsArcade;
        let assets = "deploy/simpsons-arcade/assets";
        let bin = "deploy/simpsons-arcade/bin";
        for directory in [assets, bin] {
            root.create_directories(directory)
                .map_err(|error| error.to_string())?;
        }
        root.write_file(&format!("{assets}/payload"), b"asset")
            .map_err(|error| error.to_string())?;
        let expected_assets = local_path(root.location(), assets)?;
        let mut published = Vec::new();
        for target in ["linux-x86-64", "webassembly"] {
            fixture_configuration(&root, "deploy", "deploy", target)?;
            let selection =
                Distribution::parse(title.binary(), &[target.into()])?;
            let package =
                local_path(root.location(), &format!("{bin}/{target}"))?;
            require(
                configured_with(root.location(), title, fixture_package)?
                    == expected_assets,
            )?;
            let report = Receipt::verify(selection, &package)?;
            require(
                configured_with(root.location(), title, fixture_package)
                    .is_err(),
            )?;
            require(
                path_kind(&local_path(&package, "assets")?)
                    .map_err(|error| error.to_string())?
                    == PathKind::Missing,
            )?;
            published.push((selection, package, report));
        }
        // Publishing the second target must not alter the first one.
        for (selection, package, report) in &published {
            require(Receipt::verify(*selection, package)? == *report)?;
            for (other, _, _) in
                published.iter().filter(|(other, _, _)| other != selection)
            {
                require(Receipt::verify(*other, package).is_err())?;
            }
        }
        require(
            snapshot(&local_path(&expected_assets, "payload")?, MAX_DOCUMENT)
                .map_err(|error| format!("{error:?}"))?
                == b"asset",
        )?;
        require(
            directory_entries(&expected_assets, NonZeroUsize::MIN)
                .map_err(|error| error.to_string())?
                .len()
                == 1,
        )
    }

    #[test]
    fn configured_publication_rejects_overlapping_or_unsupported_paths()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let title = Title::SimpsonsArcade;
        let unexpected =
            |_, _: &OsStr| Err(String::from("unexpected publication"));
        for (assets, output) in [
            ("deploy", "deploy/simpsons-arcade/assets"),
            ("deploy/simpsons-arcade/bin/linux-x86-64", "deploy"),
        ] {
            fixture_configuration(&root, assets, output, "linux-x86-64")?;
            require(
                configured_with(root.location(), title, unexpected)
                    == Err(String::from("package and asset paths overlap")),
            )?;
        }
        fixture_configuration(&root, "assets", "output", "android-arm64")?;
        require(
            configured_with(root.location(), title, unexpected)
                == Err(String::from(concat!(
                    "target android-arm64 is unavailable ",
                    "for distribution; core only"
                ))),
        )?;
        fixture_configuration(&root, "assets", "output", "linux-x86-64")?;
        require(
            configured_with(root.location(), title, unexpected)
                == Err(String::from(
                    "create the configured bin directory first",
                )),
        )?;
        require(root.entry_count().map_err(|error| error.to_string())? == 1)
    }

    #[test]
    fn configured_publication_rejects_both_resolved_containment_orders()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let title = Title::SimpsonsArcade;
        let unexpected =
            |_, _: &OsStr| Err(String::from("unexpected publication"));
        let asset = "deploy/simpsons-arcade/assets";
        let output = "other/simpsons-arcade/bin/linux-x86-64";
        root.create_directories(&format!("{asset}/simpsons-arcade/bin"))
            .map_err(|error| error.to_string())?;
        root.create_directories(output)
            .map_err(|error| error.to_string())?;
        root.link(&local_path(root.location(), asset)?, "output-alias")
            .map_err(|error| error.to_string())?;
        root.link(&local_path(root.location(), output)?, "asset-alias")
            .map_err(|error| error.to_string())?;
        for (assets, binaries) in
            [("deploy", "output-alias"), ("asset-alias", "other")]
        {
            fixture_configuration(&root, assets, binaries, "linux-x86-64")?;
            require(
                configured_with(root.location(), title, unexpected)
                    == Err(String::from("package and asset paths overlap")),
            )?;
        }
        require(
            path_kind(&local_path(
                root.location(),
                &format!("{asset}/simpsons-arcade/bin/linux-x86-64"),
            )?)
            .map_err(|error| error.to_string())?
                == PathKind::Missing,
        )
    }

    #[test]
    fn configured_publication_admits_independent_trusted_aliases()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let bin = "deploy/simpsons-arcade/bin";
        root.create_directories(bin)
            .map_err(|error| error.to_string())?;
        let deploy = local_path(root.location(), "deploy")?;
        for alias in ["asset-alias", "output-alias"] {
            root.link(&deploy, alias)
                .map_err(|error| error.to_string())?;
        }
        fixture_configuration(
            &root,
            "asset-alias",
            "output-alias",
            "linux-x86-64",
        )?;
        let expected =
            local_path(root.location(), "deploy/simpsons-arcade/assets")?;
        let package =
            local_path(root.location(), &format!("{bin}/linux-x86-64"))?;
        let title = Title::SimpsonsArcade;
        require(
            configured_with(root.location(), title, fixture_package)?
                == expected,
        )?;
        require(
            path_kind(&expected).map_err(|error| error.to_string())?
                == PathKind::Missing,
        )?;
        require(
            path_kind(&package).map_err(|error| error.to_string())?
                == PathKind::Directory,
        )?;
        require(
            configured_with(root.location(), title, fixture_package).is_err(),
        )
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("package contract failed"))
    }
}
