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
//   - Cargo configuration file discovery and recursive include admission.
// - Must-Not:
//   - Execute Cargo configuration or expose private configuration contents.
// - Allows:
//   - Inputs: Explicit configuration paths and Cargo's search roots.
//   - Outputs: Borrowed documents for validator inspection.
//   - Side effects: Read-only access to configuration files.
// - Split-When:
//   - A separate configuration format requires independent discovery.
// - Merge-When:
//   - Cargo exposes an equivalent local configuration inventory.
// - Summary:
//   - Visit the configuration files that can affect validation.
// - Description:
//   - Follow Cargo's documented hierarchy and relative include paths.
// - Usage:
//   - Share discovery between cache fingerprints and boundary checks.
// - Defaults:
//   - Reject malformed documents, cycles and unavailable required files.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::{fs, iter};

use toml::Value;

const MAX_DEPTH: usize = 64;
const MAX_DOCUMENTS: usize = 1024;

/// Contents and parsed values of one configuration file.
pub(crate) type Document<'config> = (&'config str, &'config Value);
type Inclusion<'path> = (&'path Path, bool);

struct Traversal {
    active: BTreeSet<PathBuf>,
    remaining: usize,
}

fn source(path: &Path, optional: bool) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {},
        Ok(_metadata) => {
            return Err(String::from("Cargo configuration is not a file"));
        },
        Err(error) if optional && error.kind() == ErrorKind::NotFound => {
            return Ok(None);
        },
        Err(error) => {
            return Err(format!("Cargo configuration: {}", error.kind()));
        },
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|error| format!("Cargo configuration: {}", error.kind()))
}

fn inclusion(value: &Value) -> Result<Inclusion<'_>, String> {
    let (name, optional) = if let Some(name) = value.as_str() {
        (name, false)
    } else {
        let table = value.as_table().ok_or("invalid Cargo include")?;
        let name = table
            .get("path")
            .and_then(Value::as_str)
            .ok_or("invalid Cargo include path")?;
        let optional = table
            .get("optional")
            .map(|flag| flag.as_bool().ok_or("invalid Cargo include flag"))
            .transpose()?
            .unwrap_or(false);
        (name, optional)
    };
    let path = Path::new(name);
    if path.extension().is_none_or(|extension| extension != "toml") {
        return Err(String::from("Cargo include must be a TOML file"));
    }
    Ok((path, optional))
}

fn walk<Inspect>(
    path: &Path,
    optional: bool,
    state: &mut Traversal,
    inspect: &mut Inspect,
) -> Result<bool, String>
where
    Inspect: FnMut(&Path, Option<Document<'_>>) -> Result<(), String>,
{
    state.remaining = state
        .remaining
        .checked_sub(1)
        .ok_or("Cargo configuration document limit")?;
    let Some(text) = source(path, optional)? else {
        inspect(path, None)?;
        return Ok(false);
    };
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Cargo configuration: {}", error.kind()))?;
    if state.active.len() >= MAX_DEPTH
        || !state.active.insert(canonical.clone())
    {
        return Err(String::from("Cargo configuration include cycle or depth"));
    }
    let value = Value::Table(
        text.parse::<toml::Table>()
            .map_err(|_error| "invalid Cargo configuration")?,
    );
    inspect(path, Some((&text, &value)))?;
    if let Some(includes) = value.get("include") {
        let parent = path.parent().ok_or("invalid Cargo configuration path")?;
        for entry in includes.as_array().ok_or("invalid Cargo include list")? {
            let (child, child_optional) = inclusion(entry)?;
            let _present =
                walk(&parent.join(child), child_optional, state, inspect)?;
        }
    }
    let _removed = state.active.remove(&canonical);
    Ok(true)
}

/// Visit a configuration and its included documents, including absent
/// optional paths. Return whether the initial file exists.
pub(crate) fn visit<Inspect>(
    path: &Path,
    optional: bool,
    inspect: &mut Inspect,
) -> Result<bool, String>
where
    Inspect: FnMut(&Path, Option<Document<'_>>) -> Result<(), String>,
{
    walk(
        path,
        optional,
        &mut Traversal {
            active: BTreeSet::new(),
            remaining: MAX_DOCUMENTS,
        },
        inspect,
    )
}

/// Visit active Cargo configuration from the current directory through its
/// ancestors and finally Cargo home. An extensionless file takes precedence.
pub(crate) fn visit_active<Inspect>(
    root: &Path,
    home: &Path,
    inspect: &mut Inspect,
) -> Result<(), String>
where
    Inspect: FnMut(&Path, Option<Document<'_>>) -> Result<(), String>,
{
    for directory in root
        .ancestors()
        .map(|ancestor| ancestor.join(".cargo"))
        .chain(iter::once(home.to_path_buf()))
    {
        if !visit(&directory.join("config"), true, inspect)? {
            let _present =
                visit(&directory.join("config.toml"), true, inspect)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{MAX_DOCUMENTS, visit};
    use crate::fixtures::{directory, require};

    #[test]
    fn rejects_cycles_and_missing_required_includes() -> Result<(), String> {
        let root = directory("cargo-include")?;
        let config = root.join("config.toml");
        let child = root.join("child.toml");
        fs::write(&config, "include = [\"child.toml\"]")
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "missing required include",
        )?;
        fs::write(&child, "include = [\"config.toml\"]")
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "configuration cycle",
        )?;
        fs::write(&child, "include = [{path=\"absent.toml\", optional=true}]")
            .map_err(|error| error.to_string())?;
        let mut absent = false;
        require(
            visit(&config, false, &mut |path, document| {
                if path.ends_with("absent.toml") && document.is_none() {
                    absent = true;
                }
                Ok(())
            })?,
            "initial configuration exists",
        )?;
        require(absent, "absent optional include is observable")?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    #[test]
    fn bounds_repeated_optional_include_attempts() -> Result<(), String> {
        let root = directory("cargo-include-budget")?;
        let config = root.join("config.toml");
        let document = |count| {
            let entries =
                vec!["{path=\"absent.toml\", optional=true}"; count].join(",");
            format!("include = [{entries}]")
        };
        fs::write(&config, document(MAX_DOCUMENTS.saturating_sub(1)))
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(()))?,
            "exact configuration visit budget",
        )?;
        fs::write(&config, document(MAX_DOCUMENTS))
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "configuration visit budget exhausted",
        )?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }
}
