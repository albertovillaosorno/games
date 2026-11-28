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
//   - Bounded target-specific package receipt and independent readback.
// - Must-Not:
//   - Authenticate origins, execute files or claim ABI compatibility.
// - Allows:
//   - Inputs: Authored file identities, title and trusted package directory.
//   - Outputs: Canonical receipt and verified package identity or error.
//   - Side effects: Bounded read-only package observation.
// - Split-When:
//   - A separate distribution format requires different receipt semantics.
// - Merge-When:
//   - Another package verifier duplicates this receipt and readback.
// - Summary:
//   - Account for every package file, including dependency notices.
// - Description:
//   - Require an exact tree and validate every listed content identity.
// - Usage:
//   - Generate on publication; verify a pristine package before release.
// - Defaults:
//   - Reject unknown receipt formats, missing files and extra entries.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::iter::once;
use std::num::{NonZeroU64, NonZeroUsize};
use std::str;

use games_assets::ContentHash;
use games_assets::archive::validate_path;
use games_platform::ir::security::staging::{Namespace, NamespaceDirectory};
use games_platform::ir::security::toolchain::artifact::admit_executable;
#[cfg(test)]
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::file::{AuthoredFileKind, snapshot_authored};
use games_platform::ir::storage::{
    PathKind, path_kind, trusted_directory_entries,
};

use super::{
    INDEX_HTML, MAX_DOCUMENT, MAX_EXECUTABLE, MAX_FILES, MAX_PACKAGE, RECEIPT,
    admit_document,
};
use crate::title_build::{
    BROWSER_INDEX, BROWSER_SCRIPT, Distribution, local_path,
};

const CONTENTS: &str = "\nContents:\n";
const MAX_TREE_ENTRIES: usize = 1024;
const REQUIRED: [&str; 3] = [
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "licenses/DEPENDENCIES.tsv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    bytes: NonZeroU64,
    digest: ContentHash,
}

#[derive(Debug, Default)]
pub(super) struct Receipt {
    entries: BTreeMap<String, Entry>,
    total: usize,
}

impl Receipt {
    pub(super) fn add(
        &mut self,
        title: Distribution,
        path: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let length = u64::try_from(bytes.len())
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or("invalid package file length")?;
        self.insert(title, path, Entry {
            bytes: length,
            digest: ContentHash::calculate(bytes),
        })
    }

    fn check_directory<'namespace>(
        path: &OsStr,
        expected: &'namespace NamespaceDirectory,
        pending: &mut Vec<(OsString, &'namespace NamespaceDirectory)>,
    ) -> Result<(), String> {
        let maximum = NonZeroUsize::MIN.saturating_add(MAX_TREE_ENTRIES - 1);
        let observed = trusted_directory_entries(path, maximum)
            .map_err(|error| format!("package directory: {error}"))?;
        if observed.len() != expected.children().len() {
            return Err(String::from("unexpected package directory size"));
        }
        let entries: BTreeMap<_, _> = observed
            .iter()
            .map(|entry| (entry.name.as_os_str(), entry.kind))
            .collect();
        for child in expected.children() {
            let kind = if child.directory().is_some() {
                PathKind::Directory
            } else {
                PathKind::RegularFile
            };
            if entries.get(OsStr::new(child.name())) != Some(&kind) {
                return Err(format!(
                    "unexpected package entry: {}",
                    child.name()
                ));
            }
            if let Some(directory) = child.directory() {
                pending.push((local_path(path, child.name())?, directory));
            }
        }
        Ok(())
    }

    fn check_file(
        title: Distribution,
        output: &OsStr,
        path: &str,
        entry: Entry,
    ) -> Result<(), String> {
        let kind = if path == title.binary()
            && !title.is_browser()
            && !title.is_android()
        {
            AuthoredFileKind::Executable
        } else {
            AuthoredFileKind::Document
        };
        let data =
            snapshot_authored(&local_path(output, path)?, entry.bytes, kind)
                .map_err(|error| format!("package file {path}: {error:?}"))?;
        if u64::try_from(data.len()).ok() != Some(entry.bytes.get())
            || ContentHash::calculate(&data) != entry.digest
        {
            return Err(format!("package file identity mismatch: {path}"));
        }
        if path == title.binary() {
            admit_executable(title.target(), &data)
                .map_err(|error| error.to_string())?;
        } else {
            if title.is_browser()
                && path == BROWSER_INDEX
                && data != INDEX_HTML.as_bytes()
            {
                return Err("unexpected browser bootstrap".into());
            }
            admit_document(&data)
                .map_err(|error| format!("package document {path}: {error}"))?;
        }
        Ok(())
    }

    fn check_tree(&self, output: &OsStr) -> Result<(), String> {
        if path_kind(output).map_err(|error| error.to_string())?
            != PathKind::Directory
        {
            return Err(String::from("expected a regular package directory"));
        }
        let namespace = self.tree()?;
        let mut pending = vec![(output.to_owned(), namespace.root())];
        while let Some((path, expected)) = pending.pop() {
            Self::check_directory(&path, expected, &mut pending)?;
        }
        Ok(())
    }

    pub(super) fn encode(&self, title: Distribution) -> Result<String, String> {
        let executable = self
            .entries
            .get(title.binary())
            .ok_or("missing package executable")?;
        if REQUIRED
            .iter()
            .any(|path| !self.entries.contains_key(*path))
        {
            return Err(String::from("missing required package document"));
        }
        let _tree = self.tree()?;
        if title.is_browser()
            && [BROWSER_INDEX, BROWSER_SCRIPT]
                .iter()
                .any(|path| !self.entries.contains_key(*path))
        {
            return Err("missing required browser binding file".into());
        }
        let (label, requirements) = distribution_description(title);
        let mut text = format!(
            "Title: {}\nTarget: {}\n{label} bytes: {}\n\
             {label} SHA-256: {}\n\
             {requirements}\
             No system libraries, original media or assets included.\n\
             Obtain and import lawful original assets locally.\n\
             This is a local prototype package, not a portable release.\n\
             Receipt format: 1\n{CONTENTS}",
            title.title().binary(),
            title.target().triple(),
            executable.bytes,
            executable.digest,
        );
        for (path, entry) in &self.entries {
            writeln!(text, "{path}\t{}\t{}", entry.bytes, entry.digest)
                .map_err(|error| error.to_string())?;
        }
        let _total = self
            .total
            .checked_add(text.len())
            .filter(|total| *total <= MAX_PACKAGE)
            .ok_or("package receipt byte budget exceeded")?;
        if u64::try_from(text.len()).map_err(|error| error.to_string())?
            > MAX_DOCUMENT.get()
        {
            return Err(String::from("package receipt exceeds budget"));
        }
        Ok(text)
    }

    fn insert(
        &mut self,
        title: Distribution,
        path: &str,
        entry: Entry,
    ) -> Result<(), String> {
        validate_path(path).map_err(|error| format!("{error:?}"))?;
        if !path.is_ascii()
            || self.entries.len() >= MAX_FILES.saturating_sub(1)
            || self.entries.contains_key(path)
        {
            return Err(String::from("invalid package receipt path or count"));
        }
        let limit = if path == title.binary() {
            MAX_EXECUTABLE
        } else if REQUIRED.contains(&path)
            || path.starts_with("licenses/")
            || (title.is_browser()
                && [BROWSER_INDEX, BROWSER_SCRIPT].contains(&path))
        {
            MAX_DOCUMENT
        } else {
            return Err(String::from("unrecognized package receipt path"));
        };
        if entry.bytes > limit {
            return Err(String::from("package receipt file exceeds budget"));
        }
        self.total = usize::try_from(entry.bytes.get())
            .ok()
            .and_then(|length| self.total.checked_add(length))
            .filter(|total| *total <= MAX_PACKAGE)
            .ok_or("package receipt byte budget exceeded")?;
        let _previous = self.entries.insert(path.to_owned(), entry);
        Ok(())
    }

    fn parse(title: Distribution, bytes: &[u8]) -> Result<Self, String> {
        let text = str::from_utf8(bytes).map_err(|error| error.to_string())?;
        let (_header, contents) = text
            .split_once(CONTENTS)
            .ok_or("unsupported package receipt format")?;
        let mut receipt = Self::default();
        for line in contents.lines() {
            let mut fields = line.split('\t');
            let (Some(path), Some(length), Some(digest), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(String::from("invalid package receipt record"));
            };
            receipt.insert(title, path, Entry {
                bytes: length
                    .parse::<NonZeroU64>()
                    .map_err(|error| error.to_string())?,
                digest: digest
                    .parse::<ContentHash>()
                    .map_err(|error| format!("{error:?}"))?,
            })?;
        }
        if receipt.encode(title)? != text {
            return Err(String::from("noncanonical package receipt"));
        }
        Ok(receipt)
    }

    fn tree(&self) -> Result<Namespace, String> {
        let mut namespace = Namespace::default();
        for file in self.entries.keys().map(String::as_str).chain(once(RECEIPT))
        {
            let _plan = namespace
                .admit_authored_path(file)
                .map_err(|error| format!("package path {file}: {error:?}"))?;
            if namespace.entry_count() > MAX_TREE_ENTRIES {
                return Err(String::from("package tree exceeds budget"));
            }
        }
        Ok(namespace)
    }

    pub(super) fn verify(
        title: Distribution,
        output: &OsStr,
    ) -> Result<String, String> {
        if path_kind(output).map_err(|error| error.to_string())?
            != PathKind::Directory
        {
            return Err(String::from("expected a regular package directory"));
        }
        let receipt_path = local_path(output, RECEIPT)?;
        let bytes = snapshot_authored(
            &receipt_path,
            MAX_DOCUMENT,
            AuthoredFileKind::Document,
        )
        .map_err(|error| format!("package receipt: {error:?}"))?;
        let receipt = Self::parse(title, &bytes)?;
        receipt.check_tree(output)?;
        for (path, entry) in &receipt.entries {
            Self::check_file(title, output, path, *entry)?;
        }
        receipt.check_tree(output)?;
        if snapshot_authored(
            &receipt_path,
            MAX_DOCUMENT,
            AuthoredFileKind::Document,
        )
        .map_err(|error| format!("package receipt: {error:?}"))?
            != bytes
        {
            return Err(String::from(
                "package receipt changed during readback",
            ));
        }
        let count = receipt.entries.len().saturating_add(1);
        let total = receipt.total.saturating_add(bytes.len());
        Ok(format!(
            "Verified {count} package files ({total} bytes); \
             receipt SHA-256: {}",
            ContentHash::calculate(&bytes),
        ))
    }
}

const fn distribution_description(
    title: Distribution,
) -> (&'static str, &'static str) {
    if title.is_android() {
        (
            "APK",
            concat!(
                "Unsigned debug development APK, ARM64, Android API 24+.\n",
                "Sign separately; provision private assets locally.\n",
            ),
        )
    } else if title.is_browser() {
        (
            "Module",
            concat!(
                "Requires a modern browser with WebGL 2 ",
                "and local directory selection.\n",
            ),
        )
    } else {
        (
            "Executable",
            "Requires compatible GNU/Linux system libraries and SDL2.\n",
        )
    }
}

#[cfg(test)]
mod tests {
    use games_platform::ir::security::toolchain::artifact::fixtures;
    use games_platform::ir::storage::fixtures::TestRoot;

    use super::super::Prepared;
    use super::*;
    use crate::title_build::Title;

    const TITLE: Distribution = Distribution::native(Title::SimpsonsArcade);
    const LICENCE: &str = "licenses/example-1/LICENSE-MIT";
    const FIXTURE_FILES: [FixtureFile; 4] = [
        ("LICENSE", b"licence"),
        ("THIRD_PARTY_NOTICES.md", b"notices"),
        ("licenses/DEPENDENCIES.tsv", b"dependencies"),
        (LICENCE, b"upstream"),
    ];

    type FixtureFile = (&'static str, &'static [u8]);

    fn package(
        root: &TestRoot,
        executable: &[u8],
        licence: &[u8],
    ) -> Result<(), String> {
        let mut prepared = Prepared::default();
        prepared.add(TITLE.binary().into(), executable.to_vec(), true)?;
        for (path, bytes) in FIXTURE_FILES {
            let content = if path == LICENCE {
                licence
            } else {
                bytes
            };
            prepared.add(path.into(), content.to_vec(), false)?;
        }
        prepared.finish_receipt(TITLE)?;
        prepared.publish(&local_path(root.location(), "package")?)
    }

    #[test]
    fn browser_receipt_requires_exact_binding_tree() -> Result<(), String> {
        let selection =
            Distribution::parse("simpsons-arcade", &["webassembly".into()])?;
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let output = local_path(root.location(), "browser")?;
        let module = fixtures::browser_module_fixture();
        let mut prepared = Prepared::default();
        prepared.add(selection.binary().into(), module.to_vec(), false)?;
        prepared.add(
            BROWSER_INDEX.into(),
            INDEX_HTML.as_bytes().to_vec(),
            false,
        )?;
        for (path, bytes) in FIXTURE_FILES {
            prepared.add(path.into(), bytes.to_vec(), false)?;
        }
        require(prepared.finish_receipt(selection).is_err())?;
        prepared.add(
            BROWSER_SCRIPT.into(),
            b"export default function init() {}".to_vec(),
            false,
        )?;
        prepared.finish_receipt(selection)?;
        prepared.publish(&output)?;
        let report = Receipt::verify(selection, &output)?;
        require(report.starts_with("Verified 8 package files ("))?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        let program = format!("browser/{}", selection.binary());
        root.make_executable(&program)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(selection, &output).is_err())?;
        root.write_file(&program, module)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(selection, &output) == Ok(report))?;
        root.write_file("browser/asset.bin", b"unexpected")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(selection, &output).is_err())
    }

    #[test]
    fn receipt_verifies_content_and_permissions() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        let executable = fixtures::executable_fixture()?;
        package(&root, &executable, b"upstream")?;
        let output = local_path(root.location(), "package")?;
        let receipt_path = local_path(&output, RECEIPT)?;
        let original = snapshot(&receipt_path, MAX_DOCUMENT)
            .map_err(|error| format!("{error:?}"))?;
        let report = Receipt::verify(TITLE, &output)?;
        require(report.starts_with("Verified 6 package files ("))?;
        require(
            report.ends_with(&ContentHash::calculate(&original).to_string()),
        )?;

        root.write_file("package/LICENSE", b"damaged")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.write_file("package/LICENSE", b"licence")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output) == Ok(report.clone()))?;

        root.make_executable("package/LICENSE")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.write_file("package/LICENSE", b"licence")
            .map_err(|error| error.to_string())?;
        let program = format!("package/{}", TITLE.binary());
        root.write_file(&program, &executable)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.make_executable(&program)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output) == Ok(report))
    }

    #[test]
    fn receipt_rejects_missing_unlisted_and_linked_entries()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        package(&root, &fixtures::executable_fixture()?, b"upstream")?;
        let output = local_path(root.location(), "package")?;
        let report = Receipt::verify(TITLE, &output)?;
        let licence = format!("package/{LICENCE}");
        root.hard_link(&licence, "alias")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.remove_file("alias")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output) == Ok(report.clone()))?;
        root.remove_file(&licence)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.write_file(&licence, b"upstream")
            .map_err(|error| error.to_string())?;
        root.write_file("package/unlisted", b"unlisted")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.remove_file("package/unlisted")
            .map_err(|error| error.to_string())?;
        root.create_directories("package/licenses/empty")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.remove_directory("package/licenses/empty")
            .map_err(|error| error.to_string())?;

        root.remove_file(&licence)
            .map_err(|error| error.to_string())?;
        root.write_file("outside", b"upstream")
            .map_err(|error| error.to_string())?;
        root.link(&local_path(root.location(), "outside")?, &licence)
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output).is_err())?;
        root.remove_file(&licence)
            .map_err(|error| error.to_string())?;
        root.write_file(&licence, b"upstream")
            .map_err(|error| error.to_string())?;
        require(Receipt::verify(TITLE, &output) == Ok(report))
    }

    #[test]
    fn receipt_rejects_unsafe_and_inconsistent_records() -> Result<(), String> {
        let mut receipt = Receipt::default();
        receipt.add(TITLE, TITLE.binary(), &fixtures::executable_fixture()?)?;
        for (path, bytes) in FIXTURE_FILES {
            receipt.add(TITLE, path, bytes)?;
        }
        let canonical = receipt.encode(TITLE)?;
        require(Receipt::parse(TITLE, canonical.as_bytes()).is_ok())?;
        require(Receipt::parse(TITLE, b"Title: simpsons-arcade\n").is_err())?;
        for (old, new) in [
            ("Receipt format: 1", "Receipt format: 2"),
            ("Executable bytes: 128", "Executable bytes: 129"),
            ("LICENSE\t7", "LICENSE\t07"),
            ("LICENSE\t7", "LICENSE\t0"),
            ("LICENSE\t7", "LICENSE\t1048577"),
            ("LICENSE\t7", "LICENSE\t18446744073709551616"),
            ("LICENSE\t7", "../LICENSE\t7"),
            ("LICENSE\t7", "assets/secret\t7"),
            ("x86_64-unknown-linux-gnu", "aarch64-linux-android"),
        ] {
            let invalid = canonical.replacen(old, new, 1);
            require(invalid != canonical)?;
            require(Receipt::parse(TITLE, invalid.as_bytes()).is_err())?;
        }
        let duplicate = format!(
            "{canonical}LICENSE\t7\t{}\n",
            ContentHash::calculate(b"licence"),
        );
        require(Receipt::parse(TITLE, duplicate.as_bytes()).is_err())?;
        require(receipt.add(TITLE, "LICENSE", b"duplicate").is_err())?;
        require(receipt.add(TITLE, "licenses/empty", b"").is_err())?;
        receipt.add(TITLE, "licenses/example-1/license-mit", b"case")?;
        require(receipt.encode(TITLE).is_err())
    }

    #[test]
    fn receipt_rejects_count_and_aggregate_budget_overflow()
    -> Result<(), String> {
        let entry = Entry {
            bytes: MAX_DOCUMENT,
            digest: ContentHash::calculate(b"tiny fixture"),
        };
        let mut oversized = Receipt::default();
        for number in 0u8..64 {
            oversized.insert(
                TITLE,
                &format!("licenses/document-{number}"),
                entry,
            )?;
        }
        require(oversized.insert(TITLE, "licenses/overflow", entry).is_err())?;

        let tiny = Entry {
            bytes: NonZeroU64::MIN,
            digest: ContentHash::calculate(b"x"),
        };
        let mut counted = Receipt::default();
        for number in 0..MAX_FILES - 1 {
            counted.insert(
                TITLE,
                &format!("licenses/document-{number}"),
                tiny,
            )?;
        }
        require(counted.insert(TITLE, "licenses/overflow", tiny).is_err())
    }

    #[test]
    fn even_a_matching_digest_requires_an_admitted_executable()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        package(&root, b"not an executable", b"upstream")?;
        let output = local_path(root.location(), "package")?;
        require(Receipt::verify(TITLE, &output).is_err())
    }

    #[test]
    fn matching_digest_does_not_admit_nontext_documents() -> Result<(), String>
    {
        let executable = fixtures::executable_fixture()?;
        for bytes in [b"\0".as_slice(), b"\xff", b" \n"] {
            let root = TestRoot::new().map_err(|error| error.to_string())?;
            package(&root, &executable, bytes)?;
            let output = local_path(root.location(), "package")?;
            require(Receipt::verify(TITLE, &output).is_err())?;
        }
        Ok(())
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("package receipt contract failed"))
    }
}
