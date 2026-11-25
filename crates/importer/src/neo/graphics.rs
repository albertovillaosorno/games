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
//   - Neo Geo cartridge graphics layouts and exact supported region sizes.
// - Must-Not:
//   - Execute original code, infer palettes or implement title gameplay.
// - Allows:
//   - Inputs: A selected cartridge profile and bounded immutable regions.
//   - Outputs: Canonical indexed fixed-layer and sprite tile records.
//   - Side effects: Requests private canonical payload staging.
// - Split-When:
//   - Encrypted or differently wired boards need independent conversion.
// - Merge-When:
//   - Another format provides the same verified hardware graphics regions.
// - Summary:
//   - Convert admitted, already interleaved Neo Geo graphics regions.
// - Description:
//   - Reuse shared tile decoding and transactional manifest construction.
// - Usage:
//   - Select the exact cartridge layout before conversion.
// - Defaults:
//   - Reject unsupported revisions and missing or differently sized regions.

use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetFormat, AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};

use crate::conversion::{
    Conversion, ConversionLimits, convert_snapshot, preflight_payloads,
};
use crate::graphics::TileLayout;
use crate::{ImportError, SourceSnapshot};

const FIXED: Region = Region {
    bytes: 0x0002_0000,
    key: "graphics/fixed",
    layout: fixed_layout(),
    name: "fixed",
};

static PROFILES: [Profile; 3] = [
    Profile {
        graphics: &[FIXED, sprite_region(0x0100_0000)],
        limits: graphics_limits(33_554_432),
        other_regions: &[
            ("program", 0x0020_0000),
            ("audio-program", 0x0002_0000),
            ("adpcm-a", 0x0080_0000),
        ],
        revision: "mvs-aes-neo-v1",
        title: "metal-slug",
    },
    Profile {
        graphics: &[FIXED, sprite_region(0x0200_0000)],
        limits: graphics_limits(67_108_864),
        other_regions: &[
            ("program", 0x0030_0000),
            ("audio-program", 0x0002_0000),
            ("adpcm-a", 0x0080_0000),
        ],
        revision: "mvs-aes-neo-v1",
        title: "metal-slug-2",
    },
    Profile {
        graphics: &[FIXED, sprite_region(0x0300_0000)],
        limits: graphics_limits(100_663_296),
        other_regions: &[
            ("program", 0x0050_0000),
            ("audio-program", 0x0002_0000),
            ("adpcm-a", 0x00a0_0000),
        ],
        revision: "mvs-aes-neo-v1",
        title: "metal-slug-x",
    },
];

#[derive(Clone, Copy, Debug)]
struct Region {
    bytes: usize,
    key: &'static str,
    layout: TileLayout<'static>,
    name: &'static str,
}

impl Region {
    fn format(self) -> Result<AssetFormat, ImportError> {
        Ok(self.layout.indexed_format(self.bytes)?)
    }
}

#[derive(Debug)]
struct Profile {
    graphics: &'static [Region],
    limits: ManifestLimits,
    other_regions: &'static [(&'static str, usize)],
    revision: &'static str,
    title: &'static str,
}

/// An admitted Neo Geo graphics source layout, selected by title and revision.
#[derive(Clone, Copy, Debug)]
pub struct CartridgeGraphics(&'static Profile);

impl CartridgeGraphics {
    /// Convert an exact supported cartridge to canonical index buffers.
    ///
    /// # Errors
    /// Rejects source, layout, budget and staging failures.
    pub fn convert<Stage>(
        self,
        source: &SourceSnapshot<'_>,
        limits: ConversionLimits,
        stage: Stage,
    ) -> Result<AssetManifest, ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        self.validate_source(source, limits.archive)?;
        preflight_payloads(
            limits.manifest,
            self.0.graphics.iter().map(|region| {
                let size = region.layout.output_len(region.bytes)?;
                u64::try_from(size).map_err(|_error| ImportError::TooLarge)
            }),
        )?;
        convert_snapshot(source, limits.manifest, stage, |verified, output| {
            verified.visit_neo(limits.archive, |name, payload| {
                self.convert_region(name, payload, output)
            })
        })
    }

    fn convert_region<Stage>(
        self,
        name: &str,
        payload: &[u8],
        output: &mut Conversion<'_, Stage>,
    ) -> Result<(), ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        let Some(region) =
            self.0.graphics.iter().find(|region| region.name == name)
        else {
            return Ok(());
        };
        output.write(
            region.key,
            region.format()?,
            &region.layout.decode_owned(payload)?,
        )
    }

    /// The caller-selected title and source layout, not media authenticity.
    #[must_use]
    pub fn identity(self) -> SourceIdentity {
        SourceIdentity {
            revision: self.0.revision.to_owned(),
            title: self.0.title.to_owned(),
        }
    }

    /// Exact canonical output bounds for this profile.
    #[must_use]
    pub const fn limits(self) -> ManifestLimits {
        self.0.limits
    }

    /// Select an exact supported cartridge layout.
    ///
    /// # Errors
    /// Rejects unknown titles and layouts.
    pub fn select(title: &str, revision: &str) -> Result<Self, ImportError> {
        PROFILES
            .iter()
            .find(|profile| {
                title == profile.title && revision == profile.revision
            })
            .map(Self)
            .ok_or(ImportError::IncorrectSource)
    }

    /// Check the selected identity and exact canonical graphics descriptors.
    ///
    /// # Errors
    /// Rejects missing, unexpected or misdescribed records.
    pub fn validate_manifest(
        self,
        manifest: &AssetManifest,
    ) -> Result<(), ImportError> {
        if manifest.source() != &self.identity()
            || manifest.records().len() != self.0.graphics.len()
        {
            return Err(ImportError::IncorrectSource);
        }
        for region in self.0.graphics {
            if manifest.record(region.key).map(|record| record.format)
                != Some(region.format()?)
            {
                return Err(ImportError::UnsupportedMember);
            }
        }
        Ok(())
    }

    fn validate_source(
        self,
        source: &SourceSnapshot<'_>,
        budget: ArchiveBudget,
    ) -> Result<(), ImportError> {
        if source.identity() != &self.identity() {
            return Err(ImportError::IncorrectSource);
        }
        let mut admitted = 0usize;
        source.visit_neo(budget, |name, payload| {
            let graphics = self.0.graphics.iter().any(|region| {
                region.name == name && region.bytes == payload.len()
            });
            if !graphics
                && !self.0.other_regions.contains(&(name, payload.len()))
            {
                return Err(ImportError::UnsupportedMember);
            }
            admitted = admitted.saturating_add(1);
            Ok(())
        })?;
        (admitted
            == self
                .0
                .graphics
                .len()
                .saturating_add(self.0.other_regions.len()))
        .then_some(())
        .ok_or(ImportError::UnsupportedMember)
    }
}

const fn graphics_limits(sprite_output_bytes: u64) -> ManifestLimits {
    ManifestLimits {
        manifest_bytes: NonZeroUsize::MIN.saturating_add(4096 - 1),
        payload_bytes: NonZeroU64::MIN
            .saturating_add(sprite_output_bytes.saturating_sub(1)),
        records: NonZeroUsize::MIN.saturating_add(2 - 1),
        total_bytes: NonZeroU64::MIN
            .saturating_add(sprite_output_bytes.saturating_add(262_143)),
    }
}

const fn sprite_region(bytes: usize) -> Region {
    Region {
        bytes,
        key: "graphics/sprites",
        layout: sprite_layout(),
        name: "sprites",
    }
}

// Public Neo Geo hardware wiring: fixed-layer tiles use low nibble first
// with four 8-byte column groups. Sprite tiles use two 64-byte halves,
// byte-interleaved planes and least-significant source bit first.
const fn fixed_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[132, 128, 196, 192, 4, 0, 68, 64],
        plane_bits: &[0, 1, 2, 3],
        row_bits: &[0, 8, 16, 24, 32, 40, 48, 56],
        tile_bytes: NonZeroUsize::MIN.saturating_add(32 - 1),
    }
}

const fn sprite_layout() -> TileLayout<'static> {
    TileLayout {
        column_bits: &[
            519, 518, 517, 516, 515, 514, 513, 512, 7, 6, 5, 4, 3, 2, 1, 0,
        ],
        plane_bits: &[24, 8, 16, 0],
        row_bits: &[
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416,
            448, 480,
        ],
        tile_bytes: NonZeroUsize::MIN.saturating_add(128 - 1),
    }
}

#[cfg(test)]
mod tests {
    use games_assets::AssetError;

    use super::*;

    static FIXTURE: Profile = Profile {
        graphics: &[
            Region {
                bytes: 32,
                key: "graphics/fixed",
                layout: fixed_layout(),
                name: "fixed",
            },
            Region {
                bytes: 128,
                key: "graphics/sprites",
                layout: sprite_layout(),
                name: "sprites",
            },
        ],
        limits: ManifestLimits {
            manifest_bytes: NonZeroUsize::MIN.saturating_add(4096 - 1),
            payload_bytes: NonZeroU64::MIN.saturating_add(256 - 1),
            records: NonZeroUsize::MIN.saturating_add(2 - 1),
            total_bytes: NonZeroU64::MIN.saturating_add(320 - 1),
        },
        other_regions: &[("program", 2)],
        revision: "neo-v1",
        title: "synthetic",
    };

    fn require(condition: bool) -> Result<(), ImportError> {
        condition.then_some(()).ok_or(ImportError::StageFailed)
    }

    fn set(
        bytes: &mut [u8],
        offset: usize,
        value: u8,
    ) -> Result<(), ImportError> {
        *bytes.get_mut(offset).ok_or(ImportError::InvalidNeo)? = value;
        Ok(())
    }

    fn cartridge() -> Result<Vec<u8>, ImportError> {
        let mut bytes = vec![0; 4096];
        bytes
            .get_mut(..4)
            .ok_or(ImportError::InvalidNeo)?
            .copy_from_slice(b"NEO\x01");
        let sizes = bytes.get_mut(4..28).ok_or(ImportError::InvalidNeo)?;
        for (field, size) in sizes
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip([2u32, 32, 0, 0, 0, 128])
        {
            field.copy_from_slice(&size.to_le_bytes());
        }
        bytes.extend_from_slice(&[0; 162]);
        Ok(bytes)
    }

    fn limits(profile: CartridgeGraphics) -> ConversionLimits {
        let bound = |value| NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN);
        ConversionLimits {
            archive: ArchiveBudget::new(bound(128), bound(162), bound(3)),
            manifest: profile.limits(),
        }
    }

    #[test]
    fn profile_output_budgets_are_exact() -> Result<(), ImportError> {
        for (title, expected_sprite_bytes) in [
            ("metal-slug", 33_554_432u64),
            ("metal-slug-2", 67_108_864),
            ("metal-slug-x", 100_663_296),
        ] {
            let profile = CartridgeGraphics::select(title, "mvs-aes-neo-v1")?;
            let limits = profile.limits();
            require(profile.identity().title == title)?;
            let mut total = 0u64;
            let mut records = Vec::new();
            for region in profile.0.graphics {
                let size =
                    u64::try_from(region.layout.output_len(region.bytes)?)
                        .map_err(|_error| ImportError::TooLarge)?;
                total = total.saturating_add(size);
                records.push(games_assets::AssetRecord {
                    digest: ContentHash::calculate(region.key.as_bytes()),
                    format: region.format()?,
                    key: region.key.to_owned(),
                    size,
                });
            }
            require(limits.payload_bytes.get() == expected_sprite_bytes)?;
            require(total == limits.total_bytes.get())?;
            let manifest =
                AssetManifest::new(profile.identity(), records, limits)?;
            profile.validate_manifest(&manifest)?;
            require(CartridgeGraphics::select(title, "unknown").is_err())?;
        }
        require(
            CartridgeGraphics::select("metal-slug-3", "mvs-aes-neo-v1")
                .is_err(),
        )
    }

    #[test]
    fn fixed_tiles_preserve_nibble_and_column_order() -> Result<(), ImportError>
    {
        let mut source = [0; 32];
        for (offset, value) in [(16, 0x21), (24, 0x43), (0, 0x65), (8, 0x87)] {
            set(&mut source, offset, value)?;
        }
        let pixels = fixed_layout().decode_owned(&source)?;
        require(
            pixels.len() == 64
                && pixels.get(..8) == Some([1, 2, 3, 4, 5, 6, 7, 8].as_slice())
                && pixels
                    .get(8..)
                    .is_some_and(|rest| rest.iter().all(|&pixel| pixel == 0)),
        )
    }

    #[test]
    fn sprites_preserve_plane_bit_and_half_order() -> Result<(), ImportError> {
        let mut source = [0; 128];
        for (offset, value) in [(64, 1), (65, 2), (66, 4), (67, 8)] {
            set(&mut source, offset, value)?;
        }
        for offset in 0..4 {
            set(&mut source, offset, 0x80)?;
        }
        set(&mut source, 68, 1)?;
        let pixels = sprite_layout().decode_owned(&source)?;
        require(
            pixels.len() == 256
                && pixels.get(..4) == Some([1, 4, 2, 8].as_slice())
                && pixels.get(15) == Some(&15)
                && pixels.get(16) == Some(&1),
        )
    }

    #[test]
    fn wrong_region_extent_and_output_budget_never_stage()
    -> Result<(), ImportError> {
        let profile = CartridgeGraphics(&FIXTURE);
        let identity = profile.identity();
        let limit = NonZeroU64::MIN.saturating_add(8192 - 1);
        let valid = cartridge()?;
        let mut wrong_size = valid.clone();
        set(&mut wrong_size, 8, 31)?;
        let _removed = wrong_size.pop();
        let invalid = SourceSnapshot::new(&wrong_size, &identity, limit)?;
        let source = SourceSnapshot::new(&valid, &identity, limit)?;
        let reject_stage = |_, _: &[u8]| Err(ImportError::StageFailed);
        require(
            profile.convert(&invalid, limits(profile), reject_stage)
                == Err(ImportError::UnsupportedMember),
        )?;
        let mut small = limits(profile);
        small.manifest.payload_bytes = NonZeroU64::MIN;
        require(
            profile.convert(&source, small, reject_stage)
                == Err(ImportError::Asset(AssetError::TooLarge)),
        )
    }

    #[test]
    fn conversion_checks_profile_before_staging() -> Result<(), ImportError> {
        let profile = CartridgeGraphics(&FIXTURE);
        let identity = profile.identity();
        let bytes = cartridge()?;
        let limit = NonZeroU64::MIN.saturating_add(8192 - 1);
        let source = SourceSnapshot::new(&bytes, &identity, limit)?;
        let mut staged = Vec::new();
        let manifest =
            profile.convert(&source, limits(profile), |_, data| {
                staged.push(data.len());
                Ok(())
            })?;
        profile.validate_manifest(&manifest)?;
        require(staged == [64, 256])?;
        let repeated =
            profile.convert(&source, limits(profile), |_, _| Ok(()))?;
        require(repeated == manifest)?;
        let mut wrong = identity.clone();
        wrong.revision = String::from("unknown");
        let wrong_source = SourceSnapshot::new(&bytes, &wrong, limit)?;
        require(
            profile.convert(&wrong_source, limits(profile), |_, _| {
                Err(ImportError::StageFailed)
            }) == Err(ImportError::IncorrectSource),
        )
    }
}
