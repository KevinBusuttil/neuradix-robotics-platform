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
    use std::path::{Component, Path, PathBuf};

    use super::{RECORD_BYTES, ReservationStore, Slot, StoreError};

    /// Why a state directory was refused.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    pub enum FileStoreError {
        /// Not a real directory (a symlink is refused).
        #[error("reservation state path is not a directory")]
        NotADirectory,
        /// The path is not in canonical spelling: a trailing `/`, a `.` or `..`
        /// component, repeated separators, or a final component that is not a
        /// plain name. Such spellings make `lstat` follow a final symlink or make
        /// the parent check inspect the wrong directory.
        #[error("reservation state path must be canonically spelled")]
        NonCanonicalPath,
        /// `lock` exists but is not a regular file (for example a symlink).
        #[error("reservation lock entry is not a regular file")]
        UnexpectedEntry,
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

    /// Test-only action run after a write step: the step and a function of the
    /// state directory path.
    #[cfg(test)]
    pub(crate) type StepAction = (Step, fn(&Path));

    #[cfg(test)]
    thread_local! {
        /// Perform the named step, then fail it with an injected error.
        pub(crate) static FAIL_AFTER: std::cell::Cell<Option<Step>> =
            const { std::cell::Cell::new(None) };
        /// Perform the named step, then run an action on the state directory
        /// path (for example replacing it) and continue normally.
        pub(crate) static ACT_AFTER: std::cell::Cell<Option<StepAction>> =
            const { std::cell::Cell::new(None) };
    }

    fn injected(_step: Step, _dir: &Path) -> io::Result<()> {
        #[cfg(test)]
        {
            if let Some((step, action)) = ACT_AFTER.with(|hook| hook.get())
                && step == _step
            {
                ACT_AFTER.with(|hook| hook.set(None));
                action(_dir);
            }
            if FAIL_AFTER.with(|hook| hook.get() == Some(_step)) {
                FAIL_AFTER.with(|hook| hook.set(None));
                return Err(io::Error::other("injected reservation store fault"));
            }
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

    /// Plain-name final component, no `.`/`..` components, and no spelling that
    /// path normalization would change (trailing `/`, `/.`, `//`).
    fn canonical_spelling(dir: &Path) -> bool {
        let rebuilt: PathBuf = dir.components().collect();
        rebuilt.as_os_str() == dir.as_os_str()
            && !dir.components().any(|c| c == Component::ParentDir)
            && matches!(dir.components().next_back(), Some(Component::Normal(_)))
    }

    /// Open the lock file without ever following a symlink: an existing entry
    /// must be a regular file and must be the file actually opened; a missing
    /// entry is created exclusively (`O_CREAT | O_EXCL` never follows links).
    fn open_lock(path: &Path) -> Result<File, FileStoreError> {
        let io = |e: io::Error| FileStoreError::Io(e.kind());
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                if !meta.file_type().is_file() {
                    return Err(FileStoreError::UnexpectedEntry);
                }
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(io)?;
                if identity(&file.metadata().map_err(io)?) != identity(&meta) {
                    return Err(FileStoreError::Replaced);
                }
                Ok(file)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(io),
            Err(e) => Err(io(e)),
        }
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
        /// Refuses a non-canonically spelled path (trailing `/`, `.` or `..`
        /// components, repeated separators), a missing directory (`Io(NotFound)`),
        /// a non-directory or symlink,
        /// any group or other permission bit, a parent that is group- or
        /// world-writable without the sticky bit, a directory replaced during the
        /// checks, and a directory locked by another handle. Removes stale
        /// temporary files best-effort. Never creates the directory.
        pub fn open(dir: &Path) -> Result<Self, FileStoreError> {
            let io = |e: io::Error| FileStoreError::Io(e.kind());
            if !canonical_spelling(dir) {
                return Err(FileStoreError::NonCanonicalPath);
            }
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
            let lock = open_lock(&dir.join(LOCK))?;
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
            let opened = file.metadata()?;
            if !opened.file_type().is_file() || identity(&opened) != identity(&meta) {
                return Err(io::Error::other("slot entry changed while opening"));
            }
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
            // Never follow a planted link: unlink any entry, then create
            // exclusively (`O_CREAT | O_EXCL` does not follow symlinks).
            let _ = fs::remove_file(&tmp);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            injected(Step::CreateTemp, &self.dir_path)?;
            file.write_all(record)?;
            injected(Step::WriteTemp, &self.dir_path)?;
            file.sync_all()?;
            injected(Step::SyncTemp, &self.dir_path)?;
            drop(file);
            fs::rename(&tmp, self.dir_path.join(name))?;
            injected(Step::Rename, &self.dir_path)?;
            if !self.same_directory() {
                return Err(io::Error::other("reservation directory replaced"));
            }
            self.dir.sync_all()?;
            injected(Step::SyncDir, &self.dir_path)
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

/// S1–S5: host store unit tests. Each test uses its own owner-only scratch root
/// under the system temporary directory; the state directory sits inside it, so
/// its parent is never group- or world-writable. The step hook is thread-local.
#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::io::ErrorKind;
    use std::num::NonZeroU32;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::file_store::{ACT_AFTER, FAIL_AFTER, Step};
    use super::record::{self, RecordFields, SlotView};
    use super::*;
    use crate::actuator::ExecutionMode;

    /// The fixed pattern a wrongly sized slot file reads as.
    const DAMAGED_HEAD: u8 = 0xDA;

    /// Unique scratch root (mode 0700) holding the state directory `state` (0700).
    struct Scratch {
        root: PathBuf,
    }
    impl Scratch {
        fn new(test: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "neuradix-reservation-{test}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            owner_only_dir(&root);
            let scratch = Self { root };
            owner_only_dir(&scratch.state());
            scratch
        }
        fn state(&self) -> PathBuf {
            self.root.join("state")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn owner_only_dir(path: &Path) {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .expect("create directory");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("chmod 0700");
    }

    fn key() -> ReservationKey {
        ReservationKey::new(
            ReceiverId::new(*b"neuradix-host-01").expect("receiver"),
            BindingKey::named("controller", "thrust", "driver/one", ExecutionMode::Live),
        )
    }

    fn epoch(value: u64) -> NamespaceEpoch {
        NamespaceEpoch::new(value).expect("nonzero epoch")
    }

    fn config(window: u32) -> ReserverConfig {
        ReserverConfig::new(
            NonZeroU32::new(window).expect("window"),
            RollbackDefense::Unprotected,
        )
    }

    const fn value(epoch: u64, counter: u64) -> u128 {
        ((epoch as u128) << 64) | counter as u128
    }

    fn slot_path(dir: &Path, slot: Slot) -> PathBuf {
        dir.join(match slot {
            Slot::A => "slot-a",
            Slot::B => "slot-b",
        })
    }

    fn tmp_path(dir: &Path, slot: Slot) -> PathBuf {
        dir.join(match slot {
            Slot::A => "slot-a.tmp",
            Slot::B => "slot-b.tmp",
        })
    }

    fn record_for(slot: Slot, high_water: u64, commits: u32) -> [u8; RECORD_BYTES] {
        record::encode(&RecordFields {
            slot,
            key: key(),
            epoch: epoch(1),
            high_water,
            commits,
        })
    }

    /// Open the directory and provision epoch 1 through a handle that is dropped.
    fn provisioned(dir: &Path) {
        let mut store = FileReservationStore::open(dir).expect("open");
        provision(&mut store, key(), epoch(1), ProvisionGuards::default()).expect("provision");
    }

    fn read_slot(store: &mut FileReservationStore, slot: Slot) -> [u8; RECORD_BYTES] {
        let mut buf = [0u8; RECORD_BYTES];
        store.read(slot, &mut buf).expect("slot read");
        buf
    }

    fn is_damaged_pattern(buf: &[u8; RECORD_BYTES]) -> bool {
        buf[0] == DAMAGED_HEAD && buf[1..].iter().all(|&b| b == 0)
    }

    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read_dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn s1_fail_after_every_write_step() {
        for step in [
            Step::CreateTemp,
            Step::WriteTemp,
            Step::SyncTemp,
            Step::Rename,
            Step::SyncDir,
        ] {
            let scratch = Scratch::new("s1");
            let dir = scratch.state();
            provisioned(&dir);
            let mut store = FileReservationStore::open(&dir).expect("open");
            let mut returned = Vec::new();
            let (old_a, old_b) = {
                let mut reserver = GenerationReserver::open(&mut store, key(), config(4))
                    .map_err(|f| f.error)
                    .expect("open reserver");
                // First reserve commits ceiling 4 (commits 2); three more from the window.
                for _ in 0..4 {
                    returned.push(reserver.reserve().expect("reserve").generation().get());
                }
                assert_eq!(reserver.status().window_remaining, 0);
                let old_a = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
                let old_b = fs::read(slot_path(&dir, Slot::B)).expect("slot-b");
                assert_eq!(old_a, record_for(Slot::A, 4, 2));
                assert_eq!(old_b, record_for(Slot::B, 4, 2));

                // The next reserve commits ceiling 8; both slots are current, so A
                // is written first and the hook fails it after `step` took effect.
                FAIL_AFTER.with(|hook| hook.set(Some(step)));
                assert_eq!(
                    reserver.reserve().map(|t| t.generation()).unwrap_err(),
                    ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::A)),
                    "{step:?}"
                );
                assert_eq!(FAIL_AFTER.with(|hook| hook.get()), None, "{step:?} fired");
                let poisoned = ReserveError::Poisoned(PoisonCause::WriteFailed(Slot::A));
                assert_eq!(reserver.reserve().map(|t| t.generation()), Err(poisoned));
                assert_eq!(
                    reserver.reserve_from_window().map(|t| t.generation()),
                    Err(poisoned)
                );
                assert_eq!(poisoned.remedy(), Remedy::ReopenStore);
                (old_a, old_b)
            };

            // The handle is poisoned permanently: reads are Unavailable.
            assert!(store.is_poisoned(), "{step:?}");
            assert_eq!(store.last_io_error(), Some(ErrorKind::Other), "{step:?}");
            let mut buf = [0u8; RECORD_BYTES];
            assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Unavailable));
            assert_eq!(store.read(Slot::B, &mut buf), Err(StoreError::Unavailable));
            let probe = record_for(Slot::B, 99, 9);
            assert_eq!(store.write(Slot::B, &probe), Err(StoreError::Unavailable));
            let refused = GenerationReserver::open(&mut store, key(), config(4))
                .map(|_| ())
                .unwrap_err();
            assert_eq!(refused.error, OpenError::StoreUnavailable);
            assert_eq!(refused.error.remedy(), Remedy::ReopenStore);

            // On-disk state after the step (host_store table).
            let new_a = record_for(Slot::A, 8, 3);
            let slot_a = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
            let tmp_a = fs::read(tmp_path(&dir, Slot::A));
            match step {
                Step::CreateTemp => {
                    assert_eq!(tmp_a.expect("empty tmp"), Vec::<u8>::new());
                    assert_eq!(slot_a, old_a);
                }
                Step::WriteTemp | Step::SyncTemp => {
                    assert_eq!(tmp_a.expect("tmp holds the new record"), new_a);
                    assert_eq!(slot_a, old_a);
                }
                Step::Rename | Step::SyncDir => {
                    assert_eq!(
                        tmp_a.map(|_| ()).unwrap_err().kind(),
                        ErrorKind::NotFound,
                        "{step:?}: no tmp"
                    );
                    assert_eq!(slot_a, new_a);
                }
            }
            assert_eq!(fs::read(slot_path(&dir, Slot::B)).expect("slot-b"), old_b);
            assert!(!tmp_path(&dir, Slot::B).exists());
            drop(store);

            // A fresh handle (which clears the stale tmp) and reserver never reuse.
            let mut fresh = FileReservationStore::open(&dir).expect("fresh handle");
            assert!(
                !tmp_path(&dir, Slot::A).exists(),
                "{step:?}: stale tmp removed"
            );
            let mut reserver = GenerationReserver::open(&mut fresh, key(), config(4))
                .map_err(|f| f.error)
                .expect("reopen");
            let max = *returned.iter().max().expect("returned values");
            let expected_first = match step {
                Step::Rename | Step::SyncDir => value(1, 9),
                _ => value(1, 5),
            };
            let mut last = max;
            for i in 0..6 {
                let g = reserver.reserve().expect("reserve").generation().get();
                if i == 0 {
                    assert_eq!(g, expected_first, "{step:?}");
                }
                assert!(g > last, "{step:?}: {g:#x} after {last:#x}");
                last = g;
            }
        }
    }

    /// S1 at the trait level: the store's own `write` returns exactly `Err(Io)`
    /// for every step (on slot B, the second-written slot of a commit), poisons the
    /// handle, and leaves the host_store table's on-disk state; a fresh handle
    /// clears the tmp file and reads the slot the step left behind.
    #[test]
    fn s1_store_write_err_io_after_every_step() {
        for step in [
            Step::CreateTemp,
            Step::WriteTemp,
            Step::SyncTemp,
            Step::Rename,
            Step::SyncDir,
        ] {
            let scratch = Scratch::new("s1-store");
            let dir = scratch.state();
            provisioned(&dir);
            let old_a = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
            let old_b = fs::read(slot_path(&dir, Slot::B)).expect("slot-b");
            assert_eq!(old_b, record_for(Slot::B, 0, 1));
            let new_b = record_for(Slot::B, 7, 2);

            let mut store = FileReservationStore::open(&dir).expect("open");
            FAIL_AFTER.with(|hook| hook.set(Some(step)));
            assert_eq!(
                store.write(Slot::B, &new_b),
                Err(StoreError::Io),
                "{step:?}"
            );
            assert_eq!(FAIL_AFTER.with(|hook| hook.get()), None, "{step:?} fired");
            assert!(store.is_poisoned(), "{step:?}");
            assert_eq!(store.last_io_error(), Some(ErrorKind::Other), "{step:?}");
            let mut buf = [0u8; RECORD_BYTES];
            for slot in [Slot::A, Slot::B] {
                assert_eq!(
                    store.read(slot, &mut buf),
                    Err(StoreError::Unavailable),
                    "{step:?} {slot:?}"
                );
            }
            assert_eq!(
                store.write(Slot::A, &record_for(Slot::A, 7, 2)),
                Err(StoreError::Unavailable),
                "{step:?}: a poisoned handle never writes again"
            );

            let slot_b = fs::read(slot_path(&dir, Slot::B)).expect("slot-b");
            let tmp_b = fs::read(tmp_path(&dir, Slot::B));
            let expected_b: Vec<u8> = match step {
                Step::CreateTemp => {
                    assert_eq!(tmp_b.expect("empty tmp"), Vec::<u8>::new());
                    old_b.clone()
                }
                Step::WriteTemp | Step::SyncTemp => {
                    assert_eq!(tmp_b.expect("tmp holds the new record"), new_b);
                    old_b.clone()
                }
                Step::Rename | Step::SyncDir => {
                    assert_eq!(
                        tmp_b.map(|_| ()).unwrap_err().kind(),
                        ErrorKind::NotFound,
                        "{step:?}: no tmp"
                    );
                    new_b.to_vec()
                }
            };
            assert_eq!(slot_b, expected_b, "{step:?}");
            assert_eq!(fs::read(slot_path(&dir, Slot::A)).expect("slot-a"), old_a);
            assert!(!tmp_path(&dir, Slot::A).exists(), "{step:?}");
            drop(store);

            let mut fresh = FileReservationStore::open(&dir).expect("fresh handle");
            assert_eq!(dir_entries(&dir), ["lock", "slot-a", "slot-b"], "{step:?}");
            assert_eq!(read_slot(&mut fresh, Slot::B).to_vec(), expected_b);
            assert!(!fresh.is_poisoned());
        }
    }

    #[test]
    fn s2_one_mebibyte_slot_file_is_bounded_and_corrupt() {
        let scratch = Scratch::new("s2");
        let dir = scratch.state();
        provisioned(&dir);
        // A valid record followed by 1 MiB of padding: only the length matters.
        let mut big = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
        big.resize(1 << 20, 0);
        fs::write(slot_path(&dir, Slot::A), &big).expect("write 1 MiB");

        let mut store = FileReservationStore::open(&dir).expect("open");
        let buf = read_slot(&mut store, Slot::A);
        assert!(is_damaged_pattern(&buf));
        assert_eq!(record::decode(&buf), SlotView::Corrupt);
        assert_eq!(
            fs::metadata(slot_path(&dir, Slot::A)).expect("meta").len(),
            1 << 20,
            "reads never modify the file"
        );
        {
            // Tolerated beside a valid B; the next commit replaces it with 64 bytes.
            let mut reserver = GenerationReserver::open(&mut store, key(), config(2))
                .map_err(|f| f.error)
                .expect("open reserver");
            assert_eq!(
                reserver.status().slots,
                [SlotCondition::Corrupt, SlotCondition::Current]
            );
            let token = reserver.reserve().expect("reserve");
            assert_eq!(token.generation().get(), value(1, 1));
        }
        assert_eq!(
            fs::read(slot_path(&dir, Slot::A)).expect("a"),
            record_for(Slot::A, 2, 2)
        );

        // Both slots oversized: Corrupt, no writes.
        fs::write(slot_path(&dir, Slot::A), &big).expect("write 1 MiB");
        fs::write(slot_path(&dir, Slot::B), &big).expect("write 1 MiB");
        let refused = GenerationReserver::open(&mut store, key(), config(2))
            .map(|_| ())
            .unwrap_err();
        assert_eq!(refused.error, OpenError::Corrupt);
        for slot in [Slot::A, Slot::B] {
            assert_eq!(
                fs::metadata(slot_path(&dir, slot)).expect("m").len(),
                1 << 20
            );
        }
    }

    #[test]
    fn s3_truncated_or_long_files_corrupt_missing_file_blank() {
        let scratch = Scratch::new("s3");
        let dir = scratch.state();
        provisioned(&dir);
        let valid = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
        assert_eq!(valid.len(), RECORD_BYTES);
        fs::remove_file(slot_path(&dir, Slot::B)).expect("remove slot-b");
        let mut store = FileReservationStore::open(&dir).expect("open");

        for len in [0usize, 40, 63, 65] {
            let mut bytes = valid.clone();
            bytes.resize(len, 0x00);
            fs::write(slot_path(&dir, Slot::A), &bytes).expect("write");
            let buf = read_slot(&mut store, Slot::A);
            assert!(is_damaged_pattern(&buf), "len {len}");
            assert_eq!(record::decode(&buf), SlotView::Corrupt, "len {len}");
            // Beside a missing B: Corrupt (not Blank), and nothing is written.
            let refused = GenerationReserver::open(&mut store, key(), config(1))
                .map(|_| ())
                .unwrap_err();
            assert_eq!(refused.error, OpenError::Corrupt, "len {len}");
            assert_eq!(fs::read(slot_path(&dir, Slot::A)).expect("a"), bytes);
            assert!(!slot_path(&dir, Slot::B).exists());
        }

        // A missing file reads as erased (Blank); both missing → Blank.
        let buf = read_slot(&mut store, Slot::B);
        assert_eq!(buf, [0xFF; RECORD_BYTES]);
        assert_eq!(record::decode(&buf), SlotView::Blank);
        fs::remove_file(slot_path(&dir, Slot::A)).expect("remove slot-a");
        let refused = GenerationReserver::open(&mut store, key(), config(1))
            .map(|_| ())
            .unwrap_err();
        assert_eq!(refused.error, OpenError::Blank);
        assert_eq!(dir_entries(&dir), ["lock"]);
        assert!(!store.is_poisoned());
    }

    #[test]
    fn s4_symlinked_slot_file_is_io() {
        let scratch = Scratch::new("s4");
        let dir = scratch.state();
        provisioned(&dir);
        let elsewhere = scratch.root.join("elsewhere");
        let valid_a = fs::read(slot_path(&dir, Slot::A)).expect("slot-a");
        fs::write(&elsewhere, &valid_a).expect("write target");
        fs::remove_file(slot_path(&dir, Slot::A)).expect("remove slot-a");
        symlink(&elsewhere, slot_path(&dir, Slot::A)).expect("symlink");

        let mut store = FileReservationStore::open(&dir).expect("open");
        let mut buf = [0u8; RECORD_BYTES];
        assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Io));
        assert_eq!(store.last_io_error(), Some(ErrorKind::Other));
        assert!(
            !store.is_poisoned(),
            "a read error does not poison the handle"
        );
        {
            // A is Unreadable beside a valid B; the commit writes A first and
            // replaces the link itself, never following it.
            let mut reserver = GenerationReserver::open(&mut store, key(), config(1))
                .map_err(|f| f.error)
                .expect("open reserver");
            assert_eq!(
                reserver.status().slots,
                [SlotCondition::Unreadable, SlotCondition::Current]
            );
            let token = reserver.reserve().expect("reserve");
            assert_eq!(token.generation().get(), value(1, 1));
        }
        let meta = fs::symlink_metadata(slot_path(&dir, Slot::A)).expect("meta");
        assert!(
            meta.file_type().is_file(),
            "link replaced by a regular file"
        );
        assert_eq!(
            fs::read(&elsewhere).expect("target"),
            valid_a,
            "target untouched"
        );

        // A symlinked slot beside a missing slot: Unreadable (RetryOpen), no writes.
        fs::remove_file(slot_path(&dir, Slot::A)).expect("remove");
        symlink(&elsewhere, slot_path(&dir, Slot::A)).expect("symlink");
        fs::remove_file(slot_path(&dir, Slot::B)).expect("remove slot-b");
        let refused = GenerationReserver::open(&mut store, key(), config(1))
            .map(|_| ())
            .unwrap_err();
        assert_eq!(refused.error, OpenError::Unreadable);
        assert_eq!(refused.error.remedy(), Remedy::RetryOpen);
        assert!(!slot_path(&dir, Slot::B).exists());
    }

    #[test]
    fn s5_directory_renamed_and_replaced_after_open() {
        // (a) the next call is a read; (b) the next call is a write.
        for read_first in [true, false] {
            let scratch = Scratch::new("s5");
            let dir = scratch.state();
            provisioned(&dir);
            let before = [
                fs::read(slot_path(&dir, Slot::A)).expect("a"),
                fs::read(slot_path(&dir, Slot::B)).expect("b"),
            ];
            let mut store = FileReservationStore::open(&dir).expect("open");
            let moved = scratch.root.join("state-moved");
            fs::rename(&dir, &moved).expect("rename directory");
            owner_only_dir(&dir);

            let mut buf = [0u8; RECORD_BYTES];
            let probe = record_for(Slot::A, 50, 5);
            if read_first {
                assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Unavailable));
                assert!(store.is_poisoned());
                assert_eq!(store.write(Slot::A, &probe), Err(StoreError::Unavailable));
            } else {
                assert_eq!(store.write(Slot::A, &probe), Err(StoreError::Io));
                assert!(store.is_poisoned());
                assert_eq!(store.last_io_error(), Some(ErrorKind::Other));
                assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Unavailable));
            }
            let refused = GenerationReserver::open(&mut store, key(), config(1))
                .map(|_| ())
                .unwrap_err();
            assert_eq!(refused.error, OpenError::StoreUnavailable);

            // Nothing was created in the replacement; the original is unchanged.
            assert!(dir_entries(&dir).is_empty(), "replacement untouched");
            assert_eq!(dir_entries(&moved), ["lock", "slot-a", "slot-b"]);
            assert_eq!(fs::read(slot_path(&moved, Slot::A)).expect("a"), before[0]);
            assert_eq!(fs::read(slot_path(&moved, Slot::B)).expect("b"), before[1]);
        }
    }

    /// Replace the state directory in place: move it aside, create a fresh
    /// owner-only directory at the same path.
    fn replace_dir(dir: &Path) {
        let aside = dir.with_file_name("state-moved");
        fs::rename(dir, &aside).expect("move state aside");
        owner_only_dir(dir);
    }

    /// S6: the directory is replaced after the rename, before the post-rename
    /// identity check. The write fails with `Io`, the handle is poisoned, the
    /// replacement directory receives nothing, and the directory sync never runs
    /// on the replacement.
    #[test]
    fn s6_directory_replaced_between_rename_and_identity_check() {
        let scratch = Scratch::new("s6");
        let mut store = FileReservationStore::open(&scratch.state()).expect("open");
        let record = record::encode(&RecordFields {
            slot: Slot::A,
            key: key(),
            epoch: epoch(1),
            high_water: 0,
            commits: 1,
        });
        ACT_AFTER.with(|hook| hook.set(Some((Step::Rename, replace_dir))));
        assert_eq!(store.write(Slot::A, &record), Err(StoreError::Io));
        assert!(ACT_AFTER.with(|hook| hook.get()).is_none(), "hook fired");
        assert!(store.is_poisoned());
        let mut buf = [0u8; RECORD_BYTES];
        assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Unavailable));
        let replacement: Vec<_> = fs::read_dir(scratch.state())
            .expect("list replacement")
            .collect();
        assert!(replacement.is_empty(), "nothing written to the replacement");
        let moved = scratch.root.join("state-moved");
        assert_eq!(
            fs::read(slot_path(&moved, Slot::A)).expect("renamed slot"),
            record.to_vec()
        );
    }

    /// S7: non-canonical spellings are refused before any filesystem check, so a
    /// trailing `/` or `/.` cannot make `lstat` follow a final symlink and `..`
    /// or `.` cannot redirect the parent check.
    #[test]
    fn s7_non_canonical_paths_refused() {
        let scratch = Scratch::new("s7");
        let state = scratch.state();
        fs::create_dir(state.join("sub")).expect("sub");
        let link = scratch.root.join("link");
        symlink(&state, &link).expect("symlink");
        assert_eq!(
            FileReservationStore::open(&link).unwrap_err(),
            FileStoreError::NotADirectory
        );
        let spellings = [
            PathBuf::from(format!("{}/", link.display())),
            link.join("."),
            PathBuf::from(format!("{}/.", link.display())),
            state.join("sub").join(".."),
            PathBuf::from(format!("{}//state", scratch.root.display())),
            PathBuf::from("."),
            PathBuf::from(".."),
            PathBuf::from("/"),
        ];
        for path in spellings {
            assert_eq!(
                FileReservationStore::open(&path).unwrap_err(),
                FileStoreError::NonCanonicalPath,
                "{}",
                path.display()
            );
        }
        assert!(!state.join("lock").exists(), "no lock created by refusals");
        FileReservationStore::open(&state).expect("canonical path opens");
    }

    /// S8: a planted `lock` symlink (dangling or not) is refused and never
    /// followed: its target is not created or opened.
    #[test]
    fn s8_lock_symlink_refused_not_followed() {
        let scratch = Scratch::new("s8");
        let state = scratch.state();
        let target = scratch.root.join("created-through-lock");
        symlink(&target, state.join("lock")).expect("plant lock link");
        assert_eq!(
            FileReservationStore::open(&state).unwrap_err(),
            FileStoreError::UnexpectedEntry
        );
        assert!(!target.exists(), "lock link target must not be created");
        fs::remove_file(state.join("lock")).expect("remove link");
        fs::create_dir(state.join("lock")).expect("lock directory");
        assert_eq!(
            FileReservationStore::open(&state).unwrap_err(),
            FileStoreError::UnexpectedEntry
        );
    }

    /// S9: a `slot-x.tmp` symlink planted after open is unlinked, not followed:
    /// the victim file keeps its content and the slot receives the record.
    #[test]
    fn s9_tmp_symlink_not_followed() {
        let scratch = Scratch::new("s9");
        let state = scratch.state();
        let mut store = FileReservationStore::open(&state).expect("open");
        let victim = scratch.root.join("victim");
        fs::write(&victim, b"victim content").expect("victim");
        symlink(&victim, tmp_path(&state, Slot::A)).expect("plant tmp link");
        let record = record::encode(&RecordFields {
            slot: Slot::A,
            key: key(),
            epoch: epoch(1),
            high_water: 0,
            commits: 1,
        });
        assert_eq!(store.write(Slot::A, &record), Ok(()));
        assert_eq!(fs::read(&victim).expect("victim"), b"victim content");
        let slot = slot_path(&state, Slot::A);
        assert!(
            fs::symlink_metadata(&slot)
                .expect("slot")
                .file_type()
                .is_file()
        );
        assert_eq!(fs::read(&slot).expect("slot"), record.to_vec());
        assert!(!store.is_poisoned());
    }
}
