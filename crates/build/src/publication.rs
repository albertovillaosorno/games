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
//   - Composition of selected asset conversion and native bundle access.
// - Must-Not:
//   - Execute originals, decode media layouts or bypass platform storage.
// - Allows:
//   - Inputs: Bounded source, explicit budgets, trusted publication parent.
//   - Outputs: Published bundle receipt or verified payload summary.
//   - Side effects: Private staging, atomic publication and stdout summaries.
// - Split-When:
//   - Other title asset policies need separate host adapters.
// - Merge-When:
//   - A common host composition can express all title policies.
// - Summary:
//   - Publish and independently check selected canonical assets.
// - Description:
//   - Connect the verified importer sink to the platform transaction.
// - Usage:
//   - Select an explicit output parent or expected manifest digest.
// - Defaults:
//   - Never overwrite and do not publish on conversion failure.

use std::io::{Write as _, stdout};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};
use games_importer::conversion::ConversionLimits;
use games_importer::mame::AssetSelection;
use games_importer::wave::{WAVE_HEADER_BYTES, WavePcm};
use games_importer::{ImportError, SourceSnapshot};
use games_platform_storage::bundle::CanonicalBundle;
use games_platform_storage::file::snapshot;
use games_platform_storage::staging::{CanonicalStaging, StagedBundle};

/// Supported user-authored media encodings, explicitly selected.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MediaEncoding {
    Raw,
    Wave,
}

impl MediaEncoding {
    fn limit(self, original: &AssetRecord) -> Result<NonZeroU64, String> {
        let size = NonZeroU64::new(original.size)
            .ok_or_else(|| String::from("empty media slot"))?;
        match self {
            Self::Raw => Ok(size),
            Self::Wave
                if matches!(original.format, AssetFormat::PcmS16Le { .. }) =>
            {
                size.checked_add(u64::try_from(WAVE_HEADER_BYTES).map_err(
                    |_error| String::from("invalid WAV header size"),
                )?)
                .ok_or_else(|| String::from("WAV size overflow"))
            },
            Self::Wave => Err(String::from("WAV requires a PCM slot")),
        }
    }

    fn payload<'source>(
        self,
        source: &'source [u8],
        original: &AssetRecord,
        limit: NonZeroU64,
    ) -> Result<&'source [u8], String> {
        match self {
            Self::Raw => Ok(source),
            Self::Wave => {
                let wave = WavePcm::new(source, limit)
                    .map_err(|error| format!("replacement WAV: {error:?}"))?;
                if wave.format() != original.format {
                    return Err(String::from(
                        "replacement WAV layout mismatch",
                    ));
                }
                Ok(wave.payload())
            },
        }
    }
}

/// Explicit authoring selection for one existing passive-media slot.
#[derive(Debug)]
pub(crate) struct MediaReplacement {
    pub(crate) base: ContentHash,
    pub(crate) encoding: MediaEncoding,
    pub(crate) key: String,
    pub(crate) limits: ManifestLimits,
    pub(crate) parent: PathBuf,
    pub(crate) source: PathBuf,
}

impl MediaReplacement {
    pub(crate) fn run(self) -> Result<(), String> {
        let bundle =
            CanonicalBundle::open(&self.parent, self.base, self.limits)
                .map_err(|error| format!("base bundle: {error:?}"))?;
        let base = bundle.manifest();
        let original = base
            .record(&self.key)
            .ok_or_else(|| String::from("unknown base slot"))?;
        base.validate_media_replacement(&[&self.key], original)
            .map_err(|error| format!("replacement slot: {error:?}"))?;
        let size = NonZeroU64::new(original.size)
            .ok_or_else(|| String::from("empty media slot"))?;
        let limit = self.encoding.limit(original)?;
        let source = snapshot(&self.source, limit)
            .map_err(|error| format!("replacement input: {error:?}"))?;
        let bytes = self.encoding.payload(&source, original, limit)?;
        if u64::try_from(bytes.len()).ok() != Some(size.get()) {
            return Err(String::from("replacement byte length mismatch"));
        }
        let mut record = original.clone();
        record.digest = ContentHash::calculate(bytes);
        let digest = record.digest;
        let limits = ManifestLimits {
            manifest_bytes: self.limits.manifest_bytes,
            payload_bytes: size,
            records: NonZeroUsize::MIN,
            total_bytes: size,
        };
        let manifest =
            AssetManifest::new(base.source().clone(), vec![record], limits)
                .map_err(|error| format!("replacement manifest: {error:?}"))?;
        let mut stage =
            CanonicalStaging::new(&self.parent, staging_budget(limits)?)
                .map_err(|error| format!("stage: {error:?}"))?;
        stage
            .write_payload(digest, bytes)
            .map_err(|error| format!("stage: {error:?}"))?;
        finish(stage, &manifest, limits)
    }
}

/// Independently verify a published asset bundle.
///
/// # Errors
/// Rejects unexpected identities, invalid assets or output failures.
pub fn check(
    selection: AssetSelection,
    parent: &Path,
    digest: ContentHash,
) -> Result<(), String> {
    let limits = selection.limits();
    let bundle = CanonicalBundle::open(parent, digest, limits)
        .map_err(|error| format!("bundle: {error:?}"))?;
    let manifest = bundle.manifest();
    selection
        .validate_manifest(manifest)
        .map_err(|error| format!("profile manifest: {error:?}"))?;
    for record in manifest.records() {
        drop(
            bundle
                .read_payload(&record.key)
                .map_err(|error| format!("{}: {error:?}", record.key))?,
        );
    }
    super::report_manifest(manifest, bundle.digest())
}

/// Convert and publish a bounded archive without replacing any output.
///
/// # Errors
/// Returns conversion, staging, publication or output failures.
pub fn publish(
    selection: AssetSelection,
    parent: &Path,
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let manifest = selection.limits();
    let mut stage = CanonicalStaging::new(parent, staging_budget(manifest)?)
        .map_err(|error| format!("stage: {error:?}"))?;
    let mut staging_error = None;
    let converted = selection
        .convert(
            source,
            ConversionLimits { archive, manifest },
            |digest, bytes| {
                stage.write_payload(digest, bytes).map_err(|error| {
                    staging_error = Some(error);
                    ImportError::StageFailed
                })
            },
        )
        .map_err(|error| {
            staging_error.map_or_else(
                || format!("assets: {error:?}"),
                |failure| format!("stage: {failure:?}"),
            )
        })?;
    finish(stage, &converted, manifest)
}

/// Finish and publish a prepared canonical bundle.
///
/// # Errors
/// Rejects invalid staging, publication or durability failures.
pub fn finish(
    stage: CanonicalStaging,
    converted: &AssetManifest,
    manifest: ManifestLimits,
) -> Result<(), String> {
    let staged = stage
        .finish(converted, manifest)
        .map_err(|error| format!("complete: {error:?}"))?;
    publish_staged(staged, converted)
}

fn publish_staged(
    staged: StagedBundle,
    converted: &AssetManifest,
) -> Result<(), String> {
    let digest = staged.digest();
    let receipt = staged
        .publish()
        .map_err(|error| format!("publish: {error:?}"))?;
    if let Err(error) = receipt.durability {
        return Err(format!(
            "published {digest} at {}, but directory sync failed: {error:?}",
            receipt.path.display()
        ));
    }
    writeln!(
        stdout().lock(),
        "Published canonical bundle {digest} at {}",
        receipt.path.display()
    )
    .map_err(|error| error.to_string())?;
    super::report_manifest(converted, digest)
}

pub(crate) fn staging_budget(
    limits: ManifestLimits,
) -> Result<ArchiveBudget, String> {
    let overflow = || String::from("staging budget overflow");
    let metadata = u64::try_from(limits.manifest_bytes.get())
        .map_err(|_error| overflow())?;
    let max_entry = NonZeroU64::new(metadata.max(limits.payload_bytes.get()))
        .ok_or_else(overflow)?;
    let total = limits
        .total_bytes
        .get()
        .checked_add(metadata)
        .and_then(NonZeroU64::new)
        .ok_or_else(overflow)?;
    let count = u64::try_from(limits.records.get())
        .ok()
        .and_then(|value| value.checked_add(2))
        .and_then(NonZeroU64::new)
        .ok_or_else(overflow)?;
    Ok(ArchiveBudget::new(max_entry, total, count))
}

/// Republish a pinned legacy canonical bundle with current metadata.
///
/// # Errors
/// Rejects already-current metadata, corruption, limits or publication errors.
pub fn upgrade(
    parent: &Path,
    digest: ContentHash,
    limits: ManifestLimits,
) -> Result<(), String> {
    let source = CanonicalBundle::open(parent, digest, limits)
        .map_err(|error| format!("bundle: {error:?}"))?;
    let manifest = source.manifest();
    let encoded = manifest
        .encode(limits)
        .map_err(|error| format!("manifest: {error:?}"))?;
    if ContentHash::calculate(&encoded) == digest {
        return Err(String::from(
            "bundle already uses current manifest metadata",
        ));
    }
    let staged = CanonicalStaging::copy_bundle(
        parent,
        staging_budget(limits)?,
        &source,
        limits,
    )
    .map_err(|error| format!("copy bundle: {error:?}"))?;
    publish_staged(staged, manifest)
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::num::{NonZeroU8, NonZeroU32};
    use std::{env, fs};

    use games_assets::SourceIdentity;
    use games_importer::mame::Machine;
    use games_platform_storage::staging::AuthoredStaging;

    use super::*;

    fn failure<E>(error: E) -> String
    where
        E: Debug,
    {
        format!("{error:?}")
    }

    fn fixture_limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4095),
            payload_bytes: NonZeroU64::MIN.saturating_add(15),
            records: NonZeroUsize::MIN.saturating_add(3),
            total_bytes: NonZeroU64::MIN.saturating_add(63),
        }
    }

    fn publish_base(
        parent: &Path,
        limits: ManifestLimits,
    ) -> Result<CanonicalBundle, String> {
        let source = SourceIdentity {
            revision: String::from("fixture"),
            title: String::from("synthetic"),
        };
        let image = AssetRecord {
            digest: ContentHash::calculate(b"abcd"),
            format: AssetFormat::Rgba8 {
                height: NonZeroU32::MIN,
                width: NonZeroU32::MIN,
            },
            key: String::from("image/hero"),
            size: 4,
        };
        let data = AssetRecord {
            digest: ContentHash::calculate(b"rule"),
            format: AssetFormat::Data,
            key: String::from("rules"),
            size: 4,
        };
        let pcm = AssetRecord {
            digest: ContentHash::calculate(b"tone"),
            format: AssetFormat::PcmS16Le {
                channels: NonZeroU8::MIN,
                frames: NonZeroU64::MIN.saturating_add(1),
                sample_rate: NonZeroU32::MIN.saturating_add(9999),
            },
            key: String::from("audio/tone"),
            size: 4,
        };
        let manifest = AssetManifest::new(
            source,
            vec![image.clone(), data.clone(), pcm.clone()],
            limits,
        )
        .map_err(failure)?;
        let mut stage = CanonicalStaging::new(parent, staging_budget(limits)?)
            .map_err(failure)?;
        stage
            .write_payload(image.digest, b"abcd")
            .map_err(failure)?;
        stage.write_payload(data.digest, b"rule").map_err(failure)?;
        stage.write_payload(pcm.digest, b"tone").map_err(failure)?;
        let staged = stage.finish(&manifest, limits).map_err(failure)?;
        let base = staged.digest();
        staged
            .publish()
            .map_err(failure)?
            .durability
            .map_err(failure)?;
        CanonicalBundle::open(parent, base, limits).map_err(failure)
    }

    #[test]
    fn media_authoring_requires_exact_passive_slots() -> Result<(), String> {
        let directory = env::current_dir().map_err(failure)?;
        let root = AuthoredStaging::new(&directory).map_err(failure)?;
        let parent = root.path();
        let limits = fixture_limits();
        let original = publish_base(parent, limits)?;
        let base = original.digest();
        let manifest = original.manifest();
        let raw = parent.join("authored.raw");
        let select = |key: &str| MediaReplacement {
            base,
            encoding: MediaEncoding::Raw,
            key: key.to_owned(),
            limits,
            parent: parent.to_path_buf(),
            source: raw.clone(),
        };
        for (key, bytes) in [
            ("image/hero", b"x".as_slice()),
            ("image/hero", b"abcde".as_slice()),
            ("rules", b"rgba".as_slice()),
            ("unknown", b"rgba".as_slice()),
        ] {
            fs::write(&raw, bytes).map_err(failure)?;
            if select(key).run().is_ok() {
                return Err(String::from("invalid replacement accepted"));
            }
        }
        fs::write(&raw, b"rgba").map_err(failure)?;
        select("image/hero").run()?;
        let image = manifest
            .record("image/hero")
            .ok_or_else(|| String::from("missing image fixture"))?;
        let replacement = AssetManifest::new(
            manifest.source().clone(),
            vec![AssetRecord {
                digest: ContentHash::calculate(b"rgba"),
                ..image.clone()
            }],
            limits,
        )
        .map_err(failure)?;
        let digest = ContentHash::calculate(
            &replacement.encode(limits).map_err(failure)?,
        );
        let published =
            CanonicalBundle::open(parent, digest, limits).map_err(failure)?;
        if published.manifest() != &replacement
            || published.read_payload("image/hero").map_err(failure)? != b"rgba"
            || original.read_payload("image/hero").map_err(failure)? != b"abcd"
            || original.read_payload("rules").map_err(failure)? != b"rule"
            || select("image/hero").run().is_ok()
        {
            return Err(String::from("replacement publication mismatch"));
        }
        Ok(())
    }

    #[test]
    fn wave_authoring_requires_exact_pcm_layout() -> Result<(), String> {
        const WAVE: [u8; 48] = [
            b'R', b'I', b'F', b'F', 40, 0, 0, 0, b'W', b'A', b'V', b'E', b'f',
            b'm', b't', b' ', 16, 0, 0, 0, 1, 0, 1, 0, 16, 39, 0, 0, 32, 78, 0,
            0, 2, 0, 16, 0, b'd', b'a', b't', b'a', 4, 0, 0, 0, b'P', b'C',
            b'M', b'!',
        ];
        let directory = env::current_dir().map_err(failure)?;
        let root = AuthoredStaging::new(&directory).map_err(failure)?;
        let parent = root.path();
        let limits = fixture_limits();
        let original = publish_base(parent, limits)?;
        let source = parent.join("authored.wav");
        let select = |key: &str| MediaReplacement {
            base: original.digest(),
            encoding: MediaEncoding::Wave,
            key: key.to_owned(),
            limits,
            parent: parent.to_path_buf(),
            source: source.clone(),
        };
        let mut wrong_rate = WAVE;
        wrong_rate
            .get_mut(24..32)
            .ok_or("bad fixture")?
            .copy_from_slice(&[64, 31, 0, 0, 128, 62, 0, 0]);
        fs::write(&source, wrong_rate).map_err(failure)?;
        if select("audio/tone").run().is_ok() {
            return Err(String::from("incompatible rate accepted"));
        }
        fs::write(&source, WAVE).map_err(failure)?;
        if select("image/hero").run().is_ok() {
            return Err(String::from("WAV admitted for image slot"));
        }
        select("audio/tone").run()?;
        let manifest = original.manifest();
        let pcm = manifest.record("audio/tone").ok_or("missing PCM fixture")?;
        let replacement = AssetManifest::new(
            manifest.source().clone(),
            vec![AssetRecord {
                digest: ContentHash::calculate(b"PCM!"),
                ..pcm.clone()
            }],
            limits,
        )
        .map_err(failure)?;
        let digest = ContentHash::calculate(
            &replacement.encode(limits).map_err(failure)?,
        );
        let published =
            CanonicalBundle::open(parent, digest, limits).map_err(failure)?;
        if published.read_payload("audio/tone").map_err(failure)? != b"PCM!"
            || original.read_payload("audio/tone").map_err(failure)? != b"tone"
        {
            return Err(String::from("WAV publication mismatch"));
        }
        Ok(())
    }

    #[test]
    fn staging_budget_rejects_overflow() -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let selection = AssetSelection::Graphics(machine);
        let mut limits = selection.limits();
        limits.total_bytes = NonZeroU64::MAX;
        (staging_budget(limits).is_err()
            && staging_budget(selection.limits()).is_ok())
        .then_some(())
        .ok_or(ImportError::TooLarge)
    }
}
