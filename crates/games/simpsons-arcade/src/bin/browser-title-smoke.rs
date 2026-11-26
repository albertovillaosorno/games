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
//   - Procedural canonical assets for an explicit live title diagnostic.
// - Must-Not:
//   - Interpret original game media or implement title behavior.
// - Allows:
//   - Inputs: Explicit diagnostic invocation and host input events.
//   - Outputs: Live title presentation and terminal completion outcome.
//   - Side effects: Delegated event-driven window creation.
// - Split-When:
//   - An independent production asset ingress needs a separate entry point.
// - Merge-When:
//   - A shared smoke can exercise the same complete title launch.
// - Summary:
//   - Exercise the ordinary title composition with synthetic art.
// - Description:
//   - Supply generated RGBA and silent PCM through canonical memory storage.
// - Usage:
//   - Run in a visible browser and use the default keyboard controls.
// - Defaults:
//   - No original assets, audible diagnostic output, persistence or network.

//! Live title diagnostic using procedural canonical art and silent PCM.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::num::{NonZeroU8, NonZeroU32, NonZeroU64, NonZeroUsize};

use games::presentation::DisplaySettings;
use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
    SourceIdentity,
};
use games_platform::ir::frame::RenderError;
use games_platform::ir::process::{write_diagnostic, write_status};
use games_platform::ir::storage::bundle::{
    MemoryBundleLimits, MemoryBundleStore,
};
use games_simpsons_arcade::runtime::memory_preview::{
    AudioSelection, Selection, start,
};

const MAX_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(399_999);
type Fixture = (MemoryBundleStore, ContentHash, ContentHash);
type FixtureManifest = (ContentHash, Vec<u8>);

fn main() -> Result<(), RenderError> {
    let (store, digest, audio) = fixture()?;
    start(
        &store,
        &Selection {
            actors: [digest; 4],
            audio: Some(AudioSelection {
                base: audio,
                replacement: None,
            }),
            display: DisplaySettings::default(),
            enemy: None,
            poses: None,
            shadow: None,
            touch_enabled: false,
            world: None,
        },
        complete,
    )
}

fn complete(result: Result<(), RenderError>) {
    match result {
        Ok(()) => {
            let _reported =
                write_status(format_args!("browser-title-smoke: passed"));
        },
        Err(error) => {
            let _reported = write_diagnostic(format_args!(
                "browser-title-smoke: failed: {error:?}"
            ));
        },
    }
}

fn failure<Error>(error: Error) -> RenderError
where
    Error: Debug,
{
    RenderError::Backend(format!("{error:?}"))
}

fn dimension(value: u32) -> Result<NonZeroU32, RenderError> {
    NonZeroU32::new(value).ok_or(RenderError::InvalidDimensions)
}

fn size(value: usize) -> Result<NonZeroUsize, RenderError> {
    NonZeroUsize::new(value).ok_or(RenderError::InvalidLength)
}

fn fixture() -> Result<Fixture, RenderError> {
    let mut objects = BTreeMap::new();
    let mut records = Vec::new();
    for (key, width, height, color) in [
        ("scene/background", 320u32, 224u32, [22u8, 35, 40, 255]),
        ("actor/marge", 12, 12, [240, 20, 180, 255]),
        ("actor/homer", 12, 12, [30, 220, 210, 255]),
        ("actor/bart", 12, 12, [255, 90, 10, 255]),
        ("actor/lisa", 12, 12, [80, 220, 40, 255]),
    ] {
        let count = width.checked_mul(height).ok_or(RenderError::TooLarge)?;
        let bytes = color.repeat(usize::try_from(count).map_err(failure)?);
        let digest = ContentHash::calculate(&bytes);
        records.push(AssetRecord {
            digest,
            format: AssetFormat::Rgba8 {
                height: dimension(height)?,
                width: dimension(width)?,
            },
            key: String::from(key),
            size: u64::try_from(bytes.len()).map_err(failure)?,
        });
        let _prior = objects.insert(digest, bytes);
    }
    let audio_records = silent_audio(&mut objects)?;
    let (digest, bytes) = fixture_manifest(records)?;
    let (audio, audio_bytes) = fixture_manifest(audio_records)?;
    let store = MemoryBundleStore::new(
        BTreeMap::from([(digest, bytes), (audio, audio_bytes)]),
        objects,
        MemoryBundleLimits {
            bytes: MAX_BYTES,
            manifests: size(2)?,
            objects: size(9)?,
        },
    )
    .map_err(failure)?;
    Ok((store, digest, audio))
}

fn fixture_manifest(
    records: Vec<AssetRecord>,
) -> Result<FixtureManifest, RenderError> {
    let limits = ManifestLimits {
        manifest_bytes: size(4096)?,
        payload_bytes: MAX_BYTES,
        records: size(records.len())?,
        total_bytes: MAX_BYTES,
    };
    let manifest = AssetManifest::new(
        SourceIdentity {
            revision: String::from("world-4p-set-1"),
            title: String::from("simpsons-arcade"),
        },
        records,
        limits,
    )
    .map_err(failure)?;
    let bytes = manifest.encode(limits).map_err(failure)?;
    Ok((ContentHash::calculate(&bytes), bytes))
}

fn silent_audio(
    objects: &mut BTreeMap<ContentHash, Vec<u8>>,
) -> Result<Vec<AssetRecord>, RenderError> {
    let mut records = Vec::new();
    for (key, frames) in [
        ("audio/sample-0004", 8335u64),
        ("audio/sample-0005", 3916),
        ("audio/sample-0006", 2944),
        ("audio/sample-0007", 3828),
    ] {
        let count = frames.checked_mul(2).ok_or(RenderError::TooLarge)?;
        let bytes = vec![0u8; usize::try_from(count).map_err(failure)?];
        let digest = ContentHash::calculate(&bytes);
        records.push(AssetRecord {
            digest,
            format: AssetFormat::PcmS16Le {
                channels: NonZeroU8::MIN,
                frames: NonZeroU64::new(frames)
                    .ok_or(RenderError::InvalidLength)?,
                sample_rate: dimension(10_000)?,
            },
            key: String::from(key),
            size: count,
        });
        let _prior = objects.insert(digest, bytes);
    }
    Ok(records)
}
