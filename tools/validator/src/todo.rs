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
//   - Typed task records, direct links and dependency ordering.
// - Must-Not:
//   - Implement titles or claim gameplay acceptance.
// - Allows:
//   - Inputs: The root roadmap and typed task documents.
//   - Outputs: Validation success or diagnostic.
//   - Side effects: Read-only filesystem access.
// - Split-When:
//   - Independent task rules require separate ownership.
// - Merge-When:
//   - A shared checker covers the same task contracts.
// - Summary:
//   - Malbolge-style task validation in Rust.
// - Description:
//   - Checks canonical metadata, sections and task relationships.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Reject malformed, orphaned or inconsistent tasks.
//

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use games_platform::ir::storage::{PathKind, directory_entries};

use crate::files;

const MAX_TASK_DEPTH: usize = 64;
const MAX_TASK_ENTRIES: NonZeroUsize = NonZeroUsize::MIN.saturating_add(65_535);

type Fields = BTreeMap<String, Vec<String>>;
type IdentifiedTask = (String, Task);

#[derive(Default)]
struct Index<'document> {
    current: Option<&'document str>,
    ordered: Vec<String>,
    previous: &'document str,
    priority: &'document str,
}

#[derive(Debug)]
struct Task {
    dependencies: Vec<String>,
    path: String,
    priority: String,
    status: String,
    title: String,
}

struct TaskInventory {
    remaining: usize,
    tasks: BTreeMap<String, Task>,
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(String::from(message))
    }
}

fn scalar(text: &str) -> Result<String, String> {
    let value: String =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    require(!value.trim().is_empty(), "empty metadata value")?;
    Ok(value)
}

fn list_field(key: &str) -> bool {
    matches!(key, "depends_on" | "validation")
}

fn scalar_field(key: &str) -> bool {
    matches!(key, "id" | "status" | "lane" | "priority" | "area" | "task")
}

fn metadata(text: &str) -> Result<Fields, String> {
    let mut fields = Fields::new();
    let mut active = "";
    for line in text.lines() {
        if let Some(item) = line.strip_prefix("  - ") {
            require(list_field(active), "unexpected list item")?;
            let values = fields.get_mut(active).ok_or("missing list")?;
            values.push(scalar(item)?);
            continue;
        }
        let (key, suffix) = line.split_once(':').ok_or("invalid metadata")?;
        require(!fields.contains_key(key), "duplicate metadata key")?;
        let values = if list_field(key) {
            require(suffix.is_empty(), "expected block list")?;
            active = key;
            Vec::new()
        } else {
            require(scalar_field(key), "unknown metadata key")?;
            active = "";
            vec![scalar(suffix.strip_prefix(' ').ok_or("missing value")?)?]
        };
        require(
            fields.insert(String::from(key), values).is_none(),
            "duplicate key",
        )?;
    }
    for values in fields.values() {
        require(!values.is_empty(), "empty metadata list")?;
        let unique: BTreeSet<_> = values.iter().collect();
        require(unique.len() == values.len(), "duplicate list item")?;
    }
    Ok(fields)
}

fn field(fields: &Fields, key: &str) -> Result<String, String> {
    fields
        .get(key)
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| format!("missing metadata: {key}"))
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit()
                })
        })
}

fn phase(value: &str) -> bool {
    matches!(value, "p0" | "p1" | "p2" | "p3" | "p4" | "p5")
}

fn section(name: &str, content: &str, status: &str) -> Result<(), String> {
    require(!content.trim().is_empty(), "empty task section")?;
    if name == "Acceptance" {
        require(
            content.lines().any(|line| {
                line.strip_prefix("- ")
                    .is_some_and(|item| !item.trim().is_empty())
            }),
            "missing acceptance items",
        )?;
    }
    if name == "Evidence" && status == "completed" {
        let evidence = content.trim_start().to_ascii_lowercase();
        require(
            !["pending", "blocked", "partial evidence"]
                .iter()
                .any(|prefix| evidence.starts_with(prefix)),
            "completed task lacks evidence",
        )?;
    }
    Ok(())
}

fn sections(body: &str, title: &str, status: &str) -> Result<(), String> {
    let mut parts = body.split("\n## ");
    let heading = parts.next().ok_or("missing title")?;
    require(heading.trim() == format!("# {title}"), "title mismatch")?;
    for name in ["Objective", "Acceptance", "Evidence"] {
        let part = parts.next().ok_or("missing task section")?;
        let (actual, content) = part.split_once('\n').ok_or("empty section")?;
        require(actual == name, "incorrect task section")?;
        section(name, content, status)?;
    }
    require(parts.next().is_none(), "unexpected task section")
}

fn record(text: &str, path: &str) -> Result<IdentifiedTask, String> {
    require(
        text.lines().all(|line| line.chars().count() <= 80),
        "task line exceeds 80 characters",
    )?;
    let (header, body) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .ok_or("invalid front matter")?;
    let fields = metadata(header)?;
    let id = field(&fields, "id")?;
    let status = field(&fields, "status")?;
    let area = field(&fields, "area")?;
    let title = field(&fields, "task")?;
    let priority = field(&fields, "priority")?;
    let lane = field(&fields, "lane")?;
    require(slug(&id) && slug(&area), "invalid task ID or area")?;
    require(
        matches!(status.as_str(), "open" | "completed"),
        "invalid status",
    )?;
    require(
        phase(&priority) && priority.strip_prefix('p') == Some(&lane),
        "invalid lane or priority",
    )?;
    require(
        path == format!("docs/todo/{status}/{area}/{id}.mdc"),
        "task metadata/path mismatch",
    )?;
    require(title.starts_with("TODO - "), "missing TODO title prefix")?;
    require(
        fields.get("validation").is_some_and(|commands| {
            commands.iter().any(|command| command == "cargo validate")
        }),
        "missing cargo validate command",
    )?;
    sections(body, &title, &status)?;
    let dependencies = fields.get("depends_on").cloned().unwrap_or_default();
    Ok((id, Task {
        dependencies,
        path: String::from(path),
        priority,
        status,
        title,
    }))
}

fn collect(
    root: &Path,
    directory: &Path,
    inventory: &mut TaskInventory,
    depth: usize,
) -> Result<(), String> {
    require(depth < MAX_TASK_DEPTH, "task directory depth limit")?;
    let maximum =
        NonZeroUsize::new(inventory.remaining).unwrap_or(NonZeroUsize::MIN);
    let entries = directory_entries(directory.as_os_str(), maximum)
        .map_err(|error| error.to_string())?;
    inventory.remaining = inventory
        .remaining
        .checked_sub(entries.len())
        .ok_or("task directory entry limit")?;
    for entry in entries {
        let path = directory.join(entry.name);
        require(
            entry.kind != PathKind::SymbolicLink,
            "symlink in task directory",
        )?;
        if entry.kind == PathKind::Directory {
            collect(root, &path, inventory, depth.saturating_add(1))?;
        } else if path.extension().is_some_and(|ext| ext == "mdc") {
            let relative =
                path.strip_prefix(root).map_err(|error| error.to_string())?;
            let name = relative.to_str().ok_or("non-UTF-8 task path")?;
            let text = files::read_text(root, relative)?;
            let (id, task) = record(&text, name)
                .map_err(|error| format!("{name}: {error}"))?;
            require(
                inventory.tasks.insert(id, task).is_none(),
                "duplicate task ID",
            )?;
        }
    }
    Ok(())
}

fn direct_link(line: &str) -> Option<(&str, &str)> {
    let (title, path) = line.strip_prefix('[')?.split_once("](")?;
    Some((title, path.strip_suffix(')')?))
}

/// Check the roadmap width and its explicit direct-link exception.
#[must_use]
pub fn roadmap_width(line: &str, previous: &str) -> bool {
    line.chars().count() <= 80
        || (previous == "<!-- MarkdownLint-disable-next-line MD013 MD044 -->"
            && direct_link(line).is_some())
}

fn add_link(
    state: &mut Index<'_>,
    tasks: &BTreeMap<String, Task>,
    title: &str,
    path: &str,
) -> Result<(), String> {
    let (id, task) = tasks
        .iter()
        .find(|(_, task)| task.path == path)
        .ok_or_else(|| format!("unknown task link: {path}"))?;
    require(
        state.current == Some(title) && task.title == title,
        "task index title mismatch",
    )?;
    require(task.status == "open", "completed task in roadmap")?;
    require(
        task.priority.strip_prefix('p') == Some(state.priority),
        "task index phase mismatch",
    )?;
    require(!state.ordered.contains(id), "duplicate indexed task")?;
    state.ordered.push(id.clone());
    state.current = None;
    Ok(())
}

fn index_line<'document>(
    state: &mut Index<'document>,
    tasks: &BTreeMap<String, Task>,
    line: &'document str,
) -> Result<(), String> {
    let direct = direct_link(line);
    require(
        roadmap_width(line, state.previous),
        "roadmap line exceeds 80 characters",
    )?;
    state.previous = line;
    if let Some(heading) = line.strip_prefix("## P") {
        require(state.current.is_none(), "task missing direct link")?;
        let (number, _) =
            heading.split_once(" - ").ok_or("invalid phase heading")?;
        require(
            matches!(number, "0" | "1" | "2" | "3" | "4" | "5")
                && number > state.priority,
            "invalid or repeated phase heading",
        )?;
        state.priority = number;
    } else if let Some(heading) = line.strip_prefix("### ") {
        require(state.current.is_none(), "task missing direct link")?;
        state.current = Some(heading);
    } else if let Some((title, path)) = direct {
        add_link(state, tasks, title, path)?;
    } else {
        require(!line.starts_with('['), "reference links are forbidden")?;
    }
    Ok(())
}

fn index(
    text: &str,
    tasks: &BTreeMap<String, Task>,
) -> Result<Vec<String>, String> {
    let mut state = Index::default();
    for line in text.lines() {
        index_line(&mut state, tasks, line)?;
    }
    require(state.current.is_none(), "last task missing direct link")?;
    let expected = tasks.values().filter(|task| task.status == "open").count();
    require(expected == state.ordered.len(), "orphan open task")?;
    Ok(state.ordered)
}

fn predecessors(
    task: &Task,
    tasks: &BTreeMap<String, Task>,
) -> Result<(), String> {
    for dep in &task.dependencies {
        let predecessor = tasks
            .get(dep)
            .ok_or_else(|| format!("missing dependency: {dep}"))?;
        require(
            predecessor.priority <= task.priority,
            "dependency in later phase",
        )?;
        require(
            task.status != "completed" || predecessor.status == "completed",
            "completed task has unfinished dependency",
        )?;
    }
    Ok(())
}

fn dependency_order(
    tasks: &BTreeMap<String, Task>,
    ordered: &[String],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut completed: BTreeSet<_> = tasks
        .iter()
        .filter(|(_, task)| task.status == "completed")
        .map(|(id, _)| id)
        .collect();
    while !completed.is_empty() {
        let ready = completed.iter().find(|id| {
            tasks.get(**id).is_some_and(|task| {
                task.dependencies.iter().all(|dep| seen.contains(dep))
            })
        });
        let id = *ready.ok_or("cycle in completed dependencies")?;
        require(completed.remove(id), "invalid completed task")?;
        require(seen.insert(id.clone()), "duplicate completed task")?;
    }
    for id in ordered {
        let task = tasks.get(id).ok_or("unknown ordered task")?;
        require(
            task.dependencies.iter().all(|dep| seen.contains(dep)),
            "dependency cycle or out-of-order task",
        )?;
        require(seen.insert(id.clone()), "duplicate task in order")?;
    }
    Ok(())
}

fn unique_roadmap(authored: &[PathBuf]) -> Result<(), String> {
    for path in authored {
        let duplicate = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("TODO.md"));
        if duplicate && path != Path::new("TODO.md") {
            return Err(format!("duplicate roadmap: {}", path.display()));
        }
    }
    Ok(())
}

/// Validate the roadmap and all typed task records.
///
/// # Errors
///
/// Returns diagnostics for inconsistent or malformed task records.
pub fn validate(root: &Path) -> Result<(), String> {
    unique_roadmap(&files::authored(root)?)?;
    let mut inventory = TaskInventory {
        remaining: MAX_TASK_ENTRIES.get(),
        tasks: BTreeMap::new(),
    };
    collect(root, &root.join("docs/todo"), &mut inventory, 0)?;
    let tasks = inventory.tasks;
    require(!tasks.is_empty(), "missing task inventory")?;
    for task in tasks.values() {
        predecessors(task, &tasks)?;
    }
    let text = files::read_text(root, Path::new("TODO.md"))?;
    let ordered = index(&text, &tasks)?;
    dependency_order(&tasks, &ordered)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::{
        MAX_TASK_DEPTH, Task, TaskInventory, collect, dependency_order,
        direct_link, index, metadata, record, section, unique_roadmap,
        validate,
    };
    use crate::fixtures::{directory, require};

    #[test]
    fn incomplete_evidence_cannot_close_task() {
        for evidence in
            ["Pending.", "BLOCKED: missing media", "Partial evidence"]
        {
            assert!(section("Evidence", evidence, "open").is_ok());
            assert!(section("Evidence", evidence, "completed").is_err());
        }
        let evidence = "Validated with recorded tests.";
        assert!(section("Evidence", evidence, "completed").is_ok());
    }

    #[test]
    fn task_inventory_rejects_depth_and_entry_exhaustion() -> Result<(), String>
    {
        let fixture = directory()?;
        let root = Path::new(fixture.location());
        let directory = root.join("records");
        fixture
            .create_directories("records/nested")
            .map_err(|error| error.to_string())?;
        fixture
            .write_file("records/ignored.txt", b"")
            .map_err(|error| error.to_string())?;
        let inventory = |remaining| TaskInventory {
            remaining,
            tasks: BTreeMap::new(),
        };
        require(
            collect(root, &directory, &mut inventory(1), 0).is_err(),
            "entry budget",
        )?;
        require(
            collect(root, &directory, &mut inventory(2), MAX_TASK_DEPTH)
                .is_err(),
            "depth budget",
        )?;
        require(
            collect(root, &directory, &mut inventory(2), 0).is_ok(),
            "exact entry budget",
        )
    }

    #[test]
    fn only_root_roadmap_is_allowed() {
        let canonical = [
            PathBuf::from("TODO.md"),
            PathBuf::from("docs/todo/open/games/example.mdc"),
        ];
        assert!(unique_roadmap(&canonical).is_ok());
        for path in [
            "crates/games/example/TODO.md",
            "tools/build/TODO.md",
            "docs/Todo.md",
            "todo.md",
        ] {
            let duplicate = [PathBuf::from("TODO.md"), PathBuf::from(path)];
            assert!(unique_roadmap(&duplicate).is_err(), "{path}");
        }
    }

    #[test]
    fn untracked_roadmaps_fail_the_real_task_gate() -> Result<(), String> {
        let root_owner = directory()?;
        let root = Path::new(root_owner.location());
        root_owner
            .create_directories("crates/games/example")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file(".gitignore", b"/.reference/\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("TODO.md", b"# Roadmap\n")
            .map_err(|error| error.to_string())?;
        root_owner
            .write_file("crates/games/example/TODO.md", b"# Duplicate\n")
            .map_err(|error| error.to_string())?;
        let result = validate(root);
        require(
            result.is_err_and(|error| {
                error == "duplicate roadmap: crates/games/example/TODO.md"
            }),
            "actual inventory must reject untracked nested roadmaps",
        )
    }

    #[test]
    fn duplicate_metadata_rejected() {
        assert!(
            metadata("id: \"a\"\nid: \"b\"").is_err(),
            "duplicate fields must fail"
        );
    }

    #[test]
    fn missing_metadata_rejected() {
        assert!(
            record("---\nid: \"a\"\n---\n# A", "a").is_err(),
            "missing fields must fail"
        );
    }

    #[test]
    fn direct_links_only() {
        assert_eq!(
            direct_link("[TODO - A](a.mdc)"),
            Some(("TODO - A", "a.mdc")),
            "direct link"
        );
        assert!(
            direct_link("[Task record][a]").is_none(),
            "reference links are not accepted"
        );
    }

    #[test]
    fn rejects_duplicate_or_unknown_phase() {
        let tasks = BTreeMap::new();
        assert!(
            index("## P0 - First\n## P0 - Repeated", &tasks).is_err(),
            "duplicate phase must fail"
        );
        assert!(
            index("## P6 - Unknown", &tasks).is_err(),
            "unknown phase must fail"
        );
    }

    #[test]
    fn orphan_and_cycle_rejected() {
        let task = Task {
            dependencies: vec![String::from("a")],
            path: String::from("a.mdc"),
            priority: String::from("p0"),
            status: String::from("open"),
            title: String::from("TODO - A"),
        };
        let tasks = BTreeMap::from([(String::from("a"), task)]);
        assert!(index("# Games TODO", &tasks).is_err(), "orphan task");
        assert!(
            dependency_order(&tasks, &[String::from("a")]).is_err(),
            "cycle must fail"
        );
    }
}
