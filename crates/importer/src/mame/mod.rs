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
//   - Shared MAME ROM-set selection, admission and graphics conversion.
// - Must-Not:
//   - Execute original code, infer palettes or implement title gameplay.
// - Allows:
//   - Inputs: Verified local source archive and explicit archive budget.
//   - Outputs: Complete tile-major arrays of row-major palette indices.
//   - Side effects: Bounded allocations only; no filesystem access.
// - Split-When:
//   - A different ROM loading strategy requires an independent module.
// - Merge-When:
//   - Another adapter provides the same MAME region assembly.
// - Summary:
//   - Convert profiled arcade graphics without retaining program ROMs.
// - Description:
//   - Assemble verified graphics lanes in hardware order and decode pixels.
// - Usage:
//   - Verify the selected archive and select a supported machine and revision.
// - Defaults:
//   - Reject any other revision and incomplete or incorrectly sized lanes.

//! Shared MAME ROM-set conversion, not an emulator or gameplay runtime.

/// Observed background raster composition for supported tilemap modes.
pub mod background;
mod konami;
/// Observed sprite object transfer and record admission.
pub mod objects;
mod palette;
/// Observed video mixer assignment and priority order.
pub mod priority;
/// Bounded signed sample ROM extraction and source-reference comparison.
pub mod samples;
/// Observed sprite composition and vertical mirror handling.
pub mod sprites;
/// Observed local tile-cell addresses and attributes.
pub mod tilemap;

use std::iter::once;
use std::num::{NonZeroU32, NonZeroUsize};

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetFormat, AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};

use crate::conversion::{ConversionLimits, convert_verified};
use crate::graphics::{TileLayout, interleave_words};
use crate::{ImportError, VerifiedSource};

#[derive(Clone, Copy, Debug)]
struct Region {
    key: &'static str,
    lane_bytes: usize,
    lanes: &'static [&'static str],
    layout: TileLayout<'static>,
    word_bytes: NonZeroUsize,
}

impl Region {
    fn format(self) -> Result<AssetFormat, ImportError> {
        let dimension = |value: usize| {
            u32::try_from(value)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or(ImportError::UnsupportedMember)
        };
        Ok(AssetFormat::IndexedTiles8 {
            height: dimension(self.layout.row_bits.len())?,
            tiles: dimension(
                self.source_bytes()?
                    .checked_div(self.layout.tile_bytes.get())
                    .ok_or(ImportError::UnsupportedMember)?,
            )?,
            width: dimension(self.layout.column_bits.len())?,
        })
    }

    fn source_bytes(self) -> Result<usize, ImportError> {
        self.lane_bytes
            .checked_mul(self.lanes.len())
            .ok_or(ImportError::TooLarge)
    }
}

#[derive(Debug)]
struct Profile {
    digest: &'static str,
    graphics: &'static [Region],
    limits: ManifestLimits,
    machine: &'static str,
    object_transfer: objects::TransferPolicy,
    palette: palette::Palette,
    raster: background::Raster,
    revision: &'static str,
    sample_members: &'static [samples::Member],
    sprite_wiring: sprites::Wiring,
    tile_code_mask: u16,
    title: &'static str,
    video: priority::Wiring,
}

/// A supported machine and pinned original revision, selected by name.
#[derive(Clone, Copy, Debug)]
pub struct Machine(&'static Profile);

impl Machine {
    /// Fixed visible raster geometry and tile-layer alignment.
    #[must_use]
    pub const fn background_raster(self) -> background::Raster {
        self.0.raster
    }

    /// Compare an observed CPU object buffer and captured device buffer.
    /// # Errors
    /// Rejects incomplete or malformed snapshots.
    pub fn compare_object_transfer(
        self,
        cpu: &[u8],
        device: &[u8],
    ) -> Result<objects::TransferReport, ImportError> {
        self.0.object_transfer.compare(cpu, device)
    }

    /// Convert every declared graphics region to a private canonical sink.
    ///
    /// # Errors
    /// Rejects mismatched sources, incomplete lanes and conversion failures.
    pub fn convert_graphics<Stage>(
        self,
        source: &VerifiedSource<'_>,
        limits: ConversionLimits,
        stage: Stage,
    ) -> Result<AssetManifest, ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        if source.identity() != &self.identity()? {
            return Err(ImportError::IncorrectSource);
        }
        convert_verified(source, limits.manifest, stage, |verified, output| {
            let mut collected: Vec<_> = self
                .0
                .graphics
                .iter()
                .copied()
                .map(RegionLanes::new)
                .collect();
            verified.visit_flat_zip(limits.archive, |name, payload| {
                collected
                    .iter_mut()
                    .try_for_each(|region| region.collect(name, payload))
            })?;
            for region in collected {
                let format = region.region.format()?;
                let key = region.region.key;
                output.write(key, format, &region.decode()?)?;
            }
            Ok(())
        })
    }

    /// Decode an exact observed palette RAM snapshot into opaque RGBA.
    ///
    /// The caller supplies transparency and pen assignment separately.
    /// # Errors
    /// Rejects mismatched source or output lengths.
    pub fn decode_palette(
        self,
        source: &[u8],
        output: &mut [[u8; 4]],
    ) -> Result<(), ImportError> {
        self.0.palette.decode(source, output)
    }

    /// Decode the selected machine's observed priority mixer registers.
    /// # Errors
    /// Rejects incomplete or malformed register snapshots.
    pub fn decode_video_priority(
        self,
        registers: &[u8],
    ) -> Result<priority::VideoPriority, ImportError> {
        self.0.video.decode(registers)
    }

    /// Exact canonical graphics resource limits for this revision.
    #[must_use]
    pub const fn graphics_limits(self) -> ManifestLimits {
        self.0.limits
    }

    /// Trusted source identity, not a hash inferred from supplied media.
    ///
    /// # Errors
    /// Rejects a malformed compiled-in profile digest.
    pub fn identity(self) -> Result<SourceIdentity, ImportError> {
        Ok(SourceIdentity {
            digest: self.0.digest.parse()?,
            revision: self.0.revision.to_owned(),
            title: self.0.title.to_owned(),
        })
    }

    /// Exact number of entries in the machine palette RAM.
    #[must_use]
    pub const fn palette_entries(self) -> NonZeroUsize {
        self.0.palette.entries
    }

    /// Assemble only the profiled sample ROM members from the verified archive.
    /// # Errors
    /// Rejects an incorrect source or missing or malformed sample members.
    pub fn sample_rom(
        self,
        source: &VerifiedSource<'_>,
        budget: ArchiveBudget,
    ) -> Result<samples::SampleRom, ImportError> {
        if source.identity() != &self.identity()? {
            return Err(ImportError::IncorrectSource);
        }
        samples::SampleRom::load(source, budget, self.0.sample_members)
    }

    /// Select a supported machine and exact revision.
    ///
    /// # Errors
    /// Rejects unknown machines and revisions rather than guessing layouts.
    pub fn select(machine: &str, revision: &str) -> Result<Self, ImportError> {
        once(&konami::SIMPSONS)
            .find(|profile| {
                profile.machine == machine && profile.revision == revision
            })
            .map(Self)
            .ok_or(ImportError::IncorrectSource)
    }

    /// Bind observed object RAM and 44 bytes of packed device controls.
    /// # Errors
    /// Rejects malformed snapshots and unimplemented display modes.
    pub fn sprite_snapshot<'source>(
        self,
        ram: &'source [u8],
        controls: &[u8],
        mixer: priority::VideoPriority,
    ) -> Result<sprites::Snapshot<'source>, ImportError> {
        sprites::Snapshot::new(self.0.sprite_wiring, ram, controls, mixer)
    }

    /// Bind observed tile RAM and six bytes of character bank/control state.
    /// # Errors
    /// Rejects malformed RAM or control snapshots.
    pub fn tilemap_snapshot<'source>(
        self,
        ram: &'source [u8],
        controls: &[u8],
        mixer: priority::VideoPriority,
    ) -> Result<tilemap::TilemapSnapshot<'source>, ImportError> {
        tilemap::TilemapSnapshot::new(
            ram,
            controls,
            mixer,
            self.0.tile_code_mask,
        )
    }

    /// Check the selected source and exact graphics manifest shape.
    ///
    /// # Errors
    /// Rejects mismatched source, missing, unexpected or misdescribed records.
    pub fn validate_manifest(
        self,
        manifest: &AssetManifest,
    ) -> Result<(), ImportError> {
        if manifest.source() != &self.identity()?
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
}

#[derive(Debug)]
struct RegionLanes {
    lanes: Vec<Vec<u8>>,
    region: Region,
}

impl RegionLanes {
    fn collect(
        &mut self,
        name: &str,
        payload: &[u8],
    ) -> Result<(), ImportError> {
        if let Some(index) = self
            .region
            .lanes
            .iter()
            .position(|candidate| *candidate == name)
        {
            let lane = self
                .lanes
                .get_mut(index)
                .ok_or(ImportError::UnsupportedMember)?;
            if payload.len() != self.region.lane_bytes || !lane.is_empty() {
                return Err(ImportError::UnsupportedMember);
            }
            lane.try_reserve_exact(payload.len())
                .map_err(|_error| ImportError::TooLarge)?;
            lane.extend_from_slice(payload);
        }
        Ok(())
    }

    fn decode(self) -> Result<Vec<u8>, ImportError> {
        if self
            .lanes
            .iter()
            .any(|lane| lane.len() != self.region.lane_bytes)
        {
            return Err(ImportError::UnsupportedMember);
        }
        let size = self.region.source_bytes()?;
        let mut region = allocated_bytes(size)?;
        let sources: Vec<_> = self.lanes.iter().map(Vec::as_slice).collect();
        interleave_words(&sources, self.region.word_bytes, &mut region)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        drop(sources);
        drop(self.lanes);
        let output = self
            .region
            .layout
            .output_len(size)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        let mut indices = allocated_bytes(output)?;
        self.region
            .layout
            .decode(&region, &mut indices)
            .map_err(|_error| ImportError::UnsupportedMember)?;
        Ok(indices)
    }

    fn new(region: Region) -> Self {
        Self {
            lanes: vec![Vec::new(); region.lanes.len()],
            region,
        }
    }
}

fn allocated_bytes(size: usize) -> Result<Vec<u8>, ImportError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_error| ImportError::TooLarge)?;
    bytes.resize(size, 0);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;

    #[test]
    fn rejects_unknown_machines_and_revisions() {
        assert!(Machine::select("simpsons", "world-4p-set-1").is_ok());
        assert!(Machine::select("simpsons", "unknown").is_err());
        assert!(Machine::select("unknown", "world-4p-set-1").is_err());
    }

    #[test]
    fn wrong_archive_identity_never_stages_payloads() -> Result<(), ImportError>
    {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let media = b"synthetic";
        let mut identity = machine.identity()?;
        identity.digest = ContentHash::calculate(media);
        let budget = NonZeroU64::new(32).unwrap_or(NonZeroU64::MIN);
        let verified = VerifiedSource::new(media, &identity, budget)?;
        let limits = ConversionLimits {
            archive: ArchiveBudget::new(budget, budget, budget),
            manifest: machine.graphics_limits(),
        };
        let mut called = false;
        let result =
            machine.convert_graphics(&verified, limits, |_digest, _bytes| {
                called = true;
                Ok(())
            });
        (result == Err(ImportError::IncorrectSource) && !called)
            .then_some(())
            .ok_or(ImportError::StageFailed)
    }
}
