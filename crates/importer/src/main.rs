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
//   - Local read-only source verification command and bounded input loading.
// - Must-Not:
//   - Execute original files, publish assets or infer source revisions.
// - Allows:
//   - Inputs: Local archive path, expected identity and explicit limits.
//   - Outputs: Verified source identity, member count and decoded size.
//   - Side effects: Reads one local file and writes a summary to stdout.
// - Split-When:
//   - A separate host tool composes import and asset publication.
// - Merge-When:
//   - The universal build command provides equivalent source verification.
// - Summary:
//   - Verify a selected original archive without extracting its contents.
// - Description:
//   - Read a bounded immutable snapshot and report decoded member totals.
// - Usage:
//   - cargo run --locked -p games-importer -- PATH TITLE REV SHA LIMITS
// - Defaults:
//   - Require the exact expected digest and fail closed on invalid media.

//! Read-only verification of a locally supplied, selected original archive.

use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write as _, stdout};
use std::num::NonZeroU64;
use std::path::PathBuf;

use games_assets::SourceIdentity;
use games_assets::archive::ArchiveBudget;
use games_importer::{ImportError, VerifiedSource};

const USAGE: &str =
    "usage: games-importer PATH TITLE REV SHA256 SOURCE ENTRY TOTAL COUNT";

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

fn main() -> Result<(), String> {
    let args = Arguments::parse(env::args_os().skip(1).collect())?;
    let file = File::open(&args.path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > args.source_limit.get() {
        return Err(String::from("source is not a bounded regular file"));
    }
    let bytes = read_source(file, args.source_limit)?;
    let source = VerifiedSource::new(&bytes, &args.identity, args.source_limit)
        .map_err(|error| format!("source: {error:?}"))?;
    let mut count = 0u64;
    let mut total = 0u64;
    source
        .visit_flat_zip(args.archive, |_name, payload| {
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
}
