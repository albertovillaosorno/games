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
//   - Authored consumer migration rehearsal and configured publication.
// - Must-Not:
//   - Delete assets, execute originals or alter source catalogues.
// - Allows:
//   - Inputs: Explicit source, bounded templates and staging budgets.
//   - Outputs: Authored title smoke outcome and publication receipt.
//   - Side effects: Private staging and optional no-replace publication.
// - Split-When:
//   - A title requires different consumer compatibility evidence.
// - Merge-When:
//   - A common migration workflow owns the same smoke lifecycle.
// - Summary:
//   - Verify an authored consumer before publishing migrated content.
// - Description:
//   - Keep translated relative catalogues with the verified candidate.
// - Usage:
//   - Select a title, template, optional sidecar and explicit bounds.
// - Defaults:
//   - No persistent publication, settings mutation or source changes.

//! Authored migration consumer rehearsal and optional publication.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
use std::str::from_utf8;

use games_assets::archive::{ArchiveBudget, EntryKind};
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::layout::Layout;
use games_platform::ir::storage::staging::migration::{
    LegacyStore, MigrationFile, MigrationMetadata, MigrationReport,
    StagedMigration,
};
use games_platform::ir::storage::{
    PathKind, canonical_path, join_path, parent_path, path_has_prefix,
    path_is_absolute, path_kind, path_leaf,
};
use games_platform::ir::toolchain;

use super::{Catalogue, CatalogueFormat, USAGE, parse_limit, rewrite, text};
use crate::title_build::{Title, local_layout, workspace_root};

const LAUNCH: &str = "launch.local";
const POSES: &str = "launch-poses.txt";

#[derive(Clone, Copy, Debug)]
enum ConfiguredAction {
    Publish,
    Verify,
}

#[derive(Debug)]
enum Output {
    Configured(NonZeroU64, ConfiguredAction),
    Private(OsString),
}

#[derive(Debug)]
struct Sidecar {
    limit: NonZeroU64,
    path: OsString,
}

/// Selected authored title with explicit store and consumer catalogue bounds.
#[derive(Debug)]
pub(crate) struct Rehearsal {
    budget: ArchiveBudget,
    catalogue: Catalogue,
    output: Output,
    sidecar: Option<Sidecar>,
}

impl Rehearsal {
    fn metadata(&self, report: &MigrationReport) -> Result<Files, String> {
        let mut files = Files::new(self.budget);
        let bytes = read(&self.catalogue.path, self.catalogue.limit)?;
        let input = from_utf8(&bytes).map_err(|error| error.to_string())?;
        let translated =
            rewrite(input, &self.catalogue.format, &report.manifests)?;
        let launch = relocate(&translated, self.sidecar.is_some())?;
        files.add(LAUNCH, launch)?;
        if let Some(sidecar) = &self.sidecar {
            files.add(
                POSES,
                translate_sidecar(&sidecar.path, sidecar.limit, report)?,
            )?;
        }
        if let Output::Configured(limit, _) = self.output {
            for name in &report.sidecars {
                let filename =
                    name.to_str().ok_or("invalid sidecar filename")?;
                let path = join_path(&self.catalogue.audit.parent, name)
                    .map_err(|error| error.to_string())?;
                files
                    .add(filename, translate_sidecar(&path, limit, report)?)?;
            }
        }
        Ok(files)
    }

    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        Self::parse_inner(args, None)
    }

    fn parse_inner(
        args: Vec<OsString>,
        action: Option<ConfiguredAction>,
    ) -> Result<Self, String> {
        let mut values = args.into_iter();
        let catalogue = Catalogue::parse(values.by_ref().take(11).collect())?;
        if !matches!(catalogue.format, CatalogueFormat::Fields(_)) {
            return Err(String::from("launch requires fields:KEY,..."));
        }
        let parent = if action.is_some() {
            None
        } else {
            Some(values.next().ok_or_else(|| String::from(USAGE))?)
        };
        let [entry, total, count] = <[OsString; 3]>::try_from(
            values.by_ref().take(3).collect::<Vec<_>>(),
        )
        .map_err(|_args| String::from(USAGE))?;
        let output = if let Some(path) = parent {
            Output::Private(path)
        } else {
            let schema = values.next().ok_or_else(|| String::from(USAGE))?;
            if text(&schema)? != "last-token" {
                return Err(String::from("legacy sidecars require last-token"));
            }
            let limit = values.next().ok_or_else(|| String::from(USAGE))?;
            Output::Configured(
                parse_limit(&limit)?,
                action.ok_or("missing configured action")?,
            )
        };
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
            output,
            sidecar,
        })
    }

    pub(crate) fn parse_publish(args: Vec<OsString>) -> Result<Self, String> {
        Self::parse_inner(args, Some(ConfiguredAction::Publish))
    }

    pub(crate) fn parse_verify(args: Vec<OsString>) -> Result<Self, String> {
        Self::parse_inner(args, Some(ConfiguredAction::Verify))
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let audit = &self.catalogue.audit;
        let title = Title::parse(&audit.title, &[])?;
        let workspace = workspace_root()?;
        let destination = match &self.output {
            Output::Configured(_, action) => Some(configured_destination(
                &local_layout(&workspace)?,
                title,
                &audit.parent,
                *action,
            )?),
            Output::Private(_) => None,
        };
        let parent = match (&self.output, &destination) {
            (Output::Private(path), None) => path.as_os_str(),
            (Output::Configured(..), Some(path)) => {
                parent_path(path).map_err(|error| error.to_string())?
            },
            _ => return Err(String::from("missing migration destination")),
        };
        let source = LegacyStore {
            limits: audit.limits,
            path: &audit.parent,
            title: &audit.title,
        };
        let report = source
            .audit()
            .map_err(|error| format!("migration audit: {error:?}"))?;
        let files = self.metadata(&report)?;
        let staged = files.borrowed();
        let metadata = MigrationMetadata {
            expected: &report,
            files: &staged,
        };
        if matches!(
            self.output,
            Output::Configured(_, ConfiguredAction::Verify)
        ) {
            let path = destination
                .as_deref()
                .ok_or("missing verification destination")?;
            return verify_published(metadata, path, source);
        }
        let store = StagedMigration::prepare_with_metadata(
            source,
            parent,
            self.budget,
            metadata,
        )
        .map_err(|error| format!("migration stage: {error:?}"))?;
        smoke(title, &workspace, store.path())?;
        store
            .verify()
            .map_err(|error| format!("stage: {error:?}"))?;
        if let Some(path) = destination {
            publish(store, &path, title, &workspace)?;
            verify_published(metadata, &path, source)?;
        }
        toolchain::write_line("Verified migration consumer smoke.")
            .map_err(|error| error.to_string())
    }
}

struct Files {
    budget: ArchiveBudget,
    values: Vec<(String, String)>,
}

impl Files {
    fn add(&mut self, name: &str, content: String) -> Result<(), String> {
        let size =
            u64::try_from(content.len()).map_err(|error| error.to_string())?;
        let mut entry = self
            .budget
            .admit(name, EntryKind::File, size)
            .map_err(|error| format!("migration metadata: {error:?}"))?;
        entry
            .record(content.len())
            .map_err(|error| format!("migration metadata: {error:?}"))?;
        entry
            .finish()
            .map_err(|error| format!("migration metadata: {error:?}"))?;
        self.values.push((name.to_owned(), content));
        Ok(())
    }

    fn borrowed(&self) -> Vec<MigrationFile<'_>> {
        self.values
            .iter()
            .map(|(name, content)| MigrationFile {
                bytes: content.as_bytes(),
                name,
            })
            .collect()
    }

    const fn new(budget: ArchiveBudget) -> Self {
        Self {
            budget,
            values: Vec::new(),
        }
    }
}

fn configured_destination(
    layout: &Layout,
    title: Title,
    source: &OsStr,
    action: ConfiguredAction,
) -> Result<OsString, String> {
    let assets = OsString::from(layout.assets(title.binary())?);
    let output = OsString::from(layout.binaries(title.binary())?);
    for path in [&assets, &output] {
        if !path_is_absolute(path).map_err(|error| error.to_string())? {
            return Err(String::from("persistent roots must be absolute"));
        }
    }
    reject_overlap(&assets, &output)?;
    let kind = path_kind(&assets).map_err(|error| error.to_string())?;
    match (action, kind) {
        (ConfiguredAction::Publish, PathKind::Missing)
        | (ConfiguredAction::Verify, PathKind::Directory) => {},
        (ConfiguredAction::Publish, _) => {
            return Err(String::from("asset destination already exists"));
        },
        (ConfiguredAction::Verify, _) => {
            return Err(String::from("asset destination is not a directory"));
        },
    }
    let parent = parent_path(&assets).map_err(|error| error.to_string())?;
    if path_kind(parent).map_err(|error| error.to_string())?
        != PathKind::Directory
    {
        return Err(String::from(
            "create the configured title directory first",
        ));
    }
    let canonical_parent =
        canonical_path(parent).map_err(|error| error.to_string())?;
    let leaf = path_leaf(&assets).map_err(|error| error.to_string())?;
    let destination = join_path(&canonical_parent, leaf)
        .map_err(|error| error.to_string())?;
    let canonical_source =
        canonical_path(source).map_err(|error| error.to_string())?;
    reject_overlap(&destination, &canonical_source)?;
    reject_overlap(&destination, &future_path(&output)?)?;
    Ok(destination)
}

fn future_path(path: &OsStr) -> Result<OsString, String> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while path_kind(ancestor).map_err(|error| error.to_string())?
        == PathKind::Missing
    {
        suffix.push(path_leaf(ancestor).map_err(|error| error.to_string())?);
        ancestor = parent_path(ancestor).map_err(|error| error.to_string())?;
    }
    let mut resolved =
        canonical_path(ancestor).map_err(|error| error.to_string())?;
    if path_kind(&resolved).map_err(|error| error.to_string())?
        != PathKind::Directory
    {
        return Err(String::from("invalid configured binary directory"));
    }
    for child in suffix.into_iter().rev() {
        resolved =
            join_path(&resolved, child).map_err(|error| error.to_string())?;
    }
    Ok(resolved)
}

fn publish(
    store: StagedMigration,
    destination: &OsStr,
    title: Title,
    workspace: &OsStr,
) -> Result<(), String> {
    let name = path_leaf(destination).map_err(|error| error.to_string())?;
    let receipt = store
        .publish(name)
        .map_err(|error| format!("migration publication: {error:?}"))?;
    let location = receipt.path.to_string_lossy();
    if let Err(error) = receipt.durability {
        return Err(format!(
            "migration visible at {location}; sync failed: {error:?}"
        ));
    }
    smoke(title, workspace, &receipt.path).map_err(|error| {
        format!("migration visible at {location}; relocated smoke: {error}")
    })?;
    toolchain::write_line(&format!("Published assets: {location}"))
        .map_err(|error| error.to_string())
}

fn read(path: &OsStr, limit: NonZeroU64) -> Result<Vec<u8>, String> {
    snapshot(path, limit).map_err(|error| format!("template: {error:?}"))
}

fn reject_overlap(left: &OsStr, right: &OsStr) -> Result<(), String> {
    if path_has_prefix(left, right).map_err(|error| error.to_string())?
        || path_has_prefix(right, left).map_err(|error| error.to_string())?
    {
        return Err(String::from("migration paths overlap"));
    }
    Ok(())
}

fn relocate(input: &str, sidecar: bool) -> Result<String, String> {
    let mut parent_seen = false;
    let mut pose_seen = false;
    let mut output = String::with_capacity(input.len());
    for line in input.lines() {
        match line.trim().split_once('=') {
            Some(("parent", "@store")) if !parent_seen => {
                output.push_str("parent=.");
                parent_seen = true;
            },
            Some(("poses", "@sidecar")) if sidecar && !pose_seen => {
                output.push_str("poses=");
                output.push_str(POSES);
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

fn smoke(title: Title, workspace: &OsStr, store: &OsStr) -> Result<(), String> {
    let path = join_path(store, LAUNCH).map_err(|error| error.to_string())?;
    title
        .request()?
        .smoke(workspace, &path)
        .map_err(|error| error.to_string())
}

fn translate_sidecar(
    path: &OsStr,
    limit: NonZeroU64,
    report: &MigrationReport,
) -> Result<String, String> {
    let bytes = read(path, limit)?;
    let input = from_utf8(&bytes).map_err(|error| error.to_string())?;
    rewrite(input, &CatalogueFormat::LastToken, &report.manifests)
}

fn verify_published(
    metadata: MigrationMetadata<'_>,
    path: &OsStr,
    source: LegacyStore<'_>,
) -> Result<(), String> {
    metadata
        .verify_published(path, source.limits.manifest)
        .map_err(|error| {
            format!("published migration verification: {error:?}")
        })?;
    let report = metadata.expected;
    toolchain::write_line(&format!(
        "Verified published migration: {} legacy manifests, {} objects, \
         {} object bytes, {} catalogue files.",
        report.manifests.len(),
        report.objects,
        report.object_bytes,
        metadata.files.len(),
    ))
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use games_assets::ContentHash;
    use games_platform::ir::storage::fixtures::TestRoot;

    use super::*;

    const PREFIX: [&str; 11] = [
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
    ];

    #[test]
    fn configured_destination_rejects_existing_and_overlapping_roots()
    -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        for directory in ["legacy", "deploy/simpsons-arcade"] {
            root.create_directories(directory)
                .map_err(|error| error.to_string())?;
        }
        let path = |name: &str| {
            join_path(root.location(), name).map_err(|error| error.to_string())
        };
        let legacy = path("legacy")?;
        let deploy = path("deploy")?;
        let layout = Layout::parse(&format!(
            "GAMES_ASSET_ROOT={}\nGAMES_OUTPUT_ROOT={}\n\
             GAMES_TARGET=linux-x86-64\n",
            deploy.to_string_lossy(),
            deploy.to_string_lossy(),
        ))?;
        let title = Title::SimpsonsArcade;
        let destination = path("deploy/simpsons-arcade/assets")?;
        let resolve =
            |action| configured_destination(&layout, title, &legacy, action);
        require(resolve(ConfiguredAction::Publish)? == destination)?;
        require(
            configured_destination(
                &layout,
                title,
                &deploy,
                ConfiguredAction::Publish,
            )
            .is_err(),
        )?;
        require(resolve(ConfiguredAction::Verify).is_err())?;
        root.create_directories("deploy/simpsons-arcade/assets")
            .map_err(|error| error.to_string())?;
        require(resolve(ConfiguredAction::Publish).is_err())?;
        require(resolve(ConfiguredAction::Verify)? == destination)
    }

    #[test]
    fn publication_translates_every_audited_sidecar() -> Result<(), String> {
        let root = TestRoot::new().map_err(|error| error.to_string())?;
        root.create_directories("legacy")
            .map_err(|error| error.to_string())?;
        let old = ContentHash::calculate(b"old");
        let new = ContentHash::calculate(b"new");
        let pose = format!("hero walk 0 {old}\n");
        for name in ["legacy/poses.txt", "legacy/extra.txt", "active.txt"] {
            root.write_file(name, pose.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        root.write_file(
            "template",
            format!("parent=@store\nhero={old}\nposes=@sidecar\n").as_bytes(),
        )
        .map_err(|error| error.to_string())?;
        let path = |name: &str| {
            join_path(root.location(), name).map_err(|error| error.to_string())
        };
        let mut rehearsal = Rehearsal::parse_publish(
            PREFIX
                .into_iter()
                .chain([
                    "512",
                    "2048",
                    "16",
                    "last-token",
                    "4096",
                    "active",
                    "4096",
                ])
                .map(OsString::from)
                .collect(),
        )?;
        rehearsal.catalogue.audit.parent = path("legacy")?;
        rehearsal.catalogue.path = path("template")?;
        rehearsal
            .sidecar
            .as_mut()
            .ok_or("missing active pose")?
            .path = path("active.txt")?;
        let report = MigrationReport {
            manifests: [(old, new)].into(),
            sidecars: vec!["poses.txt".into(), "extra.txt".into()],
            ..MigrationReport::default()
        };
        let files = rehearsal.metadata(&report)?;
        let expected_launch = format!("parent=.\nhero={new}\nposes={POSES}\n");
        let expected_pose = format!("hero walk 0 {new}\n");
        require(
            files.values
                == [
                    (LAUNCH.into(), expected_launch),
                    (POSES.into(), expected_pose.clone()),
                    ("poses.txt".into(), expected_pose.clone()),
                    ("extra.txt".into(), expected_pose),
                ],
        )?;
        root.write_file("legacy/extra.txt", b"not a pose catalogue")
            .map_err(|error| error.to_string())?;
        require(rehearsal.metadata(&report).is_err())
    }

    fn require(condition: bool) -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(String::from("migration assertion failed"))
        }
    }

    #[test]
    fn rehearsal_admits_only_explicit_bounded_catalogues() {
        let parse = |values: &[&str]| {
            let args = PREFIX
                .into_iter()
                .chain(values.iter().copied())
                .map(OsString::from)
                .collect();
            Rehearsal::parse(args)
        };
        assert!(parse(&["stage", "512", "1024", "16"]).is_ok());
        assert!(
            parse(&["stage", "512", "1024", "16", "poses.txt", "4096"]).is_ok()
        );
        assert!(parse(&["stage", "512", "1024", "16", "poses.txt"]).is_err());
        for invalid in
            [["stage", "0", "1024", "16"], ["stage", "512", "0", "16"], [
                "stage", "512", "1024", "0",
            ]]
        {
            assert!(parse(&invalid).is_err());
        }
    }

    #[test]
    fn publication_requires_explicit_sidecar_schema_and_bounds() {
        let parse = |values: &[&str]| {
            let args = PREFIX
                .into_iter()
                .chain(values.iter().copied())
                .map(OsString::from)
                .collect();
            Rehearsal::parse_publish(args)
        };
        let valid = ["512", "1024", "16", "last-token", "4096"];
        assert!(parse(&valid).is_ok());
        assert!(
            Rehearsal::parse_verify(
                PREFIX
                    .into_iter()
                    .chain(valid)
                    .map(OsString::from)
                    .collect()
            )
            .is_ok()
        );
        let mut optional = valid.to_vec();
        optional.extend(["poses.txt", "4096"]);
        assert!(parse(&optional).is_ok());
        for invalid in [
            ["512", "1024", "16", "fields:hero", "4096"],
            ["512", "1024", "16", "last-token", "0"],
            ["512", "1024", "16", "last-token", "bad"],
        ] {
            assert!(parse(&invalid).is_err());
        }
    }

    #[test]
    fn relocation_admits_only_explicit_stage_paths() {
        let input = "version=1\nparent=@store\nposes=@sidecar\n";
        assert_eq!(
            relocate(input, true),
            Ok(String::from(
                "version=1\nparent=.\nposes=launch-poses.txt\n"
            ))
        );
        assert_eq!(
            relocate("parent=@store", false),
            Ok(String::from("parent=.\n"))
        );
        for invalid in [
            "parent=/old",
            "parent=@store\nparent=@store",
            "parent=@store\nposes=/old/poses.txt",
            "parent=@store\nsettings=owner-settings",
            "parent=@store\ncontrols=external-profile",
            "parent=@store\nposes=@sidecar",
        ] {
            assert!(relocate(invalid, false).is_err());
        }
        assert!(relocate("parent=@store", true).is_err());
    }
}
