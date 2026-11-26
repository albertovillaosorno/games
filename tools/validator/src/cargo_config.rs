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
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::num::NonZeroU64;

use games_platform::ir::storage::file::snapshot;
use games_platform::ir::storage::{
    canonical_path, join_path, parent_path, path_extension,
};
use toml::Value;

const MAX_DEPTH: usize = 64;
const MAX_CONFIG_BYTES: NonZeroU64 = NonZeroU64::MIN.saturating_add(1_048_575);
const MAX_DOCUMENTS: usize = 1024;

/// Contents and parsed values of one configuration file.
pub(crate) type Document<'config> = (&'config str, &'config Value);
type Inclusion<'path> = (&'path OsStr, bool);

struct Traversal {
    active: BTreeSet<OsString>,
    remaining: usize,
}

type ConfigSource = (OsString, String);

fn source(
    path: &OsStr,
    optional: bool,
) -> Result<Option<ConfigSource>, String> {
    let canonical = match canonical_path(path) {
        Ok(value) => value,
        Err(error) if optional && error.kind() == ErrorKind::NotFound => {
            return Ok(None);
        },
        Err(error) => {
            return Err(format!("Cargo configuration: {}", error.kind()));
        },
    };
    let bytes = snapshot(canonical.as_os_str(), MAX_CONFIG_BYTES)
        .map_err(|error| format!("Cargo configuration: {error:?}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_error| "invalid Cargo configuration encoding")?;
    Ok(Some((canonical, text)))
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
    let path = OsStr::new(name);
    if path_extension(path)
        .map_err(|error| error.to_string())?
        .is_none_or(|extension| extension != "toml")
    {
        return Err(String::from("Cargo include must be a TOML file"));
    }
    Ok((path, optional))
}

fn walk<Inspect>(
    path: &OsStr,
    optional: bool,
    state: &mut Traversal,
    inspect: &mut Inspect,
) -> Result<bool, String>
where
    Inspect: FnMut(&OsStr, Option<Document<'_>>) -> Result<(), String>,
{
    state.remaining = state
        .remaining
        .checked_sub(1)
        .ok_or("Cargo configuration document limit")?;
    let Some((canonical, text)) = source(path, optional)? else {
        inspect(path, None)?;
        return Ok(false);
    };
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
        let parent = parent_path(path).map_err(|error| error.to_string())?;
        for entry in includes.as_array().ok_or("invalid Cargo include list")? {
            let (child, child_optional) = inclusion(entry)?;
            let _present = walk(
                &join_path(parent, child).map_err(|error| error.to_string())?,
                child_optional,
                state,
                inspect,
            )?;
        }
    }
    let _removed = state.active.remove(&canonical);
    Ok(true)
}

/// Visit a configuration and its included documents, including absent
/// optional paths. Return whether the initial file exists.
pub(crate) fn visit<Inspect>(
    path: &OsStr,
    optional: bool,
    inspect: &mut Inspect,
) -> Result<bool, String>
where
    Inspect: FnMut(&OsStr, Option<Document<'_>>) -> Result<(), String>,
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
    root: &OsStr,
    home: &OsStr,
    inspect: &mut Inspect,
) -> Result<(), String>
where
    Inspect: FnMut(&OsStr, Option<Document<'_>>) -> Result<(), String>,
{
    let mut ancestor = root;
    loop {
        let directory =
            join_path(ancestor, ".cargo").map_err(|error| error.to_string())?;
        visit_directory(&directory, inspect)?;
        match parent_path(ancestor) {
            Ok(parent) => ancestor = parent,
            Err(error) if error.kind() == ErrorKind::InvalidInput => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    visit_directory(home, inspect)
}

fn visit_directory<Inspect>(
    directory: &OsStr,
    inspect: &mut Inspect,
) -> Result<(), String>
where
    Inspect: FnMut(&OsStr, Option<Document<'_>>) -> Result<(), String>,
{
    let legacy =
        join_path(directory, "config").map_err(|error| error.to_string())?;
    if !visit(&legacy, true, inspect)? {
        let config = join_path(directory, "config.toml")
            .map_err(|error| error.to_string())?;
        let _present = visit(&config, true, inspect)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use games_platform::ir::storage::{join_path, path_leaf};

    use super::{MAX_CONFIG_BYTES, MAX_DOCUMENTS, visit};
    use crate::fixtures::{directory, require};

    #[test]
    fn rejects_cycles_and_missing_required_includes() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        let config = join_path(root, "config.toml")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("config.toml", b"include = [\"child.toml\"]")
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "missing required include",
        )?;
        root_owner
            .write_file("child.toml", b"include = [\"config.toml\"]")
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "configuration cycle",
        )?;
        root_owner
            .write_file(
                "child.toml",
                b"include = [{path=\"absent.toml\", optional=true}]",
            )
            .map_err(|error| error.to_string())?;
        let mut absent = false;
        require(
            visit(&config, false, &mut |path, document| {
                if path_leaf(path).is_ok_and(|leaf| leaf == "absent.toml")
                    && document.is_none()
                {
                    absent = true;
                }
                Ok(())
            })?,
            "initial configuration exists",
        )?;
        require(absent, "absent optional include is observable")?;
        Ok(())
    }

    #[test]
    fn rejects_oversized_configuration() -> Result<(), String> {
        let fixture = directory()?;
        let path = join_path(fixture.location(), "config.toml")
            .map_err(|error| error.to_string())?;
        fixture
            .create_sized_file(
                "config.toml",
                MAX_CONFIG_BYTES.get().saturating_add(1),
            )
            .map_err(|error| error.to_string())?;
        require(
            visit(&path, false, &mut |_, _| Ok(())).is_err(),
            "oversized config",
        )
    }

    #[test]
    fn bounds_repeated_optional_include_attempts() -> Result<(), String> {
        let root_owner = directory()?;
        let root = root_owner.location();
        let config = join_path(root, "config.toml")
            .map_err(|error| error.to_string())?;
        let document = |count| {
            let entries =
                vec!["{path=\"absent.toml\", optional=true}"; count].join(",");
            format!("include = [{entries}]")
        };
        root_owner
            .write_file(
                "config.toml",
                document(MAX_DOCUMENTS.saturating_sub(1)).as_bytes(),
            )
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(()))?,
            "exact configuration visit budget",
        )?;
        root_owner
            .write_file("config.toml", document(MAX_DOCUMENTS).as_bytes())
            .map_err(|error| error.to_string())?;
        require(
            visit(&config, false, &mut |_, _| Ok(())).is_err(),
            "configuration visit budget exhausted",
        )?;
        Ok(())
    }
}
