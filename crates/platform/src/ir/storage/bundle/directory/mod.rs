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
//   - Bounded admission of an explicitly selected canonical directory.
// - Must-Not:
//   - Access host file pickers, resolve paths or interpret title catalogues.
// - Allows:
//   - Inputs: Relative names, observed sizes, selected sidecars and bytes.
//   - Outputs: Shared memory bundles and selected sidecar snapshots.
//   - Side effects: Bounded caller-owned memory allocations.
// - Split-When:
//   - A different asset namespace needs independent admission.
// - Merge-When:
//   - Another directory reader duplicates canonical inventory admission.
// - Summary:
//   - Preflight the selected namespace before transferring content.
// - Description:
//   - Reject ambiguous names, legacy layouts and mismatched identities.
// - Usage:
//   - Admit all observations, transfer planned files, then finish.
// - Defaults:
//   - No sidecar content read unless explicitly selected.

use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::{AssetError, ContentHash};

use super::{
    BundleError, MANIFEST_NAME, MemoryBundleLimits, MemoryBundleStore,
    OBJECT_PAYLOAD, OBJECTS_DIRECTORY,
};
use crate::ir::security::file::allocation_length;
use crate::ir::security::staging::Namespace;

/// Admission budgets for a user-selected canonical directory.
#[derive(Clone, Copy, Debug)]
pub struct DirectoryLimits {
    /// Manifest/object counts and aggregate selected bytes, including
    /// sidecars.
    pub bundles: MemoryBundleLimits,
    /// Maximum observed files, including unselected direct sidecars.
    pub files: NonZeroUsize,
    /// Maximum length of one manifest.
    pub manifest: NonZeroU64,
    /// Maximum length of one object.
    pub object: NonZeroU64,
    /// Maximum length of one selected sidecar.
    pub sidecar: NonZeroU64,
}

/// Immutable selected assets and explicitly requested local sidecars.
#[derive(Debug)]
pub struct SelectedDirectory {
    bundles: MemoryBundleStore,
    sidecars: BTreeMap<String, Vec<u8>>,
}

impl SelectedDirectory {
    /// Access the same verified memory bundle interface as other consumers.
    #[must_use]
    pub const fn bundles(&self) -> &MemoryBundleStore {
        &self.bundles
    }

    /// Access an explicitly requested direct sidecar, if present.
    #[must_use]
    pub fn sidecar(&self, name: &str) -> Option<&[u8]> {
        self.sidecars.get(name).map(Vec::as_slice)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Identity {
    Manifest(ContentHash),
    Object(ContentHash),
    Sidecar(String),
}

/// A selected file whose exact size and canonical identity were admitted.
#[derive(Debug)]
pub struct PlannedFile {
    identity: Identity,
    size: u64,
}

impl PlannedFile {
    /// Exact admitted byte length, to check before copying host data.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// A preflighted selection under construction. Discard on admission failure.
#[derive(Debug)]
pub struct DirectoryPlan {
    bytes: u64,
    limits: DirectoryLimits,
    manifests: usize,
    namespace: Namespace,
    objects: usize,
    observed: usize,
    pending: BTreeMap<Identity, u64>,
    sidecars: BTreeSet<String>,
}

impl DirectoryPlan {
    /// Seal the selection before transferring any file contents.
    /// # Errors
    /// Rejects a selection without any canonical manifests.
    pub fn begin(self) -> Result<DirectoryRead, BundleError> {
        if self.manifests == 0 {
            return Err(BundleError::MissingKey);
        }
        Ok(DirectoryRead {
            limits: self.limits.bundles,
            manifests: BTreeMap::new(),
            objects: BTreeMap::new(),
            pending: self.pending,
            sidecars: BTreeMap::new(),
        })
    }

    /// Maximum number of files the adapter may enumerate.
    #[must_use]
    pub const fn file_limit(&self) -> NonZeroUsize {
        self.limits.files
    }

    fn identity(&self, path: &str) -> Result<Option<Identity>, BundleError> {
        let mut parts = path.split('/');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(name), None, None, None) => {
                if reserved_root(name) {
                    return Err(BundleError::UnsafeStorage);
                }
                Ok(self
                    .sidecars
                    .contains(name)
                    .then(|| Identity::Sidecar(name.to_owned())))
            },
            (Some(bundle), Some(MANIFEST_NAME), None, None) => {
                let digest = bundle
                    .strip_prefix("bundle-")
                    .ok_or(BundleError::UnsafeStorage)?
                    .parse::<ContentHash>()?;
                Ok(Some(Identity::Manifest(digest)))
            },
            (
                Some(OBJECTS_DIRECTORY),
                Some(digest),
                Some(OBJECT_PAYLOAD),
                None,
            ) => Ok(Some(Identity::Object(digest.parse()?))),
            _ => Err(BundleError::UnsafeStorage),
        }
    }

    /// Select direct sidecars and explicit memory and inventory budgets.
    /// # Errors
    /// Rejects unsafe or duplicate sidecar names and excessive counts.
    pub fn new(
        limits: DirectoryLimits,
        sidecars: &[&str],
    ) -> Result<Self, BundleError> {
        if sidecars.len() > limits.files.get() {
            return Err(AssetError::TooLarge.into());
        }
        let mut selected = BTreeSet::new();
        let mut namespace = Namespace::default();
        for name in sidecars {
            if name.contains('/') || reserved_root(name) {
                return Err(BundleError::UnsafeStorage);
            }
            let _admitted = namespace
                .admit_authored_path(name)
                .map_err(|_error| BundleError::UnsafeStorage)?;
            let _inserted = selected.insert((*name).to_owned());
        }
        Ok(Self {
            bytes: 0,
            limits,
            manifests: 0,
            namespace: Namespace::default(),
            objects: 0,
            observed: 0,
            pending: BTreeMap::new(),
            sidecars: selected,
        })
    }

    /// Admit a relative file before reading its contents.
    /// Unselected direct sidecars return None and must not be read.
    /// # Errors
    /// Rejects ambiguous names, invalid layouts and exhausted budgets.
    pub fn observe(
        &mut self,
        path: &str,
        size: u64,
    ) -> Result<Option<PlannedFile>, BundleError> {
        if self.observed >= self.limits.files.get() {
            return Err(AssetError::TooLarge.into());
        }
        let identity = self.identity(path)?;
        let _admitted = self
            .namespace
            .admit_authored_path(path)
            .map_err(|_error| BundleError::UnsafeStorage)?;
        self.observed = self.observed.saturating_add(1);
        let Some(file_identity) = identity else {
            return Ok(None);
        };
        let limit = match file_identity {
            Identity::Manifest(_) => {
                if self.manifests >= self.limits.bundles.manifests.get() {
                    return Err(AssetError::TooLarge.into());
                }
                self.manifests = self.manifests.saturating_add(1);
                self.limits.manifest
            },
            Identity::Object(_) => {
                if self.objects >= self.limits.bundles.objects.get() {
                    return Err(AssetError::TooLarge.into());
                }
                self.objects = self.objects.saturating_add(1);
                self.limits.object
            },
            Identity::Sidecar(_) => self.limits.sidecar,
        };
        if size > limit.get() {
            return Err(AssetError::TooLarge.into());
        }
        self.bytes = self
            .bytes
            .checked_add(size)
            .filter(|total| *total <= self.limits.bundles.bytes.get())
            .ok_or(AssetError::TooLarge)?;
        let _length = allocation_length(self.bytes)?;
        let _previous = self.pending.insert(file_identity.clone(), size);
        Ok(Some(PlannedFile {
            identity: file_identity,
            size,
        }))
    }
}

/// Transfer the preflighted files without further namespace expansion.
#[derive(Debug)]
pub struct DirectoryRead {
    limits: MemoryBundleLimits,
    manifests: BTreeMap<ContentHash, Vec<u8>>,
    objects: BTreeMap<ContentHash, Vec<u8>>,
    pending: BTreeMap<Identity, u64>,
    sidecars: BTreeMap<String, Vec<u8>>,
}

impl DirectoryRead {
    /// Return the complete immutable snapshot.
    /// # Errors
    /// Rejects missing or excessive contents.
    pub fn finish(self) -> Result<SelectedDirectory, BundleError> {
        if !self.pending.is_empty() {
            return Err(BundleError::MissingKey);
        }
        Ok(SelectedDirectory {
            bundles: MemoryBundleStore::new(
                self.manifests,
                self.objects,
                self.limits,
            )?,
            sidecars: self.sidecars,
        })
    }

    /// Transfer exactly one selected file and verify its content identity.
    /// # Errors
    /// Rejects unplanned, changed, truncated and corrupted files.
    pub fn receive(
        &mut self,
        file: PlannedFile,
        bytes: Vec<u8>,
    ) -> Result<(), BundleError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        if self.pending.get(&file.identity) != Some(&file.size)
            || length != file.size
        {
            return Err(AssetError::PayloadMismatch.into());
        }
        if let Identity::Manifest(digest) | Identity::Object(digest) =
            &file.identity
            && ContentHash::calculate(&bytes) != *digest
        {
            return Err(AssetError::PayloadMismatch.into());
        }
        let _pending = self.pending.remove(&file.identity);
        match file.identity {
            Identity::Manifest(digest) => {
                let _previous = self.manifests.insert(digest, bytes);
            },
            Identity::Object(digest) => {
                let _previous = self.objects.insert(digest, bytes);
            },
            Identity::Sidecar(name) => {
                let _previous = self.sidecars.insert(name, bytes);
            },
        }
        Ok(())
    }
}

pub(crate) type DirectoryComplete =
    Box<dyn FnOnce(Result<SelectedDirectory, BundleError>)>;

pub(crate) trait DirectoryPicker {
    fn select(
        plan: DirectoryPlan,
        complete: DirectoryComplete,
    ) -> Result<(), BundleError>;
}

/// Present an explicit local directory selection to the user.
///
/// Only canonical objects, manifests and the named direct sidecars are
/// transferred. Completion is asynchronous and runs once after selection,
/// cancellation or read failure. Immediate admission failures do not call it.
/// # Errors
/// Rejects unsupported hosts, invalid budgets or picker setup failure.
pub fn select<Complete>(
    limits: DirectoryLimits,
    sidecars: &[&str],
    complete: Complete,
) -> Result<(), BundleError>
where
    Complete: FnOnce(Result<SelectedDirectory, BundleError>) + 'static,
{
    let plan = DirectoryPlan::new(limits, sidecars)?;
    <() as DirectoryPicker>::select(plan, Box::new(complete))
}

fn reserved_root(name: &str) -> bool {
    name.eq_ignore_ascii_case(OBJECTS_DIRECTORY)
        || name
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bundle-"))
}

#[cfg(test)]
mod tests {
    use games_assets::{
        AssetFormat, AssetManifest, AssetRecord, ManifestLimits, SourceIdentity,
    };

    use super::*;

    fn require(condition: bool) -> Result<(), BundleError> {
        condition.then_some(()).ok_or(BundleError::UnsafeStorage)
    }

    fn limits() -> DirectoryLimits {
        DirectoryLimits {
            bundles: MemoryBundleLimits {
                bytes: NonZeroU64::MIN.saturating_add(4095),
                manifests: NonZeroUsize::MIN,
                objects: NonZeroUsize::MIN,
            },
            files: NonZeroUsize::MIN.saturating_add(3),
            manifest: NonZeroU64::MIN.saturating_add(1023),
            object: NonZeroU64::MIN.saturating_add(7),
            sidecar: NonZeroU64::MIN.saturating_add(15),
        }
    }

    fn manifest_path(bytes: &[u8]) -> String {
        format!("bundle-{}/manifest.json", ContentHash::calculate(bytes))
    }

    fn object_path(bytes: &[u8]) -> String {
        format!("objects/{}/payload", ContentHash::calculate(bytes))
    }

    fn selected(
        plan: &mut DirectoryPlan,
        path: &str,
        size: u64,
    ) -> Result<PlannedFile, BundleError> {
        plan.observe(path, size)?.ok_or(BundleError::MissingKey)
    }

    #[test]
    fn directory_admission_reuses_canonical_bundle_readback()
    -> Result<(), BundleError> {
        let content = b"tile";
        let identity = ContentHash::calculate(content);
        let manifest_limits = ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(1023),
            payload_bytes: NonZeroU64::MIN.saturating_add(7),
            records: NonZeroUsize::MIN,
            total_bytes: NonZeroU64::MIN.saturating_add(7),
        };
        let manifest = AssetManifest::new(
            SourceIdentity {
                title: "synthetic".into(),
                revision: "fixture".into(),
            },
            vec![AssetRecord {
                key: "tile".into(),
                format: AssetFormat::Data,
                size: 4,
                digest: identity,
            }],
            manifest_limits,
        )?;
        let bytes = manifest.encode(manifest_limits)?;
        let digest = ContentHash::calculate(&bytes);
        let size = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
        let manifest_file = selected(&mut plan, &manifest_path(&bytes), size)?;
        let object = selected(&mut plan, &object_path(content), 4)?;
        let sidecar = selected(&mut plan, "launch.local", 3)?;
        require(plan.observe("other.txt", u64::MAX)?.is_none())?;
        require(manifest_file.size() == size)?;
        let mut reader = plan.begin()?;
        reader.receive(object, content.to_vec())?;
        reader.receive(sidecar, b"abc".to_vec())?;
        reader.receive(manifest_file, bytes)?;
        let directory = reader.finish()?;
        require(directory.sidecar("launch.local") == Some(b"abc".as_slice()))?;
        require(directory.sidecar("other.txt").is_none())?;
        let bundle = directory.bundles().open(digest, manifest_limits)?;
        require(bundle.read_payload("tile")? == content)?;
        Ok(())
    }

    #[test]
    fn preflight_rejects_legacy_unsafe_and_ambiguous_paths()
    -> Result<(), BundleError> {
        let hash = ContentHash::calculate(b"tile");
        for path in [
            "../launch.local".into(),
            "objects".into(),
            format!("bundle-{hash}/payloads/{hash}"),
            format!("objects/{hash}/extra/payload"),
            format!("objects/{hash}/PAYLOAD"),
            format!("BUNDLE-{hash}/manifest.json"),
            format!("objects/{}/payload", hash.to_string().to_uppercase()),
        ] {
            let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
            require(plan.observe(&path, 1).is_err())?;
        }
        for sidecars in [
            vec!["../outside"],
            vec!["launch.local", "LAUNCH.LOCAL"],
            vec!["launch.local", "launch.local"],
        ] {
            require(DirectoryPlan::new(limits(), &sidecars).is_err())?;
        }
        let mut plan = DirectoryPlan::new(limits(), &[])?;
        let path = manifest_path(b"manifest");
        let _file = selected(&mut plan, &path, 8)?;
        require(plan.observe(&path, 8).is_err())?;
        Ok(())
    }

    #[test]
    fn preflight_rejects_count_and_size_before_receiving_bytes()
    -> Result<(), BundleError> {
        {
            let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
            require(plan.observe(&manifest_path(b"m"), 1025).is_err())?;
        }
        {
            let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
            require(plan.observe(&object_path(b"o"), 9).is_err())?;
        }
        {
            let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
            require(plan.observe("launch.local", 17).is_err())?;
        }
        {
            let mut plan = DirectoryPlan::new(limits(), &[])?;
            let _file = selected(&mut plan, &manifest_path(b"a"), 1)?;
            require(plan.observe(&manifest_path(b"b"), 1).is_err())?;
        }
        {
            let mut plan = DirectoryPlan::new(limits(), &[])?;
            let _file = selected(&mut plan, &object_path(b"a"), 1)?;
            require(plan.observe(&object_path(b"b"), 1).is_err())?;
        }
        {
            let mut small = limits();
            small.bundles.bytes = NonZeroU64::MIN.saturating_add(4);
            let mut plan = DirectoryPlan::new(small, &[])?;
            let _file = selected(&mut plan, &manifest_path(b"abc"), 3)?;
            require(plan.observe(&object_path(b"def"), 3).is_err())?;
        }
        {
            let mut plan = DirectoryPlan::new(limits(), &[])?;
            for n in 0u32..4 {
                require(plan.observe(&format!("ignored-{n}"), 0)?.is_none())?;
            }
            require(plan.observe("excess", 0).is_err())?;
        }
        Ok(())
    }

    #[test]
    fn transfer_rejects_missing_short_and_corrupt_files()
    -> Result<(), BundleError> {
        let mut plan = DirectoryPlan::new(limits(), &[])?;
        let manifest = selected(&mut plan, &manifest_path(b"m"), 1)?;
        let object = selected(&mut plan, &object_path(b"o"), 1)?;
        let mut reader = plan.begin()?;
        reader.receive(manifest, b"m".to_vec())?;
        require(reader.receive(object, b"x".to_vec()).is_err())?;
        require(reader.finish().is_err())?;
        let mut short_plan = DirectoryPlan::new(limits(), &[])?;
        let short_file = selected(&mut short_plan, &manifest_path(b"m"), 1)?;
        let mut short_reader = short_plan.begin()?;
        require(short_reader.receive(short_file, Vec::new()).is_err())?;
        require(short_reader.finish().is_err())?;
        require(DirectoryPlan::new(limits(), &[])?.begin().is_err())?;
        Ok(())
    }
}
