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
//   - Verification, staging and no-replace publication of title stores.
// - Must-Not:
//   - Delete originals, interpret catalogues or access native descriptors.
// - Allows:
//   - Inputs: Explicit legacy store, title, budgets and staging parent.
//   - Outputs: Verified mapping, private store or publication receipt.
//   - Side effects: Bounded reads, staging and explicit publication.
// - Split-When:
//   - Cutover introduces independent consumer or retention policy.
// - Merge-When:
//   - A shared store transaction provides identical migration guarantees.
// - Summary:
//   - Verify legacy assets before changing their persistent layout.
// - Description:
//   - Normalize metadata and stage one verified object per content hash.
// - Usage:
//   - Audit, rehearse or publish a verified store without replacement.
// - Defaults:
//   - Reject incomplete inventories and leave source files untouched.

//! Verified shared-store migration with explicit non-overwriting publication.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::archive::{ArchiveBudget, EntryKind};
use games_assets::{AssetError, ContentHash, ManifestLimits};

use super::{
    ArchiveStaging, PreparedManifest, PublishedDirectory, StagedArchive,
    StagingEntry, StagingError,
};
use crate::ir::security::staging::PublicationName;
use crate::ir::storage::bundle::{
    CanonicalBundle, MANIFEST_NAME, OBJECT_PAYLOAD, OBJECTS_DIRECTORY,
    PAYLOADS_DIRECTORY, bundle_name,
};
#[cfg(test)]
use crate::ir::storage::file::snapshot;
use crate::ir::storage::file::snapshot_secret;
use crate::ir::storage::{
    PathKind, directory_entries, join_path, trusted_directory_entries,
};

/// Explicit source selection for migration; its parent must remain trusted.
#[derive(Clone, Copy, Debug)]
pub struct LegacyStore<'source> {
    /// Per-store and per-manifest resource limits.
    pub limits: MigrationLimits,
    /// The existing legacy asset directory.
    pub path: &'source OsStr,
    /// Required canonical title identity.
    pub title: &'source str,
}

impl LegacyStore<'_> {
    /// Audit the complete store without creating another copy of assets.
    ///
    /// # Errors
    /// Rejects unexpected bundle contents, foreign titles and invalid data.
    pub fn audit(self) -> Result<MigrationReport, StagingError> {
        visit(self, |_path, _bytes| Ok(()))
    }
}

/// Bounds on a legacy store independently of staging limits.
#[derive(Clone, Copy, Debug)]
pub struct MigrationLimits {
    /// Maximum direct entries, including regular sidecars.
    pub entries: NonZeroUsize,
    /// Per-manifest limits, including logical record bytes and count.
    pub manifest: ManifestLimits,
    /// Total distinct payload bytes read per bundle across the store.
    pub source_bytes: NonZeroU64,
}

/// Verified content and address mapping for one title's legacy store.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct MigrationReport {
    /// Exact old manifest address to normalized current address.
    pub manifests: BTreeMap<ContentHash, ContentHash>,
    /// Bytes in the distinct verified objects of the shared store.
    pub object_bytes: u64,
    /// Number of distinct verified payloads across all manifests.
    pub objects: usize,
    /// Regular non-bundle files, not migrated or interpreted.
    pub sidecars: Vec<OsString>,
    /// Bytes verified separately in each legacy payload directory.
    pub source_bytes: u64,
}

/// Explicitly generated metadata to stage with the migrated content.
///
/// Names must be single portable components outside the object namespace.
#[derive(Clone, Copy, Debug)]
pub struct MigrationFile<'file> {
    /// Generated catalogue or sidecar bytes, bounded by the staging budget.
    pub bytes: &'file [u8],
    /// Unreserved root-level filename.
    pub name: &'file str,
}

/// Generated metadata paired with the audited source inventory.
#[derive(Clone, Copy, Debug)]
pub struct MigrationMetadata<'input> {
    /// Exact inventory against which the metadata was generated.
    pub expected: &'input MigrationReport,
    /// Explicit bounded files included in the same private transaction.
    pub files: &'input [MigrationFile<'input>],
}

impl MigrationMetadata<'_> {
    /// Verify the complete published shared namespace against an audited map.
    ///
    /// The caller must freshly audit the legacy source and regenerate the
    /// expected metadata. This read-only observation is not a deletion grant.
    ///
    /// # Errors
    /// Rejects missing, unexpected, changed or unsafe entries or payloads.
    pub fn verify_published(
        self,
        path: &OsStr,
        limits: ManifestLimits,
    ) -> Result<(), StagingError> {
        let mut entries = BTreeMap::new();
        let manifests: BTreeSet<_> =
            self.expected.manifests.values().copied().collect();
        for &digest in &manifests {
            let name = OsString::from(bundle_name(digest));
            let _previous = entries.insert(name, PathKind::Directory);
        }
        if self.expected.objects != 0 {
            let _previous = entries
                .insert(OsString::from(OBJECTS_DIRECTORY), PathKind::Directory);
        }
        let mut folded_names = BTreeSet::new();
        for file in self.files {
            validate_metadata_name(file.name)?;
            if !folded_names.insert(file.name.to_ascii_lowercase()) {
                return Err(StagingError::Collision);
            }
            if entries
                .insert(file.name.into(), PathKind::RegularFile)
                .is_some()
            {
                return Err(StagingError::Collision);
            }
        }
        check_directory(path, &entries)?;
        for &digest in &manifests {
            let directory = join_path(path, &bundle_name(digest))?;
            check_single_file(&directory, MANIFEST_NAME)?;
        }
        let objects = read_back(path, self.expected, limits)?;
        check_objects(path, &objects, self.expected.objects)?;
        for file in self.files {
            let location = join_path(path, file.name)?;
            let size = u64::try_from(file.bytes.len())
                .map_err(|_error| AssetError::TooLarge)?;
            let limit = NonZeroU64::new(size).unwrap_or(NonZeroU64::MIN);
            if !snapshot_secret(&location, limit)
                .is_ok_and(|bytes| bytes.as_slice() == file.bytes)
            {
                return Err(StagingError::ChangedStorage);
            }
        }
        check_directory(path, &entries)
    }
}

/// A private, verified shared-store rehearsal; dropping it removes staging.
#[derive(Debug)]
pub struct StagedMigration {
    report: MigrationReport,
    stage: StagedArchive,
}

impl StagedMigration {
    /// Opaque temporary store location for explicit consumer verification.
    #[must_use]
    pub fn path(&self) -> &OsStr {
        self.stage.path()
    }

    /// Verify all source bundles and create a bounded private shared store.
    /// Re-open all resulting manifests through the canonical runtime reader.
    ///
    /// # Errors
    /// Rejects incomplete inventories, invalid content and storage failures.
    pub fn prepare(
        source: LegacyStore<'_>,
        parent: &OsStr,
        budget: ArchiveBudget,
    ) -> Result<Self, StagingError> {
        Self::prepare_inner(source, parent, budget, None)
    }

    fn prepare_inner(
        source: LegacyStore<'_>,
        parent: &OsStr,
        budget: ArchiveBudget,
        metadata: Option<MigrationMetadata<'_>>,
    ) -> Result<Self, StagingError> {
        let mut transaction = ArchiveStaging::new(parent, budget)?;
        let report = visit(source, |path, bytes| {
            stage_file(&mut transaction, path, bytes)
        })?;
        if let Some(selection) = metadata {
            stage_metadata(&mut transaction, &report, selection)?;
        }
        let stage = transaction.finish()?;
        let _objects =
            read_back(stage.path(), &report, source.limits.manifest)?;
        stage.verify()?;
        Ok(Self { report, stage })
    }

    /// Re-audit the source and stage generated metadata with the store.
    ///
    /// Metadata must be derived from the exact expected migration report.
    /// All metadata uses the same byte and entry budgets as canonical data.
    ///
    /// # Errors
    /// Rejects changed source inventory, reserved names and excess budgets.
    pub fn prepare_with_metadata(
        source: LegacyStore<'_>,
        parent: &OsStr,
        budget: ArchiveBudget,
        metadata: MigrationMetadata<'_>,
    ) -> Result<Self, StagingError> {
        Self::prepare_inner(source, parent, budget, Some(metadata))
    }

    /// Publish the complete verified stage into a new sibling directory.
    ///
    /// The consumer must be verified before this operation. The original
    /// store remains untouched; retirement requires separate verification.
    ///
    /// # Errors
    /// Rejects changed storage, existing output or unavailable publication.
    /// A post-rename sync failure is reported in the publication receipt.
    pub fn publish(
        self,
        name: &OsStr,
    ) -> Result<PublishedDirectory, StagingError> {
        self.stage.publish(name)
    }

    /// Verified migration report, including every old manifest address.
    #[must_use]
    pub const fn report(&self) -> &MigrationReport {
        &self.report
    }

    /// Recheck the staged tree before subsequent trusted consumption.
    /// # Errors
    /// Rejects changed storage.
    pub fn verify(&self) -> Result<(), StagingError> {
        self.stage.verify()
    }
}

struct Inventory {
    normalized: BTreeSet<ContentHash>,
    objects: BTreeMap<ContentHash, u64>,
    report: MigrationReport,
}

impl Inventory {
    fn new() -> Self {
        Self {
            normalized: BTreeSet::new(),
            objects: BTreeMap::new(),
            report: MigrationReport::default(),
        }
    }

    fn record_bundle<Write>(
        &mut self,
        source: LegacyStore<'_>,
        digest: ContentHash,
        write: &mut Write,
    ) -> Result<(), StagingError>
    where
        Write: FnMut(&str, &[u8]) -> Result<(), StagingError>,
    {
        let limits = source.limits;
        let bundle =
            CanonicalBundle::open(source.path, digest, limits.manifest)?;
        if bundle.manifest().source().title != source.title {
            return Err(AssetError::InvalidIdentity.into());
        }
        let prepared =
            PreparedManifest::new(bundle.manifest(), limits.manifest)?;
        check_inventory(source.path, digest, &prepared)?;
        let mut visited = BTreeSet::new();
        for record in bundle.manifest().records() {
            if !visited.insert(record.digest) {
                continue;
            }
            self.report.source_bytes = self
                .report
                .source_bytes
                .checked_add(record.size)
                .filter(|&size| size <= limits.source_bytes.get())
                .ok_or(AssetError::TooLarge)?;
            let bytes = bundle.read_payload(&record.key)?;
            self.record_object(record.digest, &bytes, write)?;
        }
        let current = prepared.digest();
        if self.normalized.insert(current) {
            write(
                &format!("{}/{MANIFEST_NAME}", bundle_name(current)),
                prepared.bytes(),
            )?;
        }
        check_inventory(source.path, digest, &prepared)?;
        let _previous = self.report.manifests.insert(digest, current);
        Ok(())
    }

    fn record_object<Write>(
        &mut self,
        digest: ContentHash,
        bytes: &[u8],
        write: &mut Write,
    ) -> Result<(), StagingError>
    where
        Write: FnMut(&str, &[u8]) -> Result<(), StagingError>,
    {
        let size = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        if let Some(&previous) = self.objects.get(&digest) {
            if previous != size {
                return Err(AssetError::PayloadMismatch.into());
            }
        } else {
            write(
                &format!("{OBJECTS_DIRECTORY}/{digest}/{OBJECT_PAYLOAD}"),
                bytes,
            )?;
            let _previous = self.objects.insert(digest, size);
            self.report.object_bytes = self
                .report
                .object_bytes
                .checked_add(size)
                .ok_or(AssetError::TooLarge)?;
        }
        Ok(())
    }
}

fn check_directory(
    path: &OsStr,
    expected: &BTreeMap<OsString, PathKind>,
) -> Result<(), StagingError> {
    let maximum =
        NonZeroUsize::new(expected.len()).unwrap_or(NonZeroUsize::MIN);
    let entries = trusted_directory_entries(path, maximum)?;
    if entries.len() != expected.len()
        || entries
            .iter()
            .any(|entry| expected.get(&entry.name) != Some(&entry.kind))
    {
        return Err(StagingError::ChangedStorage);
    }
    Ok(())
}

fn check_inventory(
    source: &OsStr,
    digest: ContentHash,
    prepared: &PreparedManifest,
) -> Result<(), StagingError> {
    let path = join_path(source, &bundle_name(digest))?;
    let entries =
        directory_entries(&path, NonZeroUsize::MIN.saturating_add(1))?;
    let mut manifest_seen = false;
    let mut payloads_seen = false;
    for entry in entries {
        match (entry.name.to_str(), entry.kind) {
            (Some(MANIFEST_NAME), PathKind::RegularFile) => {
                manifest_seen = true;
            },
            (Some(PAYLOADS_DIRECTORY), PathKind::Directory) => {
                payloads_seen = true;
            },
            _ => return Err(StagingError::ChangedStorage),
        }
    }
    let expected = prepared.payload_sizes();
    if !manifest_seen || (!payloads_seen && !expected.is_empty()) {
        return Err(StagingError::ChangedStorage);
    }
    if payloads_seen {
        check_payload_inventory(
            &join_path(&path, PAYLOADS_DIRECTORY)?,
            expected,
        )?;
    }
    Ok(())
}

fn check_objects(
    path: &OsStr,
    objects: &BTreeSet<ContentHash>,
    expected: usize,
) -> Result<(), StagingError> {
    if objects.len() != expected {
        return Err(StagingError::ChangedStorage);
    }
    if objects.is_empty() {
        return Ok(());
    }
    let parent = join_path(path, OBJECTS_DIRECTORY)?;
    let entries = objects
        .iter()
        .map(|digest| (OsString::from(digest.to_string()), PathKind::Directory))
        .collect();
    check_directory(&parent, &entries)?;
    for digest in objects {
        let directory = join_path(&parent, &digest.to_string())?;
        check_single_file(&directory, OBJECT_PAYLOAD)?;
    }
    check_directory(&parent, &entries)
}

fn check_payload_inventory(
    path: &OsStr,
    expected: &BTreeMap<ContentHash, u64>,
) -> Result<(), StagingError> {
    let maximum =
        NonZeroUsize::new(expected.len()).unwrap_or(NonZeroUsize::MIN);
    let payloads = directory_entries(path, maximum)?;
    if payloads.len() != expected.len() {
        return Err(StagingError::ChangedStorage);
    }
    for entry in payloads {
        let name = entry.name.to_str().ok_or(AssetError::InvalidIdentity)?;
        if entry.kind != PathKind::RegularFile
            || !expected.contains_key(&name.parse()?)
        {
            return Err(StagingError::ChangedStorage);
        }
    }
    Ok(())
}

fn check_single_file(path: &OsStr, name: &str) -> Result<(), StagingError> {
    check_directory(
        path,
        &BTreeMap::from([(name.into(), PathKind::RegularFile)]),
    )
}

fn read_back(
    stage: &OsStr,
    report: &MigrationReport,
    limits: ManifestLimits,
) -> Result<BTreeSet<ContentHash>, StagingError> {
    let mut manifests = BTreeSet::new();
    let mut unique = BTreeSet::new();
    for &digest in report.manifests.values() {
        if !manifests.insert(digest) {
            continue;
        }
        let bundle = CanonicalBundle::open(stage, digest, limits)?;
        let mut objects = BTreeSet::new();
        for record in bundle.manifest().records() {
            if objects.insert(record.digest) {
                drop(bundle.read_payload(&record.key)?);
                let _inserted = unique.insert(record.digest);
            }
        }
    }
    Ok(unique)
}

fn stage_file(
    transaction: &mut ArchiveStaging,
    path: &str,
    bytes: &[u8],
) -> Result<(), StagingError> {
    let mut input = bytes;
    transaction.write_entry(
        StagingEntry {
            kind: EntryKind::File,
            path,
            size: u64::try_from(bytes.len())
                .map_err(|_error| AssetError::TooLarge)?,
        },
        &mut input,
    )
}

fn stage_metadata(
    transaction: &mut ArchiveStaging,
    report: &MigrationReport,
    selection: MigrationMetadata<'_>,
) -> Result<(), StagingError> {
    if selection.expected != report {
        return Err(StagingError::ChangedStorage);
    }
    for file in selection.files {
        validate_metadata_name(file.name)?;
        stage_file(transaction, file.name, file.bytes)?;
    }
    Ok(())
}

fn validate_metadata_name(name: &str) -> Result<(), StagingError> {
    let _admitted = PublicationName::new(OsStr::new(name))?;
    let folded = name.to_ascii_lowercase();
    if folded == OBJECTS_DIRECTORY || folded.starts_with("bundle-") {
        return Err(StagingError::UnsupportedPath);
    }
    Ok(())
}

fn visit<Write>(
    source: LegacyStore<'_>,
    mut write: Write,
) -> Result<MigrationReport, StagingError>
where
    Write: FnMut(&str, &[u8]) -> Result<(), StagingError>,
{
    let entries = directory_entries(source.path, source.limits.entries)?;
    let mut inventory = Inventory::new();
    for entry in &entries {
        if let Some(digest) = entry
            .name
            .to_str()
            .and_then(|name| name.strip_prefix("bundle-"))
        {
            if entry.kind != PathKind::Directory {
                return Err(StagingError::ChangedStorage);
            }
            inventory.record_bundle(source, digest.parse()?, &mut write)?;
        } else if entry.kind == PathKind::RegularFile {
            inventory.report.sidecars.push(entry.name.clone());
        } else {
            return Err(StagingError::ChangedStorage);
        }
    }
    if inventory.report.manifests.is_empty()
        || entries != directory_entries(source.path, source.limits.entries)?
    {
        return Err(StagingError::ChangedStorage);
    }
    inventory.report.objects = inventory.objects.len();
    Ok(inventory.report)
}
#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::str::from_utf8;

    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, SourceIdentity,
    };

    use super::*;
    use crate::ir::storage::fixtures::{AVAILABLE, TestRoot};
    use crate::ir::storage::path_leaf;

    fn limits() -> MigrationLimits {
        MigrationLimits {
            entries: NonZeroUsize::MIN.saturating_add(7),
            manifest: ManifestLimits {
                manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
                payload_bytes: NonZeroU64::MIN.saturating_add(15),
                records: NonZeroUsize::MIN.saturating_add(3),
                total_bytes: NonZeroU64::MIN.saturating_add(63),
            },
            source_bytes: NonZeroU64::MIN.saturating_add(63),
        }
    }

    fn budget() -> ArchiveBudget {
        ArchiveBudget::new(
            NonZeroU64::MIN.saturating_add(4095),
            NonZeroU64::MIN.saturating_add(8191),
            NonZeroU64::MIN.saturating_add(31),
        )
    }

    fn fixture_manifest(key: &str) -> Result<Vec<u8>, StagingError> {
        let manifest = AssetManifest::new(
            SourceIdentity {
                title: String::from("example"),
                revision: String::from("fixture"),
            },
            vec![AssetRecord {
                digest: ContentHash::calculate(b"tile"),
                key: String::from(key),
                format: AssetFormat::Data,
                size: 4,
            }],
            limits().manifest,
        )?;
        Ok(manifest.encode(limits().manifest)?)
    }

    fn legacy(root: &TestRoot) -> LegacyStore<'_> {
        LegacyStore {
            limits: limits(),
            path: root.location(),
            title: "example",
        }
    }

    fn legacy_metadata(current: &[u8]) -> Result<Vec<u8>, StagingError> {
        let document =
            from_utf8(current).map_err(|_error| AssetError::InvalidEncoding)?;
        let fingerprint = ["0"; 32].join(",");
        Ok(document
            .replace(
                "\"source\":{",
                &format!("\"source\":{{\"digest\":[{fingerprint}],"),
            )
            .replace("\"version\":2", "\"version\":1")
            .into_bytes())
    }

    fn payload_name(digest: ContentHash) -> String {
        format!(
            "{}/{PAYLOADS_DIRECTORY}/{}",
            bundle_name(digest),
            ContentHash::calculate(b"tile")
        )
    }

    fn store_bundle(
        root: &TestRoot,
        document: &[u8],
    ) -> Result<ContentHash, StagingError> {
        let digest = ContentHash::calculate(document);
        let name = bundle_name(digest);
        root.create_directories(&format!("{name}/{PAYLOADS_DIRECTORY}"))?;
        root.write_file(&format!("{name}/{MANIFEST_NAME}"), document)?;
        root.write_file(&payload_name(digest), b"tile")?;
        Ok(digest)
    }

    fn require(condition: bool) -> Result<(), StagingError> {
        condition.then_some(()).ok_or(StagingError::ChangedStorage)
    }

    fn verify_mutations(
        parent: &TestRoot,
        output: &OsStr,
        metadata: MigrationMetadata<'_>,
    ) -> Result<(), StagingError> {
        let verify = || metadata.verify_published(output, limits().manifest);
        verify()?;
        parent.hard_link("assets/launch.local", "alias")?;
        require(verify().is_err())?;
        parent.remove_file("alias")?;
        verify()?;
        parent.write_file("assets/launch.local", b"parent=x\n")?;
        require(verify().is_err())?;
        parent.write_file("assets/launch.local", b"parent=.\n")?;
        verify()?;
        parent.write_file("assets/unexpected.txt", b"extra")?;
        require(verify().is_err())?;
        parent.remove_file("assets/unexpected.txt")?;
        let object = format!(
            "assets/{OBJECTS_DIRECTORY}/{}/{OBJECT_PAYLOAD}",
            ContentHash::calculate(b"tile")
        );
        parent.write_file(&object, b"evil")?;
        require(verify().is_err())?;
        parent.write_file(&object, b"tile")?;
        verify()
    }

    #[test]
    fn migration_reopens_normalized_manifests_and_deduplicates_objects()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let current = fixture_manifest("tiles/one")?;
        let old = store_bundle(&source, &legacy_metadata(&current)?)?;
        let first = store_bundle(&source, &current)?;
        let second = store_bundle(&source, &fixture_manifest("tiles/two")?)?;
        source.write_file("poses.txt", b"private sidecar")?;
        let report = legacy(&source).audit()?;
        require(
            report.manifests
                == BTreeMap::from([
                    (old, first),
                    (first, first),
                    (second, second),
                ]),
        )?;
        require(report.objects == 1 && report.object_bytes == 4)?;
        require(report.source_bytes == 12)?;
        require(report.sidecars == [OsString::from("poses.txt")])?;
        require(source.entry_count()? == 4)?;
        let stage = StagedMigration::prepare(
            legacy(&source),
            parent.location(),
            budget(),
        )?;
        require(stage.report() == &report)?;
        let object_path = join_path(stage.path(), OBJECTS_DIRECTORY)?;
        let objects = directory_entries(&object_path, NonZeroUsize::MIN)?;
        require(objects.len() == 1)?;
        let count = NonZeroUsize::MIN.saturating_add(2);
        require(directory_entries(stage.path(), count)?.len() == 3)?;
        for (digest, key) in [(first, "tiles/one"), (second, "tiles/two")] {
            let opened =
                CanonicalBundle::open(stage.path(), digest, limits().manifest)?;
            require(opened.read_payload(key)? == b"tile")?;
        }
        stage.verify()?;
        drop(stage);
        require(parent.entry_count()? == 0)?;
        require(legacy(&source).audit()? == report)
    }

    #[test]
    fn migration_publishes_content_and_metadata_together_without_replacement()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let digest = store_bundle(&source, &fixture_manifest("tiles/one")?)?;
        let report = legacy(&source).audit()?;
        let files = [MigrationFile {
            bytes: b"parent=.\n",
            name: "launch.local",
        }];
        let prepare = || {
            StagedMigration::prepare_with_metadata(
                legacy(&source),
                parent.location(),
                budget(),
                MigrationMetadata {
                    expected: &report,
                    files: &files,
                },
            )
        };
        let stage = prepare()?;
        let path = join_path(stage.path(), "launch.local")?;
        let maximum = NonZeroU64::MIN.saturating_add(63);
        require(
            snapshot(&path, maximum).is_ok_and(|bytes| bytes == b"parent=.\n"),
        )?;
        let receipt = match stage.publish(OsStr::new("assets")) {
            Ok(receipt) => receipt,
            Err(StagingError::Io(ErrorKind::Unsupported)) => {
                return require(parent.entry_count()? == 0);
            },
            Err(error) => return Err(error),
        };
        receipt.durability.map_err(StagingError::Io)?;
        let output = join_path(parent.location(), "assets")?;
        require(receipt.path == output)?;
        let catalogue = join_path(&output, "launch.local")?;
        require(
            snapshot(&catalogue, maximum)
                .is_ok_and(|bytes| bytes == b"parent=.\n"),
        )?;
        let bundle = CanonicalBundle::open(&output, digest, limits().manifest)?;
        require(bundle.read_payload("tiles/one")? == b"tile")?;
        require(matches!(
            prepare()?.publish(OsStr::new("assets")),
            Err(StagingError::Collision)
        ))?;
        require(
            snapshot(&catalogue, maximum)
                .is_ok_and(|bytes| bytes == b"parent=.\n"),
        )?;
        let metadata = MigrationMetadata {
            expected: &report,
            files: &files,
        };
        verify_mutations(&parent, &output, metadata)?;
        require(parent.entry_count()? == 1)?;
        require(legacy(&source).audit()? == report)
    }

    #[test]
    fn metadata_admission_rejects_reserved_duplicate_and_unbounded_files()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let _digest = store_bundle(&source, &fixture_manifest("tiles/one")?)?;
        let report = legacy(&source).audit()?;
        let prepare = |expected, files: &[MigrationFile<'_>]| {
            StagedMigration::prepare_with_metadata(
                legacy(&source),
                parent.location(),
                budget(),
                MigrationMetadata { expected, files },
            )
        };
        let file = |name| MigrationFile { bytes: b"fixture", name };
        for invalid in [
            "objects",
            "OBJECTS",
            "bundle-example",
            "BUNDLE-example",
            "nested/file.txt",
            "../outside.txt",
        ] {
            require(prepare(&report, &[file(invalid)]).is_err())?;
            require(parent.entry_count()? == 0)?;
        }
        require(
            prepare(&report, &[file("poses.txt"), file("POSES.txt")]).is_err(),
        )?;
        let oversize = vec![0; 4097];
        require(
            prepare(&report, &[MigrationFile {
                bytes: &oversize,
                name: "oversize.txt",
            }])
            .is_err(),
        )?;
        source.write_file("new-sidecar.txt", b"changed inventory")?;
        require(matches!(
            prepare(&report, &[file("launch.local")]),
            Err(StagingError::ChangedStorage)
        ))?;
        require(parent.entry_count()? == 0)
    }

    #[test]
    fn migration_refuses_publication_after_observed_metadata_change()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let _digest = store_bundle(&source, &fixture_manifest("tiles/one")?)?;
        let report = legacy(&source).audit()?;
        let stage = StagedMigration::prepare_with_metadata(
            legacy(&source),
            parent.location(),
            budget(),
            MigrationMetadata {
                expected: &report,
                files: &[MigrationFile {
                    bytes: b"parent=.\n",
                    name: "launch.local",
                }],
            },
        )?;
        let leaf = path_leaf(stage.path())?
            .to_str()
            .ok_or(StagingError::UnsupportedPath)?;
        parent.write_file(&format!("{leaf}/launch.local"), b"changed")?;
        require(matches!(
            stage.publish(OsStr::new("assets")),
            Err(StagingError::ChangedStorage)
        ))?;
        require(parent.entry_count()? == 0)?;
        require(legacy(&source).audit()? == report)
    }

    #[test]
    fn migration_verifies_each_legacy_copy_even_when_already_deduplicated()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let first = store_bundle(&source, &fixture_manifest("tiles/one")?)?;
        let second = store_bundle(&source, &fixture_manifest("tiles/two")?)?;
        source.write_file(&payload_name(first.max(second)), b"evil")?;
        require(legacy(&source).audit().is_err())?;
        require(
            StagedMigration::prepare(
                legacy(&source),
                parent.location(),
                budget(),
            )
            .is_err(),
        )?;
        require(parent.entry_count()? == 0)?;
        require(source.entry_count()? == 2)
    }

    #[test]
    fn migration_rejects_foreign_incomplete_and_unbounded_inventories()
    -> Result<(), StagingError> {
        if !AVAILABLE {
            return Ok(());
        }
        let source = TestRoot::new()?;
        let parent = TestRoot::new()?;
        let digest = store_bundle(&source, &fixture_manifest("tiles/one")?)?;
        require(
            LegacyStore {
                title: "foreign",
                ..legacy(&source)
            }
            .audit()
            .is_err(),
        )?;
        let mut short = limits();
        short.source_bytes = NonZeroU64::MIN;
        require(
            LegacyStore {
                limits: short,
                ..legacy(&source)
            }
            .audit()
            .is_err(),
        )?;
        let tiny = ArchiveBudget::new(
            NonZeroU64::MIN,
            NonZeroU64::MIN,
            NonZeroU64::MIN,
        );
        require(
            StagedMigration::prepare(legacy(&source), parent.location(), tiny)
                .is_err(),
        )?;
        require(parent.entry_count()? == 0)?;
        source.write_file(
            &format!("{}/unexpected", bundle_name(digest)),
            b"data",
        )?;
        require(legacy(&source).audit().is_err())?;
        source.remove_file(&format!("{}/unexpected", bundle_name(digest)))?;
        source.remove_file(&payload_name(digest))?;
        require(legacy(&source).audit().is_err())
    }
}
