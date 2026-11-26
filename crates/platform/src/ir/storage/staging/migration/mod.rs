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
//   - Verification and private staging of legacy canonical title stores.
// - Must-Not:
//   - Delete originals, publish a migration or access native descriptors.
// - Allows:
//   - Inputs: Explicit legacy store, title, budgets and staging parent.
//   - Outputs: Old-to-current manifest mapping and verified private store.
//   - Side effects: Bounded reads and optional private staging.
// - Split-When:
//   - Cutover introduces independent consumer or retention policy.
// - Merge-When:
//   - A shared store transaction provides identical migration guarantees.
// - Summary:
//   - Verify legacy assets before changing their persistent layout.
// - Description:
//   - Normalize metadata and stage one verified object per content hash.
// - Usage:
//   - Audit a trusted legacy store or prepare a private rehearsal.
// - Defaults:
//   - Reject incomplete inventories and leave source files untouched.

//! Read-only migration planning and private shared-store rehearsal.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::archive::{ArchiveBudget, EntryKind};
use games_assets::{AssetError, ContentHash, ManifestLimits};

use super::{
    ArchiveStaging, PreparedManifest, StagedArchive, StagingEntry, StagingError,
};
use crate::ir::storage::bundle::{
    CanonicalBundle, MANIFEST_NAME, OBJECT_PAYLOAD, OBJECTS_DIRECTORY,
    PAYLOADS_DIRECTORY, bundle_name,
};
use crate::ir::storage::{PathKind, directory_entries, join_path};

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
        let mut transaction = ArchiveStaging::new(parent, budget)?;
        let report = visit(source, |path, bytes| {
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
        })?;
        let stage = transaction.finish()?;
        read_back(stage.path(), &report, source.limits.manifest)?;
        stage.verify()?;
        Ok(Self { report, stage })
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

fn read_back(
    stage: &OsStr,
    report: &MigrationReport,
    limits: ManifestLimits,
) -> Result<(), StagingError> {
    let mut manifests = BTreeSet::new();
    for &digest in report.manifests.values() {
        if !manifests.insert(digest) {
            continue;
        }
        let bundle = CanonicalBundle::open(stage, digest, limits)?;
        let mut objects = BTreeSet::new();
        for record in bundle.manifest().records() {
            if objects.insert(record.digest) {
                drop(bundle.read_payload(&record.key)?);
            }
        }
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
    use std::str::from_utf8;

    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, SourceIdentity,
    };

    use super::*;
    use crate::ir::storage::fixtures::{AVAILABLE, TestRoot};

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
