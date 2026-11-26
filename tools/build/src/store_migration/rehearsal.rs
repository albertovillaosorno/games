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
//   - Rehearsal composition for a verified private migration candidate.
// - Must-Not:
//   - Publish or delete assets, execute originals or alter source catalogues.
// - Allows:
//   - Inputs: Explicit legacy store, bounded templates and staging budgets.
//   - Outputs: Authored title smoke outcome and private-stage readback.
//   - Side effects: Temporary private staging and fixed Cargo title smoke.
// - Split-When:
//   - A title requires different consumer compatibility evidence.
// - Merge-When:
//   - A common migration workflow already owns the same smoke lifecycle.
// - Summary:
//   - Verify an authored consumer before retiring any legacy content.
// - Description:
//   - Keep the candidate store and its translated catalogue alive for smoke.
// - Usage:
//   - Select a title, template, optional sidecar and explicit bounds.
// - Defaults:
//   - No persistent publication, settings mutation or source changes.

//! Private migration consumer rehearsal.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
use std::str::from_utf8;

use games_assets::archive::ArchiveBudget;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::join_path;
use games_platform::ir::storage::staging::AuthoredStaging;
use games_platform::ir::storage::staging::migration::{
    LegacyStore, StagedMigration,
};
use games_platform::ir::toolchain;

use super::{Catalogue, CatalogueFormat, USAGE, parse_limit, rewrite};
use crate::title_build::{Title, workspace_root};

#[derive(Debug)]
struct Sidecar {
    limit: NonZeroU64,
    path: OsString,
}

/// Selected authored title with private store and consumer catalogue bounds.
#[derive(Debug)]
pub(crate) struct Rehearsal {
    budget: ArchiveBudget,
    catalogue: Catalogue,
    parent: OsString,
    sidecar: Option<Sidecar>,
}

impl Rehearsal {
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut values = args.into_iter();
        let catalogue = Catalogue::parse(values.by_ref().take(11).collect())?;
        if !matches!(catalogue.format, CatalogueFormat::Fields(_)) {
            return Err(String::from("launch requires fields:KEY,..."));
        }
        let [parent, entry, total, count] = <[OsString; 4]>::try_from(
            values.by_ref().take(4).collect::<Vec<_>>(),
        )
        .map_err(|_args| String::from(USAGE))?;
        let sidecar = match (values.next(), values.next(), values.next()) {
            (None, None, None) => None,
            (Some(path), Some(limit), None) => Some(Sidecar {
                limit: parse_limit(&limit)?,
                path,
            }),
            _ => return Err(String::from(USAGE)),
        };
        Ok(Self {
            budget: ArchiveBudget::new(
                parse_limit(&entry)?,
                parse_limit(&total)?,
                parse_limit(&count)?,
            ),
            catalogue,
            parent,
            sidecar,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let audit = &self.catalogue.audit;
        let title = Title::parse(&audit.title, &[])?;
        let workspace = workspace_root()?;
        let store = StagedMigration::prepare(
            LegacyStore {
                limits: audit.limits,
                path: &audit.parent,
                title: &audit.title,
            },
            &self.parent,
            self.budget,
        )
        .map_err(|error| format!("migration stage: {error:?}"))?;
        let mut config = AuthoredStaging::new(&self.parent)
            .map_err(|error| format!("catalogue stage: {error:?}"))?;
        let map = &store.report().manifests;
        let bytes = read(&self.catalogue.path, self.catalogue.limit)?;
        let input = from_utf8(&bytes).map_err(|error| error.to_string())?;
        let translated = rewrite(input, &self.catalogue.format, map)?;
        if let Some(sidecar) = &self.sidecar {
            let side_bytes = read(&sidecar.path, sidecar.limit)?;
            let side_text =
                from_utf8(&side_bytes).map_err(|error| error.to_string())?;
            let output = rewrite(side_text, &CatalogueFormat::LastToken, map)?;
            config
                .write_file("poses.txt", output.as_bytes(), false)
                .map_err(|error| format!("pose stage: {error:?}"))?;
        }
        let launch =
            relocate(&translated, store.path(), self.sidecar.is_some())?;
        config
            .write_file("launch.local", launch.as_bytes(), false)
            .map_err(|error| format!("launch stage: {error:?}"))?;
        let path = join_path(config.path(), "launch.local")
            .map_err(|error| error.to_string())?;
        store
            .verify()
            .map_err(|error| format!("stage: {error:?}"))?;
        title
            .request()?
            .smoke(&workspace, &path)
            .map_err(|error| error.to_string())?;
        store
            .verify()
            .map_err(|error| format!("stage: {error:?}"))?;
        toolchain::write_line("Verified private migration consumer smoke.")
            .map_err(|error| error.to_string())
    }
}

fn read(path: &OsStr, limit: NonZeroU64) -> Result<Vec<u8>, String> {
    snapshot(path, limit).map_err(|error| format!("template: {error:?}"))
}

fn relocate(
    input: &str,
    location: &OsStr,
    sidecar: bool,
) -> Result<String, String> {
    let store = location.to_str().ok_or("migration path is not UTF-8")?;
    if store.is_empty() || store.chars().any(char::is_control) {
        return Err(String::from("invalid migration path"));
    }
    let mut parent_seen = false;
    let mut pose_seen = false;
    let mut output = String::with_capacity(input.len());
    for line in input.lines() {
        match line.trim().split_once('=') {
            Some(("parent", "@store")) if !parent_seen => {
                output.push_str("parent=");
                output.push_str(store);
                parent_seen = true;
            },
            Some(("poses", "@sidecar")) if sidecar && !pose_seen => {
                output.push_str("poses=poses.txt");
                pose_seen = true;
            },
            Some(("parent" | "poses" | "settings" | "controls", _)) => {
                return Err(String::from("unsafe or ambiguous template path"));
            },
            _ => output.push_str(line),
        }
        output.push('\n');
    }
    if !parent_seen || pose_seen != sidecar {
        return Err(String::from("template requires explicit path markers"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The command requires separate source and candidate resource budgets.
    #[test]
    fn rehearsal_admits_only_explicit_bounded_catalogues() {
        let valid = [
            "simpsons-arcade",
            "source",
            "512",
            "1024",
            "4096",
            "512",
            "1024",
            "16",
            "template",
            "4096",
            "fields:hero",
            "stage",
            "512",
            "1024",
            "16",
        ];
        let parse = |values: &[&str]| {
            Rehearsal::parse(values.iter().map(OsString::from).collect())
        };
        assert!(parse(&valid).is_ok());
        let mut poses = valid.to_vec();
        poses.extend(["poses.txt", "4096"]);
        assert!(parse(&poses).is_ok());
        poses.push("extra");
        assert!(parse(&poses).is_err());
        for (index, replacement) in
            [(10, "last-token"), (12, "0"), (13, "0"), (14, "0")]
        {
            let mut invalid = valid;
            if let Some(value) = invalid.get_mut(index) {
                *value = replacement;
            }
            assert!(parse(&invalid).is_err());
        }
    }

    #[test]
    fn relocation_admits_only_explicit_stage_paths() {
        let store = OsStr::new("/private/store");
        let input = "version=1\nparent=@store\nposes=@sidecar\n";
        assert_eq!(
            relocate(input, store, true),
            Ok(String::from(
                "version=1\nparent=/private/store\nposes=poses.txt\n"
            ))
        );
        assert_eq!(
            relocate("parent=@store", store, false),
            Ok(String::from("parent=/private/store\n"))
        );
        for invalid in [
            "parent=/old",
            "parent=@store\nparent=@store",
            "parent=@store\nposes=/old/poses.txt",
            "parent=@store\nsettings=owner-settings",
            "parent=@store\ncontrols=external-profile",
            "parent=@store\nposes=@sidecar",
        ] {
            assert!(relocate(invalid, store, false).is_err());
        }
        assert!(relocate("parent=@store", store, true).is_err());
        assert!(
            relocate("parent=@store", OsStr::new("bad\npath"), false).is_err()
        );
    }
}
