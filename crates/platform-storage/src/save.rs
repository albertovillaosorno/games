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
//   - Bounded, integrity-checked native save-slot persistence.
// - Must-Not:
//   - Interpret title payloads or select a title's storage location.
// - Allows:
//   - Inputs: Trusted directory, numeric slots and bounded save payloads.
//   - Outputs: Verified payloads or atomic publication receipts.
//   - Side effects: Owner-only files in a pinned host directory.
// - Split-When:
//   - A platform requires a different persistence mechanism.
// - Merge-When:
//   - A shared storage adapter provides identical save guarantees.
// - Summary:
//   - Replace a local save only after its new envelope is fully written.
// - Description:
//   - Pin the directory, verify envelopes and sync before atomic rename.
// - Usage:
//   - Open a title-specific trusted directory and use numeric slot IDs.
// - Defaults:
//   - No traversal, links, special files or partial publication.

use std::fs::File;
use std::io::{self, Write as _};
use std::num::NonZeroU64;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use games::safety::{
    SafetyError, decode_save, encode_save_header, save_envelope_limit,
};
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat, renameat};
use nix::sys::stat::{Mode, fstatat};
use nix::unistd::{UnlinkatFlags, unlinkat};

use crate::file::{
    DIRECTORY_FLAGS, FileReadError, private_regular, read_regular,
    trusted_directory,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Save admission or pre-publication failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveError {
    /// The save envelope is invalid or exceeds its payload limit.
    Envelope(SafetyError),
    /// A native storage operation failed before publication.
    Io(io::ErrorKind),
    /// Bounded file admission rejected the requested slot.
    Read(FileReadError),
    /// The directory or destination is linked or otherwise untrusted.
    UnsafeStorage,
}

impl From<Errno> for SaveError {
    fn from(error: Errno) -> Self {
        Self::Io(io::Error::from(error).kind())
    }
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<SafetyError> for SaveError {
    fn from(error: SafetyError) -> Self {
        Self::Envelope(error)
    }
}

impl From<FileReadError> for SaveError {
    fn from(error: FileReadError) -> Self {
        Self::Read(error)
    }
}

/// Save publication succeeded; directory sync can fail after publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveReceipt {
    /// Error here means the new save is visible but crash durability is
    /// uncertain. It does not mean the old save was preserved.
    pub durability: Result<(), io::ErrorKind>,
}

/// Pinned directory of integrity-checked numeric save slots.
///
/// The caller chooses an existing title-specific directory whose ancestors
/// are trusted. Other processes running as the same user remain trusted.
/// Concurrent writes to one slot have last-publication-wins semantics.
/// Loads pin a complete file before reading through trusted kernel procfs.
#[derive(Debug)]
pub struct SaveStore {
    directory: File,
    maximum: NonZeroU64,
}

impl SaveStore {
    /// Load a slot and validate its exact envelope before returning payload.
    ///
    /// # Errors
    /// Rejects unsafe storage, missing or malformed files and oversized data.
    pub fn load(&self, slot: u32) -> Result<Vec<u8>, SaveError> {
        let directory = &self.directory;
        let mut bytes = read_regular(
            directory,
            &slot_name(slot),
            save_envelope_limit(self.maximum),
        )?;
        let length = decode_save(&bytes, self.maximum)?.len();
        let offset = bytes
            .len()
            .checked_sub(length)
            .ok_or(SafetyError::InvalidSave)?;
        bytes.copy_within(offset.., 0);
        bytes.truncate(length);
        Ok(bytes)
    }

    /// Open a trusted save directory without following a final symlink.
    ///
    /// # Errors
    /// Rejects missing, linked, non-directory or writable-by-others paths.
    pub fn open(
        directory: &Path,
        maximum: NonZeroU64,
    ) -> Result<Self, SaveError> {
        let fd = open(directory, DIRECTORY_FLAGS, Mode::empty())?;
        if !trusted_directory(&fd)? {
            return Err(SaveError::UnsafeStorage);
        }
        Ok(Self {
            directory: File::from(fd),
            maximum,
        })
    }

    /// Replace a slot atomically using a synced private temporary file.
    ///
    /// # Errors
    /// Rejects invalid storage and oversized saves before publication.
    /// A pre-publication failure leaves an existing slot unchanged.
    pub fn store(
        &self,
        slot: u32,
        payload: &[u8],
    ) -> Result<SaveReceipt, SaveError> {
        let directory = &self.directory;
        if !trusted_directory(directory)? {
            return Err(SaveError::UnsafeStorage);
        }
        let name = slot_name(slot);
        match fstatat(directory, name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) if private_regular(&stat) => {},
            Ok(_) => return Err(SaveError::UnsafeStorage),
            Err(Errno::ENOENT) => {},
            Err(error) => return Err(error.into()),
        }
        let header = encode_save_header(payload, self.maximum)?;
        let mut pending = PendingSave::create(directory)?;
        pending.file.write_all(&header)?;
        pending.file.write_all(payload)?;
        pending.file.sync_all()?;
        renameat(directory, pending.name.as_str(), directory, name.as_str())?;
        pending.published = true;
        Ok(SaveReceipt {
            durability: self.directory.sync_all().map_err(|error| error.kind()),
        })
    }
}

#[derive(Debug)]
struct PendingSave<'directory> {
    directory: &'directory File,
    file: File,
    name: String,
    published: bool,
}

impl<'directory> PendingSave<'directory> {
    fn create(directory: &'directory File) -> Result<Self, SaveError> {
        for _attempt in 0u8..128 {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".save-{}-{sequence}", process::id());
            match openat(
                directory,
                name.as_str(),
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::S_IRUSR | Mode::S_IWUSR,
            ) {
                Ok(fd) => {
                    return Ok(Self {
                        directory,
                        file: File::from(fd),
                        name,
                        published: false,
                    });
                },
                Err(Errno::EEXIST) => {},
                Err(error) => return Err(error.into()),
            }
        }
        Err(Errno::EEXIST.into())
    }
}

impl Drop for PendingSave<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _cleanup = unlinkat(
                self.directory,
                self.name.as_str(),
                UnlinkatFlags::NoRemoveDir,
            );
        }
    }
}

fn slot_name(slot: u32) -> String {
    format!("slot-{slot}.gms")
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::sync::Barrier;
    use std::{fs, thread};

    use games::safety::encode_save;
    use nix::unistd::mkfifo;

    use super::*;
    use crate::bundle::TestRoot;

    fn publish_replacements(
        store: &SaveStore,
        first: &[u8],
        second: &[u8],
    ) -> Result<(), SaveError> {
        for payload in [second, first].into_iter().cycle().take(32) {
            require(store.store(0, payload)?.durability.is_ok())?;
        }
        Ok(())
    }

    fn require(condition: bool) -> Result<(), SaveError> {
        condition
            .then_some(())
            .ok_or(SaveError::Io(io::ErrorKind::InvalidData))
    }

    #[test]
    fn concurrent_loads_observe_only_complete_replacements()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let maximum = NonZeroU64::new(4096).unwrap_or(NonZeroU64::MIN);
        let store = SaveStore::open(&root.path, maximum)?;
        let first = [0x12; 4096];
        let second = [0x34; 4096];
        require(store.store(0, &first)?.durability.is_ok())?;
        let barrier = Barrier::new(2);
        thread::scope(|scope| {
            let writer = scope.spawn(|| -> Result<(), SaveError> {
                let _ready = barrier.wait();
                publish_replacements(&store, &first, &second)
            });
            let _ready = barrier.wait();
            for _index in 0u8..128 {
                let payload = store.load(0)?;
                require(payload == first || payload == second)?;
            }
            writer
                .join()
                .map_err(|_error| SaveError::Io(io::ErrorKind::Other))?
        })?;
        require(store.load(0)? == first)?;
        require(fs::read_dir(&root.path)?.count() == 1)
    }

    #[test]
    fn concurrent_publishers_leave_one_complete_durable_envelope()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let maximum = NonZeroU64::new(4096).unwrap_or(NonZeroU64::MIN);
        let store = SaveStore::open(&root.path, maximum)?;
        let first = [0x12; 4096];
        let second = [0x34; 4096];
        require(store.store(0, b"previous")?.durability.is_ok())?;
        let barrier = Barrier::new(2);
        thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let _ready = barrier.wait();
                store.store(0, &first)
            });
            let _ready = barrier.wait();
            let receipt = store.store(0, &second);
            let other = writer
                .join()
                .map_err(|_error| SaveError::Io(io::ErrorKind::Other))??;
            require(other.durability.is_ok() && receipt?.durability.is_ok())
        })?;
        let payload = store.load(0)?;
        require(payload == first || payload == second)?;
        let bytes = fs::read(root.path.join(slot_name(0)))?;
        require(decode_save(&bytes, maximum)? == payload)?;
        require(fs::read_dir(&root.path)?.count() == 1)
    }

    #[test]
    fn stores_portable_envelopes_and_preserves_previous_save_on_rejection()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let maximum = NonZeroU64::new(4).unwrap_or(NonZeroU64::MIN);
        let store = SaveStore::open(&root.path, maximum)?;
        require(store.store(0, b"four")?.durability.is_ok())?;
        let slot = root.path.join(slot_name(0));
        let original = fs::read(&slot)?;
        require(decode_save(&original, maximum)? == b"four")?;
        require(original == encode_save(b"four", maximum)?)?;
        require(fs::metadata(&slot)?.permissions().mode() & 0o777 == 0o600)?;
        require(store.load(0)? == b"four")?;
        require(
            store.store(0, b"fives")
                == Err(SaveError::Envelope(SafetyError::TooLarge)),
        )?;
        require(fs::read(&slot)? == original)?;
        require(store.store(0, b"")?.durability.is_ok())?;
        require(store.load(0)?.is_empty())?;
        require(fs::read(&slot)? == encode_save(b"", maximum)?)?;
        require(store.store(u32::MAX, b"next")?.durability.is_ok())?;
        require(store.load(u32::MAX)? == b"next")?;
        require(fs::read_dir(&root.path)?.count() == 2)
    }

    #[test]
    fn rejects_corruption_and_linked_or_special_slots() -> Result<(), SaveError>
    {
        let root = TestRoot::new()?;
        let store = SaveStore::open(&root.path, NonZeroU64::MIN)?;
        require(store.store(1, b"x")?.durability.is_ok())?;
        let slot = root.path.join(slot_name(1));
        let linked = root.path.join(slot_name(2));
        let hard = root.path.join(slot_name(3));
        let fifo = root.path.join(slot_name(4));
        symlink(&slot, &linked)?;
        fs::hard_link(&slot, &hard)?;
        mkfifo(&fifo, Mode::S_IRUSR | Mode::S_IWUSR)?;
        for id in [1, 2, 3, 4] {
            require(store.load(id).is_err())?;
            require(store.store(id, b"y") == Err(SaveError::UnsafeStorage))?;
        }
        fs::remove_file(hard)?;
        fs::write(&slot, b"invalid")?;
        require(store.load(1).is_err())?;
        require(store.store(1, b"z")?.durability.is_ok())?;
        require(store.load(1)? == b"z")
    }

    #[test]
    fn rejects_untrusted_directory_and_cleans_unpublished_file()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let store = SaveStore::open(&root.path, NonZeroU64::MIN)?;
        let directory = &store.directory;
        let pending = PendingSave::create(directory)?;
        let path = root.path.join(&pending.name);
        require(path.exists())?;
        drop(pending);
        require(!path.exists())?;
        fs::set_permissions(&root.path, fs::Permissions::from_mode(0o777))?;
        require(matches!(
            SaveStore::open(&root.path, NonZeroU64::MIN),
            Err(SaveError::UnsafeStorage)
        ))?;
        require(store.store(0, b"x") == Err(SaveError::UnsafeStorage))?;
        fs::set_permissions(&root.path, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    #[test]
    fn reads_bound_the_envelope_and_reject_same_size_tampering()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let maximum = NonZeroU64::MIN;
        let store = SaveStore::open(&root.path, maximum)?;
        require(store.load(1).is_err())?;
        require(store.store(1, b"x")?.durability.is_ok())?;
        let slot = root.path.join(slot_name(1));
        let mut altered = fs::read(&slot)?;
        let last = altered.last_mut().ok_or(SafetyError::InvalidSave)?;
        *last = b'y';
        fs::write(&slot, &altered)?;
        require(
            store.load(1) == Err(SaveError::Envelope(SafetyError::InvalidSave)),
        )?;
        altered.push(0);
        fs::write(&slot, &altered)?;
        require(store.load(1) == Err(SaveError::Read(FileReadError::TooLarge)))
    }

    #[test]
    fn directory_handle_does_not_follow_a_replaced_path()
    -> Result<(), SaveError> {
        let root = TestRoot::new()?;
        let directory = root.path.join("saves");
        let moved = root.path.join("moved");
        fs::create_dir_all(&directory)?;
        let store = SaveStore::open(&directory, NonZeroU64::MIN)?;
        require(store.store(0, b"a")?.durability.is_ok())?;
        fs::rename(&directory, &moved)?;
        fs::create_dir_all(&directory)?;
        require(store.load(0)? == b"a")?;
        require(store.store(0, b"b")?.durability.is_ok())?;
        require(store.load(0)? == b"b")?;
        require(!directory.join(slot_name(0)).exists())?;
        require(
            decode_save(&fs::read(moved.join(slot_name(0)))?, NonZeroU64::MIN)?
                == b"b",
        )
    }
}
