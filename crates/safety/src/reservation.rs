//! Durable generation reservation for host trusted startup.
//!
//! Re-exports the portable `neuradix_command_core::reservation` allocator and,
//! on Unix, supplies [`FileReservationStore`]: a two-file store for one
//! `(receiver, binding)` key in an existing, owner-only state directory.
//!
//! Claimed for Linux on a local filesystem: per-slot atomic replacement
//! (temporary file, `fsync`, `rename`, directory `fsync`), exclusive advisory
//! locking across handles and processes, directory identity re-verification
//! before every operation, and permanent poisoning of the handle on any write-path
//! error. Durability after `Ok` relies on the filesystem and device honouring
//! cache flushes; it is argued by design, not power-cut tested. Network, FUSE,
//! tmpfs and overlay filesystems, snapshots or restored copies of the directory,
//! writers that bypass the lock, and privileged administrators are outside the
//! claim (see WP-A04.4).

pub use neuradix_command_core::reservation::*;

#[cfg(unix)]
pub use file_store::{FileReservationStore, FileStoreError};

#[cfg(unix)]
mod file_store {
    use std::fs::{self, File, OpenOptions, TryLockError};
    use std::io::{self, ErrorKind, Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use super::{RECORD_BYTES, ReservationStore, Slot, StoreError};

    /// Why a state directory was refused.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    pub enum FileStoreError {
        /// Not a real directory (a symlink is refused).
        #[error("reservation state path is not a directory")]
        NotADirectory,
        /// The directory grants any group or other permission bit.
        #[error("reservation state directory has group/other permissions (mode {mode:o})")]
        InsecurePermissions {
            /// Permission bits found.
            mode: u32,
        },
        /// The parent is group- or world-writable without the sticky bit.
        #[error("parent of reservation state directory is writable by others (mode {mode:o})")]
        InsecureParent {
            /// Permission bits found.
            mode: u32,
        },
        /// The directory was replaced between checks.
        #[error("reservation state directory was replaced")]
        Replaced,
        /// Another handle holds the store lock.
        #[error("reservation state directory is locked by another handle")]
        Locked,
        /// Other I/O failure, including a missing directory.
        #[error("reservation state I/O error: {0:?}")]
        Io(ErrorKind),
    }

    /// Internal write steps, for the unit-test fault hook.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Step {
        CreateTemp,
        WriteTemp,
        SyncTemp,
        Rename,
        SyncDir,
    }

    #[cfg(test)]
    thread_local! {
        /// Perform the named step, then fail it with an injected error.
        pub(crate) static FAIL_AFTER: std::cell::Cell<Option<Step>> =
            const { std::cell::Cell::new(None) };
    }

    fn injected(_step: Step) -> io::Result<()> {
        #[cfg(test)]
        if FAIL_AFTER.with(|hook| hook.get() == Some(_step)) {
            FAIL_AFTER.with(|hook| hook.set(None));
            return Err(io::Error::other("injected reservation store fault"));
        }
        Ok(())
    }

    const LOCK: &str = "lock";
    /// Fixed pattern for a slot file of the wrong length; decodes as Corrupt.
    const DAMAGED: [u8; RECORD_BYTES] = {
        let mut bytes = [0u8; RECORD_BYTES];
        bytes[0] = 0xDA;
        bytes
    };

    fn slot_name(slot: Slot) -> &'static str {
        match slot {
            Slot::A => "slot-a",
            Slot::B => "slot-b",
        }
    }

    fn identity(meta: &fs::Metadata) -> (u64, u64) {
        (meta.dev(), meta.ino())
    }

    /// Two 64-byte slot files (`slot-a`, `slot-b`, mode 0600) and a `lock` file in
    /// one existing directory with no group or other permission bits, owned by the
    /// service user and created by provisioning tooling (never by `open`).
    ///
    /// Not `Clone`. Any write-path error poisons the handle permanently: later
    /// reads return `Unavailable`, so the reserver reports `ReopenStore` and
    /// recovery drops this handle and opens a new one.
    #[derive(Debug)]
    pub struct FileReservationStore {
        dir_path: PathBuf,
        dir: File,
        dir_id: (u64, u64),
        _lock: File,
        poisoned: bool,
        last_io_error: Option<ErrorKind>,
    }

    impl FileReservationStore {
        /// Open an existing state directory and take its exclusive lock.
        ///
        /// Refuses a missing directory (`Io(NotFound)`), a non-directory or symlink,
        /// any group or other permission bit, a parent that is group- or
        /// world-writable without the sticky bit, a directory replaced during the
        /// checks, and a directory locked by another handle. Removes stale
        /// temporary files best-effort. Never creates the directory.
        pub fn open(dir: &Path) -> Result<Self, FileStoreError> {
            let io = |e: io::Error| FileStoreError::Io(e.kind());
            let meta = fs::symlink_metadata(dir).map_err(io)?;
            if !meta.file_type().is_dir() {
                return Err(FileStoreError::NotADirectory);
            }
            let mode = meta.mode() & 0o7777;
            if mode & 0o077 != 0 {
                return Err(FileStoreError::InsecurePermissions { mode });
            }
            let parent = match dir.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let parent_mode = fs::metadata(parent).map_err(io)?.mode() & 0o7777;
            if parent_mode & 0o022 != 0 && parent_mode & 0o1000 == 0 {
                return Err(FileStoreError::InsecureParent { mode: parent_mode });
            }
            let handle = File::open(dir).map_err(io)?;
            let dir_id = identity(&handle.metadata().map_err(io)?);
            if dir_id != identity(&meta) {
                return Err(FileStoreError::Replaced);
            }
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(dir.join(LOCK))
                .map_err(io)?;
            match lock.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Err(FileStoreError::Locked),
                Err(TryLockError::Error(e)) => return Err(io(e)),
            }
            if identity(&fs::symlink_metadata(dir).map_err(io)?) != dir_id {
                return Err(FileStoreError::Replaced);
            }
            for slot in [Slot::A, Slot::B] {
                let _ = fs::remove_file(dir.join(format!("{}.tmp", slot_name(slot))));
            }
            Ok(Self {
                dir_path: dir.to_path_buf(),
                dir: handle,
                dir_id,
                _lock: lock,
                poisoned: false,
                last_io_error: None,
            })
        }

        /// Whether a write-path error or identity change poisoned this handle.
        pub fn is_poisoned(&self) -> bool {
            self.poisoned
        }

        /// Kind of the most recent I/O error, for diagnostics.
        pub fn last_io_error(&self) -> Option<ErrorKind> {
            self.last_io_error
        }

        fn same_directory(&self) -> bool {
            fs::symlink_metadata(&self.dir_path).is_ok_and(|m| identity(&m) == self.dir_id)
        }

        fn read_slot(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> io::Result<()> {
            let path = self.dir_path.join(slot_name(slot));
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    *buf = [0xFF; RECORD_BYTES];
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if !meta.file_type().is_file() {
                return Err(io::Error::other("slot is not a regular file"));
            }
            let mut file = File::open(&path)?;
            // Bounded: never read more than one byte past a record.
            let mut staging = [0u8; RECORD_BYTES + 1];
            let mut filled = 0;
            while filled < staging.len() {
                match file.read(&mut staging[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            if filled == RECORD_BYTES {
                buf.copy_from_slice(&staging[..RECORD_BYTES]);
            } else {
                *buf = DAMAGED;
            }
            Ok(())
        }

        fn write_slot(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> io::Result<()> {
            let name = slot_name(slot);
            let tmp = self.dir_path.join(format!("{name}.tmp"));
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            injected(Step::CreateTemp)?;
            file.write_all(record)?;
            injected(Step::WriteTemp)?;
            file.sync_all()?;
            injected(Step::SyncTemp)?;
            drop(file);
            fs::rename(&tmp, self.dir_path.join(name))?;
            injected(Step::Rename)?;
            if !self.same_directory() {
                return Err(io::Error::other("reservation directory replaced"));
            }
            self.dir.sync_all()?;
            injected(Step::SyncDir)
        }
    }

    impl ReservationStore for FileReservationStore {
        fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
            if self.poisoned {
                return Err(StoreError::Unavailable);
            }
            if !self.same_directory() {
                self.poisoned = true;
                return Err(StoreError::Unavailable);
            }
            self.read_slot(slot, buf).map_err(|e| {
                self.last_io_error = Some(e.kind());
                StoreError::Io
            })
        }

        fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
            if self.poisoned {
                return Err(StoreError::Unavailable);
            }
            // After any failure the page cache is no longer a durability witness
            // (fsyncgate): poison the handle permanently.
            let result = if self.same_directory() {
                self.write_slot(slot, record)
            } else {
                Err(io::Error::other("reservation directory replaced"))
            };
            result.map_err(|e| {
                self.poisoned = true;
                self.last_io_error = Some(e.kind());
                StoreError::Io
            })
        }
    }
}
