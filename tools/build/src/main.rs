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
//   - Host command parsing and asset conversion/publication composition.
// - Must-Not:
//   - Execute original binaries, interpret graphics layouts or bypass limits.
// - Allows:
//   - Inputs: Local archive, expected identity, limits and output directory.
//   - Outputs: Verified summaries or private canonical bundle publication.
//   - Side effects: Explicit local file access and stdout summaries.
// - Split-When:
//   - Game packaging introduces independent target-selection policies.
// - Merge-When:
//   - Another host tool implements the same import orchestration.
// - Summary:
//   - Verify and import locally supplied originals without executing them.
// - Description:
//   - Compose importer and platform services without coupling their crates.
// - Usage:
//   - cargo run --locked -p games-build -- mame import MACHINE REV PATH LIMITS
// - Defaults:
//   - Read-only verification and explicit opt-in to publication.

//! Host-side original media verification and asset import.
#[cfg(feature = "native-preview")]
/// Bounded canonical PCM consumption and native queue diagnostics.
pub mod audio_check;

/// Independent portable canonical bundle verification.
pub mod bundle_check;
/// Generate contributor Ghidra processor descriptions.
pub mod ghidra;
/// Generate contributor Ghidra board mappers.
pub mod ghidra_board;
/// Native canonical bundle publication.
pub mod publication;
/// Exact bounded comparison of private PCM reference recordings.
pub mod reference_audio;
/// Bounded background-reference comparison.
pub mod reference_background;
/// Bounded read-only function ledger consistency check.
pub mod reference_ledger;
/// Read-only CPU and device object buffer comparison.
pub mod reference_objects;
/// Bounded read-only palette comparison against local observations.
pub mod reference_palette;
/// Bounded read-only video mixer comparison against local observations.
pub mod reference_priority;
/// Explicit bounded private research archive retention.
pub mod reference_retention;
/// Bounded local tile RAM admission diagnostic.
pub mod reference_tilemap;
#[cfg(feature = "native-preview")]
/// Finite native graphics benchmark measurements.
pub mod render_benchmark;
#[cfg(feature = "native-preview")]
/// Bounded native diagnostic rendering of selected imported graphics.
pub mod render_check;
/// Read-only verification of legacy store migration prerequisites.
pub mod store_migration;

/// Compile one admitted independently authored title.
pub mod title_build;

/// Package the selected authored distribution.
pub mod title_package;

use std::ffi::{OsStr, OsString};
use std::num::{NonZeroU64, NonZeroUsize};

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};
use games_platform::ir::importer::reference::{Machine, compare_observations};
use games_platform::ir::importer::{
    AssetFamily, ImportProfile, SourceFormat, SourceSnapshot, Verification,
    import, verify,
};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain;

const USAGE: &str = concat!(
    "games-build ghidra konami cspec|ldefs|pspec|slaspec\n",
    "games-build ghidra-board simpsons [fixture|flow]\n",
    "games-build ledger check PATH BYTES ROWS\n",
    "games-build retention plan AGE_SECONDS QUOTA_BYTES ENTRIES BYTES\n",
    "games-build retention apply AGE_SECONDS QUOTA_BYTES ENTRIES BYTES HASH\n",
    "usage: games-build build TITLE [linux-x86-64|webassembly]\n",
    "games-build package TITLE [OUTPUT [TARGET]]\n",
    "games-build package-check TITLE OUTPUT [TARGET]\n",
    "games-build bundle migration-audit TITLE PARENT STORE_ENTRIES ",
    "STORE_BYTES MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "games-build bundle migration-catalogue TITLE PARENT STORE_ENTRIES ",
    "STORE_BYTES MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT ",
    "CATALOGUE CATALOGUE_BYTES last-token|fields:KEY,...\n",
    "games-build bundle migration-rehearse TITLE LEGACY_ROOT ",
    "STORE_ENTRIES STORE_BYTES MANIFEST_BYTES PAYLOAD_BYTES ",
    "TOTAL_BYTES COUNT TEMPLATE TEMPLATE_BYTES fields:KEY,... ",
    "STAGING_PARENT STAGE_ENTRY STAGE_TOTAL STAGE_COUNT ",
    "[POSE_FILE POSE_BYTES]\n",
    "games-build bundle migration-publish TITLE LEGACY_ROOT ",
    "STORE_ENTRIES STORE_BYTES MANIFEST_BYTES PAYLOAD_BYTES ",
    "TOTAL_BYTES COUNT TEMPLATE TEMPLATE_BYTES fields:KEY,... ",
    "STAGE_ENTRY STAGE_TOTAL STAGE_COUNT last-token SIDECAR_BYTES ",
    "[POSE_FILE POSE_BYTES]\n",
    "games-build bundle migration-verify TITLE LEGACY_ROOT ",
    "STORE_ENTRIES STORE_BYTES MANIFEST_BYTES PAYLOAD_BYTES ",
    "TOTAL_BYTES COUNT TEMPLATE TEMPLATE_BYTES fields:KEY,... ",
    "STAGE_ENTRY STAGE_TOTAL STAGE_COUNT last-token SIDECAR_BYTES ",
    "[POSE_FILE POSE_BYTES]\n",
    "games-build bundle upgrade PARENT MANIFEST_SHA256 ",
    "MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "games-build bundle replace-media PARENT BASE_SHA256 KEY ",
    "RAW MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "games-build bundle replace-wave PARENT BASE_SHA256 KEY ",
    "WAV MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "games-build neo import TITLE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "neo publish TITLE REV PARENT PATH SOURCE ENTRY TOTAL COUNT\n",
    "neo check TITLE REV PARENT MANIFEST_SHA256\n",
    "games-build zip|neo verify PATH TITLE REV ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "games-build import FORMAT FAMILY PROFILE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "games-build mame verify|import|audio-import ",
    "MACHINE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "mame publish|audio-publish MACHINE REV PARENT PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "mame check|audio-check MACHINE REV PARENT MANIFEST_SHA256\n",
    "mame art-publish MACHINE REV OUTPUT PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS KEY:INDEX,...\n",
    "mame strip-publish MACHINE REV OUTPUT PARENT SHA256 ",
    "TOP SIDE [--scene-limit COUNT] ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS ",
    "[MORE SCENES...]\n",
    "mame background-check MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32\n",
    "mame scene-check MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS\n",
    "mame scene-layer-digests MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS\n",
    "mame scene-background-digest MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS TOP\n",
    "mame scene-background-publish MACHINE REV OUTPUT PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS TOP\n",
    "native-preview: mame scene-sequence-bench MACHINE REV ",
    "PARENT SHA256 REPEATS RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS ",
    "CONTROLS [MORE SCENES...]\n",
    "native-preview: mame scene-render-check MACHINE REV ",
    "PARENT SHA256 RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS\n",
    "mame sample-check MACHINE REV OBSERVATIONS LIMIT ",
    "PATH SOURCE ENTRY TOTAL COUNT\n",
    "mame object-check MACHINE REV CPU DEVICE\n",
    "mame palette-check MACHINE REV RAW ARGB32",
    "\nmame priority-check MACHINE REV RAW OBSERVED_LE32",
    "\nmame tilemap-check MACHINE REV RAM REGISTERS",
    "\n",
    "native-preview: mame audio-render-check MACHINE REV ",
    "PARENT SHA256 LEFT_KEY RIGHT_KEY FRAMES\n",
    "native-preview: mame render-check MACHINE REV ",
    "PARENT SHA256 KEY RAW FIRST COUNT COLUMNS PEN_START PEN_COUNT",
    "\nnative-preview: mame render-bench MACHINE REV ",
    "PARENT SHA256 KEY RAW FIRST COUNT COLUMNS PEN_START PEN_COUNT FRAMES",
    "\n",
    "wave compare LEFT RIGHT LIMIT\n",
    "wave recurrence WAV START END FIRST LAST STEP STRIDE WORK LIMIT\n",
    "wave publish TITLE REV PARENT WAV KEY START END LIMIT"
);

#[derive(Debug)]
struct Arguments {
    archive: ArchiveBudget,
    identity: SourceIdentity,
    path: OsString,
    source_limit: NonZeroU64,
}

impl Arguments {
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [path, title, revision, source, entry, total, count] =
            <[OsString; 7]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let identity = SourceIdentity {
            revision: text(&revision)?.to_owned(),
            title: text(&title)?.to_owned(),
        };
        identity
            .validate()
            .map_err(|_error| String::from("invalid title or revision"))?;
        Self::with_identity(identity, path, [source, entry, total, count])
    }

    fn parse_profile(
        identity: SourceIdentity,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [path, source, entry, total, count] =
            <[OsString; 5]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Self::with_identity(identity, path, [source, entry, total, count])
    }

    fn snapshot(&self) -> Result<Vec<u8>, String> {
        snapshot(&self.path, self.source_limit)
            .map_err(|error| format!("source file: {error:?}"))
    }

    fn with_identity(
        identity: SourceIdentity,
        path: OsString,
        limits: [OsString; 4],
    ) -> Result<Self, String> {
        let [source, entry, total, count] = limits;
        Ok(Self {
            archive: ArchiveBudget::new(
                parse_limit(&entry)?,
                parse_limit(&total)?,
                parse_limit(&count)?,
            ),
            identity,
            path,
            source_limit: parse_limit(&source)?,
        })
    }
}

#[derive(Debug)]
enum Command {
    ArtPublish(Machine, reference_background::art::Art),
    #[cfg(feature = "native-preview")]
    AudioCheck(Machine, audio_check::Arguments),
    BackgroundCheck(Machine, reference_background::Arguments),
    BackgroundPublish(
        Machine,
        reference_background::art::BackgroundPublication,
    ),
    BuildTitle(title_build::Distribution),
    Check(ImportProfile, OsString, ContentHash),
    Ghidra(ghidra::Component),
    GhidraBoard(ghidra_board::Board),
    Import(ImportProfile, Arguments),
    LedgerCheck(reference_ledger::Arguments),
    MigrationAudit(store_migration::Audit),
    MigrationCatalogue(Box<store_migration::Catalogue>),
    MigrationRehearsal(Box<store_migration::rehearsal::Rehearsal>),
    ObjectCheck(Machine, OsString, OsString),
    PackageCheck(title_build::Distribution, OsString),
    PackageExplicit(title_build::Distribution, OsString),
    PackageTitle(title_build::Title),
    PaletteCheck(Machine, OsString, OsString),
    PriorityCheck(Machine, OsString, OsString),
    Publish(ImportProfile, OsString, Arguments),
    #[cfg(feature = "native-preview")]
    RenderCheck(Machine, render_check::Arguments),
    ReplaceMedia(publication::MediaReplacement),
    Retention(reference_retention::Arguments),
    SampleCheck(Machine, OsString, NonZeroU64, Arguments),
    #[cfg(feature = "native-preview")]
    SceneSequence(Machine, reference_background::sequence::Sequence),
    StripPublish(Machine, reference_background::art::Strip),
    TilemapCheck(Machine, OsString, OsString),
    Upgrade(OsString, ContentHash, ManifestLimits),
    Verify(Verification, Arguments),
    WaveCompare(OsString, OsString, NonZeroU64),
    WavePublish(reference_audio::Publication),
    WaveRecurrence(reference_audio::Recurrence),
}

impl Command {
    fn parse<Args>(mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let format = args.next().ok_or_else(|| String::from(USAGE))?;
        let action = args.next().ok_or_else(|| String::from(USAGE))?;
        match (format.to_str(), action.to_str()) {
            (Some("ghidra-board"), Some(board)) => Ok(Self::GhidraBoard(
                ghidra_board::Board::parse(board, &args.collect::<Vec<_>>())?,
            )),
            (Some("ghidra"), Some(processor)) => Ok(Self::Ghidra(
                ghidra::Component::parse(processor, &args.collect::<Vec<_>>())?,
            )),
            (Some("ledger"), Some("check")) => Ok(Self::LedgerCheck(
                reference_ledger::Arguments::parse(args.collect())?,
            )),
            (Some("retention"), Some(operation)) => {
                Ok(Self::Retention(reference_retention::Arguments::parse(
                    operation,
                    &args.collect::<Vec<_>>(),
                )?))
            },
            (Some("import"), Some(source_format)) => {
                Self::parse_import(source_format, args)
            },
            (Some("build"), Some(title)) => {
                Ok(Self::BuildTitle(title_build::Distribution::parse(
                    title,
                    &args.collect::<Vec<_>>(),
                )?))
            },
            (Some("package-check"), Some(title)) => {
                let output = args.next().ok_or_else(|| String::from(USAGE))?;
                Ok(Self::PackageCheck(
                    title_build::Distribution::parse(
                        title,
                        &args.collect::<Vec<_>>(),
                    )?,
                    output,
                ))
            },
            (Some("package"), Some(title)) => Self::parse_package(title, args),
            (Some("bundle"), Some(operation)) => {
                Self::parse_bundle(operation, args.collect())
            },
            (Some(kind @ ("zip" | "neo")), Some("verify")) => {
                Self::parse_verification(kind, args.collect())
            },
            (Some("wave"), Some(operation)) => {
                Self::parse_wave(operation, args.collect())
            },
            (Some("neo"), Some(operation)) => Self::parse_neo(operation, args),
            (Some("mame"), Some(operation)) => {
                Self::parse_mame(operation, args)
            },
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_art(
        action: &str,
        machine: Machine,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        if action == "strip-publish" {
            Ok(Self::StripPublish(
                machine,
                reference_background::art::Strip::parse(&args)?,
            ))
        } else {
            Ok(Self::ArtPublish(
                machine,
                reference_background::art::Art::parse(args)?,
            ))
        }
    }

    fn parse_assets<Args>(
        action: &str,
        selection: ImportProfile,
        mut args: Args,
    ) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        match action {
            "import" | "audio-import" => Ok(Self::Import(
                selection,
                Arguments::parse_profile(
                    selection.identity(),
                    args.by_ref().collect(),
                )?,
            )),
            "publish" | "audio-publish" => {
                let parent = args.next().ok_or_else(|| String::from(USAGE))?;
                Ok(Self::Publish(
                    selection,
                    parent,
                    Arguments::parse_profile(
                        selection.identity(),
                        args.collect(),
                    )?,
                ))
            },
            "check" | "audio-check" => {
                Self::parse_check(selection, args.collect())
            },
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_bundle(
        operation: &str,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        match operation {
            "migration-audit" => {
                Ok(Self::MigrationAudit(store_migration::Audit::parse(args)?))
            },
            "migration-catalogue" => Ok(Self::MigrationCatalogue(Box::new(
                store_migration::Catalogue::parse(args)?,
            ))),
            "migration-rehearse" => Ok(Self::MigrationRehearsal(Box::new(
                store_migration::rehearsal::Rehearsal::parse(args)?,
            ))),
            "migration-publish" => Ok(Self::MigrationRehearsal(Box::new(
                store_migration::rehearsal::Rehearsal::parse_publish(args)?,
            ))),
            "migration-verify" => Ok(Self::MigrationRehearsal(Box::new(
                store_migration::rehearsal::Rehearsal::parse_verify(args)?,
            ))),
            "upgrade" => Self::parse_upgrade(args),
            "replace-media" => {
                Self::parse_replace_media(args, publication::MediaEncoding::Raw)
            },
            "replace-wave" => Self::parse_replace_media(
                args,
                publication::MediaEncoding::Wave,
            ),
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_check(
        selection: ImportProfile,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [parent, digest_argument] = <[OsString; 2]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        let digest = text(&digest_argument)?
            .parse()
            .map_err(|_error| String::from("invalid manifest hash"))?;
        Ok(Self::Check(selection, parent, digest))
    }

    fn parse_import<Args>(
        format_name: &str,
        mut args: Args,
    ) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let format = match format_name {
            "mame" => SourceFormat::Mame,
            "neo" => SourceFormat::Neo,
            _ => return Err(String::from(USAGE)),
        };
        let family_name = args.next().ok_or_else(|| String::from(USAGE))?;
        let family = match text(&family_name)? {
            "audio" => AssetFamily::Audio,
            "graphics" => AssetFamily::Graphics,
            _ => return Err(String::from(USAGE)),
        };
        let profile = args.next().ok_or_else(|| String::from(USAGE))?;
        let revision = args.next().ok_or_else(|| String::from(USAGE))?;
        let selection = ImportProfile::select(
            format,
            family,
            text(&profile)?,
            text(&revision)?,
        )
        .map_err(|error| format!("import profile: {error:?}"))?;
        Self::parse_assets("import", selection, args)
    }

    fn parse_mame<Args>(action: &str, mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let selected = select_machine(&mut args)?;
        #[cfg(feature = "native-preview")]
        if matches!(action, "audio-render-check" | "scene-sequence-bench") {
            return Self::parse_native_workload(
                action,
                selected,
                args.collect(),
            );
        }
        match action {
            "art-publish" | "strip-publish" => {
                Self::parse_art(action, selected, args.collect())
            },
            "scene-background-publish" => Ok(Self::BackgroundPublish(
                selected,
                reference_background::art::BackgroundPublication::parse(
                    args.collect(),
                )?,
            )),
            "background-check"
            | "scene-check"
            | "scene-render-check"
            | "scene-layer-digests"
            | "scene-background-digest" => Ok(Self::BackgroundCheck(
                selected,
                reference_background::Arguments::parse_command(
                    action,
                    args.collect(),
                )?,
            )),
            "sample-check" => Self::parse_sample(selected, args),
            "verify" => Ok(Self::Verify(
                Verification::Mame(
                    ImportProfile::select(
                        SourceFormat::Mame,
                        AssetFamily::Graphics,
                        selected.name(),
                        &selected.identity().revision,
                    )
                    .map_err(|error| format!("machine profile: {error:?}"))?,
                ),
                Arguments::parse_profile(selected.identity(), args.collect())?,
            )),
            "object-check" | "palette-check" | "priority-check"
            | "tilemap-check" => {
                Self::parse_reference(action, selected, args.collect())
            },
            #[cfg(feature = "native-preview")]
            "render-check" => Ok(Self::RenderCheck(
                selected,
                render_check::Arguments::parse(args.collect())?,
            )),
            #[cfg(feature = "native-preview")]
            "render-bench" => Ok(Self::RenderCheck(
                selected,
                render_check::Arguments::parse_benchmark(args.collect())?,
            )),
            _ => Self::parse_mame_assets(action, selected, args),
        }
    }

    fn parse_mame_assets<Args>(
        action: &str,
        machine: Machine,
        args: Args,
    ) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let family = if action.starts_with("audio-") {
            AssetFamily::Audio
        } else {
            AssetFamily::Graphics
        };
        let identity = machine.identity();
        let selection = ImportProfile::select(
            SourceFormat::Mame,
            family,
            machine.name(),
            &identity.revision,
        )
        .map_err(|error| format!("machine profile: {error:?}"))?;
        Self::parse_assets(action, selection, args)
    }

    #[cfg(feature = "native-preview")]
    fn parse_native_workload(
        action: &str,
        machine: Machine,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        match action {
            "audio-render-check" => Ok(Self::AudioCheck(
                machine,
                audio_check::Arguments::parse(args)?,
            )),
            "scene-sequence-bench" => Ok(Self::SceneSequence(
                machine,
                reference_background::sequence::Sequence::parse(&args)?,
            )),
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_neo<Args>(action: &str, mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        if !matches!(action, "import" | "publish" | "check") {
            return Err(String::from(USAGE));
        }
        let title = args.next().ok_or_else(|| String::from(USAGE))?;
        let revision = args.next().ok_or_else(|| String::from(USAGE))?;
        let profile = ImportProfile::select(
            SourceFormat::Neo,
            AssetFamily::Graphics,
            text(&title)?,
            text(&revision)?,
        )
        .map_err(|error| format!("cartridge profile: {error:?}"))?;
        Self::parse_assets(action, profile, args)
    }

    fn parse_package<Args>(title: &str, mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let Some(output) = args.next() else {
            return Ok(Self::PackageTitle(title_build::Title::parse(
                title,
                &[],
            )?));
        };
        Ok(Self::PackageExplicit(
            title_build::Distribution::parse(title, &args.collect::<Vec<_>>())?,
            output,
        ))
    }

    fn parse_reference(
        action: &str,
        machine: Machine,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [raw, reference] = <[OsString; 2]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        match action {
            "object-check" => Ok(Self::ObjectCheck(machine, raw, reference)),
            "palette-check" => Ok(Self::PaletteCheck(machine, raw, reference)),
            "priority-check" => {
                Ok(Self::PriorityCheck(machine, raw, reference))
            },
            "tilemap-check" => Ok(Self::TilemapCheck(machine, raw, reference)),
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_replace_media(
        args: Vec<OsString>,
        encoding: publication::MediaEncoding,
    ) -> Result<Self, String> {
        let [parent, digest, key, source, metadata, payload, total, count] =
            <[OsString; 8]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self::ReplaceMedia(publication::MediaReplacement {
            base: parse_manifest_digest(&digest)?,
            encoding,
            key: text(&key)?.to_owned(),
            limits: parse_manifest_limits([metadata, payload, total, count])?,
            parent,
            source,
        }))
    }

    fn parse_sample<Args>(
        machine: Machine,
        mut args: Args,
    ) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let raw = args.next().ok_or_else(|| String::from(USAGE))?;
        let limit = args.next().ok_or_else(|| String::from(USAGE))?;
        Ok(Self::SampleCheck(
            machine,
            raw,
            parse_limit(&limit)?,
            Arguments::parse_profile(machine.identity(), args.collect())?,
        ))
    }

    fn parse_upgrade(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, raw_digest, metadata, payload, total, count] =
            <[OsString; 6]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self::Upgrade(
            parent,
            parse_manifest_digest(&raw_digest)?,
            parse_manifest_limits([metadata, payload, total, count])?,
        ))
    }

    fn parse_verification(
        kind: &str,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let verification = if kind == "neo" {
            Verification::Neo
        } else {
            Verification::Zip
        };
        Ok(Self::Verify(verification, Arguments::parse(args)?))
    }

    fn parse_wave(
        operation: &str,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        match operation {
            "compare" => {
                let [left, right, limit] = <[OsString; 3]>::try_from(args)
                    .map_err(|_args| String::from(USAGE))?;
                Ok(Self::WaveCompare(left, right, parse_limit(&limit)?))
            },
            "recurrence" => Ok(Self::WaveRecurrence(
                reference_audio::Recurrence::parse(args)?,
            )),
            "publish" => Ok(Self::WavePublish(
                reference_audio::Publication::parse(args)?,
            )),
            _ => Err(String::from(USAGE)),
        }
    }

    fn run(self) -> Result<(), String> {
        match self {
            Self::BuildTitle(title) => title.run(),
            Self::LedgerCheck(arguments) => arguments.run(),
            Self::Retention(arguments) => arguments.run(),
            Self::Ghidra(component) => component.run(),
            Self::GhidraBoard(board) => board.run(),
            Self::PackageCheck(title, output) => {
                title_package::check(title, &output)
            },
            Self::PackageExplicit(title, output) => {
                title_package::run(title, &output)
            },
            Self::PackageTitle(title) => title_package::configured(title),
            Self::ArtPublish(machine, args) => args.run(machine),
            Self::StripPublish(machine, args) => args.run(machine),
            #[cfg(feature = "native-preview")]
            Self::AudioCheck(machine, args) => args.run(machine),
            #[cfg(feature = "native-preview")]
            Self::SceneSequence(machine, args) => args.run(machine),
            Self::BackgroundCheck(machine, args) => args.run(machine),
            Self::BackgroundPublish(machine, args) => args.run(machine),
            Self::ObjectCheck(machine, cpu, device) => {
                reference_objects::check(machine, &cpu, &device)
            },
            Self::PaletteCheck(machine, raw, reference) => {
                reference_palette::check(machine, &raw, &reference)
            },
            Self::PriorityCheck(machine, raw, reference) => {
                reference_priority::check(machine, &raw, &reference)
            },
            Self::TilemapCheck(machine, raw, registers) => {
                reference_tilemap::check(machine, &raw, &registers)
            },
            #[cfg(feature = "native-preview")]
            Self::RenderCheck(machine, args) => args.run(machine),
            Self::WaveCompare(left, right, limit) => {
                reference_audio::check(&left, &right, limit)
            },
            Self::WavePublish(publication) => publication.run(),
            Self::WaveRecurrence(recurrence) => recurrence.run(),
            Self::SampleCheck(machine, raw, limit, args) => {
                report_samples(machine, &raw, limit, &args)
            },
            command => command.run_assets(),
        }
    }

    fn run_assets(self) -> Result<(), String> {
        match self {
            Self::MigrationAudit(audit) => audit.run(),
            Self::MigrationCatalogue(catalogue) => catalogue.run(),
            Self::MigrationRehearsal(rehearsal) => rehearsal.run(),
            Self::Upgrade(parent, digest, limits) => {
                publication::upgrade(&parent, digest, limits)
            },
            Self::ReplaceMedia(replacement) => replacement.run(),
            Self::Verify(format, args) => {
                with_source(&args, |source, archive| {
                    verify_source(source, archive, format)
                })
            },
            Self::Import(selection, args) => {
                with_source(&args, |source, archive| {
                    report_assets(selection, source, archive)
                })
            },
            Self::Publish(selection, parent, args) => {
                with_source(&args, |source, archive| {
                    publication::publish(selection, &parent, source, archive)
                })
            },
            Self::Check(selection, parent, digest) => {
                bundle_check::check(selection, &parent, digest)
            },
            #[cfg(feature = "native-preview")]
            Self::AudioCheck(..)
            | Self::RenderCheck(..)
            | Self::SceneSequence(..) => Err(String::from(USAGE)),
            Self::ArtPublish(..)
            | Self::BackgroundCheck(..)
            | Self::BackgroundPublish(..)
            | Self::BuildTitle(..)
            | Self::LedgerCheck(..)
            | Self::ObjectCheck(..)
            | Self::PackageCheck(..)
            | Self::PackageTitle(..)
            | Self::PackageExplicit(..)
            | Self::PaletteCheck(..)
            | Self::PriorityCheck(..)
            | Self::Retention(..)
            | Self::Ghidra(..)
            | Self::GhidraBoard(..)
            | Self::SampleCheck(..)
            | Self::StripPublish(..)
            | Self::TilemapCheck(..)
            | Self::WaveCompare(..)
            | Self::WavePublish(..)
            | Self::WaveRecurrence(..) => Err(String::from(USAGE)),
        }
    }
}

fn select_machine<Args>(args: &mut Args) -> Result<Machine, String>
where
    Args: Iterator<Item = OsString>,
{
    let machine = args.next().ok_or_else(|| String::from(USAGE))?;
    let revision = args.next().ok_or_else(|| String::from(USAGE))?;
    Machine::select(text(&machine)?, text(&revision)?)
        .map_err(|error| format!("MAME profile: {error:?}"))
}

fn text(value: &OsString) -> Result<&str, String> {
    value
        .to_str()
        .ok_or_else(|| String::from("expected UTF-8 argument"))
}

fn parse_manifest_digest(value: &OsString) -> Result<ContentHash, String> {
    text(value)?
        .parse()
        .map_err(|_error| String::from("invalid manifest hash"))
}

fn parse_manifest_limits(
    [metadata, payload, total, count]: [OsString; 4],
) -> Result<ManifestLimits, String> {
    let size_limit = |argument: &OsString| {
        text(argument)?
            .parse::<NonZeroUsize>()
            .map_err(|_error| String::from("invalid metadata or record limit"))
    };
    Ok(ManifestLimits {
        manifest_bytes: size_limit(&metadata)?,
        payload_bytes: parse_limit(&payload)?,
        records: size_limit(&count)?,
        total_bytes: parse_limit(&total)?,
    })
}

fn parse_limit(value: &OsString) -> Result<NonZeroU64, String> {
    text(value)?
        .parse()
        .map_err(|_error| String::from("expected positive byte or count limit"))
}

fn with_source<Run>(args: &Arguments, run: Run) -> Result<(), String>
where
    Run: FnOnce(&SourceSnapshot<'_>, ArchiveBudget) -> Result<(), String>,
{
    let bytes = args.snapshot()?;
    let source = SourceSnapshot::new(&bytes, &args.identity, args.source_limit)
        .map_err(|error| format!("source: {error:?}"))?;
    run(&source, args.archive)
}

fn main() -> Result<(), String> {
    Command::parse(
        toolchain::arguments()
            .map_err(|error| error.to_string())?
            .into_iter(),
    )?
    .run()
}

fn verify_source(
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
    format: Verification,
) -> Result<(), String> {
    let report = verify(source, archive, format)
        .map_err(|error| format!("archive: {error:?}"))?;
    let count = report.members;
    let total = report.decoded_bytes;
    let identity = source.identity();
    toolchain::write_line(&format!(
        "Admitted {} {}: {count} members, {total} decoded bytes",
        identity.title, identity.revision,
    ))
    .map_err(|error| error.to_string())
}

fn report_samples(
    machine: Machine,
    raw: &OsStr,
    limit: NonZeroU64,
    args: &Arguments,
) -> Result<(), String> {
    with_source(args, |source, archive| {
        let rom = machine
            .sample_rom(source, archive)
            .map_err(|error| format!("sample ROM: {error:?}"))?;
        let bytes = snapshot(raw, limit)
            .map_err(|error| format!("observation: {error:?}"))?;
        let report = compare_observations(&rom, &bytes)
            .map_err(|error| format!("sample comparison: {error:?}"))?;
        toolchain::write_line(&format!(
            "{} frames: {} PCM, {} differential, {} mismatches",
            report.frames, report.pcm, report.differential, report.mismatches,
        ))
        .map_err(|error| error.to_string())?;
        (report.mismatches == 0)
            .then_some(())
            .ok_or_else(|| String::from("observed sample amplitudes differ"))
    })
}

fn report_manifest(
    converted: &AssetManifest,
    digest: ContentHash,
) -> Result<(), String> {
    for record in converted.records() {
        toolchain::write_line(&format!(
            "{}: {} bytes ({})",
            record.key, record.size, record.digest
        ))
        .map_err(|error| error.to_string())?;
    }
    toolchain::write_line(&format!("Canonical manifest: {digest}"))
        .map_err(|error| error.to_string())
}

fn report_assets(
    selection: ImportProfile,
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let manifest = selection.limits();
    let converted =
        import(selection, source, archive, |_digest, _bytes| Ok(()))
            .map_err(|error| format!("assets: {error:?}"))?;
    let encoded = converted
        .encode(manifest)
        .map_err(|error| format!("manifest: {error:?}"))?;
    report_manifest(&converted, ContentHash::calculate(&encoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_build_command_requires_known_title_and_optional_target() {
        let args = ["build", "simpsons-arcade", "linux-x86-64"];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::BuildTitle(selection))
                if !selection.is_browser()
        ));
        for invalid in [["build", "metal-slug", "linux-x86-64"], [
            "build",
            "simpsons-arcade",
            "android-arm64",
        ]] {
            assert!(
                Command::parse(invalid.into_iter().map(OsString::from))
                    .is_err()
            );
        }
    }

    #[test]
    fn package_command_admits_explicit_or_configured_output() {
        let args = ["package", "simpsons-arcade", "local-package"];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::PackageExplicit(selection, _))
                if !selection.is_browser()
        ));
        assert!(matches!(
            Command::parse(
                ["package", "simpsons-arcade"]
                    .map(OsString::from)
                    .into_iter()
            ),
            Ok(Command::PackageTitle(title_build::Title::SimpsonsArcade))
        ));
        let invalid: [&[&str]; 2] = [&["package", "metal-slug", "out"], &[
            "package",
            "simpsons-arcade",
            "out",
            "extra",
        ]];
        for candidate in invalid {
            assert!(
                Command::parse(candidate.iter().map(OsString::from)).is_err()
            );
        }
    }

    #[test]
    fn browser_commands_admit_only_the_explicit_webassembly_target() {
        for command in ["build", "package", "package-check"] {
            let mut args = vec![command, "simpsons-arcade"];
            if command != "build" {
                args.push("local-package");
            }
            args.push("webassembly");
            let parsed = Command::parse(args.into_iter().map(OsString::from));
            assert!(matches!(
                parsed,
                Ok(Command::BuildTitle(selection)
                    | Command::PackageExplicit(selection, _)
                    | Command::PackageCheck(selection, _))
                    if selection.is_browser()
            ));
        }
    }

    #[test]
    fn package_check_requires_a_title_and_explicit_directory() {
        let args = ["package-check", "simpsons-arcade", "local-package"];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::PackageCheck(selection, _)) if !selection.is_browser()
        ));
        let invalid: [&[&str]; 3] = [
            &["package-check", "simpsons-arcade"],
            &["package-check", "unknown", "local-package"],
            &["package-check", "simpsons-arcade", "local-package", "extra"],
        ];
        for candidate in invalid {
            assert!(
                Command::parse(candidate.iter().map(OsString::from)).is_err()
            );
        }
    }

    #[test]
    fn command_requires_all_explicit_limits_and_identity() {
        let mut args = [
            "archive.zip",
            "synthetic",
            "world-set-1",
            "100",
            "50",
            "200",
            "4",
        ]
        .map(OsString::from);
        assert!(Arguments::parse(args.to_vec()).is_ok());
        let mut legacy = args.to_vec();
        legacy.insert(3, OsString::from("1".repeat(64)));
        assert!(Arguments::parse(legacy).is_err());
        if let Some(count) = args.last_mut() {
            *count = OsString::from("0");
        }
        assert!(Arguments::parse(args.to_vec()).is_err());
        assert!(Arguments::parse(Vec::new()).is_err());
    }

    #[test]
    fn bundle_upgrade_requires_pinned_address_and_all_limits() {
        let digest = "1".repeat(64);
        let args = [
            "bundle", "upgrade", "output", &digest, "4096", "16", "64", "4",
        ];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::Upgrade(_, _, _))
        ));
        assert!(
            Command::parse(args.into_iter().take(7).map(OsString::from))
                .is_err()
        );
        let mut invalid = args;
        invalid[3] = "wrong";
        assert!(
            Command::parse(invalid.into_iter().map(OsString::from)).is_err()
        );
        invalid = args;
        invalid[7] = "0";
        assert!(
            Command::parse(invalid.into_iter().map(OsString::from)).is_err()
        );
    }

    #[test]
    fn media_replacement_requires_explicit_slot_and_limits() {
        let digest = "1".repeat(64);
        let args = [
            "bundle",
            "replace-media",
            "output",
            &digest,
            "audio/sample",
            "raw.pcm",
            "4096",
            "16",
            "64",
            "4",
        ];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::ReplaceMedia(_))
        ));
        assert!(
            Command::parse(args.into_iter().take(9).map(OsString::from))
                .is_err()
        );
        let mut wave = args;
        wave[1] = "replace-wave";
        let parsed = Command::parse(wave.into_iter().map(OsString::from));
        assert!(matches!(
            parsed,
            Ok(Command::ReplaceMedia(publication::MediaReplacement {
                encoding: publication::MediaEncoding::Wave,
                ..
            }))
        ));
        let mut invalid = args;
        invalid[3] = "wrong";
        assert!(
            Command::parse(invalid.into_iter().map(OsString::from)).is_err()
        );
        invalid = args;
        invalid[8] = "0";
        assert!(
            Command::parse(invalid.into_iter().map(OsString::from)).is_err()
        );
    }

    #[test]
    fn bundle_verification_requires_a_pinned_identity() {
        let args = ["mame", "check", "simpsons", "world-4p-set-1", "output"];
        assert!(Command::parse(args.into_iter().map(OsString::from)).is_err());
        let valid = args
            .into_iter()
            .map(OsString::from)
            .chain([OsString::from("1".repeat(64))]);
        assert!(matches!(Command::parse(valid), Ok(Command::Check(_, _, _))));
    }

    #[test]
    fn publication_requires_explicit_output_and_source() {
        let publish =
            ["mame", "publish", "simpsons", "world-4p-set-1", "output"];
        assert!(
            Command::parse(publish.into_iter().map(OsString::from)).is_err()
        );
    }

    #[test]
    fn palette_check_requires_profile_and_two_files() {
        let args = [
            "mame",
            "palette-check",
            "simpsons",
            "world-4p-set-1",
            "raw.bin",
            "pens.bin",
        ];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::PaletteCheck(_, _, _))
        ));
        assert!(
            Command::parse(args.into_iter().take(5).map(OsString::from))
                .is_err()
        );
        assert!(
            Command::parse(
                args.into_iter().chain(["extra"]).map(OsString::from)
            )
            .is_err()
        );
    }

    #[test]
    fn universal_import_selects_format_and_family() {
        let mut args = [
            "import",
            "neo",
            "graphics",
            "metal-slug",
            "mvs-aes-neo-v1",
            "cartridge.neo",
            "27529216",
            "16777216",
            "27525120",
            "5",
        ];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::Import(profile, _))
                if profile.format() == SourceFormat::Neo
                    && profile.family() == AssetFamily::Graphics
        ));
        args[2] = "audio";
        assert!(Command::parse(args.into_iter().map(OsString::from)).is_err());
    }

    #[test]
    fn cartridge_import_requires_known_graphics_profile() {
        let mut args = [
            "neo",
            "import",
            "metal-slug",
            "mvs-aes-neo-v1",
            "cartridge.neo",
            "27529216",
            "16777216",
            "27525120",
            "5",
        ];
        let expected_format = SourceFormat::Neo;
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::Import(profile, _))
                if profile.format() == expected_format
        ));
        args[1] = "audio-import";
        assert!(Command::parse(args.into_iter().map(OsString::from)).is_err());
        args[1] = "import";
        args[3] = "unknown";
        assert!(Command::parse(args.into_iter().map(OsString::from)).is_err());
    }

    #[test]
    fn cartridge_verification_requires_explicit_identity_and_limits() {
        let args = [
            "neo",
            "verify",
            "cartridge.neo",
            "synthetic",
            "v1",
            "8192",
            "1024",
            "2048",
            "6",
        ];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::Verify(Verification::Neo, _))
        ));
        assert!(
            Command::parse(args.into_iter().take(8).map(OsString::from))
                .is_err()
        );
    }

    #[test]
    fn mame_admission_selects_a_pinned_profile() {
        let args = [
            "mame",
            "import",
            "simpsons",
            "world-4p-set-1",
            "archive.zip",
            "3000000",
            "1048576",
            "7209088",
            "14",
        ];
        let command = Command::parse(args.into_iter().map(OsString::from));
        let expected = AssetFamily::Graphics;
        assert!(matches!(
            command,
            Ok(Command::Import(profile, _)) if profile.family() == expected
        ));
        let mut audio = args;
        audio[1] = "audio-import";
        let audio_family = AssetFamily::Audio;
        assert!(matches!(
            Command::parse(audio.into_iter().map(OsString::from)),
            Ok(Command::Import(profile, _)) if profile.family() == audio_family
        ));
        let wrong = [
            "mame",
            "import",
            "simpsons",
            "wrong",
            "archive.zip",
            "3000000",
            "1048576",
            "7209088",
            "14",
        ];
        assert!(Command::parse(wrong.into_iter().map(OsString::from)).is_err());
    }
}
