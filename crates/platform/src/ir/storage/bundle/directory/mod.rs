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
//   - Preflight names, then select sidecars, manifests and objects.
// - Defaults:
//   - No file content read unless explicitly selected.

use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::{AssetError, ContentHash, ManifestLimits};

use super::{
    BundleError, MANIFEST_NAME, MemoryBundleLimits, MemoryBundleStore,
    OBJECT_PAYLOAD, OBJECTS_DIRECTORY,
};
use crate::ir::security::bundle::VerifiedManifest;
use crate::ir::security::file::allocation_length;
use crate::ir::security::staging::Namespace;

/// Admission budgets for a user-selected canonical directory.
#[derive(Clone, Copy, Debug)]
pub struct DirectoryLimits {
    /// Manifest/object counts and aggregate transferred bytes, including
    /// sidecars. Unselected files consume only the inventory file budget.
    pub bundles: MemoryBundleLimits,
    /// Maximum observed files, including unselected direct sidecars.
    pub files: NonZeroUsize,
    /// Maximum length of one selected manifest.
    pub manifest: NonZeroU64,
    /// Maximum length of one selected object.
    pub object: NonZeroU64,
    /// Maximum length of one selected sidecar.
    pub sidecar: NonZeroU64,
}

/// A caller-pinned manifest and its logical asset admission limits.
#[derive(Clone, Copy, Debug)]
pub struct ManifestRequest {
    /// Exact canonical manifest address from the caller's selection.
    pub digest: ContentHash,
    /// Per-manifest limits, independent of the directory transfer budget.
    pub limits: ManifestLimits,
}

/// Immutable explicitly selected direct sidecar bytes.
#[derive(Debug, Default)]
pub struct SidecarSnapshot(BTreeMap<String, Vec<u8>>);

impl SidecarSnapshot {
    /// Access an explicitly requested direct sidecar, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// Immutable selected assets and explicitly requested local sidecars.
#[derive(Debug)]
pub struct SelectedDirectory {
    bundles: MemoryBundleStore,
    sidecars: SidecarSnapshot,
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
        self.sidecars.get(name)
    }

    /// Access the admitted sidecar snapshot as supplied to the selector.
    #[must_use]
    pub const fn sidecars(&self) -> &SidecarSnapshot {
        &self.sidecars
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Identity {
    Manifest(ContentHash),
    Object(ContentHash),
    Sidecar(String),
}

/// An observed file. Its contents are not admitted until a transfer phase
/// requests its exact canonical identity and observed length.
#[derive(Debug)]
pub struct PlannedFile {
    identity: Identity,
    size: u64,
}

impl PlannedFile {
    /// Exact observed byte length, to check before copying host data.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// A preflighted selection under construction. Discard on admission failure.
#[derive(Debug)]
pub struct DirectoryPlan {
    inventory: BTreeMap<Identity, u64>,
    limits: DirectoryLimits,
    namespace: Namespace,
    observed: usize,
    sidecars: BTreeSet<String>,
}

impl DirectoryPlan {
    /// Seal the inventory and admit only the selected sidecar transfers.
    /// # Errors
    /// Rejects a selection without any canonical manifests or over budget.
    pub fn begin(self) -> Result<DirectoryRead, BundleError> {
        if !self
            .inventory
            .keys()
            .any(|key| matches!(key, Identity::Manifest(_)))
        {
            return Err(BundleError::MissingKey);
        }
        let mut plan = TransferPlan::new(&self.inventory, self.limits, 0);
        for identity in self.inventory.keys() {
            if matches!(identity, Identity::Sidecar(_)) {
                plan.admit(identity.clone(), self.limits.sidecar.get(), None)?;
            }
        }
        let bytes = plan.bytes;
        let pending = plan.pending;
        Ok(DirectoryRead {
            bytes,
            inventory: self.inventory,
            limits: self.limits,
            manifests: BTreeMap::new(),
            objects: BTreeMap::new(),
            pending,
            phase: Phase::Sidecars,
            requests: BTreeMap::new(),
            sidecars: SidecarSnapshot::default(),
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
            inventory: BTreeMap::new(),
            limits,
            namespace: Namespace::default(),
            observed: 0,
            sidecars: selected,
        })
    }

    /// Admit a relative name and observe its size before any content read.
    /// Unselected direct sidecars return None and must not be read.
    /// # Errors
    /// Rejects ambiguous names, invalid layouts and exhausted inventory.
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
        let previous = self.inventory.insert(file_identity.clone(), size);
        if previous.is_some() {
            return Err(BundleError::UnsafeStorage);
        }
        Ok(Some(PlannedFile {
            identity: file_identity,
            size,
        }))
    }
}

/// A complete transfer admission for one phase, built before reading bytes.
struct TransferPlan<'inventory> {
    bytes: u64,
    inventory: &'inventory BTreeMap<Identity, u64>,
    limits: DirectoryLimits,
    pending: BTreeMap<Identity, u64>,
}

impl<'inventory> TransferPlan<'inventory> {
    fn admit(
        &mut self,
        identity: Identity,
        maximum: u64,
        expected: Option<u64>,
    ) -> Result<(), BundleError> {
        let size = *self
            .inventory
            .get(&identity)
            .ok_or(BundleError::MissingKey)?;
        if expected.is_some_and(|length| length != size) {
            return Err(AssetError::PayloadMismatch.into());
        }
        if size > maximum {
            return Err(AssetError::TooLarge.into());
        }
        if self.pending.contains_key(&identity) {
            return Ok(());
        }
        self.bytes = self
            .bytes
            .checked_add(size)
            .filter(|total| *total <= self.limits.bundles.bytes.get())
            .ok_or(AssetError::TooLarge)?;
        let _length = allocation_length(self.bytes)?;
        let object = matches!(identity, Identity::Object(_));
        let _previous = self.pending.insert(identity, size);
        if object && self.pending.len() > self.limits.bundles.objects.get() {
            return Err(AssetError::TooLarge.into());
        }
        Ok(())
    }

    const fn new(
        inventory: &'inventory BTreeMap<Identity, u64>,
        limits: DirectoryLimits,
        bytes: u64,
    ) -> Self {
        Self {
            bytes,
            inventory,
            limits,
            pending: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Manifests,
    Objects,
    Sidecars,
}

/// Transfer only the admitted phase: sidecars, pinned manifests, objects.
#[derive(Debug)]
pub struct DirectoryRead {
    bytes: u64,
    inventory: BTreeMap<Identity, u64>,
    limits: DirectoryLimits,
    manifests: BTreeMap<ContentHash, Vec<u8>>,
    objects: BTreeMap<ContentHash, Vec<u8>>,
    pending: BTreeMap<Identity, u64>,
    phase: Phase,
    requests: BTreeMap<ContentHash, ManifestLimits>,
    sidecars: SidecarSnapshot,
}

impl DirectoryRead {
    fn completed(&self, phase: Phase) -> Result<(), BundleError> {
        if self.phase != phase {
            return Err(BundleError::UnsafeStorage);
        }
        if !self.pending.is_empty() {
            return Err(BundleError::MissingKey);
        }
        Ok(())
    }

    /// Return the complete immutable snapshot of the requested closure.
    /// # Errors
    /// Rejects missing content or an incomplete transfer phase.
    pub fn finish(self) -> Result<SelectedDirectory, BundleError> {
        self.completed(Phase::Objects)?;
        Ok(SelectedDirectory {
            bundles: MemoryBundleStore::new(
                self.manifests,
                self.objects,
                self.limits.bundles,
            )?,
            sidecars: self.sidecars,
        })
    }

    /// Transfer exactly one requested file and verify its content identity.
    /// # Errors
    /// Rejects unplanned, changed, truncated and corrupted files.
    pub fn receive(
        &mut self,
        file: &PlannedFile,
        bytes: Vec<u8>,
    ) -> Result<(), BundleError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_error| AssetError::TooLarge)?;
        if !self.wants(file) || length != file.size {
            return Err(AssetError::PayloadMismatch.into());
        }
        if let Identity::Manifest(digest) | Identity::Object(digest) =
            &file.identity
            && ContentHash::calculate(&bytes) != *digest
        {
            return Err(AssetError::PayloadMismatch.into());
        }
        let _pending = self.pending.remove(&file.identity);
        match &file.identity {
            Identity::Manifest(digest) => {
                let _previous = self.manifests.insert(*digest, bytes);
            },
            Identity::Object(digest) => {
                let _previous = self.objects.insert(*digest, bytes);
            },
            Identity::Sidecar(name) => {
                let _previous = self.sidecars.0.insert(name.clone(), bytes);
            },
        }
        Ok(())
    }

    /// Admit all pinned manifests, coalescing repeated selections.
    /// Repeated identities must satisfy every caller's limits.
    /// # Errors
    /// Rejects missing content, an incomplete phase or exceeded budgets.
    pub fn request_manifests(
        &mut self,
        requests: &[ManifestRequest],
    ) -> Result<(), BundleError> {
        self.completed(Phase::Sidecars)?;
        if requests.is_empty() {
            return Err(BundleError::MissingKey);
        }
        let mut selected = BTreeMap::new();
        for request in requests {
            let _limits = selected
                .entry(request.digest)
                .and_modify(|limits| {
                    *limits = intersect(*limits, request.limits);
                })
                .or_insert(request.limits);
            if selected.len() > self.limits.bundles.manifests.get() {
                return Err(AssetError::TooLarge.into());
            }
        }
        let mut plan =
            TransferPlan::new(&self.inventory, self.limits, self.bytes);
        for (&digest, limits) in &selected {
            let maximum = u64::try_from(limits.manifest_bytes.get())
                .map_err(|_error| AssetError::TooLarge)?
                .min(self.limits.manifest.get());
            plan.admit(Identity::Manifest(digest), maximum, None)?;
        }
        self.bytes = plan.bytes;
        self.pending = plan.pending;
        self.requests = selected;
        self.phase = Phase::Manifests;
        Ok(())
    }

    /// Validate every requested manifest and admit its exact payload closure.
    /// Shared object identities are transferred only once.
    /// # Errors
    /// Rejects noncanonical manifests, missing objects, conflicting declared
    /// lengths, incomplete phases and exceeded transfer budgets.
    pub fn request_objects(&mut self) -> Result<(), BundleError> {
        self.completed(Phase::Manifests)?;
        let mut plan =
            TransferPlan::new(&self.inventory, self.limits, self.bytes);
        for (&digest, &limits) in &self.requests {
            let bytes =
                self.manifests.get(&digest).ok_or(BundleError::MissingKey)?;
            let manifest = VerifiedManifest::new(bytes, digest, limits)?;
            for (identity, size) in manifest.metadata().payload_sizes() {
                plan.admit(
                    Identity::Object(identity),
                    self.limits.object.get(),
                    Some(size),
                )?;
            }
        }
        self.bytes = plan.bytes;
        self.pending = plan.pending;
        self.requests.clear();
        self.phase = Phase::Objects;
        Ok(())
    }

    /// Inspect the explicitly selected sidecars before choosing manifests.
    /// # Errors
    /// Rejects an incomplete sidecar transfer or a different phase.
    pub fn sidecars(&self) -> Result<&SidecarSnapshot, BundleError> {
        self.completed(Phase::Sidecars)?;
        Ok(&self.sidecars)
    }

    /// Whether this exact observed file is admitted in the current phase.
    #[must_use]
    pub fn wants(&self, file: &PlannedFile) -> bool {
        self.pending.get(&file.identity) == Some(&file.size)
    }
}

/// Failure of either directory admission or the caller's selection.
#[derive(Debug)]
pub enum DirectorySelectionError<Error> {
    /// Picker, inventory or transfer admission failed.
    Bundle(BundleError),
    /// The caller rejected the sidecar selection.
    Selection(Error),
}

impl<Error> From<BundleError> for DirectorySelectionError<Error> {
    fn from(error: BundleError) -> Self {
        Self::Bundle(error)
    }
}

/// A pinned manifest selection and the caller state admitted with it.
pub type DirectoryChoice<State, Error> =
    Result<(Vec<ManifestRequest>, State), Error>;

/// The transferred directory and the caller state, or one rejection.
pub type DirectoryOutcome<State, Error> =
    Result<(SelectedDirectory, State), DirectorySelectionError<Error>>;
pub(crate) type DirectoryComplete<State, Error> =
    Box<dyn FnOnce(DirectoryOutcome<State, Error>)>;
pub(crate) type DirectorySelector<State, Error> =
    Box<dyn FnOnce(&SidecarSnapshot) -> DirectoryChoice<State, Error>>;

pub(crate) trait DirectoryPicker {
    fn select<State, Error>(
        plan: DirectoryPlan,
        choose: DirectorySelector<State, Error>,
        complete: DirectoryComplete<State, Error>,
    ) -> Result<(), BundleError>
    where
        State: 'static,
        Error: 'static;
}

fn intersect(first: ManifestLimits, second: ManifestLimits) -> ManifestLimits {
    ManifestLimits {
        manifest_bytes: first.manifest_bytes.min(second.manifest_bytes),
        payload_bytes: first.payload_bytes.min(second.payload_bytes),
        records: first.records.min(second.records),
        total_bytes: first.total_bytes.min(second.total_bytes),
    }
}

/// Present an explicit local directory selection to the user.
///
/// The complete namespace is preflighted. Only named sidecars, the caller's
/// pinned manifests and their verified payload closure are transferred.
/// Completion receives the caller state only after successful admission.
/// It is asynchronous and runs once after selection, cancellation
/// or read failure. Immediate admission failures do not call it.
/// # Errors
/// Rejects unsupported hosts, invalid budgets or picker setup failure.
pub fn select<Choose, Complete, State, Error>(
    limits: DirectoryLimits,
    sidecars: &[&str],
    choose: Choose,
    complete: Complete,
) -> Result<(), BundleError>
where
    Choose: FnOnce(&SidecarSnapshot) -> DirectoryChoice<State, Error> + 'static,
    Complete: FnOnce(DirectoryOutcome<State, Error>) + 'static,
    State: 'static,
    Error: 'static,
{
    let plan = DirectoryPlan::new(limits, sidecars)?;
    <() as DirectoryPicker>::select(plan, Box::new(choose), Box::new(complete))
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
        AssetFormat, AssetManifest, AssetRecord, SourceIdentity,
    };

    use super::*;

    type FixtureRecord<'fixture> = (&'fixture str, &'fixture [u8]);

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
            files: NonZeroUsize::MIN.saturating_add(7),
            manifest: NonZeroU64::MIN.saturating_add(1023),
            object: NonZeroU64::MIN.saturating_add(7),
            sidecar: NonZeroU64::MIN.saturating_add(15),
        }
    }

    fn manifest_limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(1023),
            payload_bytes: NonZeroU64::MIN.saturating_add(7),
            records: NonZeroUsize::MIN.saturating_add(1),
            total_bytes: NonZeroU64::MIN.saturating_add(15),
        }
    }

    fn manifest(records: &[FixtureRecord<'_>]) -> Result<Vec<u8>, BundleError> {
        let descriptors = records
            .iter()
            .map(|(key, bytes)| {
                Ok(AssetRecord {
                    key: (*key).into(),
                    format: AssetFormat::Data,
                    size: length(bytes)?,
                    digest: ContentHash::calculate(bytes),
                })
            })
            .collect::<Result<Vec<_>, BundleError>>()?;
        let limits = manifest_limits();
        Ok(AssetManifest::new(
            SourceIdentity {
                title: "synthetic".into(),
                revision: "fixture".into(),
            },
            descriptors,
            limits,
        )?
        .encode(limits)?)
    }

    fn length(bytes: &[u8]) -> Result<u64, BundleError> {
        u64::try_from(bytes.len()).map_err(|_error| AssetError::TooLarge.into())
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

    fn request(bytes: &[u8]) -> ManifestRequest {
        ManifestRequest {
            digest: ContentHash::calculate(bytes),
            limits: manifest_limits(),
        }
    }

    #[test]
    fn directory_admission_reuses_canonical_bundle_readback()
    -> Result<(), BundleError> {
        let content = b"tile";
        let bytes = manifest(&[("tile", content)])?;
        let second = manifest(&[("other", content)])?;
        let mut bounds = limits();
        bounds.bundles.manifests = NonZeroUsize::MIN.saturating_add(1);
        bounds.bundles.bytes = NonZeroU64::new(
            length(&bytes)?
                .saturating_add(length(&second)?)
                .saturating_add(length(content)?)
                .saturating_add(3),
        )
        .ok_or(BundleError::UnsafeStorage)?;
        let mut plan = DirectoryPlan::new(bounds, &["launch.local"])?;
        let first =
            selected(&mut plan, &manifest_path(&bytes), length(&bytes)?)?;
        let second_file =
            selected(&mut plan, &manifest_path(&second), length(&second)?)?;
        let object = selected(&mut plan, &object_path(content), 4)?;
        let sidecar = selected(&mut plan, "launch.local", 3)?;
        let unused = selected(&mut plan, &manifest_path(b"unused"), u64::MAX)?;
        let unused_object =
            selected(&mut plan, &object_path(b"unused"), u64::MAX)?;
        require(plan.observe("other.txt", u64::MAX)?.is_none())?;
        let mut reader = plan.begin()?;
        require(reader.wants(&sidecar) && !reader.wants(&first))?;
        require(reader.sidecars().is_err())?;
        require(reader.request_manifests(&[request(&bytes)]).is_err())?;
        require(reader.receive(&object, content.to_vec()).is_err())?;
        reader.receive(&sidecar, b"abc".to_vec())?;
        require(
            reader.sidecars()?.get("launch.local") == Some(b"abc".as_slice()),
        )?;
        reader.request_manifests(&[
            request(&bytes),
            request(&second),
            request(&bytes),
        ])?;
        require(reader.wants(&first) && reader.wants(&second_file))?;
        require(!reader.wants(&unused) && !reader.wants(&object))?;
        reader.receive(&first, bytes.clone())?;
        require(reader.request_objects().is_err())?;
        reader.receive(&second_file, second.clone())?;
        reader.request_objects()?;
        require(reader.wants(&object) && !reader.wants(&unused_object))?;
        reader.receive(&object, content.to_vec())?;
        let directory = reader.finish()?;
        require(directory.sidecar("launch.local") == Some(b"abc".as_slice()))?;
        require(directory.sidecar("other.txt").is_none())?;
        for (encoded, key) in [(&bytes, "tile"), (&second, "other")] {
            let bundle = directory
                .bundles()
                .open(ContentHash::calculate(encoded), manifest_limits())?;
            require(bundle.read_payload(key)? == content)?;
        }
        require(
            directory
                .bundles()
                .open(ContentHash::calculate(b"unused"), manifest_limits())
                .is_err(),
        )
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
        let mut full = DirectoryPlan::new(limits(), &[])?;
        for n in 0u32..8 {
            require(full.observe(&format!("ignored-{n}"), 0)?.is_none())?;
        }
        require(full.observe("excess", 0).is_err())
    }

    #[test]
    fn manifest_requests_reject_count_size_and_missing_before_transfer()
    -> Result<(), BundleError> {
        let mut plan = DirectoryPlan::new(limits(), &["launch.local"])?;
        let _manifest = selected(&mut plan, &manifest_path(b"m"), 1)?;
        let _sidecar = selected(&mut plan, "launch.local", 17)?;
        require(plan.begin().is_err())?;

        let mut inventory = DirectoryPlan::new(limits(), &[])?;
        let oversized = selected(&mut inventory, &manifest_path(b"m"), 1025)?;
        let normal = selected(&mut inventory, &manifest_path(b"n"), 1)?;
        let mut reader = inventory.begin()?;
        require(reader.request_manifests(&[]).is_err())?;
        require(reader.request_manifests(&[request(b"missing")]).is_err())?;
        require(reader.request_manifests(&[request(b"m")]).is_err())?;
        require(
            reader
                .request_manifests(&[request(b"m"), request(b"n")])
                .is_err(),
        )?;
        require(!reader.wants(&oversized) && !reader.wants(&normal))?;
        reader.request_manifests(&[request(b"n"), request(b"n")])?;
        require(reader.wants(&normal))
    }

    #[test]
    fn object_closure_rejects_unavailable_or_overbudget_before_transfer()
    -> Result<(), BundleError> {
        let content = b"tile";
        let bytes = manifest(&[("a", content), ("b", b"other")])?;
        for (object_size, object_limit, count, bytes_limit) in [
            (4, 8u64, 1usize, 4096),
            (3, 8, 2, 4096),
            (4, 3, 2, 4096),
            (4, 8, 2, length(&bytes)?.saturating_add(8)),
        ] {
            let mut bounds = limits();
            bounds.bundles.objects =
                NonZeroUsize::MIN.saturating_add(count.saturating_sub(1));
            bounds.object =
                NonZeroU64::MIN.saturating_add(object_limit.saturating_sub(1));
            bounds.bundles.bytes = NonZeroU64::new(bytes_limit)
                .ok_or(BundleError::UnsafeStorage)?;
            let mut plan = DirectoryPlan::new(bounds, &[])?;
            let manifest_file =
                selected(&mut plan, &manifest_path(&bytes), length(&bytes)?)?;
            let first =
                selected(&mut plan, &object_path(content), object_size)?;
            let second = selected(&mut plan, &object_path(b"other"), 5)?;
            let mut reader = plan.begin()?;
            reader.request_manifests(&[request(&bytes)])?;
            reader.receive(&manifest_file, bytes.clone())?;
            require(reader.request_objects().is_err())?;
            require(!reader.wants(&first) && !reader.wants(&second))?;
        }
        let mut plan = DirectoryPlan::new(limits(), &[])?;
        let manifest_file =
            selected(&mut plan, &manifest_path(&bytes), length(&bytes)?)?;
        let mut reader = plan.begin()?;
        reader.request_manifests(&[request(&bytes)])?;
        reader.receive(&manifest_file, bytes)?;
        require(reader.request_objects().is_err())
    }

    #[test]
    fn transfer_rejects_missing_short_and_corrupt_files()
    -> Result<(), BundleError> {
        require(DirectoryPlan::new(limits(), &[])?.begin().is_err())?;
        let content = b"tile";
        let bytes = manifest(&[("tile", content)])?;
        let mut plan = DirectoryPlan::new(limits(), &[])?;
        let manifest_file =
            selected(&mut plan, &manifest_path(&bytes), length(&bytes)?)?;
        let object = selected(&mut plan, &object_path(content), 4)?;
        let mut reader = plan.begin()?;
        reader.request_manifests(&[request(&bytes)])?;
        require(reader.receive(&manifest_file, Vec::new()).is_err())?;
        require(
            reader
                .receive(&manifest_file, vec![0; bytes.len()])
                .is_err(),
        )?;
        reader.receive(&manifest_file, bytes)?;
        reader.request_objects()?;
        require(reader.receive(&object, b"fake".to_vec()).is_err())?;
        require(reader.receive(&object, b"til".to_vec()).is_err())?;
        require(reader.finish().is_err())
    }

    #[test]
    fn canonical_manifest_and_all_repeated_limits_precede_objects()
    -> Result<(), BundleError> {
        let bytes = manifest(&[("a", b"one"), ("b", b"two")])?;
        let mut plan = DirectoryPlan::new(limits(), &[])?;
        let file =
            selected(&mut plan, &manifest_path(&bytes), length(&bytes)?)?;
        let mut reader = plan.begin()?;
        let mut strict = request(&bytes);
        strict.limits.manifest_bytes = NonZeroUsize::MIN;
        require(
            reader
                .request_manifests(&[request(&bytes), strict])
                .is_err(),
        )?;
        strict = request(&bytes);
        strict.limits.records = NonZeroUsize::MIN;
        reader.request_manifests(&[request(&bytes), strict])?;
        reader.receive(&file, bytes)?;
        require(reader.request_objects().is_err())?;

        let mut invalid = DirectoryPlan::new(limits(), &[])?;
        let invalid_file = selected(&mut invalid, &manifest_path(b"bad"), 3)?;
        let mut rejected = invalid.begin()?;
        rejected.request_manifests(&[request(b"bad")])?;
        rejected.receive(&invalid_file, b"bad".to_vec())?;
        require(rejected.request_objects().is_err())
    }
}
