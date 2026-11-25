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
//   - Konami graphics chip layouts and selected MAME machine profile.
// - Must-Not:
//   - Execute original code, infer palettes or implement title gameplay.
// - Allows:
//   - Inputs: Bounded local source archive and explicit archive budget.
//   - Outputs: Immutable graphics region descriptors.
//   - Side effects: None.
// - Split-When:
//   - A different source revision requires independent conversion rules.
// - Merge-When:
//   - Shared source mapping supports these layouts without title branches.
// - Summary:
//   - Describe reusable Konami graphics layouts and a supported ROM layout.
// - Description:
//   - Describe word-lane order and pixel bit offsets without executing ROMs.
// - Usage:
//   - Select the profile through the MAME adapter.
// - Defaults:
//   - Reject any other revision and incomplete or incorrectly sized lanes.

//! Konami K052109 tile and K053246/K053247 sprite graphics layouts.

use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::ManifestLimits;

use super::background::Raster;
use super::objects::TransferPolicy;
use super::palette::{Palette, xbgr555_be};
use super::priority::Wiring;
use super::{Profile, Region};
use crate::graphics::TileLayout;

pub(super) static SIMPSONS: Profile = Profile {
    audio_limits: ManifestLimits {
        manifest_bytes: NonZeroUsize::MIN.saturating_add(65_536 - 1),
        payload_bytes: NonZeroU64::MIN.saturating_add(264_324 - 1),
        records: NonZeroUsize::MIN.saturating_add(161 - 1),
        total_bytes: NonZeroU64::MIN.saturating_add(3_798_888 - 1),
    },
    graphics: &[
        Region {
            key: "graphics/tiles",
            lane_bytes: 0x0008_0000,
            lanes: &["072-b07.18h", "072-b06.16h"],
            layout: tile_layout(),
            word_bytes: NonZeroUsize::MIN.saturating_add(2 - 1),
        },
        Region {
            key: "graphics/sprites",
            lane_bytes: 0x0010_0000,
            lanes: &["072-b08.3n", "072-b09.8n", "072-b10.12n", "072-b11.16l"],
            layout: sprite_layout(),
            word_bytes: NonZeroUsize::MIN.saturating_add(2 - 1),
        },
    ],
    limits: ManifestLimits {
        manifest_bytes: NonZeroUsize::MIN.saturating_add(4096 - 1),
        payload_bytes: NonZeroU64::MIN.saturating_add(8_388_608 - 1),
        records: NonZeroUsize::MIN.saturating_add(2 - 1),
        total_bytes: NonZeroU64::MIN.saturating_add(10_485_760 - 1),
    },
    machine: "simpsons",
    object_transfer: TransferPolicy { nonzero_depth: true },
    palette: Palette {
        decode_word: xbgr555_be,
        entries: NonZeroUsize::MIN.saturating_add(2048 - 1),
    },
    raster: Raster {
        height: 224,
        layer_x: [96, 90, 90],
        origin_y: 16,
        width: 320,
    },
    revision: "world-4p-set-1",
    sample_members: &[
        super::samples::Member {
            bytes: 0x0010_0000,
            name: "072-d05.1f",
        },
        super::samples::Member {
            bytes: 0x0004_0000,
            name: "072-d04.1d",
        },
    ],
    sprite_wiring: super::sprites::Wiring { x: -43, y: -23 },
    tile_code_mask: 0x7fff,
    title: "simpsons-arcade",
    video: Wiring {
        backdrop: 0,
        layers: [2, 3, 4],
        sprites: 1,
    },
};
// Layout parameters from pinned public MAME hardware documentation:
// k052109, k053246/k053247 NORMAL_PLANE_ORDER, and the simpsons ROM map.
// These describe source bit positions, not executable code or media.
const fn sprite_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[
            8, 12, 0, 4, 24, 28, 16, 20, 40, 44, 32, 36, 56, 60, 48, 52,
        ],
        plane_bits: &[0, 1, 2, 3],
        row_bits: &[
            0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 832,
            896, 960,
        ],
        tile_bytes: NonZeroUsize::MIN.saturating_add(128 - 1),
    }
}

const fn tile_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[0, 1, 2, 3, 4, 5, 6, 7],
        plane_bits: &[24, 16, 8, 0],
        row_bits: &[0, 32, 64, 96, 128, 160, 192, 224],
        tile_bytes: NonZeroUsize::MIN.saturating_add(32 - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::super::RegionLanes;
    use super::*;
    use crate::ImportError;

    #[test]
    fn tile_rows_observe_plane_and_lane_order() -> Result<(), ImportError> {
        let mut first = [0u8; 16];
        *first.first_mut().ok_or(ImportError::UnsupportedMember)? = 0x80;
        let mut second = [0u8; 16];
        *second.get_mut(1).ok_or(ImportError::UnsupportedMember)? = 0x40;
        let mut region = RegionLanes::new(Region {
            lane_bytes: 16,
            ..*SIMPSONS
                .graphics
                .first()
                .ok_or(ImportError::UnsupportedMember)?
        });
        region.collect("072-b06.16h", &second)?;
        region.collect("072-b07.18h", &first)?;
        let pixels = region.decode()?;
        (pixels.len() == 64
            && pixels.get(..4) == Some([1u8, 8, 0, 0].as_slice()))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn sprite_rows_observe_nibble_permutation() -> Result<(), ImportError> {
        let mut first = [0u8; 32];
        *first.first_mut().ok_or(ImportError::UnsupportedMember)? = 0x12;
        *first.get_mut(1).ok_or(ImportError::UnsupportedMember)? = 0x34;
        let descriptor = Region {
            lane_bytes: 32,
            ..*SIMPSONS
                .graphics
                .last()
                .ok_or(ImportError::UnsupportedMember)?
        };
        let mut region = RegionLanes::new(descriptor);
        for name in descriptor.lanes {
            region.collect(
                name,
                if *name == "072-b08.3n" {
                    &first
                } else {
                    &[0; 32]
                },
            )?;
        }
        let pixels = region.decode()?;
        (pixels.len() == 256
            && pixels.get(..4) == Some([3u8, 4, 1, 2].as_slice()))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }

    #[test]
    fn incomplete_lanes_fail_without_decoding() -> Result<(), ImportError> {
        let descriptor = Region {
            lane_bytes: 16,
            ..*SIMPSONS
                .graphics
                .first()
                .ok_or(ImportError::UnsupportedMember)?
        };
        let mut region = RegionLanes::new(descriptor);
        region.collect("072-b07.18h", &[0; 16])?;
        (region.decode() == Err(ImportError::UnsupportedMember))
            .then_some(())
            .ok_or(ImportError::UnsupportedMember)
    }
}
