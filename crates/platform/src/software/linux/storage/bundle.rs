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
use std::num::NonZeroUsize;
use std::os::fd::OwnedFd;
use std::path::Path;

use games_assets::{AssetError, AssetRecord, ContentHash};
use nix::errno::Errno;
use nix::fcntl::{open, openat};
use nix::sys::stat::Mode;

use super::file::{DIRECTORY_FLAGS, read_regular, trusted_directory};
#[cfg(test)]
pub(super) use crate::ir::storage::bundle::CanonicalBundle;
pub(super) use crate::ir::storage::bundle::{
    BundleError, OBJECT_PAYLOAD, OBJECTS_DIRECTORY, PAYLOADS_DIRECTORY,
    bundle_name,
};
use crate::ir::storage::bundle::{
    BundlePort, MANIFEST_NAME, OpenBundle, OpenPayloads, PayloadSource,
};

impl From<Errno> for BundleError {
    fn from(error: Errno) -> Self {
        Self::Io(io::Error::from(error).kind())
    }
}

/// A pinned payload namespace remains alive for the bundle lifetime.
#[derive(Debug)]
enum DirectorySource {
    Empty,
    Legacy(OwnedFd),
    Shared(OwnedFd),
}

impl PayloadSource for DirectorySource {
    fn read(&self, record: &AssetRecord) -> Result<Vec<u8>, BundleError> {
        let name = record.digest.to_string();
        match self {
            Self::Empty => Err(BundleError::MissingKey),
            Self::Legacy(directory) => {
                Ok(read_regular(directory, &name, record.size)?)
            },
            Self::Shared(directory) => {
                let object = open_directory(directory, &name)?;
                Ok(read_regular(&object, OBJECT_PAYLOAD, record.size)?)
            },
        }
    }
}

#[derive(Debug)]
struct DirectoryBundle {
    parent: OwnedFd,
    root: OwnedFd,
}

impl BundlePort for DirectoryBundle {
    fn open_payloads(self: Box<Self>, has_payloads: bool) -> OpenPayloads {
        let source = if has_payloads {
            match open_directory(&self.root, PAYLOADS_DIRECTORY) {
                Ok(directory) => DirectorySource::Legacy(directory),
                Err(BundleError::Io(io::ErrorKind::NotFound)) => {
                    DirectorySource::Shared(open_directory(
                        &self.parent,
                        OBJECTS_DIRECTORY,
                    )?)
                },
                Err(error) => return Err(error),
            }
        } else {
            DirectorySource::Empty
        };
        Ok(Box::new(source))
    }

    fn read_manifest(
        &self,
        maximum: NonZeroUsize,
    ) -> Result<Vec<u8>, BundleError> {
        Ok(read_regular(
            &self.root,
            MANIFEST_NAME,
            u64::try_from(maximum.get())
                .map_err(|_error| AssetError::TooLarge)?,
        )?)
    }
}

pub(super) fn open_bundle(parent: &Path, digest: ContentHash) -> OpenBundle {
    let parent_fd = open(parent, DIRECTORY_FLAGS, Mode::empty())?;
    let root = open_directory(&parent_fd, &bundle_name(digest))?;
    Ok(Box::new(DirectoryBundle { parent: parent_fd, root }))
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
    use std::fs;
    use std::num::{NonZeroU32, NonZeroU64};
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::path::PathBuf;

    use games_assets::{
        AssetFormat, AssetManifest, ManifestLimits, SourceIdentity,
    };
    use nix::unistd::mkfifo;

    use super::super::fixtures::TestRoot;
    use super::*;
    use crate::ir::storage::bundle::MediaOverlay;

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
    fn legacy_bundle_keeps_its_address_and_verifies_payloads()
    -> Result<(), BundleError> {
        let fixture = Fixture::new()?;
        let encoded = fixture.manifest.encode(limits())?;
        let document = String::from_utf8(encoded)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let fingerprint = ["0"; 32].join(",");
        let legacy = document
            .replace(
                "\"source\":{",
                &format!("\"source\":{{\"digest\":[{fingerprint}],"),
            )
            .replace("\"version\":2", "\"version\":1");
        let digest = ContentHash::calculate(legacy.as_bytes());
        fs::write(fixture.bundle.join(MANIFEST_NAME), legacy.as_bytes())?;
        require(matches!(
            fixture.open(),
            Err(BundleError::Asset(AssetError::PayloadMismatch))
        ))?;
        let directory = fixture.parent.path.join(bundle_name(digest));
        fs::rename(&fixture.bundle, &directory)?;
        let bundle =
            CanonicalBundle::open(&fixture.parent.path, digest, limits())?;
        require(bundle.digest() == digest)?;
        require(bundle.manifest() == &fixture.manifest)?;
        require(bundle.read_payload("tiles/hero")? == b"tile")?;
        let payload = directory
            .join(PAYLOADS_DIRECTORY)
            .join(ContentHash::calculate(b"tile").to_string());
        fs::write(payload, b"evil")?;
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
