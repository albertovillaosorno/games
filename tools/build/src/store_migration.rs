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
//   - Bounded legacy store audits and explicit catalogue translation.
// - Must-Not:
//   - Publish assets, overwrite catalogues or open native storage directly.
// - Allows:
//   - Inputs: Title, legacy root, explicit limits and catalogue selectors.
//   - Outputs: Verified mappings, content counts or translated catalogue text.
//   - Side effects: Bounded reads and explicit stdout output.
// - Split-When:
//   - Production cutover requires a separate consumer workflow.
// - Merge-When:
//   - A common command already provides the same migration summary.
// - Summary:
//   - Prepare consumer references without creating a second store.
// - Description:
//   - Use the platform's bounded canonical inventory audit.
// - Usage:
//   - Select an existing legacy store and explicit resource limits.
// - Defaults:
//   - Read only; never remove or publish source assets.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::{NonZeroU64, NonZeroUsize};
use std::str::from_utf8;

use games_assets::{ContentHash, SourceIdentity};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::staging::migration::{
    LegacyStore, MigrationLimits, MigrationReport,
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

    fn report(&self) -> Result<MigrationReport, String> {
        LegacyStore {
            limits: self.limits,
            path: &self.parent,
            title: &self.title,
        }
        .audit()
        .map_err(|error| format!("migration audit: {error:?}"))
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let report = self.report()?;
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

#[derive(Debug)]
enum CatalogueFormat {
    Fields(BTreeSet<String>),
    LastToken,
}

impl CatalogueFormat {
    fn parse(value: &str) -> Result<Self, String> {
        if value == "last-token" {
            return Ok(Self::LastToken);
        }
        let list = value
            .strip_prefix("fields:")
            .ok_or("expected last-token or fields:KEY,...")?;
        let mut fields = BTreeSet::new();
        for field in list.split(',') {
            if field.is_empty()
                || !field.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'_'
                        || byte == b'-'
                })
                || !fields.insert(field.to_owned())
            {
                return Err(String::from("invalid catalogue field selection"));
            }
        }
        Ok(Self::Fields(fields))
    }

    fn selected<'line>(
        &self,
        line: &'line str,
        seen: &mut BTreeSet<&'line str>,
    ) -> Result<Option<&'line str>, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        match self {
            Self::Fields(fields) => {
                let Some((key, value)) = trimmed.split_once('=') else {
                    return Ok(None);
                };
                if !fields.contains(key) {
                    return Ok(None);
                }
                if !seen.insert(key) {
                    return Err(String::from("duplicate selected field"));
                }
                Ok(Some(value))
            },
            Self::LastToken => trimmed
                .rsplit_once(char::is_whitespace)
                .map(|(_, value)| Some(value))
                .ok_or_else(|| String::from("expected final manifest token")),
        }
    }
}

/// Explicit, read-only catalogue transformation from a freshly verified map.
#[derive(Debug)]
pub(crate) struct Catalogue {
    audit: Audit,
    format: CatalogueFormat,
    limit: NonZeroU64,
    path: OsString,
}

impl Catalogue {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [
            title,
            parent,
            entries,
            source,
            metadata,
            payload,
            total,
            count,
            path,
            limit,
            format,
        ] = <[OsString; 11]>::try_from(args)
            .map_err(|_args| String::from(USAGE))?;
        Ok(Self {
            audit: Audit::parse(vec![
                title, parent, entries, source, metadata, payload, total, count,
            ])?,
            format: CatalogueFormat::parse(text(&format)?)?,
            limit: parse_limit(&limit)?,
            path,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let bytes = snapshot(&self.path, self.limit)
            .map_err(|error| format!("catalogue: {error:?}"))?;
        let input = from_utf8(&bytes).map_err(|error| error.to_string())?;
        let report = self.audit.report()?;
        let output = rewrite(input, &self.format, &report.manifests)?;
        // The process port emits a final newline.
        let line = output.strip_suffix('\n').unwrap_or(&output);
        toolchain::write_line(line).map_err(|error| error.to_string())
    }
}

fn rewrite(
    input: &str,
    format: &CatalogueFormat,
    manifests: &BTreeMap<ContentHash, ContentHash>,
) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    let mut seen = BTreeSet::new();
    let mut selected = false;
    for line in input.split_inclusive('\n') {
        let Some(value) = format.selected(line, &mut seen)? else {
            output.push_str(line);
            continue;
        };
        let old = value
            .parse::<ContentHash>()
            .map_err(|_error| String::from("invalid manifest address"))?;
        let current = manifests
            .get(&old)
            .ok_or("manifest address is absent from verified migration")?;
        let record = line.trim_end();
        let prefix = record
            .strip_suffix(value)
            .ok_or("invalid manifest field boundary")?;
        let suffix = line
            .strip_prefix(record)
            .ok_or("invalid catalogue line boundary")?;
        output.push_str(prefix);
        output.push_str(&current.to_string());
        output.push_str(suffix);
        selected = true;
    }
    if !selected {
        return Err(String::from("no selected manifest addresses"));
    }
    Ok(output)
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

    #[test]
    fn catalogue_preserves_unselected_fields_and_whitespace() {
        let old = ContentHash::calculate(b"old");
        let current = ContentHash::calculate(b"current");
        let map = BTreeMap::from([(old, current), (current, current)]);
        let format = CatalogueFormat::Fields(BTreeSet::from([
            String::from("hero"),
            String::from("audio"),
            String::from("missing"),
        ]));
        let input = format!(
            "version=1\r\n  hero={old} \r\n\naudio={current}\nlabel={old}\n"
        );
        let expected = format!(
            "version=1\r\n  hero={current} \r\n\naudio={current}\nlabel={old}\n"
        );
        assert_eq!(rewrite(&input, &format, &map), Ok(expected));
    }

    #[test]
    fn catalogue_rejects_missing_malformed_duplicate_or_no_selection() {
        let old = ContentHash::calculate(b"old");
        let map = BTreeMap::from([(old, ContentHash::calculate(b"new"))]);
        let format =
            CatalogueFormat::Fields(BTreeSet::from([String::from("hero")]));
        let missing = ContentHash::calculate(b"missing");
        for invalid in [
            format!("hero={old}\nhero={missing}"),
            format!("hero={old}\nhero={old}"),
            String::from("hero=invalid"),
            format!("unselected={old}"),
            format!("hero={missing}"),
        ] {
            assert!(rewrite(&invalid, &format, &map).is_err());
        }
        for invalid in ["fields:", "fields:hero,hero", "fields: hero", "other"]
        {
            assert!(CatalogueFormat::parse(invalid).is_err());
        }
    }

    #[test]
    fn token_catalogue_changes_only_final_addresses() {
        let old = ContentHash::calculate(b"old");
        let current = ContentHash::calculate(b"current");
        let map = BTreeMap::from([(old, current)]);
        let format = CatalogueFormat::LastToken;
        let input = format!("  hero walk 0\t{old} \r\n\n{old} attack 2 {old}");
        let expected =
            format!("  hero walk 0\t{current} \r\n\n{old} attack 2 {current}");
        assert_eq!(rewrite(&input, &format, &map), Ok(expected));
        assert!(
            rewrite(&format!("hero walk 0 {current}"), &format, &map).is_err()
        );
        assert!(rewrite(&old.to_string(), &format, &map).is_err());
    }
}
