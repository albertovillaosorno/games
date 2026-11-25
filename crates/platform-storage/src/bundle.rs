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

use games_assets::{AssetError, AssetManifest, ContentHash, ManifestLimits};
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{FileStat, Mode, fstat, fstatat};
use nix::unistd::geteuid;
#[cfg(test)]
use nix::unistd::mkdtemp;

use crate::file::{FileReadError, file_size, read_bounded_descriptor, regular};

pub(crate) const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);
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

pub(crate) fn bundle_name(digest: ContentHash) -> String {
    format!("bundle-{digest}")
}

pub(crate) fn trusted_directory(fd: &OwnedFd) -> nix::Result<bool> {
    let stat = fstat(fd)?;
    Ok(stat.st_mode & 0o022 == 0 && stat.st_uid == geteuid().as_raw())
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

fn private_regular(stat: &FileStat) -> bool {
    regular(stat)
        && stat.st_nlink == 1
        && stat.st_uid == geteuid().as_raw()
        && stat.st_mode & 0o022 == 0
}

fn read_regular(
    parent: &OwnedFd,
    name: &str,
    maximum: u64,
) -> Result<Vec<u8>, BundleError> {
    if !trusted_directory(parent)? {
        return Err(BundleError::UnsafeStorage);
    }
    let before = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if !private_regular(&before) {
        return Err(BundleError::UnsafeStorage);
    }
    let fd = openat(
        parent,
        name,
        OFlag::O_RDONLY
            | OFlag::O_NOFOLLOW
            | OFlag::O_NONBLOCK
            | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let stat = fstat(&fd)?;
    if !private_regular(&stat)
        || stat.st_dev != before.st_dev
        || stat.st_ino != before.st_ino
    {
        return Err(BundleError::UnsafeStorage);
    }
    Ok(read_bounded_descriptor(fd, file_size(&stat)?, maximum)?)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU64, NonZeroUsize};
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use games_assets::{AssetFormat, AssetRecord, SourceIdentity};
    use nix::unistd::mkfifo;

    use super::*;

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
            let parent = TestRoot::new()?;
            let record = AssetRecord {
                digest: ContentHash::calculate(b"tile"),
                format: AssetFormat::Data,
                key: String::from("tiles/hero"),
                size: 4,
            };
            let name = record.digest.to_string();
            let manifest = AssetManifest::new(
                SourceIdentity {
                    digest: ContentHash::calculate(b"source"),
                    revision: String::from("fixture"),
                    title: String::from("synthetic"),
                },
                vec![record],
                limits(),
            )?;
            let encoded = manifest.encode(limits())?;
            let digest = ContentHash::calculate(&encoded);
            let bundle = parent.path.join(bundle_name(digest));
            let directory = bundle.join(PAYLOADS_DIRECTORY);
            fs::create_dir_all(&directory)?;
            let payload = directory.join(name);
            fs::write(bundle.join(MANIFEST_NAME), encoded)?;
            fs::write(&payload, b"tile")?;
            Ok(Self {
                bundle,
                digest,
                manifest,
                parent,
                payload,
            })
        }

        fn open(&self) -> Result<CanonicalBundle, BundleError> {
            CanonicalBundle::open(&self.parent.path, self.digest, limits())
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
