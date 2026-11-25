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
#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Bounded canonical PCM consumption and native queue diagnostics.
pub mod audio_check;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
/// Native canonical bundle publication and verification.
pub mod publication;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Exact bounded comparison of private PCM reference recordings.
pub mod reference_audio;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
/// Bounded background-reference comparison.
pub mod reference_background;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Read-only CPU and device object buffer comparison.
pub mod reference_objects;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Bounded read-only palette comparison against local observations.
pub mod reference_palette;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Bounded read-only video mixer comparison against local observations.
pub mod reference_priority;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Bounded local tile RAM admission diagnostic.
pub mod reference_tilemap;
#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Finite native graphics benchmark measurements.
pub mod render_benchmark;
#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Bounded native diagnostic rendering of selected imported graphics.
pub mod render_check;

/// Compile one admitted independently authored title.
pub mod title_build;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
/// Package the selected authored native executable.
pub mod title_package;

use std::env;
use std::ffi::OsString;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::num::NonZeroUsize;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use games_assets::ManifestLimits;
use games_assets::archive::ArchiveBudget;
use games_assets::{AssetManifest, ContentHash, SourceIdentity};
use games_importer::conversion::ConversionLimits;
use games_importer::mame::Machine;
#[cfg(any(target_os = "linux", target_os = "android"))]
use games_importer::mame::samples::compare_observations;
use games_importer::neo::CartridgeGraphics;
use games_importer::selection::AssetSelection;
use games_importer::{ImportError, SourceSnapshot};
use games_platform_storage::snapshot;

const USAGE: &str = concat!(
    "usage: games-build build TITLE [linux-x86-64]\n",
    "GNU/Linux: games-build package TITLE OUTPUT\n",
    "GNU/Linux: games-build bundle upgrade PARENT MANIFEST_SHA256 ",
    "MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "GNU/Linux: games-build bundle replace-media PARENT BASE_SHA256 KEY ",
    "RAW MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "GNU/Linux: games-build bundle replace-wave PARENT BASE_SHA256 KEY ",
    "WAV MANIFEST_BYTES PAYLOAD_BYTES TOTAL_BYTES COUNT\n",
    "Linux/Android: games-build neo import TITLE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: neo publish TITLE REV PARENT PATH SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: neo check TITLE REV PARENT MANIFEST_SHA256\n",
    "Linux/Android: games-build zip|neo verify PATH TITLE REV ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "Linux/Android: games-build mame verify|import|audio-import ",
    "MACHINE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: mame publish|audio-publish MACHINE REV PARENT PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: mame check|audio-check MACHINE REV PARENT MANIFEST_SHA256\n",
    "GNU/Linux: mame art-publish MACHINE REV OUTPUT PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS KEY:INDEX,...\n",
    "GNU/Linux: mame strip-publish MACHINE REV OUTPUT PARENT SHA256 ",
    "TOP SIDE RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS ",
    "[MORE SCENES...]\n",
    "GNU/Linux: mame background-check MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32\n",
    "GNU/Linux: mame scene-check MACHINE REV PARENT SHA256 ",
    "RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS\n",
    "GNU/Linux (native-preview): mame scene-sequence-bench MACHINE REV ",
    "PARENT SHA256 REPEATS RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS ",
    "CONTROLS [MORE SCENES...]\n",
    "GNU/Linux (native-preview): mame scene-render-check MACHINE REV ",
    "PARENT SHA256 RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS\n",
    "Linux/Android: mame sample-check MACHINE REV OBSERVATIONS LIMIT ",
    "PATH SOURCE ENTRY TOTAL COUNT\n",
    "Linux/Android: mame object-check MACHINE REV CPU DEVICE\n",
    "Linux/Android: mame palette-check MACHINE REV RAW ARGB32",
    "\nLinux/Android: mame priority-check MACHINE REV RAW OBSERVED_LE32",
    "\nLinux/Android: mame tilemap-check MACHINE REV RAM REGISTERS",
    "\n",
    "GNU/Linux (native-preview): mame audio-render-check MACHINE REV ",
    "PARENT SHA256 LEFT_KEY RIGHT_KEY FRAMES\n",
    "GNU/Linux (native-preview): mame render-check MACHINE REV ",
    "PARENT SHA256 KEY RAW FIRST COUNT COLUMNS PEN_START PEN_COUNT",
    "\nGNU/Linux (native-preview): mame render-bench MACHINE REV ",
    "PARENT SHA256 KEY RAW FIRST COUNT COLUMNS PEN_START PEN_COUNT FRAMES",
    "\n",
    "Linux/Android: wave compare LEFT RIGHT LIMIT"
);

#[derive(Debug)]
struct Arguments {
    archive: ArchiveBudget,
    identity: SourceIdentity,
    path: PathBuf,
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
            path: PathBuf::from(path),
            source_limit: parse_limit(&source)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum Verification {
    Mame(Machine),
    Neo,
    Zip,
}

#[derive(Debug)]
enum Command {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    ArtPublish(Machine, reference_background::art::Art),
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    AudioCheck(Machine, audio_check::Arguments),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    BackgroundCheck(Machine, reference_background::Arguments),
    BuildTitle(title_build::Title),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Check(AssetSelection, PathBuf, ContentHash),
    Import(AssetSelection, Arguments),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    ObjectCheck(Machine, PathBuf, PathBuf),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    PackageTitle(title_build::Title, PathBuf),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    PaletteCheck(Machine, PathBuf, PathBuf),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    PriorityCheck(Machine, PathBuf, PathBuf),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Publish(AssetSelection, PathBuf, Arguments),
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    RenderCheck(Machine, render_check::Arguments),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    ReplaceMedia(publication::MediaReplacement),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    SampleCheck(Machine, PathBuf, NonZeroU64, Arguments),
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    SceneSequence(Machine, reference_background::sequence::Sequence),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    StripPublish(Machine, reference_background::art::Strip),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    TilemapCheck(Machine, PathBuf, PathBuf),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Upgrade(PathBuf, ContentHash, ManifestLimits),
    Verify(Verification, Arguments),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    WaveCompare(PathBuf, PathBuf, NonZeroU64),
}

impl Command {
    fn parse<Args>(mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let format = args.next().ok_or_else(|| String::from(USAGE))?;
        let action = args.next().ok_or_else(|| String::from(USAGE))?;
        match (format.to_str(), action.to_str()) {
            (Some("build"), Some(title)) => Ok(Self::BuildTitle(
                title_build::Title::parse(title, &args.collect::<Vec<_>>())?,
            )),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            (Some("package"), Some(title)) => {
                let [output] =
                    <[OsString; 1]>::try_from(args.collect::<Vec<_>>())
                        .map_err(|_args| String::from(USAGE))?;
                Ok(Self::PackageTitle(
                    title_build::Title::parse(title, &[])?,
                    output.into(),
                ))
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            (Some("bundle"), Some("upgrade")) => {
                Self::parse_upgrade(args.collect())
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            (Some("bundle"), Some("replace-media")) => {
                Self::parse_replace_media(
                    args.collect(),
                    publication::MediaEncoding::Raw,
                )
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            (Some("bundle"), Some("replace-wave")) => {
                Self::parse_replace_media(
                    args.collect(),
                    publication::MediaEncoding::Wave,
                )
            },
            (Some(kind @ ("zip" | "neo")), Some("verify")) => {
                Self::parse_verification(kind, args.collect())
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            (Some("wave"), Some("compare")) => {
                let [left, right, limit] =
                    <[OsString; 3]>::try_from(args.collect::<Vec<_>>())
                        .map_err(|_args| String::from(USAGE))?;
                Ok(Self::WaveCompare(
                    left.into(),
                    right.into(),
                    parse_limit(&limit)?,
                ))
            },
            (Some("neo"), Some(operation)) => Self::parse_neo(operation, args),
            (Some("mame"), Some(operation)) => {
                Self::parse_mame(operation, args)
            },
            _ => Err(String::from(USAGE)),
        }
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
        selection: AssetSelection,
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
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "publish" | "audio-publish" => {
                let parent = args.next().ok_or_else(|| String::from(USAGE))?;
                Ok(Self::Publish(
                    selection,
                    PathBuf::from(parent),
                    Arguments::parse_profile(
                        selection.identity(),
                        args.collect(),
                    )?,
                ))
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "check" | "audio-check" => {
                Self::parse_check(selection, args.collect())
            },
            _ => Err(String::from(USAGE)),
        }
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn parse_check(
        selection: AssetSelection,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [parent, digest_argument] = <[OsString; 2]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        let digest = text(&digest_argument)?
            .parse()
            .map_err(|_error| String::from("invalid manifest hash"))?;
        Ok(Self::Check(selection, parent.into(), digest))
    }

    fn parse_mame<Args>(action: &str, mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let selected = select_machine(&mut args)?;
        #[cfg(all(
            feature = "native-preview",
            target_os = "linux",
            target_env = "gnu"
        ))]
        if matches!(action, "audio-render-check" | "scene-sequence-bench") {
            return Self::parse_native_workload(
                action,
                selected,
                args.collect(),
            );
        }
        match action {
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "art-publish" | "strip-publish" => {
                Self::parse_art(action, selected, args.collect())
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "background-check" | "scene-check" | "scene-render-check" => {
                Ok(Self::BackgroundCheck(
                    selected,
                    reference_background::Arguments::parse_command(
                        action,
                        args.collect(),
                    )?,
                ))
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            "sample-check" => Self::parse_sample(selected, args),
            "verify" => Ok(Self::Verify(
                Verification::Mame(selected),
                Arguments::parse_profile(selected.identity(), args.collect())?,
            )),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            "object-check" | "palette-check" | "priority-check"
            | "tilemap-check" => {
                Self::parse_reference(action, selected, args.collect())
            },
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
            "render-check" => Ok(Self::RenderCheck(
                selected,
                render_check::Arguments::parse(args.collect())?,
            )),
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
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
        let selection = if action.starts_with("audio-") {
            AssetSelection::Audio(machine)
        } else {
            AssetSelection::Graphics(machine)
        };
        Self::parse_assets(action, selection, args)
    }

    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
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
        let profile =
            CartridgeGraphics::select(text(&title)?, text(&revision)?)
                .map_err(|error| format!("cartridge profile: {error:?}"))?;
        Self::parse_assets(action, AssetSelection::Cartridge(profile), args)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn parse_reference(
        action: &str,
        machine: Machine,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [raw, reference] = <[OsString; 2]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        match action {
            "object-check" => {
                Ok(Self::ObjectCheck(machine, raw.into(), reference.into()))
            },
            "palette-check" => {
                Ok(Self::PaletteCheck(machine, raw.into(), reference.into()))
            },
            "priority-check" => {
                Ok(Self::PriorityCheck(machine, raw.into(), reference.into()))
            },
            "tilemap-check" => {
                Ok(Self::TilemapCheck(machine, raw.into(), reference.into()))
            },
            _ => Err(String::from(USAGE)),
        }
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
            parent: parent.into(),
            source: source.into(),
        }))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
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
            raw.into(),
            parse_limit(&limit)?,
            Arguments::parse_profile(machine.identity(), args.collect())?,
        ))
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn parse_upgrade(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, raw_digest, metadata, payload, total, count] =
            <[OsString; 6]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self::Upgrade(
            parent.into(),
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

    fn run(self) -> Result<(), String> {
        match self {
            Self::BuildTitle(title) => title.run(),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::PackageTitle(title, output) => {
                title_package::run(title, &output)
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::ArtPublish(machine, args) => args.run(machine),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::StripPublish(machine, args) => args.run(machine),
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
            Self::AudioCheck(machine, args) => args.run(machine),
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
            Self::SceneSequence(machine, args) => args.run(machine),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::BackgroundCheck(machine, args) => args.run(machine),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::ObjectCheck(machine, cpu, device) => {
                reference_objects::check(machine, &cpu, &device)
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::PaletteCheck(machine, raw, reference) => {
                reference_palette::check(machine, &raw, &reference)
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::PriorityCheck(machine, raw, reference) => {
                reference_priority::check(machine, &raw, &reference)
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::TilemapCheck(machine, raw, registers) => {
                reference_tilemap::check(machine, &raw, &registers)
            },
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
            Self::RenderCheck(machine, args) => args.run(machine),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::WaveCompare(left, right, limit) => {
                reference_audio::check(&left, &right, limit)
            },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::SampleCheck(machine, raw, limit, args) => {
                report_samples(machine, &raw, limit, &args)
            },
            command => command.run_assets(),
        }
    }

    fn run_assets(self) -> Result<(), String> {
        match self {
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Upgrade(parent, digest, limits) => {
                publication::upgrade(&parent, digest, limits)
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Publish(selection, parent, args) => {
                with_source(&args, |source, archive| {
                    publication::publish(selection, &parent, source, archive)
                })
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Check(selection, parent, digest) => {
                publication::check(selection, &parent, digest)
            },
            _ => Err(String::from(USAGE)),
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

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn parse_manifest_digest(value: &OsString) -> Result<ContentHash, String> {
    text(value)?
        .parse()
        .map_err(|_error| String::from("invalid manifest hash"))
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
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
    Command::parse(env::args_os().skip(1))?.run()
}

fn verify_source(
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
    format: Verification,
) -> Result<(), String> {
    let mut count = 0u64;
    let mut total = 0u64;
    let visit = |_name: &str, payload: &[u8]| {
        count = count.checked_add(1).ok_or(ImportError::TooLarge)?;
        total = total
            .checked_add(
                u64::try_from(payload.len())
                    .map_err(|_error| ImportError::TooLarge)?,
            )
            .ok_or(ImportError::TooLarge)?;
        Ok(())
    };
    match format {
        Verification::Mame(selected) => {
            selected.visit_layout(source, archive, visit)
        },
        Verification::Neo => source.visit_neo(archive, visit),
        Verification::Zip => source.visit_flat_zip(archive, visit),
    }
    .map_err(|error| format!("archive: {error:?}"))?;
    let identity = source.identity();
    writeln!(
        stdout().lock(),
        "Admitted {} {}: {count} members, {total} decoded bytes",
        identity.title,
        identity.revision,
    )
    .map_err(|error| error.to_string())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn report_samples(
    machine: Machine,
    raw: &Path,
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
        writeln!(
            stdout().lock(),
            "{} frames: {} PCM, {} differential, {} mismatches",
            report.frames,
            report.pcm,
            report.differential,
            report.mismatches,
        )
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
    let mut output = stdout().lock();
    for record in converted.records() {
        writeln!(
            output,
            "{}: {} bytes ({})",
            record.key, record.size, record.digest
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(output, "Canonical manifest: {digest}")
        .map_err(|error| error.to_string())
}

fn report_assets(
    selection: AssetSelection,
    source: &SourceSnapshot<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let manifest = selection.limits();
    let converted = selection
        .convert(
            source,
            ConversionLimits { archive, manifest },
            |_digest, _bytes| Ok(()),
        )
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
            Ok(Command::BuildTitle(title_build::Title::SimpsonsArcade))
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

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn package_command_requires_admitted_title_and_output() {
        let args = ["package", "simpsons-arcade", "local-package"];
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::PackageTitle(title_build::Title::SimpsonsArcade, _))
        ));
        let invalid: [&[&str]; 3] = [
            &["package", "simpsons-arcade"],
            &["package", "metal-slug", "out"],
            &["package", "simpsons-arcade", "out", "extra"],
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

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
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

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
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

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn publication_modes_require_explicit_output_and_identity() {
        let args = ["mame", "check", "simpsons", "world-4p-set-1", "output"];
        assert!(Command::parse(args.into_iter().map(OsString::from)).is_err());
        let valid = args
            .into_iter()
            .map(OsString::from)
            .chain([OsString::from("1".repeat(64))]);
        assert!(matches!(Command::parse(valid), Ok(Command::Check(_, _, _))));
        let publish =
            ["mame", "publish", "simpsons", "world-4p-set-1", "output"];
        assert!(
            Command::parse(publish.into_iter().map(OsString::from)).is_err()
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "android"))]
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
        assert!(matches!(
            Command::parse(args.into_iter().map(OsString::from)),
            Ok(Command::Import(AssetSelection::Cartridge(_), _))
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
        assert!(matches!(
            command,
            Ok(Command::Import(AssetSelection::Graphics(_), _))
        ));
        let mut audio = args;
        audio[1] = "audio-import";
        assert!(matches!(
            Command::parse(audio.into_iter().map(OsString::from)),
            Ok(Command::Import(AssetSelection::Audio(_), _))
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
