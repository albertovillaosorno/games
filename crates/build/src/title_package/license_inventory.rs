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
//   - Selected native title dependency licence-text inventory.
// - Must-Not:
//   - Copy original media, resolve new dependencies or claim legal clearance.
// - Allows:
//   - Inputs: Locked Cargo metadata and cached dependency licence paths.
//   - Outputs: Selected dependency documents and declared licence inventory.
//   - Side effects: Reads metadata from the trusted local Cargo toolchain.
// - Split-When:
//   - A separate target needs different licence admission policies.
// - Merge-When:
//   - Another packaging tool resolves the same Cargo dependency inventory.
// - Summary:
//   - Collect upstream licence texts for a selected title build.
// - Description:
//   - Follow normal and build dependency edges, excluding development tests.
// - Usage:
//   - Prepare native packaging inputs before creating the output directory.
// - Defaults:
//   - Reject unreviewed licence expressions and missing required texts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::title_build::{NATIVE_TARGET, PREVIEW_FEATURE, Title};

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";
const SDL_VERSION: &str = "0.39.0";
const DOCUMENTS: [&str; 11] = [
    "COPYING",
    "COPYRIGHT",
    "LICENSE",
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "LICENSE-UNICODE",
    "LICENSE.txt",
    "NOTICE",
    "NOTICE.md",
    "NOTICES.md",
    "UNLICENSE",
];

type Documents = BTreeMap<String, PathBuf>;

#[derive(Debug)]
pub(super) struct Inventory {
    pub(super) documents: BTreeMap<PathBuf, PathBuf>,
    pub(super) report: String,
}

impl Inventory {
    fn insert_documents(
        &mut self,
        package: &Package,
        selected: &[&Package],
    ) -> Result<(), String> {
        let parent = PathBuf::from("licenses").join(package.label()?);
        for (name, source) in package.documents(selected)? {
            if self.documents.insert(parent.join(name), source).is_some() {
                return Err(String::from("duplicate licence document"));
            }
        }
        Ok(())
    }

    pub(super) fn load(root: &Path, title: Title) -> Result<Self, String> {
        let manifest = root
            .join("crates")
            .join(format!("game-{}", title.binary()))
            .join("Cargo.toml");
        let output = Command::new("cargo")
            .current_dir(root)
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--filter-platform",
                NATIVE_TARGET,
                "--features",
                PREVIEW_FEATURE,
                "--manifest-path",
            ])
            .arg(&manifest)
            .output()
            .map_err(|error| format!("Cargo metadata: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let metadata: Metadata = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Cargo metadata JSON: {error}"))?;
        metadata.inventory(root, &manifest)
    }
}

#[derive(Debug, Deserialize)]
struct Dependency {
    dep_kinds: Vec<DependencyKind>,
    pkg: String,
}

impl Dependency {
    fn included(&self) -> bool {
        self.dep_kinds
            .iter()
            .any(|kind| matches!(kind.kind.as_deref(), None | Some("build")))
    }
}

#[derive(Debug, Deserialize)]
struct DependencyKind {
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Resolve,
    workspace_members: BTreeSet<String>,
}

impl Metadata {
    fn inventory(
        &self,
        root: &Path,
        manifest: &Path,
    ) -> Result<Inventory, String> {
        let selected = self.selected()?;
        let root_package = selected
            .iter()
            .find(|package| package.id == self.resolve.root)
            .ok_or_else(|| String::from("missing selected title"))?;
        if root_package.manifest_path != manifest {
            return Err(String::from("unexpected selected Cargo manifest"));
        }
        let mut inventory = Inventory {
            documents: BTreeMap::new(),
            report: String::from("package\tversion\tdeclared licence\n"),
        };
        for package in &selected {
            let licence = package
                .license
                .as_deref()
                .ok_or_else(|| format!("missing licence: {}", package.name))?;
            let local = self.workspace_members.contains(&package.id);
            package.validate(root, local)?;
            writeln!(
                inventory.report,
                "{}\t{}\t{licence}",
                package.name, package.version
            )
            .map_err(|error| error.to_string())?;
            if local {
                continue;
            }
            inventory.insert_documents(package, &selected)?;
        }
        Ok(inventory)
    }

    fn selected(&self) -> Result<Vec<&Package>, String> {
        let packages: BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let nodes: BTreeMap<_, _> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut pending = vec![self.resolve.root.as_str()];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let node = nodes
                .get(id)
                .ok_or_else(|| format!("missing resolved node: {id}"))?;
            pending.extend(
                node.deps
                    .iter()
                    .filter(|edge| edge.included())
                    .map(|edge| edge.pkg.as_str()),
            );
        }
        visited
            .into_iter()
            .map(|id| {
                packages
                    .get(id)
                    .copied()
                    .ok_or_else(|| format!("missing resolved package: {id}"))
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct Node {
    deps: Vec<Dependency>,
    id: String,
}

#[derive(Debug, Deserialize)]
struct Package {
    id: String,
    license: Option<String>,
    license_file: Option<PathBuf>,
    manifest_path: PathBuf,
    name: String,
    source: Option<String>,
    version: String,
}

impl Package {
    fn directory(&self) -> Result<&Path, String> {
        self.manifest_path
            .parent()
            .ok_or_else(|| format!("missing package directory: {}", self.name))
    }

    fn documents(&self, selected: &[&Self]) -> Result<Documents, String> {
        let directory = self.directory()?;
        let mut files = BTreeMap::new();
        if self.name == "sdl2-sys" {
            return self.sdl_documents(selected);
        }
        for name in DOCUMENTS {
            let path = directory.join(name);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    let _previous = files.insert(name.to_owned(), path);
                },
                Ok(_) => {
                    return Err(format!(
                        "not a licence file: {}",
                        path.display()
                    ));
                },
                Err(error) if error.kind() == ErrorKind::NotFound => {},
                Err(error) => return Err(error.to_string()),
            }
        }
        self.validate_documents(&files)?;
        Ok(files)
    }

    fn label(&self) -> Result<String, String> {
        let safe = |value: &str| {
            !value.is_empty()
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'-' | b'_' | b'.' | b'+')
                })
                && !matches!(value, "." | "..")
        };
        if !safe(&self.name) || !safe(&self.version) {
            return Err(String::from("unsafe Cargo package identifier"));
        }
        Ok(format!("{}-{}", self.name, self.version))
    }

    fn sdl_documents(&self, selected: &[&Self]) -> Result<Documents, String> {
        // The two published 0.39.0 crates share the same upstream revision
        // 9db18c80549c7cacf17581c02bfeaf98785afbb6. The sys crate omits
        // the root MIT text, while its SDL tree contains the Zlib text.
        if self.version != SDL_VERSION
            || self.license.as_deref() != Some("MIT AND Zlib")
        {
            return Err(String::from("review new SDL binding licence files"));
        }
        let wrapper = selected
            .iter()
            .find(|package| {
                package.name == "sdl2"
                    && package.version == SDL_VERSION
                    && package.source.as_deref() == Some(REGISTRY)
                    && package.license.as_deref() == Some("MIT")
            })
            .ok_or_else(|| String::from("missing companion SDL MIT licence"))?;
        Ok(BTreeMap::from([
            (
                String::from("LICENSE-MIT"),
                wrapper.directory()?.join("LICENSE"),
            ),
            (
                String::from("LICENSE-ZLIB"),
                self.directory()?.join("SDL/LICENSE.txt"),
            ),
        ]))
    }

    fn validate(&self, root: &Path, local: bool) -> Result<(), String> {
        let _label = self.label()?;
        if self.license_file.is_some() {
            return Err(format!("review explicit licence file: {}", self.name));
        }
        if local {
            if self.source.is_none()
                && self.license.as_deref() == Some("Apache-2.0")
                && self.manifest_path.starts_with(root.join("crates"))
            {
                return Ok(());
            }
        } else if self.source.as_deref() == Some(REGISTRY) {
            return Ok(());
        }
        Err(format!("unsupported dependency source: {}", self.name))
    }

    fn validate_documents(
        &self,
        files: &BTreeMap<String, PathBuf>,
    ) -> Result<(), String> {
        let has = |name| files.contains_key(name);
        let mit = has("LICENSE-MIT") || has("LICENSE");
        let apache = has("LICENSE-APACHE");
        let admitted = match self.license.as_deref() {
            Some("MIT") => mit,
            Some("MIT OR Apache-2.0" | "Apache-2.0 OR MIT") => mit || apache,
            Some("Unlicense OR MIT") => mit || has("UNLICENSE"),
            Some("(MIT OR Apache-2.0) AND Unicode-3.0") => {
                (mit || apache) && has("LICENSE-UNICODE")
            },
            _ => false,
        };
        if admitted {
            Ok(())
        } else {
            Err(format!("review licence documents: {}", self.name))
        }
    }
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
    root: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, license: &str) -> Package {
        Package {
            id: name.to_owned(),
            license: Some(license.to_owned()),
            license_file: None,
            manifest_path: PathBuf::from(format!("/cache/{name}/Cargo.toml")),
            name: name.to_owned(),
            source: Some(REGISTRY.to_owned()),
            version: String::from("1.0.0"),
        }
    }

    fn dependency(pkg: &str, kind: Option<&str>) -> Dependency {
        Dependency {
            dep_kinds: vec![DependencyKind {
                kind: kind.map(str::to_owned),
            }],
            pkg: pkg.to_owned(),
        }
    }

    #[test]
    fn selected_graph_excludes_development_only_dependencies() {
        let metadata = Metadata {
            packages: ["title", "runtime", "builder", "dev"]
                .map(|name| package(name, "MIT"))
                .into(),
            resolve: Resolve {
                nodes: vec![
                    Node {
                        deps: vec![
                            dependency("runtime", None),
                            dependency("builder", Some("build")),
                            dependency("dev", Some("dev")),
                        ],
                        id: String::from("title"),
                    },
                    Node {
                        deps: vec![dependency("builder", None)],
                        id: String::from("runtime"),
                    },
                    Node {
                        deps: Vec::new(),
                        id: String::from("builder"),
                    },
                ],
                root: String::from("title"),
            },
            workspace_members: BTreeSet::new(),
        };
        let selected = metadata.selected().map(|packages| {
            packages
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
        });
        assert_eq!(selected, Ok(vec!["builder", "runtime", "title"]));
    }

    #[test]
    fn missing_texts_and_unreviewed_licences_are_rejected() {
        let mut files = BTreeMap::from([(
            String::from("LICENSE-MIT"),
            PathBuf::from("MIT"),
        )]);
        let unicode = package("unicode", "(MIT OR Apache-2.0) AND Unicode-3.0");
        assert!(unicode.validate_documents(&files).is_err());
        let _previous = files
            .insert(String::from("LICENSE-UNICODE"), PathBuf::from("Unicode"));
        assert!(unicode.validate_documents(&files).is_ok());
        assert!(
            package("unknown", "Unreviewed")
                .validate_documents(&files)
                .is_err()
        );
        assert!(package("../escape", "MIT").label().is_err());
    }
}
