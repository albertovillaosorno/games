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
//   - Explicit bounded legacy store audit requests and summaries.
// - Must-Not:
//   - Migrate assets, change catalogues or open native storage directly.
// - Allows:
//   - Inputs: Local source title, legacy root and explicit limits.
//   - Outputs: Verified old-to-current addresses and content counts.
//   - Side effects: Read-only verification and stdout summaries.
// - Split-When:
//   - Production cutover requires a separate consumer workflow.
// - Merge-When:
//   - A common command already provides the same migration summary.
// - Summary:
//   - Report migration prerequisites without creating a second store.
// - Description:
//   - Use the platform's bounded canonical inventory audit.
// - Usage:
//   - Select an existing legacy store and explicit resource limits.
// - Defaults:
//   - Read only; never remove or publish source assets.

use std::ffi::OsString;
use std::num::NonZeroUsize;

use games_assets::SourceIdentity;
use games_platform::ir::storage::staging::migration::{
    LegacyStore, MigrationLimits,
};
use games_platform::ir::toolchain;

use super::{USAGE, parse_limit, parse_manifest_limits, text};

#[derive(Debug)]
/// An explicit read-only migration audit command.
pub(crate) struct Audit {
    limits: MigrationLimits,
    parent: OsString,
    title: String,
}

impl Audit {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [
            title_arg,
            parent,
            entry_arg,
            source,
            metadata,
            payload,
            total,
            count,
        ] = <[OsString; 8]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        let title = text(&title_arg)?.to_owned();
        SourceIdentity {
            title: title.clone(),
            revision: String::from("migration"),
        }
        .validate()
        .map_err(|_error| String::from("invalid title identifier"))?;
        let entries = text(&entry_arg)?
            .parse::<NonZeroUsize>()
            .map_err(|_error| String::from("invalid store entry limit"))?;
        Ok(Self {
            limits: MigrationLimits {
                entries,
                manifest: parse_manifest_limits([
                    metadata, payload, total, count,
                ])?,
                source_bytes: parse_limit(&source)?,
            },
            parent,
            title,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let report = LegacyStore {
            limits: self.limits,
            path: &self.parent,
            title: &self.title,
        }
        .audit()
        .map_err(|error| format!("migration audit: {error:?}"))?;
        let write = |line: &str| {
            toolchain::write_line(line).map_err(|error| error.to_string())
        };
        write(&format!("Title: {}", self.title))?;
        write(&format!("Legacy manifests: {}", report.manifests.len()))?;
        write(&format!("Verified source bytes: {}", report.source_bytes))?;
        write(&format!("Distinct objects: {}", report.objects))?;
        write(&format!("Distinct object bytes: {}", report.object_bytes))?;
        write(&format!(
            "Unmigrated regular sidecars: {}",
            report.sidecars.len()
        ))?;
        for (old, current) in report.manifests {
            write(&format!("{old}\t{current}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_requires_title_and_all_explicit_positive_limits() {
        let args = ["example", "source", "8", "64", "4096", "16", "64", "4"];
        assert!(Audit::parse(args.map(OsString::from).to_vec()).is_ok());
        for (index, replacement) in [
            (0, "../example"),
            (2, "0"),
            (3, "0"),
            (4, "0"),
            (5, "0"),
            (6, "0"),
            (7, "0"),
        ] {
            let mut invalid = args;
            if let Some(value) = invalid.get_mut(index) {
                *value = replacement;
            }
            assert!(
                Audit::parse(invalid.map(OsString::from).to_vec()).is_err()
            );
        }
        assert!(
            Audit::parse(args.iter().take(7).map(OsString::from).collect())
                .is_err()
        );
    }
}
