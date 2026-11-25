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
//   - Selected Simpsons arcade source graphics mapping and revision pin.
// - Must-Not:
//   - Execute original code, infer palettes or implement title gameplay.
// - Allows:
//   - Inputs: Verified local source archive and explicit archive budget.
//   - Outputs: Complete tile-major arrays of row-major palette indices.
//   - Side effects: Bounded allocations only; no filesystem access.
// - Split-When:
//   - A different source revision requires independent conversion rules.
// - Merge-When:
//   - Shared source mapping supports these layouts without title branches.
// - Summary:
//   - Convert the selected original graphics without retaining program ROMs.
// - Description:
//   - Assemble verified graphics lanes in hardware order and decode pixels.
// - Usage:
//   - Verify the selected archive and call decode_graphics.
// - Defaults:
//   - Reject any other revision and incomplete or incorrectly sized lanes.

//! Selected World four-player set 1 graphics conversion.

use std::num::{NonZeroU32, NonZeroUsize};

use games_assets::archive::ArchiveBudget;
use games_assets::{AssetFormat, AssetManifest, ContentHash};

use crate::conversion::{ConversionLimits, convert_verified};
use crate::graphics::{TileLayout, interleave_words};
use crate::{ImportError, VerifiedSource};

/// Canonical selected source revision identifier.
pub const REVISION: &str = "world-4p-set-1";
/// Canonical title identity.
pub const TITLE: &str = "simpsons-arcade";
/// Exact SHA-256 of the selected local archive.
pub const SHA256: &str =
    "c88e549ab8f039ec3079b34bc39af6738379d306ca06cf81123f4d3501cf8dfa";

const TILE_LANES: [&str; 2] = ["072-b07.18h", "072-b06.16h"];
const SPRITE_LANES: [&str; 4] =
    ["072-b08.3n", "072-b09.8n", "072-b10.12n", "072-b11.16l"];

/// Source-independent four-bit color indices in tile-major order.
///
/// Color palettes are dynamic and are deliberately not guessed.
#[derive(Debug, Eq, PartialEq)]
pub struct IndexedGraphics {
    /// Row-major 16 by 16 sprite tiles, one palette index per byte.
    pub sprites: Vec<u8>,
    /// Row-major 8 by 8 background tiles, one palette index per byte.
    pub tiles: Vec<u8>,
}

fn allocated_bytes(size: usize) -> Result<Vec<u8>, ImportError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_error| ImportError::TooLarge)?;
    bytes.resize(size, 0);
    Ok(bytes)
}

fn collect_lane<const N: usize>(
    names: &[&str; N],
    lanes: &mut [Vec<u8>; N],
    member: (&str, &[u8]),
    expected: usize,
) -> Result<(), ImportError> {
    let (name, payload) = member;
    if let Some(index) = names.iter().position(|candidate| *candidate == name) {
        let lane =
            lanes.get_mut(index).ok_or(ImportError::UnsupportedMember)?;
        if payload.len() != expected || !lane.is_empty() {
            return Err(ImportError::UnsupportedMember);
        }
        lane.try_reserve_exact(expected)
            .map_err(|_error| ImportError::TooLarge)?;
        lane.extend_from_slice(payload);
    }
    Ok(())
}

fn decode_region<const N: usize>(
    lanes: &[Vec<u8>; N],
    lane_bytes: usize,
    layout: &TileLayout<'_>,
) -> Result<Vec<u8>, ImportError> {
    if lanes.iter().any(|lane| lane.len() != lane_bytes) {
        return Err(ImportError::UnsupportedMember);
    }
    let size = lane_bytes.checked_mul(N).ok_or(ImportError::TooLarge)?;
    let mut region = allocated_bytes(size)?;
    let sources = lanes.each_ref().map(Vec::as_slice);
    let word = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
    interleave_words(&sources, word, &mut region)
        .map_err(|_error| ImportError::UnsupportedMember)?;
    let output = layout
        .output_len(size)
        .map_err(|_error| ImportError::UnsupportedMember)?;
    let mut indices = allocated_bytes(output)?;
    layout
        .decode(&region, &mut indices)
        .map_err(|_error| ImportError::UnsupportedMember)?;
    Ok(indices)
}

// Layout parameters from pinned public MAME hardware documentation:
// k052109, k053246/k053247 NORMAL_PLANE_ORDER, and the simpsons ROM map.
// These describe source bit positions, not executable code or media.
fn sprite_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[
            8, 12, 0, 4, 24, 28, 16, 20, 40, 44, 32, 36, 56, 60, 48, 52,
        ],
        plane_bits: &[0, 1, 2, 3],
        row_bits: &[
            0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 832,
            896, 960,
        ],
        tile_bytes: NonZeroUsize::new(128).unwrap_or(NonZeroUsize::MIN),
    }
}

fn tile_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[0, 1, 2, 3, 4, 5, 6, 7],
        plane_bits: &[24, 16, 8, 0],
        row_bits: &[0, 32, 64, 96, 128, 160, 192, 224],
        tile_bytes: NonZeroUsize::new(32).unwrap_or(NonZeroUsize::MIN),
    }
}

/// Decode both graphics regions after all source entries are verified.
///
/// Non-graphics members are checked but not retained or executed.
/// # Errors
/// Rejects a different source identity, malformed archive, incomplete
/// graphics lanes, allocation failure or invalid source layout.
pub fn decode_graphics(
    source: &VerifiedSource<'_>,
    budget: ArchiveBudget,
) -> Result<IndexedGraphics, ImportError> {
    let identity = source.identity();
    if identity.title != TITLE
        || identity.revision != REVISION
        || identity.digest != SHA256.parse()?
    {
        return Err(ImportError::IncorrectSource);
    }
    let mut tile_lanes = [const { Vec::new() }; 2];
    let mut sprite_lanes = [const { Vec::new() }; 4];
    source.visit_flat_zip(budget, |name, payload| {
        collect_lane(
            &TILE_LANES,
            &mut tile_lanes,
            (name, payload),
            0x0008_0000,
        )?;
        collect_lane(
            &SPRITE_LANES,
            &mut sprite_lanes,
            (name, payload),
            0x0010_0000,
        )
    })?;
    let tiles = decode_region(&tile_lanes, 0x0008_0000, &tile_layout())?;
    drop(tile_lanes);
    let sprites = decode_region(&sprite_lanes, 0x0010_0000, &sprite_layout())?;
    Ok(IndexedGraphics { sprites, tiles })
}

/// Convert the selected graphics into a canonical private transaction.
///
/// The staging callback must retain no published bytes until success.
/// # Errors
/// Rejects unverified media, layout errors and canonical budget or sink
/// failures.
pub fn convert_graphics<Stage>(
    source: &VerifiedSource<'_>,
    limits: ConversionLimits,
    stage: Stage,
) -> Result<AssetManifest, ImportError>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
{
    convert_verified(source, limits.manifest, stage, |verified, output| {
        let decoded = decode_graphics(verified, limits.archive)?;
        let tiles = NonZeroU32::new(32_768).unwrap_or(NonZeroU32::MIN);
        let background = NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN);
        let sprite = NonZeroU32::new(16).unwrap_or(NonZeroU32::MIN);
        output.write(
            "graphics/tiles",
            AssetFormat::IndexedTiles8 {
                height: background,
                tiles,
                width: background,
            },
            &decoded.tiles,
        )?;
        output.write(
            "graphics/sprites",
            AssetFormat::IndexedTiles8 {
                height: sprite,
                tiles,
                width: sprite,
            },
            &decoded.sprites,
        )
    })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use games_assets::{ManifestLimits, SourceIdentity};

    use super::*;

    #[test]
    fn wrong_archive_identity_never_stages_payloads() -> Result<(), ImportError>
    {
        let media = b"synthetic";
        let identity = SourceIdentity {
            digest: ContentHash::calculate(media),
            revision: REVISION.to_owned(),
            title: TITLE.to_owned(),
        };
        let budget = NonZeroU64::new(32).unwrap_or(NonZeroU64::MIN);
        let verified = VerifiedSource::new(media, &identity, budget)?;
        let limits = ConversionLimits {
            archive: ArchiveBudget::new(budget, budget, budget),
            manifest: ManifestLimits {
                manifest_bytes: NonZeroUsize::MIN,
                payload_bytes: budget,
                records: NonZeroUsize::MIN,
                total_bytes: budget,
            },
        };
        let mut called = false;
        let result = convert_graphics(&verified, limits, |_digest, _bytes| {
            called = true;
            Ok(())
        });
        (result == Err(ImportError::IncorrectSource) && !called)
            .then_some(())
            .ok_or(ImportError::StageFailed)
    }

    #[test]
    fn tile_rows_observe_plane_and_lane_order() -> Result<(), ImportError> {
        let mut lanes = [vec![0u8; 16], vec![0u8; 16]];
        let first = lanes.first_mut().ok_or(ImportError::UnsupportedMember)?;
        *first.first_mut().ok_or(ImportError::UnsupportedMember)? = 0x80;
        let second = lanes.last_mut().ok_or(ImportError::UnsupportedMember)?;
        *second.get_mut(1).ok_or(ImportError::UnsupportedMember)? = 0x40;
        let pixels = decode_region(&lanes, 16, &tile_layout())?;
        (pixels.len() == 64
            && pixels.get(..4) == Some([1u8, 8, 0, 0].as_slice()))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn sprite_rows_observe_nibble_permutation() -> Result<(), ImportError> {
        let mut lanes = [const { Vec::new() }; 4];
        for lane in &mut lanes {
            lane.resize(32, 0);
        }
        let first = lanes.first_mut().ok_or(ImportError::UnsupportedMember)?;
        *first.first_mut().ok_or(ImportError::UnsupportedMember)? = 0x12;
        *first.get_mut(1).ok_or(ImportError::UnsupportedMember)? = 0x34;
        let pixels = decode_region(&lanes, 32, &sprite_layout())?;
        (pixels.len() == 256
            && pixels.get(..4) == Some([3u8, 4, 1, 2].as_slice()))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn incomplete_lanes_fail_without_decoding() {
        let lanes = [vec![0; 16], Vec::new()];
        assert_eq!(
            decode_region(&lanes, 16, &tile_layout()),
            Err(ImportError::UnsupportedMember)
        );
    }
}
