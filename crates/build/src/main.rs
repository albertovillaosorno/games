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

#[cfg(all(target_os = "linux", target_env = "gnu"))]
/// Native canonical bundle publication and verification.
pub mod publication;
#[cfg(any(target_os = "linux", target_os = "android"))]
/// Bounded read-only palette comparison against local observations.
pub mod reference_palette;
#[cfg(all(
    feature = "native-preview",
    target_os = "linux",
    target_env = "gnu"
))]
/// Bounded native diagnostic rendering of selected imported graphics.
pub mod render_check;

use std::env;
use std::ffi::OsString;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::fs::{self, File};
#[cfg(any(test, not(any(target_os = "linux", target_os = "android"))))]
use std::io::Read;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::path::PathBuf;

use games_assets::archive::ArchiveBudget;
use games_assets::{
    AssetManifest, ContentHash, ManifestLimits, SourceIdentity,
};
use games_importer::conversion::ConversionLimits;
use games_importer::mame::Machine;
use games_importer::{ImportError, VerifiedSource};
#[cfg(any(target_os = "linux", target_os = "android"))]
use games_platform_storage::file::snapshot;

const USAGE: &str = concat!(
    "usage: games-build zip verify PATH TITLE REV SHA256 ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "games-build mame verify|import MACHINE REV PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: mame publish MACHINE REV PARENT PATH ",
    "SOURCE ENTRY TOTAL COUNT\n",
    "GNU/Linux: mame check MACHINE REV PARENT MANIFEST_SHA256\n",
    "Linux/Android: mame palette-check MACHINE REV RAW ARGB32",
    "\n",
    "GNU/Linux (native-preview): mame render-check MACHINE REV ",
    "PARENT SHA256 KEY RAW FIRST COUNT COLUMNS PEN_START PEN_COUNT"
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
        let [path, title, revision, digest, source, entry, total, count] =
            <[OsString; 8]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let identity = SourceIdentity {
            digest: text(&digest)?
                .parse()
                .map_err(|_error| String::from("invalid lowercase SHA-256"))?,
            revision: text(&revision)?.to_owned(),
            title: text(&title)?.to_owned(),
        };
        identity
            .validate()
            .map_err(|_error| String::from("invalid title or revision"))?;
        Self::with_identity(identity, path, [source, entry, total, count])
    }

    fn parse_mame(
        machine: Machine,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        let [path, source, entry, total, count] =
            <[OsString; 5]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let identity = machine
            .identity()
            .map_err(|error| format!("profile: {error:?}"))?;
        Self::with_identity(identity, path, [source, entry, total, count])
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        let path_metadata = fs::symlink_metadata(&self.path)
            .map_err(|error| error.to_string())?;
        if !path_metadata.is_file() {
            return Err(String::from("source is not a regular file"));
        }
        let file = File::open(&self.path).map_err(|error| error.to_string())?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > self.source_limit.get() {
            return Err(String::from("source is not a bounded regular file"));
        }
        read_source(file, self.source_limit)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
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

#[derive(Debug)]
enum Command {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Check(Machine, PathBuf, ContentHash),
    Import(Machine, Arguments),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    PaletteCheck(Machine, PathBuf, PathBuf),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Publish(Machine, PathBuf, Arguments),
    #[cfg(all(
        feature = "native-preview",
        target_os = "linux",
        target_env = "gnu"
    ))]
    RenderCheck(Machine, render_check::Arguments),
    Verify(Arguments),
}

impl Command {
    fn parse<Args>(mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let format = args.next().ok_or_else(|| String::from(USAGE))?;
        let action = args.next().ok_or_else(|| String::from(USAGE))?;
        match (format.to_str(), action.to_str()) {
            (Some("zip"), Some("verify")) => {
                Ok(Self::Verify(Arguments::parse(args.collect())?))
            },
            (Some("mame"), Some(operation)) => {
                Self::parse_mame(operation, args)
            },
            _ => Err(String::from(USAGE)),
        }
    }

    fn parse_mame<Args>(action: &str, mut args: Args) -> Result<Self, String>
    where
        Args: Iterator<Item = OsString>,
    {
        let machine = args.next().ok_or_else(|| String::from(USAGE))?;
        let revision = args.next().ok_or_else(|| String::from(USAGE))?;
        let selected = Machine::select(text(&machine)?, text(&revision)?)
            .map_err(|error| format!("MAME profile: {error:?}"))?;
        match action {
            "verify" => Ok(Self::Verify(Arguments::parse_mame(
                selected,
                args.collect(),
            )?)),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            "palette-check" => {
                let [raw, reference] =
                    <[OsString; 2]>::try_from(args.collect::<Vec<_>>())
                        .map_err(|_args| String::from(USAGE))?;
                Ok(Self::PaletteCheck(selected, raw.into(), reference.into()))
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
            "import" => Ok(Self::Import(
                selected,
                Arguments::parse_mame(selected, args.collect())?,
            )),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "publish" => {
                let parent = args.next().ok_or_else(|| String::from(USAGE))?;
                Ok(Self::Publish(
                    selected,
                    PathBuf::from(parent),
                    Arguments::parse_mame(selected, args.collect())?,
                ))
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            "check" => {
                let [parent, digest_argument] =
                    <[OsString; 2]>::try_from(args.collect::<Vec<_>>())
                        .map_err(|_args| String::from(USAGE))?;
                let digest = text(&digest_argument)?
                    .parse()
                    .map_err(|_error| String::from("invalid manifest hash"))?;
                Ok(Self::Check(selected, PathBuf::from(parent), digest))
            },
            _ => Err(String::from(USAGE)),
        }
    }

    fn run(self) -> Result<(), String> {
        match self {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            Self::PaletteCheck(machine, raw, reference) => {
                reference_palette::check(machine, &raw, &reference)
            },
            #[cfg(all(
                feature = "native-preview",
                target_os = "linux",
                target_env = "gnu"
            ))]
            Self::RenderCheck(machine, args) => args.run(machine),
            Self::Verify(args) => with_source(&args, verify_archive),
            Self::Import(machine, args) => {
                with_source(&args, |source, archive| {
                    report_graphics(machine, source, archive)
                })
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Publish(machine, parent, args) => {
                with_source(&args, |source, archive| {
                    publication::publish(machine, &parent, source, archive)
                })
            },
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Check(machine, parent, digest) => {
                publication::check(machine, &parent, digest)
            },
        }
    }
}

fn text(value: &OsString) -> Result<&str, String> {
    value
        .to_str()
        .ok_or_else(|| String::from("expected UTF-8 argument"))
}

fn parse_limit(value: &OsString) -> Result<NonZeroU64, String> {
    text(value)?
        .parse()
        .map_err(|_error| String::from("expected positive byte or count limit"))
}

#[cfg(any(test, not(any(target_os = "linux", target_os = "android"))))]
fn read_source<R>(reader: R, limit: NonZeroU64) -> Result<Vec<u8>, String>
where
    R: Read,
{
    let mut bytes = Vec::new();
    let mut bounded = reader.take(limit.get().saturating_add(1));
    let _count = bounded
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit.get() {
        return Err(String::from("source exceeds input limit"));
    }
    Ok(bytes)
}

fn with_source<Run>(args: &Arguments, run: Run) -> Result<(), String>
where
    Run: FnOnce(&VerifiedSource<'_>, ArchiveBudget) -> Result<(), String>,
{
    let bytes = args.snapshot()?;
    let source = VerifiedSource::new(&bytes, &args.identity, args.source_limit)
        .map_err(|error| format!("source: {error:?}"))?;
    run(&source, args.archive)
}

fn main() -> Result<(), String> {
    Command::parse(env::args_os().skip(1))?.run()
}

fn verify_archive(
    source: &VerifiedSource<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let mut count = 0u64;
    let mut total = 0u64;
    source
        .visit_flat_zip(archive, |_name, payload| {
            count = count.checked_add(1).ok_or(ImportError::TooLarge)?;
            total = total
                .checked_add(
                    u64::try_from(payload.len())
                        .map_err(|_error| ImportError::TooLarge)?,
                )
                .ok_or(ImportError::TooLarge)?;
            Ok(())
        })
        .map_err(|error| format!("archive: {error:?}"))?;
    let identity = source.identity();
    writeln!(
        stdout().lock(),
        "Verified {} {} ({}): {count} members, {total} decoded bytes",
        identity.title,
        identity.revision,
        identity.digest,
    )
    .map_err(|error| error.to_string())
}

fn report_manifest(
    converted: &AssetManifest,
    limits: ManifestLimits,
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
    let encoded = converted
        .encode(limits)
        .map_err(|error| format!("manifest: {error:?}"))?;
    writeln!(
        output,
        "Canonical manifest: {}",
        ContentHash::calculate(&encoded)
    )
    .map_err(|error| error.to_string())
}

fn report_graphics(
    machine: Machine,
    source: &VerifiedSource<'_>,
    archive: ArchiveBudget,
) -> Result<(), String> {
    let manifest = machine.graphics_limits();
    let converted = machine
        .convert_graphics(
            source,
            ConversionLimits { archive, manifest },
            |_digest, _bytes| Ok(()),
        )
        .map_err(|error| format!("graphics: {error:?}"))?;
    report_manifest(&converted, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_rejects_growing_or_oversized_sources() {
        let limit = NonZeroU64::new(4).unwrap_or(NonZeroU64::MIN);
        assert_eq!(
            read_source(b"four".as_slice(), limit),
            Ok(b"four".to_vec())
        );
        assert!(read_source(b"fives".as_slice(), limit).is_err());
        assert_eq!(read_source(b"".as_slice(), limit), Ok(Vec::new()));
    }

    #[test]
    fn command_requires_all_explicit_limits_and_identity() {
        let mut args = [
            "archive.zip",
            "synthetic",
            "world-set-1",
            "abc",
            "100",
            "50",
            "200",
            "4",
        ]
        .map(OsString::from);
        assert!(Arguments::parse(args.to_vec()).is_err());
        if let Some(digest) = args.get_mut(3) {
            *digest = OsString::from("1".repeat(64));
        }
        assert!(Arguments::parse(args.to_vec()).is_ok());
        if let Some(count) = args.last_mut() {
            *count = OsString::from("0");
        }
        assert!(Arguments::parse(args.to_vec()).is_err());
        assert!(Arguments::parse(Vec::new()).is_err());
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
        assert!(matches!(command, Ok(Command::Import(_, _))));
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
