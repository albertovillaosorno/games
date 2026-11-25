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
//   - Inputs: Bounded local source archive and explicit archive budget.
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
//   - Select a supported machine and layout, then decode the archive.
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

use std::collections::BTreeMap;
use std::iter::once;
use std::num::NonZeroUsize;

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetFormat, AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};

use crate::conversion::{
    ConversionLimits, convert_snapshot, preflight_payloads,
};
use crate::graphics::{TileLayout, allocated_bytes, interleave_words};
use crate::{ImportError, SourceSnapshot};

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
        Ok(self.layout.indexed_format(self.source_bytes()?)?)
    }

    fn source_bytes(self) -> Result<usize, ImportError> {
        self.lane_bytes
            .checked_mul(self.lanes.len())
            .ok_or(ImportError::TooLarge)
    }
}

#[derive(Debug)]
struct Profile {
    audio_limits: ManifestLimits,
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

/// A supported machine and source layout, selected by name.
#[derive(Clone, Copy, Debug)]
pub struct Machine(&'static Profile);

impl Machine {
    /// Exact canonical audio resource limits for this revision.
    #[must_use]
    pub const fn audio_limits(self) -> ManifestLimits {
        self.0.audio_limits
    }

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

    /// Convert the declared audio catalogue to a private canonical sink.
    /// # Errors
    /// Rejects mismatched sources, invalid sample data and staging failures.
    pub fn convert_audio<Stage>(
        self,
        source: &SourceSnapshot<'_>,
        limits: ConversionLimits,
        stage: Stage,
    ) -> Result<AssetManifest, ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        convert_snapshot(source, limits.manifest, stage, |verified, output| {
            let rom = self.sample_rom(verified, limits.archive)?;
            samples::catalogue::convert(
                &rom,
                self.audio_limits(),
                limits.manifest,
                output,
            )
        })
    }

    /// Convert every declared graphics region to a private canonical sink.
    ///
    /// # Errors
    /// Rejects mismatched sources, incomplete lanes and conversion failures.
    pub fn convert_graphics<Stage>(
        self,
        source: &SourceSnapshot<'_>,
        limits: ConversionLimits,
        stage: Stage,
    ) -> Result<AssetManifest, ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        if source.identity() != &self.identity() {
            return Err(ImportError::IncorrectSource);
        }
        preflight_payloads(
            limits.manifest,
            self.0.graphics.iter().map(|region| {
                let size = region.layout.output_len(region.source_bytes()?)?;
                u64::try_from(size).map_err(|_error| ImportError::TooLarge)
            }),
        )?;
        convert_snapshot(source, limits.manifest, stage, |verified, output| {
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

    /// The selected title and supported layout, not media authenticity.
    #[must_use]
    pub fn identity(self) -> SourceIdentity {
        SourceIdentity {
            revision: self.0.revision.to_owned(),
            title: self.0.title.to_owned(),
        }
    }

    /// Source-format machine identifier.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0.machine
    }

    /// Exact number of entries in the machine palette RAM.
    #[must_use]
    pub const fn palette_entries(self) -> NonZeroUsize {
        self.0.palette.entries
    }

    /// Assemble only the profiled sample ROM members from a bounded archive.
    /// # Errors
    /// Rejects an incorrect source or missing or malformed sample members.
    pub fn sample_rom(
        self,
        source: &SourceSnapshot<'_>,
        budget: ArchiveBudget,
    ) -> Result<samples::SampleRom, ImportError> {
        if source.identity() != &self.identity() {
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

    /// Check canonical audio descriptors against the selected source.
    /// # Errors
    /// Rejects mismatched identities and invalid audio catalogues.
    pub fn validate_audio_manifest(
        self,
        manifest: &AssetManifest,
    ) -> Result<(), ImportError> {
        if manifest.source() != &self.identity() {
            return Err(ImportError::IncorrectSource);
        }
        samples::catalogue::validate_manifest(
            manifest,
            self.audio_limits().records,
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

    /// Visit a bounded archive and require every profiled media lane.
    ///
    /// This checks layout compatibility, not original-media authenticity.
    /// The callback can run before a later member fails, so any outputs
    /// must remain private until the entire visit succeeds.
    ///
    /// # Errors
    /// Rejects the wrong layout, absent or incorrectly sized required
    /// lanes, unsafe archive members and callback failures.
    pub fn visit_layout<Visit>(
        self,
        source: &SourceSnapshot<'_>,
        budget: ArchiveBudget,
        mut visit: Visit,
    ) -> Result<(), ImportError>
    where
        Visit: FnMut(&str, &[u8]) -> Result<(), ImportError>,
    {
        if source.identity() != &self.identity() {
            return Err(ImportError::IncorrectSource);
        }
        let mut required: BTreeMap<_, _> = self
            .0
            .graphics
            .iter()
            .flat_map(|region| {
                region.lanes.iter().map(|name| (*name, region.lane_bytes))
            })
            .chain(
                self.0
                    .sample_members
                    .iter()
                    .map(|member| (member.name, member.bytes)),
            )
            .collect();
        source.visit_flat_zip(budget, |name, payload| {
            if required
                .remove(name)
                .is_some_and(|size| payload.len() != size)
            {
                return Err(ImportError::UnsupportedMember);
            }
            visit(name, payload)
        })?;
        required
            .is_empty()
            .then_some(())
            .ok_or(ImportError::UnsupportedMember)
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
        Ok(self.region.layout.decode_owned(&region)?)
    }

    fn new(region: Region) -> Self {
        Self {
            lanes: vec![Vec::new(); region.lanes.len()],
            region,
        }
    }
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
    fn graphics_budget_rejects_before_archive_decoding()
    -> Result<(), ImportError> {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let identity = machine.identity();
        let budget = NonZeroU64::new(32).unwrap_or(NonZeroU64::MIN);
        let source = SourceSnapshot::new(b"not a ZIP", &identity, budget)?;
        let expected = machine.graphics_limits();
        let too_small = [
            ManifestLimits {
                payload_bytes: NonZeroU64::MIN,
                ..expected
            },
            ManifestLimits {
                total_bytes: NonZeroU64::MIN,
                ..expected
            },
            ManifestLimits {
                records: NonZeroUsize::MIN,
                ..expected
            },
        ];
        for manifest in too_small {
            let limits = ConversionLimits {
                archive: ArchiveBudget::new(budget, budget, budget),
                manifest,
            };
            let result =
                machine.convert_graphics(&source, limits, |_digest, _bytes| {
                    Err(ImportError::StageFailed)
                });
            if result != Err(games_assets::AssetError::TooLarge.into()) {
                return Err(ImportError::StageFailed);
            }
        }
        Ok(())
    }

    #[test]
    fn wrong_archive_identity_never_stages_payloads() -> Result<(), ImportError>
    {
        let machine = Machine::select("simpsons", "world-4p-set-1")?;
        let media = b"synthetic";
        let mut identity = machine.identity();
        identity.revision = String::from("different-layout");
        let budget = NonZeroU64::new(32).unwrap_or(NonZeroU64::MIN);
        let verified = SourceSnapshot::new(media, &identity, budget)?;
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
