// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Source profile selection and canonical conversion dispatch.
// - Must-Not:
//   - Decode original formats, publish files or execute source programs.
// - Allows:
//   - Inputs: Selected format, family, bounded snapshot and staging sink.
//   - Outputs: Verified canonical manifest or a typed import error.
//   - Side effects: Calls the private caller-owned staging sink.
// - Split-When:
//   - A distinct operation requires independent resource admission.
// - Merge-When:
//   - A conversion facade implements the same public contract.
// - Summary:
//   - Universal import transaction independent of source format.
// - Description:
//   - Selects existing pure decoders without leaking concrete types.
// - Usage:
//   - Import any admitted profile through the same function.
// - Defaults:
//   - Unsupported combinations fail before converting.

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};
use games_importer::conversion::ConversionLimits;
use games_importer::mame::Machine;
use games_importer::neo::CartridgeGraphics;
use games_importer::{ImportError, SourceSnapshot};

/// Canonical output family, independent of the source encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetFamily {
    /// Decoded canonical sound samples.
    Audio,
    /// Decoded canonical indexed graphics.
    Graphics,
}

/// Explicit source-format selection, not a runtime dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFormat {
    /// An admitted MAME machine layout.
    Mame,
    /// An admitted Neo Geo cartridge layout.
    Neo,
}

/// The selected source layout and canonical asset family.
#[derive(Clone, Copy, Debug)]
enum Profile {
    /// Mono PCM sound samples at their source-declared nominal rates.
    Audio(Machine),
    /// Indexed fixed-layer and sprite graphics from a NEO cartridge.
    Cartridge(CartridgeGraphics),
    /// Indexed background and sprite graphics from a MAME ROM set.
    Graphics(Machine),
}

/// Opaque supported source and canonical asset selection.
#[derive(Clone, Copy, Debug)]
pub struct ImportProfile(Profile);

impl ImportProfile {
    /// Canonical output family selected by the caller.
    #[must_use]
    pub const fn family(self) -> AssetFamily {
        match self.0 {
            Profile::Audio(_) => AssetFamily::Audio,
            Profile::Cartridge(_) | Profile::Graphics(_) => {
                AssetFamily::Graphics
            },
        }
    }

    /// Source adapter selected by the caller.
    #[must_use]
    pub const fn format(self) -> SourceFormat {
        match self.0 {
            Profile::Cartridge(_) => SourceFormat::Neo,
            Profile::Audio(_) | Profile::Graphics(_) => SourceFormat::Mame,
        }
    }

    /// Return the canonical title and revision.
    #[must_use]
    pub fn identity(self) -> SourceIdentity {
        match self.0 {
            Profile::Audio(machine) | Profile::Graphics(machine) => {
                machine.identity()
            },
            Profile::Cartridge(profile) => profile.identity(),
        }
    }

    /// Return the admitted profile's output budget.
    #[must_use]
    pub const fn limits(self) -> ManifestLimits {
        match self.0 {
            Profile::Audio(machine) => machine.audio_limits(),
            Profile::Cartridge(profile) => profile.limits(),
            Profile::Graphics(machine) => machine.graphics_limits(),
        }
    }

    /// Select a source layout and requested canonical asset family.
    /// # Errors
    /// Rejects unsupported combinations, titles and revisions.
    pub fn select(
        format: SourceFormat,
        family: AssetFamily,
        title: &str,
        revision: &str,
    ) -> Result<Self, ImportError> {
        let selection = match (format, family) {
            (SourceFormat::Mame, AssetFamily::Audio) => {
                Profile::Audio(Machine::select(title, revision)?)
            },
            (SourceFormat::Mame, AssetFamily::Graphics) => {
                Profile::Graphics(Machine::select(title, revision)?)
            },
            (SourceFormat::Neo, AssetFamily::Graphics) => {
                Profile::Cartridge(CartridgeGraphics::select(title, revision)?)
            },
            (SourceFormat::Neo, AssetFamily::Audio) => {
                return Err(ImportError::IncorrectSource);
            },
        };
        Ok(Self(selection))
    }

    /// Independently verify canonical identity and family descriptors.
    /// # Errors
    /// Rejects mismatched source or output descriptors.
    pub fn validate_manifest(
        self,
        manifest: &AssetManifest,
    ) -> Result<(), ImportError> {
        match self.0 {
            Profile::Audio(machine) => {
                machine.validate_audio_manifest(manifest)
            },
            Profile::Cartridge(profile) => profile.validate_manifest(manifest),
            Profile::Graphics(machine) => machine.validate_manifest(manifest),
        }
    }
}

/// Structural verification request for a source snapshot.
#[derive(Clone, Copy, Debug)]
pub enum Verification {
    /// A profiled machine, including all required source lanes.
    Mame(ImportProfile),
    /// Generic NEO region admission, without profile conversion.
    Neo,
    /// Generic flat ZIP member admission.
    Zip,
}

/// Checked aggregate observation of verified source members.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedSource {
    /// Decoded byte count.
    pub decoded_bytes: u64,
    /// Admitted member count.
    pub members: u64,
}

/// Convert an admitted source through one shared import entry point.
///
/// Payloads must remain private until the complete manifest is returned
/// successfully. The caller must discard staged data on every failure.
/// # Errors
/// Rejects invalid source media, output budgets and staging failures.
pub fn import<Stage>(
    profile: ImportProfile,
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
    stage: Stage,
) -> Result<AssetManifest, ImportError>
where
    Stage: FnMut(ContentHash, &[u8]) -> Result<(), ImportError>,
{
    let manifest = profile.limits();
    let limits = ConversionLimits { archive, manifest };
    let result = match profile.0 {
        Profile::Audio(machine) => machine.convert_audio(source, limits, stage),
        Profile::Cartridge(cartridge) => {
            cartridge.convert(source, limits, stage)
        },
        Profile::Graphics(machine) => {
            machine.convert_graphics(source, limits, stage)
        },
    }?;
    profile.validate_manifest(&result)?;
    Ok(result)
}

/// Verify source structure without converting or executing source data.
/// # Errors
/// Rejects malformed archives, incorrect profiles and exhausted budgets.
pub fn verify(
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
    format: Verification,
) -> Result<VerifiedSource, ImportError> {
    let mut report = VerifiedSource {
        decoded_bytes: 0,
        members: 0,
    };
    let visit = |_name: &str, payload: &[u8]| {
        report.members =
            report.members.checked_add(1).ok_or(ImportError::TooLarge)?;
        report.decoded_bytes = report
            .decoded_bytes
            .checked_add(
                u64::try_from(payload.len())
                    .map_err(|_error| ImportError::TooLarge)?,
            )
            .ok_or(ImportError::TooLarge)?;
        Ok(())
    };
    match format {
        Verification::Mame(profile) => match profile.0 {
            Profile::Audio(machine) | Profile::Graphics(machine) => {
                machine.visit_layout(source, archive, visit)
            },
            Profile::Cartridge(_) => Err(ImportError::IncorrectSource),
        },
        Verification::Neo => source.visit_neo(archive, visit),
        Verification::Zip => source.visit_flat_zip(archive, visit),
    }?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use games_assets::archive::ArchiveBudget;

    use super::{
        AssetFamily, ImportError, ImportProfile, SourceFormat, SourceSnapshot,
        import,
    };

    #[test]
    fn malformed_sources_never_reach_staging() -> Result<(), ImportError> {
        let limit = NonZeroU64::MIN;
        for (format, family, name, revision) in [
            (
                SourceFormat::Mame,
                AssetFamily::Graphics,
                "simpsons",
                "world-4p-set-1",
            ),
            (
                SourceFormat::Mame,
                AssetFamily::Audio,
                "simpsons",
                "world-4p-set-1",
            ),
            (
                SourceFormat::Neo,
                AssetFamily::Graphics,
                "metal-slug",
                "mvs-aes-neo-v1",
            ),
        ] {
            let profile =
                ImportProfile::select(format, family, name, revision)?;
            let identity = profile.identity();
            let source = SourceSnapshot::new(&[0], &identity, limit)?;
            let mut called = false;
            let result = import(
                profile,
                &source,
                ArchiveBudget::new(limit, limit, limit),
                |_digest, _bytes| {
                    called = true;
                    Err(ImportError::StageFailed)
                },
            );
            if called || result.is_ok() {
                return Err(ImportError::StageFailed);
            }
        }
        Ok(())
    }

    #[test]
    fn profiles_reject_unsupported_combinations() {
        assert!(
            ImportProfile::select(
                SourceFormat::Mame,
                AssetFamily::Graphics,
                "simpsons",
                "world-4p-set-1",
            )
            .is_ok()
        );
        assert!(
            ImportProfile::select(
                SourceFormat::Neo,
                AssetFamily::Graphics,
                "metal-slug",
                "mvs-aes-neo-v1",
            )
            .is_ok()
        );
        assert!(
            ImportProfile::select(
                SourceFormat::Neo,
                AssetFamily::Audio,
                "metal-slug",
                "mvs-aes-neo-v1",
            )
            .is_err()
        );
        assert!(
            ImportProfile::select(
                SourceFormat::Mame,
                AssetFamily::Graphics,
                "unknown",
                "unknown",
            )
            .is_err()
        );
    }
}
