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
//   - Bounded native reads of pinned canonical asset bundles.
// - Must-Not:
//   - Interpret title data, original media or execute imported files.
// - Allows:
//   - Inputs: Trusted parent, expected manifest digest and read limits.
//   - Outputs: Validated metadata and individually verified payload bytes.
//   - Side effects: Read-only access to descriptor-relative storage.
// - Split-When:
//   - Browser or mobile storage requires a distinct read adapter.
// - Merge-When:
//   - A shared asset reader supplies identical integrity guarantees.
// - Summary:
//   - Load canonical assets without following links or trusting file sizes.
// - Description:
//   - Pins the manifest and verifies payload integrity on every read.
// - Usage:
//   - Open a bundle and read a declared logical key before interpretation.
// - Defaults:
//   - Reject untrusted directories, special files and mismatched content.

use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
#[cfg(test)]
use std::{env, fs, path::PathBuf};

use games_assets::{
    AssetError, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};
use nix::errno::Errno;
use nix::fcntl::{open, openat};
use nix::sys::stat::Mode;
#[cfg(test)]
use nix::unistd::mkdtemp;

use crate::file::{
    DIRECTORY_FLAGS, FileReadError, read_regular, trusted_directory,
};

pub(crate) const MANIFEST_NAME: &str = "manifest.json";
pub(crate) const PAYLOADS_DIRECTORY: &str = "payloads";

/// A canonical bundle could not be safely read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleError {
    /// Canonical metadata or payload integrity failed.
    Asset(AssetError),
    /// The host filesystem rejected an operation.
    Io(io::ErrorKind),
    /// The requested logical key is absent from the pinned manifest.
    MissingKey,
    /// A directory or file is untrusted, linked or not a regular file.
    UnsafeStorage,
}

impl From<AssetError> for BundleError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

impl From<Errno> for BundleError {
    fn from(error: Errno) -> Self {
        Self::Io(io::Error::from(error).kind())
    }
}

impl From<FileReadError> for BundleError {
    fn from(error: FileReadError) -> Self {
        match error {
            FileReadError::Io(kind) => Self::Io(kind),
            FileReadError::TooLarge => Self::Asset(AssetError::TooLarge),
            FileReadError::UnsafeFile => Self::UnsafeStorage,
        }
    }
}

impl From<io::Error> for BundleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Read-only handle to a canonical asset manifest and payload directory.
///
/// The caller supplies a trusted manifest digest, not one taken from
/// untrusted metadata. A digest alone does not authenticate its origin.
/// Host-controlled ancestors must remain trusted; payloads are individually
/// opened through trusted kernel procfs after metadata-only pinning, then
/// verified on demand, not assumed valid because their filenames match.
#[derive(Debug)]
pub struct CanonicalBundle {
    manifest: AssetManifest,
    payloads: Option<OwnedFd>,
}

impl CanonicalBundle {
    /// Access the validated metadata for title-specific interpretation.
    #[must_use]
    pub const fn manifest(&self) -> &AssetManifest {
        &self.manifest
    }

    /// Open a manifest-addressed bundle beneath a trusted parent.
    ///
    /// # Errors
    /// Rejects unsafe storage, oversized or noncanonical metadata and a
    /// manifest whose bytes do not match the expected SHA-256 identity.
    pub fn open(
        parent: &Path,
        digest: ContentHash,
        limits: ManifestLimits,
    ) -> Result<Self, BundleError> {
        let parent_fd = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
        let root = open_directory(&parent_fd, &bundle_name(digest))?;
        let bytes = read_regular(
            &root,
            MANIFEST_NAME,
            u64::try_from(limits.manifest_bytes.get())
                .map_err(|_error| AssetError::TooLarge)?,
        )?;
        if ContentHash::calculate(&bytes) != digest {
            return Err(AssetError::PayloadMismatch.into());
        }
        let manifest = AssetManifest::parse(&bytes, limits)?;
        if manifest.encode(limits)? != bytes {
            return Err(AssetError::InvalidEncoding.into());
        }
        let payloads = if manifest.records().is_empty() {
            None
        } else {
            Some(open_directory(&root, PAYLOADS_DIRECTORY)?)
        };
        Ok(Self { manifest, payloads })
    }

    /// Read the declared payload for one logical key and verify its bytes.
    ///
    /// # Errors
    /// Rejects unknown keys, invalid storage, short or oversized files and
    /// any mismatch against the pinned asset descriptor.
    pub fn read_payload(&self, key: &str) -> Result<Vec<u8>, BundleError> {
        let record =
            self.manifest.record(key).ok_or(BundleError::MissingKey)?;
        let directory =
            self.payloads.as_ref().ok_or(BundleError::MissingKey)?;
        let bytes =
            read_regular(directory, &record.digest.to_string(), record.size)?;
        record.verify(&bytes)?;
        Ok(bytes)
    }
}

/// An explicitly selected, read-only passive-media replacement bundle.
///
/// The caller must pin the exact base and replacement manifests. Each
/// replacement must address an existing title-approved RGBA or PCM slot
/// with the same format and dimensions. Unknown or unapproved records
/// reject the entire overlay; corrupted payloads never silently fall back.
#[derive(Debug)]
pub struct MediaOverlay<'bundle> {
    base: &'bundle CanonicalBundle,
    replacement: &'bundle CanonicalBundle,
}

impl<'bundle> MediaOverlay<'bundle> {
    fn bundle_for(&self, key: &str) -> &CanonicalBundle {
        if self.replacement.manifest().record(key).is_some() {
            self.replacement
        } else {
            self.base
        }
    }

    /// Select a replacement after checking every declared descriptor.
    ///
    /// The allowlist must come from the trusted title, not either manifest.
    /// Payload integrity is verified on each subsequent read.
    ///
    /// # Errors
    /// Rejects another title, an unknown or unapproved slot, incompatible
    /// media, and attempts to replace declarative game data.
    pub fn new(
        base: &'bundle CanonicalBundle,
        replacement: &'bundle CanonicalBundle,
        allowed_keys: &[&str],
    ) -> Result<Self, BundleError> {
        if base.manifest().source().title
            != replacement.manifest().source().title
        {
            return Err(AssetError::InvalidIdentity.into());
        }
        for record in replacement.manifest().records() {
            base.manifest()
                .validate_media_replacement(allowed_keys, record)?;
        }
        Ok(Self { base, replacement })
    }

    /// Read the selected payload with its original bundle's integrity checks.
    ///
    /// # Errors
    /// Rejects absent keys, invalid storage and corrupted replacement or base
    /// content. A failed replacement read is never retried against the base.
    pub fn read_payload(&self, key: &str) -> Result<Vec<u8>, BundleError> {
        self.bundle_for(key).read_payload(key)
    }

    /// Return the effective descriptor for the selected payload, if present.
    #[must_use]
    pub fn record(&self, key: &str) -> Option<&AssetRecord> {
        self.bundle_for(key).manifest().record(key)
    }
}

pub(crate) fn bundle_name(digest: ContentHash) -> String {
    format!("bundle-{digest}")
}

fn open_directory(
    parent: &OwnedFd,
    name: &str,
) -> Result<OwnedFd, BundleError> {
    if !trusted_directory(parent)? {
        return Err(BundleError::UnsafeStorage);
    }
    let fd = openat(parent, name, DIRECTORY_FLAGS, Mode::empty())?;
    if !trusted_directory(&fd)? {
        return Err(BundleError::UnsafeStorage);
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use games_assets::{AssetFormat, SourceIdentity};
    use nix::unistd::mkfifo;

    use super::*;

    type FixturePayload<'data> = (AssetRecord, &'data [u8]);

    #[derive(Debug)]
    struct Fixture {
        bundle: PathBuf,
        digest: ContentHash,
        manifest: AssetManifest,
        parent: TestRoot,
        payload: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self, BundleError> {
            Self::with_records("synthetic", &[(
                record("tiles/hero", AssetFormat::Data, b"tile"),
                b"tile",
            )])
        }

        fn open(&self) -> Result<CanonicalBundle, BundleError> {
            CanonicalBundle::open(&self.parent.path, self.digest, limits())
        }

        fn with_records(
            title: &str,
            records: &[FixturePayload<'_>],
        ) -> Result<Self, BundleError> {
            let parent = TestRoot::new()?;
            let first = records.first().ok_or(BundleError::MissingKey)?;
            let name = first.0.digest.to_string();
            let manifest = AssetManifest::new(
                SourceIdentity {
                    digest: ContentHash::calculate(b"source"),
                    revision: String::from("fixture"),
                    title: String::from(title),
                },
                records
                    .iter()
                    .map(|(record, _bytes)| record.clone())
                    .collect(),
                limits(),
            )?;
            let encoded = manifest.encode(limits())?;
            let digest = ContentHash::calculate(&encoded);
            let bundle = parent.path.join(bundle_name(digest));
            let directory = bundle.join(PAYLOADS_DIRECTORY);
            fs::create_dir_all(&directory)?;
            let payload = directory.join(name);
            fs::write(bundle.join(MANIFEST_NAME), encoded)?;
            for (record, bytes) in records {
                fs::write(directory.join(record.digest.to_string()), bytes)?;
            }
            Ok(Self {
                bundle,
                digest,
                manifest,
                parent,
                payload,
            })
        }
    }

    fn limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::new(4096)
                .unwrap_or(NonZeroUsize::MIN),
            payload_bytes: NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            records: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            total_bytes: NonZeroU64::new(64).unwrap_or(NonZeroU64::MIN),
        }
    }

    fn require(condition: bool) -> Result<(), BundleError> {
        condition
            .then_some(())
            .ok_or(BundleError::Io(io::ErrorKind::InvalidData))
    }

    fn media_fixture(bytes: &[u8]) -> Result<Fixture, BundleError> {
        Fixture::with_records("synthetic", &[(
            record("sprite/hero", rgba(), bytes),
            bytes,
        )])
    }

    fn record(key: &str, format: AssetFormat, bytes: &[u8]) -> AssetRecord {
        AssetRecord {
            digest: ContentHash::calculate(bytes),
            format,
            key: String::from(key),
            size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        }
    }

    fn rgba() -> AssetFormat {
        AssetFormat::Rgba8 {
            height: NonZeroU32::MIN,
            width: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        }
    }

    #[test]
    fn overlay_reads_only_approved_media_and_keeps_base_game_data()
    -> Result<(), BundleError> {
        let base = Fixture::with_records("synthetic", &[
            (record("sprite/hero", rgba(), b"12345678"), b"12345678"),
            (record("stage/rules", AssetFormat::Data, b"rules"), b"rules"),
        ])?;
        let replacement = media_fixture(b"abcdefgh")?;
        let original = base.open()?;
        let changes = replacement.open()?;
        let overlay = MediaOverlay::new(&original, &changes, &["sprite/hero"])?;
        require(overlay.read_payload("sprite/hero")? == b"abcdefgh")?;
        require(overlay.read_payload("stage/rules")? == b"rules")?;
        require(
            overlay.record("sprite/hero")
                == changes.manifest().record("sprite/hero"),
        )?;
        require(
            overlay.record("stage/rules")
                == original.manifest().record("stage/rules"),
        )?;
        require(overlay.record("sprite/missing").is_none())?;
        require(
            overlay.read_payload("sprite/missing")
                == Err(BundleError::MissingKey),
        )
    }

    #[test]
    fn overlay_rejects_all_records_if_one_slot_is_unapproved()
    -> Result<(), BundleError> {
        let base = Fixture::with_records("synthetic", &[
            (record("sprite/hero", rgba(), b"12345678"), b"12345678"),
            (record("stage/rules", AssetFormat::Data, b"rules"), b"rules"),
        ])?;
        let replacement = Fixture::with_records("synthetic", &[
            (record("sprite/hero", rgba(), b"abcdefgh"), b"abcdefgh"),
            (record("stage/rules", AssetFormat::Data, b"other"), b"other"),
        ])?;
        let original = base.open()?;
        let changes = replacement.open()?;
        require(matches!(
            MediaOverlay::new(&original, &changes, &["sprite/hero"]),
            Err(BundleError::Asset(AssetError::InvalidIdentity))
        ))?;
        require(matches!(
            MediaOverlay::new(&original, &changes, &[
                "sprite/hero",
                "stage/rules"
            ]),
            Err(BundleError::Asset(AssetError::InvalidFormat))
        ))
    }

    #[test]
    fn overlay_rejects_other_titles_unknown_keys_and_incompatible_layout()
    -> Result<(), BundleError> {
        let base = media_fixture(b"12345678")?;
        let original = base.open()?;
        let transposed = AssetFormat::Rgba8 {
            height: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            width: NonZeroU32::MIN,
        };
        for (title, key, format, expected) in [
            (
                "other-title",
                "sprite/hero",
                rgba(),
                AssetError::InvalidIdentity,
            ),
            (
                "synthetic",
                "sprite/unknown",
                rgba(),
                AssetError::InvalidIdentity,
            ),
            (
                "synthetic",
                "sprite/hero",
                transposed,
                AssetError::InvalidFormat,
            ),
        ] {
            let replacement = Fixture::with_records(title, &[(
                record(key, format, b"abcdefgh"),
                b"abcdefgh",
            )])?;
            let changes = replacement.open()?;
            require(matches!(
                MediaOverlay::new(&original, &changes, &[key]),
                Err(BundleError::Asset(error)) if error == expected
            ))?;
        }
        Ok(())
    }

    #[test]
    fn overlay_fails_closed_on_tampering_and_never_falls_back()
    -> Result<(), BundleError> {
        let base = media_fixture(b"12345678")?;
        let replacement = media_fixture(b"abcdefgh")?;
        let original = base.open()?;
        let changes = replacement.open()?;
        let overlay = MediaOverlay::new(&original, &changes, &["sprite/hero"])?;
        fs::write(&replacement.payload, b"87654321")?;
        require(
            overlay.read_payload("sprite/hero")
                == Err(AssetError::PayloadMismatch.into()),
        )?;
        fs::remove_file(&replacement.payload)?;
        symlink(&base.payload, &replacement.payload)?;
        require(
            overlay.read_payload("sprite/hero")
                == Err(BundleError::UnsafeStorage),
        )?;
        require(original.read_payload("sprite/hero")? == b"12345678")
    }

    #[test]
    fn pinned_manifest_and_payload_are_verified() -> Result<(), BundleError> {
        let fixture = Fixture::new()?;
        let bundle = fixture.open()?;
        require(bundle.manifest() == &fixture.manifest)?;
        require(bundle.read_payload("tiles/hero")? == b"tile")?;
        for key in ["../manifest.json", "tiles/unknown", ""] {
            require(bundle.read_payload(key) == Err(BundleError::MissingKey))?;
        }
        fs::write(&fixture.payload, b"evil")?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(AssetError::PayloadMismatch.into()),
        )?;
        fs::write(&fixture.payload, b"tiles")?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(AssetError::TooLarge.into()),
        )?;
        fs::write(&fixture.payload, b"")?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(AssetError::PayloadMismatch.into()),
        )
    }

    #[test]
    fn rejects_altered_oversized_and_noncanonical_manifests()
    -> Result<(), BundleError> {
        let fixture = Fixture::new()?;
        let mut short_limits = limits();
        short_limits.manifest_bytes = NonZeroUsize::MIN;
        require(matches!(
            CanonicalBundle::open(
                &fixture.parent.path,
                fixture.digest,
                short_limits,
            ),
            Err(BundleError::Asset(AssetError::TooLarge))
        ))?;
        let mut bytes = fixture.manifest.encode(limits())?;
        bytes.push(b' ');
        fs::write(fixture.bundle.join(MANIFEST_NAME), &bytes)?;
        require(matches!(
            fixture.open(),
            Err(BundleError::Asset(AssetError::PayloadMismatch))
        ))?;
        let altered = ContentHash::calculate(&bytes);
        let directory = fixture.parent.path.join(bundle_name(altered));
        fs::rename(&fixture.bundle, &directory)?;
        require(matches!(
            CanonicalBundle::open(&fixture.parent.path, altered, limits()),
            Err(BundleError::Asset(AssetError::InvalidEncoding))
        ))
    }

    #[test]
    fn rejects_links_fifos_and_writable_storage() -> Result<(), BundleError> {
        let fixture = Fixture::new()?;
        let bundle = fixture.open()?;
        let outside = fixture.parent.path.join("outside");
        fs::write(&outside, b"tile")?;
        fs::remove_file(&fixture.payload)?;
        symlink(&outside, &fixture.payload)?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(BundleError::UnsafeStorage),
        )?;
        fs::remove_file(&fixture.payload)?;
        fs::hard_link(&outside, &fixture.payload)?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(BundleError::UnsafeStorage),
        )?;
        fs::remove_file(&fixture.payload)?;
        mkfifo(&fixture.payload, Mode::S_IRUSR | Mode::S_IWUSR)?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(BundleError::UnsafeStorage),
        )?;
        fs::remove_file(&fixture.payload)?;
        fs::write(&fixture.payload, b"tile")?;
        fs::set_permissions(
            &fixture.payload,
            fs::Permissions::from_mode(0o666),
        )?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(BundleError::UnsafeStorage),
        )?;
        let payload_directory = fixture.bundle.join(PAYLOADS_DIRECTORY);
        fs::set_permissions(
            &payload_directory,
            fs::Permissions::from_mode(0o777),
        )?;
        require(matches!(fixture.open(), Err(BundleError::UnsafeStorage)))?;
        require(
            bundle.read_payload("tiles/hero")
                == Err(BundleError::UnsafeStorage),
        )
    }

    #[test]
    fn rejects_linked_directories_and_manifest() -> Result<(), BundleError> {
        let fixture = Fixture::new()?;
        let original = fixture.parent.path.join("original");
        fs::rename(&fixture.bundle, &original)?;
        symlink(&original, &fixture.bundle)?;
        require(fixture.open().is_err())?;
        fs::remove_file(&fixture.bundle)?;
        fs::rename(&original, &fixture.bundle)?;
        let manifest_path = fixture.bundle.join(MANIFEST_NAME);
        fs::rename(&manifest_path, &original)?;
        symlink(&original, &manifest_path)?;
        require(matches!(fixture.open(), Err(BundleError::UnsafeStorage)))?;
        fs::remove_file(&manifest_path)?;
        fs::rename(&original, &manifest_path)?;
        let payloads = fixture.bundle.join(PAYLOADS_DIRECTORY);
        fs::rename(&payloads, &original)?;
        symlink(&original, &payloads)?;
        require(fixture.open().is_err())
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct TestRoot {
    pub(crate) path: PathBuf,
}

#[cfg(test)]
impl TestRoot {
    pub(crate) fn new() -> io::Result<Self> {
        let template = env::temp_dir().join("games-fixture-XXXXXX");
        Ok(Self {
            path: mkdtemp(&template)?,
        })
    }
}

#[cfg(test)]
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.path);
    }
}
