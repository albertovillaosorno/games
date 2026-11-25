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
//   - Dispatch to a selected source profile and canonical asset family.
// - Must-Not:
//   - Interpret hardware data, read paths or publish assets.
// - Allows:
//   - Inputs: A supported profile, immutable snapshot and conversion limits.
//   - Outputs: A validated canonical manifest and staging requests.
//   - Side effects: Calls the selected converter's private staging sink.
// - Split-When:
//   - A distinct conversion contract cannot share manifest admission.
// - Merge-When:
//   - Another adapter implements equivalent asset selection.
// - Summary:
//   - Share import, publication and read-back dispatch across formats.
// - Description:
//   - Delegate source interpretation and family validation to profiles.
// - Usage:
//   - Select a known source layout and use the shared conversion transaction.
// - Defaults:
//   - No implicit family or source-layout selection.

//! Canonical asset selection across supported media profiles.

use games_assets::{
    AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};

use crate::conversion::ConversionLimits;
use crate::mame::Machine;
use crate::neo::CartridgeGraphics;
use crate::{ImportError, SourceSnapshot};

/// The selected source layout and canonical asset family.
#[derive(Clone, Copy, Debug)]
pub enum AssetSelection {
    /// Mono PCM sound samples at their source-declared nominal rates.
    Audio(Machine),
    /// Indexed fixed-layer and sprite graphics from a NEO cartridge.
    Cartridge(CartridgeGraphics),
    /// Indexed background and sprite graphics from a MAME ROM set.
    Graphics(Machine),
}

impl AssetSelection {
    /// Convert a complete asset family using the shared transaction.
    /// # Errors
    /// Rejects invalid source identity, assets or staging failures.
    pub fn convert<Stage>(
        self,
        source: &SourceSnapshot<'_>,
        limits: ConversionLimits,
        stage: Stage,
    ) -> Result<AssetManifest, ImportError>
    where
        Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
    {
        match self {
            Self::Audio(machine) => {
                machine.convert_audio(source, limits, stage)
            },
            Self::Cartridge(profile) => profile.convert(source, limits, stage),
            Self::Graphics(machine) => {
                machine.convert_graphics(source, limits, stage)
            },
        }
    }

    /// The selected title and source layout, not media authenticity.
    #[must_use]
    pub fn identity(self) -> SourceIdentity {
        match self {
            Self::Audio(machine) | Self::Graphics(machine) => {
                machine.identity()
            },
            Self::Cartridge(profile) => profile.identity(),
        }
    }

    /// The selected profile's independent canonical output limits.
    #[must_use]
    pub const fn limits(self) -> ManifestLimits {
        match self {
            Self::Audio(machine) => machine.audio_limits(),
            Self::Cartridge(profile) => profile.limits(),
            Self::Graphics(machine) => machine.graphics_limits(),
        }
    }

    /// Check identity and declared family shape of a pinned manifest.
    /// # Errors
    /// Rejects another source or unexpected asset keys or types.
    pub fn validate_manifest(
        self,
        manifest: &AssetManifest,
    ) -> Result<(), ImportError> {
        match self {
            Self::Audio(machine) => machine.validate_audio_manifest(manifest),
            Self::Cartridge(profile) => profile.validate_manifest(manifest),
            Self::Graphics(machine) => machine.validate_manifest(manifest),
        }
    }
}
