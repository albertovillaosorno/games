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
//   - Pinned Linux research archive admission and reviewed removal.
// - Must-Not:
//   - Traverse historical captures or delete unclassified investigations.
// - Allows:
//   - Inputs: Explicit retention policy, operation and fixed workspace root.
//   - Outputs: Bounded inventory, preview and reviewed removal receipt.
//   - Side effects: Read-only inventory or explicit named archive removal.
// - Split-When:
//   - Another archive namespace needs independent retention semantics.
// - Merge-When:
//   - Shared native storage owns the same archive lifecycle.
// - Summary:
//   - Inspect only sealed owner-controlled research archive packages.
// - Description:
//   - Verify retained ledger content and remove only reviewed regular files.
// - Usage:
//   - Dispatch the portable reference retention contract.
// - Defaults:
//   - No directory creation, implicit removal or recursive deletion.

//! Linux adapter for explicitly sealed, flat research archive packages.

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::str::from_utf8;
use std::time::{SystemTime, UNIX_EPOCH};

use games_assets::ContentHash;
use nix::errno::Errno;
use nix::fcntl::{AtFlags, open, openat};
use nix::sys::stat::{FileStat, Mode, fstat, fstatat};
use nix::unistd::{UnlinkatFlags, unlinkat};

use super::file::{
    DIRECTORY_FLAGS, private_regular, read_regular, same_inode,
    trusted_directory_stat,
};
use super::pinned_directory_entries;
use crate::ir::toolchain::reference::retention::{
    Candidate, Inventory, Operation, Plan, Policy, Removal, RetentionBackend,
    select,
};
use crate::ir::toolchain::{ToolchainError, workspace_root};

const SEAL: &str = ".games-disposable-v1";
const PAYLOAD: &str = "captures.tar.zst";
const SEAL_LIMIT: u64 = 4096;
const OPERATION: &str = "research retention";

type Outcome<T> = Result<T, ToolchainError>;
type Observed = (Vec<Run>, Vec<String>);

struct Roots {
    archive: OwnedFd,
    reference: OwnedFd,
}

impl Roots {
    fn open(workspace: &Path) -> Outcome<Option<Self>> {
        let workspace_fd =
            open(workspace, DIRECTORY_FLAGS, Mode::empty()).map_err(native)?;
        let _workspace_stat = trusted(&workspace_fd)?;
        let Some(reference) = child(&workspace_fd, ".reference")? else {
            return Ok(None);
        };
        let Some(investigations) = child(&reference, "investigations")? else {
            return Ok(None);
        };
        let Some(archive) = child(&investigations, "archived-disposable")?
        else {
            return Ok(None);
        };
        Ok(Some(Self { archive, reference }))
    }

    fn verify(&self, workspace: &Path) -> Outcome<()> {
        let current = Self::open(workspace)?
            .ok_or_else(|| failure("archive root disappeared"))?;
        if !same_inode(&trusted(&self.archive)?, &trusted(&current.archive)?)
            || !same_inode(
                &trusted(&self.reference)?,
                &trusted(&current.reference)?,
            )
        {
            return Err(failure("archive root changed"));
        }
        Ok(())
    }
}

struct Budget {
    bytes: u64,
    entries: usize,
    policy: Policy,
}

impl Budget {
    fn bytes(&mut self, count: u64) -> Outcome<()> {
        self.bytes = self
            .bytes
            .checked_add(count)
            .filter(|total| *total <= self.policy.maximum_bytes.get())
            .ok_or_else(|| failure("research byte budget exceeded"))?;
        Ok(())
    }

    fn entry(&mut self) -> Outcome<()> {
        self.entries = self
            .entries
            .checked_add(1)
            .filter(|count| *count <= self.policy.maximum_entries.get())
            .ok_or_else(|| failure("research entry budget exceeded"))?;
        Ok(())
    }

    const fn new(policy: Policy) -> Self {
        Self {
            bytes: 0,
            entries: 0,
            policy,
        }
    }
}

struct Seal {
    digest: ContentHash,
    evidence: String,
}

impl Seal {
    fn parse(bytes: &[u8]) -> Outcome<Self> {
        let text =
            from_utf8(bytes).map_err(|_error| failure("seal must be UTF-8"))?;
        let lines: Vec<_> = text.lines().collect();
        let [
            "games-disposable-v1",
            "status=completed-archived-regenerable",
            evidence,
            digest,
            reproduce,
        ] = lines.as_slice()
        else {
            return Err(failure("invalid disposable run seal"));
        };
        let path = evidence
            .strip_prefix("evidence=")
            .ok_or_else(|| failure("missing retained evidence"))?;
        let (title, record) = path
            .split_once('/')
            .ok_or_else(|| failure("expected title/ledger.jsonl"))?;
        if !safe_component(title)
            || record != "ledger.jsonl"
            || !reproduce.strip_prefix("reproduce=").is_some_and(|note| {
                !note.trim().is_empty() && !note.chars().any(char::is_control)
            })
        {
            return Err(failure("invalid evidence path or reproduction note"));
        }
        let identity = digest
            .strip_prefix("sha256=")
            .ok_or_else(|| failure("missing evidence identity"))?
            .parse()
            .map_err(|_error| failure("invalid evidence identity"))?;
        Ok(Self {
            digest: identity,
            evidence: path.to_owned(),
        })
    }

    fn verify(&self, reference: &OwnedFd, budget: &mut Budget) -> Outcome<()> {
        let (title, record) = self
            .evidence
            .split_once('/')
            .ok_or_else(|| failure("invalid retained evidence path"))?;
        let ledger = required_child(reference, "games")?;
        let directory = required_child(&ledger, title)?;
        let stat = ordinary(&directory, record)?;
        budget.bytes(size(&stat)?)?;
        let bytes = read_bounded(&directory, record, size(&stat)?)?;
        if !bytes.split(|byte| *byte == b'\n').any(|line| {
            !line.is_empty() && ContentHash::calculate(line) == self.digest
        }) || stamp(&ordinary(&directory, record)?) != stamp(&stat)
        {
            return Err(failure("retained evidence identity changed"));
        }
        Ok(())
    }
}

struct Run {
    candidate: Candidate,
    directory: FileStat,
    payload: FileStat,
    seal: FileStat,
}

impl Run {
    fn observe(
        roots: &Roots,
        name: &str,
        directory: &OwnedFd,
        budget: &mut Budget,
    ) -> Outcome<Self> {
        let directory_stat = trusted(directory)?;
        let seal = ordinary(directory, SEAL)?;
        let payload = ordinary(directory, PAYLOAD)?;
        if directory_stat.st_dev != seal.st_dev
            || directory_stat.st_dev != payload.st_dev
        {
            return Err(failure("archive crosses a device boundary"));
        }
        Self::verify_layout(directory, budget)?;
        let bytes = size(&seal)?
            .checked_add(size(&payload)?)
            .ok_or_else(|| failure("archive byte count overflow"))?;
        budget.bytes(bytes)?;
        let seal_bytes = read_bounded(directory, SEAL, SEAL_LIMIT)?;
        let record = Seal::parse(&seal_bytes)?;
        record.verify(&roots.reference, budget)?;
        let fingerprint = ContentHash::calculate(
            format!(
                "{} {} {} {}",
                stamp(&directory_stat),
                stamp(&seal),
                stamp(&payload),
                ContentHash::calculate(&seal_bytes),
            )
            .as_bytes(),
        );
        let result = Self {
            candidate: Candidate {
                fingerprint,
                newest: newest(&directory_stat)?
                    .max(newest(&seal)?)
                    .max(newest(&payload)?),
                removal: Removal {
                    bytes,
                    evidence: record.evidence,
                    evidence_digest: record.digest,
                    name: name.to_owned(),
                },
            },
            directory: directory_stat,
            payload,
            seal,
        };
        result.verify_files(directory)?;
        Ok(result)
    }

    fn remove(&self, roots: &Roots, policy: Policy) -> Outcome<()> {
        let name = self.candidate.removal.name.as_str();
        let directory = required_child(&roots.archive, name)?;
        self.verify_files(&directory)?;
        let mut budget = Budget::new(policy);
        Self::verify_layout(&directory, &mut budget)?;
        let seal = Seal::parse(&read_bounded(&directory, SEAL, SEAL_LIMIT)?)?;
        seal.verify(&roots.reference, &mut budget)?;
        self.verify_files(&directory)?;
        // Never recursively delete or remove an unobserved entry. An
        // unexpected concurrent entry makes the final empty-directory
        // removal fail instead of deleting that entry.
        remove_file(&directory, PAYLOAD, &self.payload)?;
        remove_file(&directory, SEAL, &self.seal)?;
        let current = observed(&roots.archive, name)?
            .ok_or_else(|| failure("reviewed run disappeared"))?;
        if !trusted_directory_stat(&current)
            || !same_inode(&current, &self.directory)
        {
            return Err(failure("reviewed run replaced"));
        }
        unlinkat(&roots.archive, name, UnlinkatFlags::RemoveDir).map_err(native)
    }

    fn verify_files(&self, directory: &OwnedFd) -> Outcome<()> {
        if stamp(&trusted(directory)?) != stamp(&self.directory)
            || stamp(&ordinary(directory, SEAL)?) != stamp(&self.seal)
            || stamp(&ordinary(directory, PAYLOAD)?) != stamp(&self.payload)
        {
            return Err(failure("reviewed archive changed"));
        }
        Ok(())
    }

    fn verify_layout(directory: &OwnedFd, budget: &mut Budget) -> Outcome<()> {
        for item in pinned_directory_entries(directory).map_err(io_failure)? {
            budget.entry()?;
            let name = item.map_err(io_failure)?.file_name();
            if name != SEAL && name != PAYLOAD {
                return Err(failure("sealed archive has unclassified entries"));
            }
        }
        Ok(())
    }
}

impl RetentionBackend for () {
    fn retain(policy: Policy, operation: Operation) -> Outcome<Plan> {
        let workspace = workspace_root()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| failure(error.to_string()))?
            .as_secs();
        execute(Path::new(&workspace), policy, operation, now)
    }
}

fn failure<Diagnostic>(diagnostic: Diagnostic) -> ToolchainError
where
    Diagnostic: Into<String>,
{
    ToolchainError::Failed {
        operation: OPERATION,
        diagnostic: diagnostic.into(),
    }
}

fn native(error: Errno) -> ToolchainError {
    io_failure(error.into())
}

const fn io_failure(source: io::Error) -> ToolchainError {
    ToolchainError::Io {
        operation: OPERATION,
        source,
    }
}

fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn trusted(directory: &OwnedFd) -> Outcome<FileStat> {
    let stat = fstat(directory).map_err(native)?;
    if !trusted_directory_stat(&stat) {
        return Err(failure("untrusted research directory"));
    }
    Ok(stat)
}

fn child(parent: &OwnedFd, name: &str) -> Outcome<Option<OwnedFd>> {
    match openat(parent, name, DIRECTORY_FLAGS, Mode::empty()) {
        Ok(directory) => {
            let _directory_stat = trusted(&directory)?;
            Ok(Some(directory))
        },
        Err(Errno::ENOENT) => Ok(None),
        Err(error) => Err(native(error)),
    }
}

fn required_child(parent: &OwnedFd, name: &str) -> Outcome<OwnedFd> {
    child(parent, name)?.ok_or_else(|| failure("required directory missing"))
}

fn observed(parent: &OwnedFd, name: &str) -> Outcome<Option<FileStat>> {
    match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(stat)),
        Err(Errno::ENOENT) => Ok(None),
        Err(error) => Err(native(error)),
    }
}

fn ordinary(parent: &OwnedFd, name: &str) -> Outcome<FileStat> {
    let stat = observed(parent, name)?
        .ok_or_else(|| failure("required archive file missing"))?;
    if !private_regular(&stat) || stat.st_nlink != 1 {
        return Err(failure("unsafe archive or evidence file"));
    }
    Ok(stat)
}

fn size(stat: &FileStat) -> Outcome<u64> {
    u64::try_from(stat.st_size).map_err(|_error| failure("invalid file size"))
}

fn newest(stat: &FileStat) -> Outcome<u64> {
    let seconds = stat.st_mtime.max(stat.st_ctime);
    let subsecond = if stat.st_mtime == seconds {
        stat.st_mtime_nsec
    } else {
        0
    }
    .max(if stat.st_ctime == seconds {
        stat.st_ctime_nsec
    } else {
        0
    });
    let whole = u64::try_from(seconds)
        .map_err(|_error| failure("invalid archive timestamp"))?;
    whole
        .checked_add(u64::from(subsecond > 0))
        .ok_or_else(|| failure("archive timestamp overflow"))
}

fn stamp(stat: &FileStat) -> ContentHash {
    let description = format!(
        "{} {} {} {} {} {} {} {} {} {}",
        stat.st_dev,
        stat.st_ino,
        stat.st_mode,
        stat.st_uid,
        stat.st_nlink,
        stat.st_size,
        stat.st_mtime,
        stat.st_mtime_nsec,
        stat.st_ctime,
        stat.st_ctime_nsec,
    );
    ContentHash::calculate(description.as_bytes())
}

fn read_bounded(
    directory: &OwnedFd,
    name: &str,
    limit: u64,
) -> Outcome<Vec<u8>> {
    read_regular(directory, name, limit)
        .map_err(|_error| failure("unsafe or changed retention record"))
}

fn remove_file(
    directory: &OwnedFd,
    name: &str,
    reviewed: &FileStat,
) -> Outcome<()> {
    if stamp(&ordinary(directory, name)?) != stamp(reviewed) {
        return Err(failure("reviewed archive file replaced"));
    }
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir).map_err(native)
}

fn inventory(roots: &Roots, policy: Policy) -> Outcome<Observed> {
    let mut budget = Budget::new(policy);
    let mut runs = Vec::new();
    let mut protected = Vec::new();
    let root_stat = trusted(&roots.archive)?;
    for item in pinned_directory_entries(&roots.archive).map_err(io_failure)? {
        budget.entry()?;
        let name = item
            .map_err(io_failure)?
            .file_name()
            .into_string()
            .map_err(|_error| failure("non-portable archive name"))?;
        if !safe_component(&name) {
            return Err(failure("invalid archive directory name"));
        }
        let directory = required_child(&roots.archive, &name)?;
        if trusted(&directory)?.st_dev != root_stat.st_dev {
            return Err(failure("archive crosses a device boundary"));
        }
        if observed(&directory, SEAL)?.is_some() {
            runs.push(Run::observe(roots, &name, &directory, &mut budget)?);
        } else {
            protected.push(name);
        }
    }
    if stamp(&trusted(&roots.archive)?) != stamp(&root_stat) {
        return Err(failure("archive inventory changed"));
    }
    Ok((runs, protected))
}

fn execute(
    workspace: &Path,
    policy: Policy,
    operation: Operation,
    now: u64,
) -> Outcome<Plan> {
    let Some(roots) = Roots::open(workspace)? else {
        let plan = select(policy, now, Inventory {
            candidates: Vec::new(),
            protected: Vec::new(),
            root: ContentHash::calculate(b"absent"),
        })?;
        verify_plan(operation, &plan)?;
        return Ok(plan);
    };
    let lock = File::from(roots.archive.try_clone().map_err(io_failure)?);
    match operation {
        Operation::Plan => lock.try_lock_shared(),
        Operation::Apply(_) => lock.try_lock(),
    }
    .map_err(|_error| failure("archive retention is already in progress"))?;
    let root_stamp = stamp(&trusted(&roots.archive)?);
    let (runs, protected) = inventory(&roots, policy)?;
    if stamp(&trusted(&roots.archive)?) != root_stamp {
        return Err(failure("archive inventory changed"));
    }
    let candidates = runs.iter().map(|run| run.candidate.clone()).collect();
    let plan = select(policy, now, Inventory {
        candidates,
        protected,
        root: root_stamp,
    })?;
    roots.verify(workspace)?;
    verify_plan(operation, &plan)?;
    if matches!(operation, Operation::Apply(_)) {
        for removal in &plan.removals {
            roots.verify(workspace)?;
            let run = runs
                .iter()
                .find(|entry| entry.candidate.removal.name == removal.name)
                .ok_or_else(|| failure("missing reviewed run"))?;
            run.remove(&roots, policy)?;
        }
    }
    Ok(plan)
}

fn verify_plan(operation: Operation, plan: &Plan) -> Outcome<()> {
    if let Operation::Apply(expected) = operation
        && expected != plan.identity
    {
        return Err(failure("retention plan changed; preview again"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::num::{NonZeroU64, NonZeroUsize};
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::software::linux::storage::fixtures::TestRoot;

    const ROOT: &str = ".reference/investigations/archived-disposable";
    const LEDGER: &str = ".reference/games/title";
    const EVIDENCE: &[u8] = br#"{"evidence":"retained","reproduce":"fixture"}"#;

    fn require(condition: bool, message: &str) -> io::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(io::Error::other(message))
        }
    }

    fn policy() -> io::Result<Policy> {
        Ok(Policy {
            maximum_bytes: NonZeroU64::new(10000)
                .ok_or(io::ErrorKind::InvalidInput)?,
            maximum_entries: NonZeroUsize::new(20)
                .ok_or(io::ErrorKind::InvalidInput)?,
            minimum_age: NonZeroU64::new(10)
                .ok_or(io::ErrorKind::InvalidInput)?,
            quota: 0,
        })
    }

    fn aged_now() -> io::Result<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs().saturating_add(60))
            .map_err(io::Error::other)
    }

    fn fixture() -> io::Result<TestRoot> {
        let root = TestRoot::new()?;
        fs::create_dir_all(root.path().join(ROOT))?;
        let ledger = root.path().join(LEDGER);
        fs::create_dir_all(&ledger)?;
        fs::write(ledger.join("ledger.jsonl"), EVIDENCE)?;
        let run = root.path().join(ROOT).join("old-run");
        fs::create_dir_all(&run)?;
        fs::write(run.join(PAYLOAD), b"tiny disposable fixture")?;
        fs::write(
            run.join(SEAL),
            format!(
                concat!(
                    "games-disposable-v1\n",
                    "status=completed-archived-regenerable\n",
                    "evidence=title/ledger.jsonl\nsha256={}\n",
                    "reproduce=fixture command and inputs\n"
                ),
                ContentHash::calculate(EVIDENCE)
            ),
        )?;
        Ok(root)
    }

    fn preview(root: &TestRoot, limits: Policy) -> io::Result<Plan> {
        execute(root.path(), limits, Operation::Plan, aged_now()?)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    #[test]
    fn preview_and_apply_preserve_ledger_and_unsealed_runs() -> io::Result<()> {
        let root = fixture()?;
        let active = root.path().join(ROOT).join("active");
        fs::create_dir_all(&active)?;
        fs::write(active.join("in-progress"), b"keep")?;
        let plan = preview(&root, policy()?)?;
        require(
            plan.removals.len() == 1
                && plan.bytes_after == 0
                && plan.bytes_before > 0
                && plan.protected == ["active"],
            "only completed sealed run selected",
        )?;
        let old = root.path().join(ROOT).join("old-run");
        let ledger = root.path().join(LEDGER).join("ledger.jsonl");
        let appended = [EVIDENCE, b"\n{\"another\":\"record\"}\n"].concat();
        fs::write(&ledger, &appended)?;
        require(
            preview(&root, policy()?)? == plan,
            "unchanged evidence record should survive ledger append",
        )?;
        require(old.join(PAYLOAD).exists(), "preview modified archive")?;
        let result = execute(
            root.path(),
            policy()?,
            Operation::Apply(plan.identity),
            aged_now()?,
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        require(result == plan && !old.exists(), "reviewed run not removed")?;
        require(
            fs::read(active.join("in-progress"))? == b"keep",
            "active investigation modified",
        )?;
        require(fs::read(&ledger)? == appended, "retained ledger modified")
    }

    #[test]
    fn stale_plan_and_changed_ledger_fail_closed() -> io::Result<()> {
        let root = fixture()?;
        let plan = preview(&root, policy()?)?;
        let payload = root.path().join(ROOT).join("old-run").join(PAYLOAD);
        fs::write(&payload, b"new contents")?;
        require(
            execute(
                root.path(),
                policy()?,
                Operation::Apply(plan.identity),
                aged_now()?,
            )
            .is_err(),
            "stale identity accepted",
        )?;
        require(
            fs::read(&payload)? == b"new contents",
            "changed file removed",
        )?;
        fs::write(root.path().join(LEDGER).join("ledger.jsonl"), b"changed")?;
        require(
            preview(&root, policy()?).is_err(),
            "changed evidence accepted",
        )?;
        require(payload.exists(), "archive removed without evidence")
    }

    #[test]
    fn links_extra_entries_and_bounds_are_rejected() -> io::Result<()> {
        let root = fixture()?;
        let run = root.path().join(ROOT).join("old-run");
        let outside = root.path().join("outside");
        fs::write(&outside, b"keep")?;
        fs::hard_link(&outside, run.join("extra"))?;
        require(preview(&root, policy()?).is_err(), "extra entry accepted")?;
        fs::remove_file(run.join("extra"))?;
        let payload = run.join(PAYLOAD);
        fs::remove_file(&payload)?;
        fs::hard_link(&outside, &payload)?;
        require(preview(&root, policy()?).is_err(), "hard link accepted")?;
        fs::remove_file(&payload)?;
        symlink(&outside, &payload)?;
        require(preview(&root, policy()?).is_err(), "symbolic link accepted")?;
        fs::remove_file(&payload)?;
        fs::write(&payload, b"fixture")?;
        let mut limits = policy()?;
        limits.maximum_entries = NonZeroUsize::MIN;
        require(preview(&root, limits).is_err(), "entry limit ignored")?;
        limits = policy()?;
        limits.maximum_bytes = NonZeroU64::MIN;
        require(preview(&root, limits).is_err(), "byte limit ignored")?;
        require(fs::read(outside)? == b"keep", "external file modified")
    }

    #[test]
    fn absent_root_is_read_only_and_symlink_root_is_rejected() -> io::Result<()>
    {
        let root = TestRoot::new()?;
        require(
            preview(&root, policy()?)?.removals.is_empty(),
            "absent archive should have empty plan",
        )?;
        require(
            !root.path().join(".reference").exists(),
            "preview created reference storage",
        )?;
        let outside = root.path().join("outside");
        fs::create_dir_all(&outside)?;
        symlink(outside, root.path().join(".reference"))?;
        require(preview(&root, policy()?).is_err(), "symlink root accepted")
    }
}
