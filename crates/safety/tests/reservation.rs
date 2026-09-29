//! Host generation-reservation scenarios H1–H14 (WP-A04.4).
//!
//! Every scenario runs in a child process of this test binary through
//! [`run_child`]: stdout is piped and drained by a reader thread (so a chatty
//! child can never deadlock on a full pipe), stdin is piped for the H3
//! handshake, and a 10 s deadline kills a hung child. A child prints and flushes
//! `NRX-GEN <decimal>` before activating each generation (and `NRX-LOCKED` in H3);
//! it simulates a crash with `std::process::exit(86)` at the fault point, which
//! keeps page-cache semantics identical to an abort without a core-dump handler.
//! Each scenario asserts its expected exit status.
//!
//! Each scenario owns `CARGO_TARGET_TMPDIR/reservation-<case>-<pid>` (removed
//! first, created 0700, removed afterwards). State directories are created inside
//! it, so their parent is never group- or world-writable. The suite runs as any
//! user including root: refusals are asserted from mode bits, never from denied
//! access.
#![cfg(unix)]

#[path = "../../command-core/tests/support/fault_store.rs"]
mod support;

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::num::NonZeroU32;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration as WallDuration, Instant};

use neuradix_safety::actuator::{
    ActuatorAdapter, ActuatorBinding, ActuatorConfig, ActuatorDriver, DriverError,
    DriverPermission, ExecutionMode, PermissionError, PermissionStatus,
};
use neuradix_safety::reservation::record::{self, RecordFields, SlotView};
use neuradix_safety::reservation::{
    BindingKey, FileReservationStore, FileStoreError, GenerationReserver, MAX_STORE_CALLS_PER_OPEN,
    MAX_STORE_CALLS_PER_RESERVE, NamespaceEpoch, OpenError, PoisonCause, ProvisionGuards,
    ProvisionReport, RECORD_BYTES, ReceiverId, Remedy, ReservationKey, ReservationStore,
    ReserveError, ReservedGeneration, ReserverConfig, RollbackDefense, Slot, SlotCondition,
    StoreError, provision,
};
use neuradix_safety::{
    AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Generation, Identity,
    Outcome, RejectReason, SessionConfig, SessionError, SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use support::{FaultStore, provisioned};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const CASE_ENV: &str = "NEURADIX_RESERVATION_CASE";
const DIR_ENV: &str = "NEURADIX_RESERVATION_DIR";
const EXIT_AFTER_ENV: &str = "NEURADIX_RESERVATION_EXIT_AFTER";
const REPLAY_ENV: &str = "NEURADIX_RESERVATION_REPLAY";
const GEN_MARKER: &str = "NRX-GEN ";
const LOCKED_MARKER: &str = "NRX-LOCKED";
/// Printed by the child dispatcher after its scenario returned normally, so a
/// child that ran no scenario at all (a renamed or filtered-out dispatcher test
/// exits 0 with "0 passed") can never count as a successful scenario.
const DONE_MARKER: &str = "NRX-DONE ";
/// Exit status of a child that simulates a crash at its fault point.
const FAULT_EXIT: i32 = 86;
const DEADLINE: WallDuration = WallDuration::from_secs(10);
const POLL: WallDuration = WallDuration::from_millis(5);
const SIGKILL: i32 = 9;

/// A running child scenario: `current_exe --exact reservation_child --nocapture`.
struct ChildRun {
    case: String,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    deadline: Instant,
    generations: Vec<u128>,
    transcript: Vec<String>,
    /// Whether the child printed `NRX-DONE <case>` for this case.
    done: bool,
}

impl ChildRun {
    fn spawn(case: &str, env: &[(&str, &OsStr)]) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        command
            .args(["--exact", "reservation_child", "--nocapture"])
            .env(CASE_ENV, case)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("spawn child scenario");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = mpsc::channel();
        // Drain stdout continuously so the child never blocks on a full pipe.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            case: case.to_owned(),
            stdin: child.stdin.take(),
            child,
            lines,
            deadline: Instant::now() + DEADLINE,
            generations: Vec::new(),
            transcript: Vec::new(),
            done: false,
        }
    }

    /// Record one stdout line. libtest may prefix a line, so markers are found
    /// anywhere in it.
    fn absorb(&mut self, line: String) {
        if let Some(at) = line.find(GEN_MARKER) {
            let digits: String = line[at + GEN_MARKER.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let value = digits
                .parse()
                .unwrap_or_else(|_| panic!("{}: malformed marker {line:?}", self.case));
            self.generations.push(value);
        }
        if let Some(at) = line.find(DONE_MARKER) {
            let case = line[at + DONE_MARKER.len()..].split_whitespace().next();
            assert_eq!(case, Some(self.case.as_str()), "{}: {line:?}", self.case);
            self.done = true;
        }
        self.transcript.push(line);
    }

    /// Block until a line containing `marker` arrives, within the deadline.
    fn wait_for(&mut self, marker: &str) {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    let found = line.contains(marker);
                    self.absorb(line);
                    if found {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.kill_now();
                    panic!(
                        "{}: no {marker} within {DEADLINE:?}; stdout {:?}",
                        self.case, self.transcript
                    );
                }
                Err(RecvTimeoutError::Disconnected) => panic!(
                    "{}: stdout closed before {marker}; stdout {:?}",
                    self.case, self.transcript
                ),
            }
        }
    }

    /// Release a child blocked on its stdin handshake (H3).
    fn release(&mut self) {
        let mut stdin = self.stdin.take().expect("stdin still open");
        writeln!(stdin, "release").expect("write release line");
        stdin.flush().expect("flush release line");
    }

    fn kill_now(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Kill the child (no release, no destructors) and reap it.
    fn kill(mut self) -> ExitStatus {
        self.child.kill().expect("kill child");
        let status = self.child.wait().expect("reap child");
        self.drain();
        status
    }

    /// Wait for exit within the deadline (killing on expiry) and collect markers.
    fn finish(mut self) -> (ExitStatus, Vec<u128>) {
        drop(self.stdin.take());
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break status;
            }
            if Instant::now() >= self.deadline {
                self.kill_now();
                panic!(
                    "{}: deadline {DEADLINE:?} exceeded; stdout {:?}",
                    self.case, self.transcript
                );
            }
            if let Ok(line) = self.lines.recv_timeout(POLL) {
                self.absorb(line);
            }
        };
        self.drain();
        // Captured by the parent test harness; shown only when a test fails.
        eprintln!(
            "child {} exited with {status}; stdout {:#?}",
            self.case, self.transcript
        );
        // A successful exit must come from a completed scenario, and a fault exit
        // (or a failed child) must not have reached the end of its scenario.
        assert_eq!(
            self.done,
            status.success(),
            "{}: exit {status} vs {DONE_MARKER}marker; stdout {:?}",
            self.case,
            self.transcript
        );
        (status, std::mem::take(&mut self.generations))
    }

    /// Collect the remaining lines of an exited child until its stdout closes.
    fn drain(&mut self) {
        let limit = Instant::now() + WallDuration::from_secs(2);
        loop {
            match self
                .lines
                .recv_timeout(limit.saturating_duration_since(Instant::now()))
            {
                Ok(line) => self.absorb(line),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("{}: stdout still open after exit", self.case)
                }
            }
        }
    }
}

impl Drop for ChildRun {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            self.kill_now();
        }
    }
}

/// Run one child scenario to completion: its exit status and every `NRX-GEN`.
fn run_child(case: &str, env: &[(&str, &OsStr)]) -> (ExitStatus, Vec<u128>) {
    ChildRun::spawn(case, env).finish()
}

fn assert_success(case: &str, status: ExitStatus) {
    assert!(status.success(), "{case}: expected success, got {status}");
}

fn assert_fault_exit(case: &str, status: ExitStatus) {
    assert_eq!(status.code(), Some(FAULT_EXIT), "{case}: got {status}");
}

/// Child side: report a generation about to be activated, flushed before use.
fn announce(value: u128) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{GEN_MARKER}{value}").expect("write marker");
    stdout.flush().expect("flush marker");
}

/// `CARGO_TARGET_TMPDIR/reservation-<case>-<pid>`, removed first, created 0700,
/// removed on drop.
struct ScenarioDir {
    root: PathBuf,
}
impl ScenarioDir {
    fn new(case: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("reservation-{case}-{}", std::process::id()));
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("remove stale {}: {e}", root.display()),
        }
        Self {
            root: owner_only(root),
        }
    }
    /// Environment handing the scenario root to a child.
    fn env(&self) -> [(&'static str, &OsStr); 1] {
        [(DIR_ENV, self.root.as_os_str())]
    }
}
impl Drop for ScenarioDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn scenario_root() -> PathBuf {
    std::env::var_os(DIR_ENV)
        .map(PathBuf::from)
        .expect("scenario directory in the environment")
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const RECEIVER: [u8; 16] = *b"neuradix-host-01";
const OTHER_RECEIVER: [u8; 16] = *b"neuradix-host-02";
const HOLDER: &str = "controller";
const CAPABILITY: &str = "thrust";
const DRIVER: &str = "driver/one";

fn binding() -> ActuatorBinding {
    ActuatorBinding::new(HOLDER, CAPABILITY, DRIVER, ExecutionMode::Live).expect("binding")
}

fn key_for(receiver: [u8; 16], binding: &ActuatorBinding) -> ReservationKey {
    ReservationKey::new(
        ReceiverId::new(receiver).expect("receiver"),
        binding.reservation_key(),
    )
}

fn key() -> ReservationKey {
    key_for(RECEIVER, &binding())
}

fn epoch(value: u64) -> NamespaceEpoch {
    NamespaceEpoch::new(value).expect("nonzero epoch")
}

const fn value(epoch: u64, counter: u64) -> u128 {
    ((epoch as u128) << 64) | counter as u128
}

fn generation(value: u128) -> Generation {
    Generation::new(value).expect("nonzero generation")
}

fn config(window: u32, rollback: RollbackDefense) -> ReserverConfig {
    ReserverConfig::new(NonZeroU32::new(window).expect("window"), rollback)
}

fn unprotected(window: u32) -> ReserverConfig {
    config(window, RollbackDefense::Unprotected)
}

fn owner_only(path: PathBuf) -> PathBuf {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    chmod(&path, 0o700);
    path
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).expect("metadata").mode() & 0o7777
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const STATE_FILES: [&str; 3] = ["lock", "slot-a", "slot-b"];

fn slot_file(dir: &Path, slot: Slot) -> PathBuf {
    dir.join(match slot {
        Slot::A => "slot-a",
        Slot::B => "slot-b",
    })
}

fn tmp_file(dir: &Path, slot: Slot) -> PathBuf {
    dir.join(match slot {
        Slot::A => "slot-a.tmp",
        Slot::B => "slot-b.tmp",
    })
}

type Image = [Vec<u8>; 2];

fn read_image(dir: &Path) -> Image {
    [Slot::A, Slot::B].map(|slot| fs::read(slot_file(dir, slot)).expect("slot file"))
}

/// Write back copied slot files (a backup restore or reflash).
fn restore_image(dir: &Path, image: &Image) {
    for (slot, bytes) in [Slot::A, Slot::B].into_iter().zip(image) {
        fs::write(slot_file(dir, slot), bytes).expect("restore slot file");
    }
}

fn open_store(dir: &Path) -> FileReservationStore {
    FileReservationStore::open(dir).unwrap_or_else(|e| panic!("open {}: {e}", dir.display()))
}

fn open_error(dir: &Path) -> FileStoreError {
    FileReservationStore::open(dir)
        .map(|_| ())
        .expect_err("store open must be refused")
}

/// Trusted provisioning through a short-lived handle.
fn provision_dir(dir: &Path, key: ReservationKey, epoch_value: u64) -> ProvisionReport {
    let mut store = open_store(dir);
    provision(
        &mut store,
        key,
        epoch(epoch_value),
        ProvisionGuards::default(),
    )
    .expect("provision")
}

fn open_reserver<S: ReservationStore>(
    store: S,
    key: ReservationKey,
    config: ReserverConfig,
) -> GenerationReserver<S> {
    GenerationReserver::open(store, key, config)
        .map_err(|f| f.error)
        .expect("open reserver")
}

fn refused<S: ReservationStore>(
    store: S,
    key: ReservationKey,
    config: ReserverConfig,
) -> OpenError {
    GenerationReserver::open(store, key, config)
        .map(|_| ())
        .map_err(|f| f.error)
        .expect_err("open must fail closed")
}

fn take<S: ReservationStore>(reserver: &mut GenerationReserver<S>) -> u128 {
    reserver.reserve().expect("reserve").generation().get()
}

fn reserve_error<S: ReservationStore>(reserver: &mut GenerationReserver<S>) -> ReserveError {
    reserver
        .reserve()
        .map(|t| t.generation())
        .expect_err("reserve must fail")
}

/// One boot: open, reserve `count` values (announced), drop everything.
fn boot(dir: &Path, config: ReserverConfig, count: usize) -> Vec<u128> {
    let mut store = open_store(dir);
    let mut reserver = open_reserver(&mut store, key(), config);
    (0..count)
        .map(|_| {
            let g = take(&mut reserver);
            announce(g);
            g
        })
        .collect()
}

fn assert_strictly_increasing(values: &[u128]) {
    for pair in values.windows(2) {
        assert!(pair[1] > pair[0], "{:#x} after {:#x}", pair[1], pair[0]);
    }
}

/// Counts trait-level store calls.
struct Counting<S> {
    inner: S,
    reads: u32,
    writes: u32,
}
impl<S> Counting<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            reads: 0,
            writes: 0,
        }
    }
    fn calls(&self) -> (u32, u32) {
        (self.reads, self.writes)
    }
}
impl<S: ReservationStore> ReservationStore for Counting<S> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.reads += 1;
        self.inner.read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.writes += 1;
        self.inner.write(slot, record)
    }
}

// Host actuator fixtures.

fn t(ms: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000)
}

fn policy() -> CommandPolicy {
    CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).expect("timeline"),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(2_000),
    )
    .expect("policy")
}

fn lease(g: Generation, issued_ms: i128, expires_ms: i128) -> AuthorityLease {
    AuthorityLease::new(
        Identity::new(HOLDER),
        Capability::new(CAPABILITY),
        SessionConfig::new(g, t(issued_ms), t(expires_ms), policy()).expect("session"),
        None,
    )
}

fn command(g: Generation, sequence: u64, ms: i128, value: f64) -> CommandRequest {
    CommandRequest::new(
        Identity::new(HOLDER),
        Capability::new(CAPABILITY),
        value,
        CommandMeta {
            generation: g,
            sequence,
            source_at: t(ms),
            deadline: t(ms + 500),
            timeline: 1,
        },
    )
}

#[derive(Default)]
struct Trace {
    calls: Vec<f64>,
    fail_at: Option<usize>,
}

struct Driver {
    trace: Rc<RefCell<Trace>>,
    endpoint: &'static str,
}
impl ActuatorDriver for Driver {
    fn endpoint(&self) -> &str {
        self.endpoint
    }
    fn mode(&self) -> ExecutionMode {
        ExecutionMode::Live
    }
    fn write(&mut self, value: f64) -> Result<(), DriverError> {
        let mut trace = self.trace.borrow_mut();
        trace.calls.push(value);
        if trace.fail_at == Some(trace.calls.len()) {
            Err(DriverError::UnknownOutcome)
        } else {
            Ok(())
        }
    }
}

type Shared = Rc<RefCell<Trace>>;

/// A `new_reserved` (strict) or legacy `new` adapter over a traced driver.
fn adapter(strict: bool, fail_at: Option<usize>) -> (ActuatorAdapter<Driver>, Shared) {
    let trace = Rc::new(RefCell::new(Trace {
        calls: Vec::new(),
        fail_at,
    }));
    let driver = Driver {
        trace: Rc::clone(&trace),
        endpoint: DRIVER,
    };
    let config = ActuatorConfig::new(binding(), -1.0, 1.0, 10.0, 0.0).expect("config");
    let adapter = if strict {
        ActuatorAdapter::new_reserved(config, driver)
    } else {
        ActuatorAdapter::new(config, driver)
    }
    .expect("adapter");
    (adapter, trace)
}

fn driver_calls(trace: &Shared) -> usize {
    trace.borrow().calls.len()
}

fn only_safe_output(trace: &Shared) -> bool {
    trace.borrow().calls.iter().all(|&v| v == 0.0)
}

fn reserved_permission(g: Generation, token: ReservedGeneration) -> DriverPermission {
    DriverPermission::reserved(binding(), lease(g, 0, 60_000), token).expect("reserved permission")
}

// ---------------------------------------------------------------------------
// Child dispatch
// ---------------------------------------------------------------------------

#[test]
fn reservation_child() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    match case.as_str() {
        "h1" => child_h1(&scenario_root()),
        "h2" => child_h2(&scenario_root()),
        "h3" => child_h3_in_process(&scenario_root()),
        "h3_holder" => child_h3_holder(&scenario_root()),
        "h4" => child_h4(&scenario_root()),
        "h5" => child_h5(&scenario_root()),
        "h6" => child_h6(&scenario_root()),
        "h7" => child_h7(&scenario_root()),
        "h8" => child_h8(&scenario_root()),
        "h9" => child_h9(&scenario_root()),
        "h10_boot1" => child_h10_boot1(&scenario_root()),
        "h10_boot2" => child_h10_boot2(&scenario_root()),
        "h11" => child_h11(),
        "h12" => child_h12(),
        "h13" => child_h13(&scenario_root()),
        "h14" => child_h14(&scenario_root()),
        other => panic!("unknown reservation child case {other:?}"),
    }
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{DONE_MARKER}{case}").expect("write done marker");
    stdout.flush().expect("flush done marker");
}

// ---------------------------------------------------------------------------
// H1 file_store_round_trip
// ---------------------------------------------------------------------------

#[test]
fn h1_file_store_round_trip() {
    let scenario = ScenarioDir::new("h1");
    let (status, generations) = run_child("h1", &scenario.env());
    assert_success("h1", status);
    let expected = [1, 2, 3, 4, 7, 8, 9, 10, 13, 14, 15, 16].map(|c| value(1, c));
    assert_eq!(generations, expected);
}

fn child_h1(root: &Path) {
    // A missing directory is refused and never created.
    let missing = root.join("missing");
    assert_eq!(
        open_error(&missing),
        FileStoreError::Io(ErrorKind::NotFound)
    );
    assert!(!missing.exists());

    // An empty 0700 directory opens (creating only the lock) and fails closed Blank.
    let dir = owner_only(root.join("store"));
    let mut store = open_store(&dir);
    assert_eq!(entries(&dir), ["lock"]);
    let lock = fs::metadata(dir.join("lock")).expect("lock");
    assert_eq!(lock.len(), 0);
    assert_eq!(mode_of(&dir.join("lock")), 0o600);
    let mut counted = Counting::new(&mut store);
    let error = refused(&mut counted, key(), unprotected(3));
    assert_eq!(error, OpenError::Blank);
    assert_eq!(error.remedy(), Remedy::Provision);
    assert_eq!(counted.calls(), (2, 0));
    assert_eq!(entries(&dir), ["lock"]);

    // Provisioning: two 64-byte 0600 slot files and the lock, no tmp files.
    let report =
        provision(&mut store, key(), epoch(1), ProvisionGuards::default()).expect("provision");
    assert_eq!(
        report,
        ProvisionReport {
            epoch: epoch(1),
            replaced: None,
            commits: 1
        }
    );
    assert_eq!(entries(&dir), STATE_FILES);
    for slot in [Slot::A, Slot::B] {
        let path = slot_file(&dir, slot);
        assert_eq!(mode_of(&path), 0o600, "{slot:?}");
        let bytes: [u8; RECORD_BYTES] = fs::read(&path)
            .expect("slot file")
            .try_into()
            .expect("exactly 64 bytes");
        assert_eq!(
            record::decode(&bytes),
            SlotView::Valid(RecordFields {
                slot,
                key: key(),
                epoch: epoch(1),
                high_water: 0,
                commits: 1
            })
        );
    }
    drop(store);

    // Values increase across reopen (window 3: each boot skips its unused values).
    let mut history = Vec::new();
    for _ in 0..3 {
        history.extend(boot(&dir, unprotected(3), 4));
        assert_eq!(entries(&dir), STATE_FILES, "no tmp files remain");
    }
    assert_strictly_increasing(&history);
}

// ---------------------------------------------------------------------------
// H2 insecure_dirs_refused
// ---------------------------------------------------------------------------

#[test]
fn h2_insecure_dirs_refused() {
    let scenario = ScenarioDir::new("h2");
    let (status, generations) = run_child("h2", &scenario.env());
    assert_success("h2", status);
    assert!(generations.is_empty());
}

fn child_h2(root: &Path) {
    // Any group or other permission bit on the state directory.
    for mode in [0o750, 0o777, 0o740, 0o720, 0o710, 0o704, 0o702, 0o701] {
        let dir = owner_only(root.join(format!("mode-{mode:o}")));
        chmod(&dir, mode);
        assert_eq!(
            open_error(&dir),
            FileStoreError::InsecurePermissions { mode },
            "{mode:o}"
        );
        assert!(entries(&dir).is_empty(), "{mode:o}: nothing created");
    }

    // A symlink to a valid directory, and a regular file, are not directories.
    let real = owner_only(root.join("real"));
    let link = root.join("link");
    symlink(&real, &link).expect("symlink");
    assert_eq!(open_error(&link), FileStoreError::NotADirectory);
    assert!(
        entries(&real).is_empty(),
        "nothing created through the link"
    );
    let file = root.join("file");
    fs::write(&file, b"").expect("file");
    chmod(&file, 0o600);
    assert_eq!(open_error(&file), FileStoreError::NotADirectory);

    // A group- or world-writable parent without the sticky bit.
    for parent_mode in [0o770, 0o777, 0o720, 0o702] {
        let parent = owner_only(root.join(format!("parent-{parent_mode:o}")));
        let dir = owner_only(parent.join("store"));
        chmod(&parent, parent_mode);
        assert_eq!(
            open_error(&dir),
            FileStoreError::InsecureParent { mode: parent_mode },
            "{parent_mode:o}"
        );
        assert!(entries(&dir).is_empty(), "{parent_mode:o}: nothing created");
    }

    // Accepted: a sticky shared parent (like /tmp) or a parent nobody else can write.
    for parent_mode in [0o1777, 0o1770, 0o755, 0o750, 0o700] {
        let parent = owner_only(root.join(format!("ok-{parent_mode:o}")));
        let dir = owner_only(parent.join("store"));
        chmod(&parent, parent_mode);
        drop(open_store(&dir));
        assert_eq!(entries(&dir), ["lock"], "{parent_mode:o}");
    }

    // A relative path with an empty parent checks the current directory. This
    // child process runs a single test, so changing its directory is contained.
    std::env::set_current_dir(root.join("parent-770")).expect("chdir");
    assert_eq!(
        open_error(Path::new("store")),
        FileStoreError::InsecureParent { mode: 0o770 }
    );
    std::env::set_current_dir(root.join("ok-755")).expect("chdir");
    drop(open_store(Path::new("store")));
}

// ---------------------------------------------------------------------------
// H3 exclusive_lock
// ---------------------------------------------------------------------------

#[test]
fn h3_exclusive_lock() {
    let scenario = ScenarioDir::new("h3");
    let dir = owner_only(scenario.root.join("store"));

    // In-process: a second open of the same directory conflicts.
    let (status, _) = run_child("h3", &scenario.env());
    assert_success("h3", status);

    // Cross-process, released by the holder: Locked while held, then open succeeds.
    let mut holder = ChildRun::spawn("h3_holder", &scenario.env());
    holder.wait_for(LOCKED_MARKER);
    assert_eq!(open_error(&dir), FileStoreError::Locked);
    assert_eq!(open_error(&dir), FileStoreError::Locked);
    holder.release();
    let (status, _) = holder.finish();
    assert_success("h3_holder", status);
    drop(open_store(&dir));

    // Cross-process, released by the kernel when the holder dies without releasing.
    let mut holder = ChildRun::spawn("h3_holder", &scenario.env());
    holder.wait_for(LOCKED_MARKER);
    assert_eq!(open_error(&dir), FileStoreError::Locked);
    let status = holder.kill();
    assert_eq!(status.signal(), Some(SIGKILL), "{status}");
    drop(open_store(&dir));
    assert_eq!(entries(&dir), ["lock"]);
}

fn child_h3_in_process(root: &Path) {
    let dir = root.join("store");
    let first = open_store(&dir);
    assert_eq!(open_error(&dir), FileStoreError::Locked);
    drop(first);
    let second = open_store(&dir);
    assert_eq!(open_error(&dir), FileStoreError::Locked);
    drop(second);
}

fn child_h3_holder(root: &Path) {
    let store = open_store(&root.join("store"));
    {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{LOCKED_MARKER}").expect("marker");
        stdout.flush().expect("flush");
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("release line");
    assert_eq!(line.trim(), "release");
    drop(store);
}

// ---------------------------------------------------------------------------
// H4 stale_tmp_ignored_and_removed
// ---------------------------------------------------------------------------

#[test]
fn h4_stale_tmp_ignored_and_removed() {
    let scenario = ScenarioDir::new("h4");
    let (status, generations) = run_child("h4", &scenario.env());
    assert_success("h4", status);
    assert_eq!(generations, [value(1, 1)]);
}

fn child_h4(root: &Path) {
    let dir = owner_only(root.join("store"));
    provision_dir(&dir, key(), 1);
    // Leftovers of an interrupted writer: a higher, never committed ceiling and junk.
    let uncommitted = record::encode(&RecordFields {
        slot: Slot::A,
        key: key(),
        epoch: epoch(1),
        high_water: 1_000,
        commits: 9,
    });
    fs::write(tmp_file(&dir, Slot::A), uncommitted).expect("stale tmp a");
    fs::write(tmp_file(&dir, Slot::B), [0x5A; 17]).expect("stale tmp b");

    let mut store = open_store(&dir);
    assert_eq!(
        entries(&dir),
        STATE_FILES,
        "stale tmp files removed at open"
    );

    // A tmp file is never read: one reappearing under an open handle is ignored
    // and consumed by the next write of that slot.
    fs::write(tmp_file(&dir, Slot::A), uncommitted).expect("tmp under open handle");
    {
        let mut reserver = open_reserver(&mut store, key(), unprotected(2));
        assert_eq!(reserver.status().durable_ceiling, value(1, 0));
        let g = take(&mut reserver);
        announce(g);
        assert_eq!(g, value(1, 1));
    }
    assert_eq!(entries(&dir), STATE_FILES);
    let slot_a: [u8; RECORD_BYTES] = fs::read(slot_file(&dir, Slot::A))
        .expect("slot-a")
        .try_into()
        .expect("64 bytes");
    let SlotView::Valid(fields) = record::decode(&slot_a) else {
        panic!("slot-a must hold a valid record");
    };
    assert_eq!(fields.high_water, 2);
}

// ---------------------------------------------------------------------------
// H5 damaged_slot_files_tolerated
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Damage {
    FlipByte,
    Truncate,
    OneByteLong,
    Empty,
    Oversized,
    Missing,
}

fn damage(dir: &Path, slot: Slot, kind: Damage) {
    let path = slot_file(dir, slot);
    let mut bytes = fs::read(&path).expect("slot file");
    match kind {
        Damage::FlipByte => bytes[40] ^= 0x01,
        Damage::Truncate => bytes.truncate(40),
        Damage::OneByteLong => bytes.push(0),
        Damage::Empty => bytes.clear(),
        Damage::Oversized => bytes = vec![0xA5; 1024],
        Damage::Missing => {
            fs::remove_file(&path).expect("remove slot file");
            return;
        }
    }
    fs::write(&path, bytes).expect("damage slot file");
}

#[test]
fn h5_damaged_slot_files_tolerated() {
    let scenario = ScenarioDir::new("h5");
    let (status, generations) = run_child("h5", &scenario.env());
    assert_success("h5", status);
    // One initial boot plus 2 slots × 6 damage kinds, one value each.
    assert_eq!(generations.len(), 13);
    assert_strictly_increasing(&generations);
}

fn child_h5(root: &Path) {
    let dir = owner_only(root.join("store"));
    provision_dir(&dir, key(), 1);
    let mut history = boot(&dir, unprotected(2), 1);
    let kinds = [
        Damage::FlipByte,
        Damage::Truncate,
        Damage::OneByteLong,
        Damage::Empty,
        Damage::Oversized,
        Damage::Missing,
    ];
    for slot in [Slot::A, Slot::B] {
        for kind in kinds {
            damage(&dir, slot, kind);
            let mut store = open_store(&dir);
            let mut reserver = open_reserver(&mut store, key(), unprotected(2));
            let expected = match kind {
                Damage::Missing => SlotCondition::Blank,
                _ => SlotCondition::Corrupt,
            };
            let mut slots = [SlotCondition::Current; 2];
            slots[slot.index()] = expected;
            assert_eq!(reserver.status().slots, slots, "{slot:?} {kind:?}");
            let g = take(&mut reserver);
            announce(g);
            assert!(g > *history.last().expect("history"), "{slot:?} {kind:?}");
            history.push(g);
            // The commit rewrote the damaged slot first; both are whole again.
            assert_eq!(
                reserver.status().slots,
                [SlotCondition::Current; 2],
                "{slot:?} {kind:?}"
            );
            for s in [Slot::A, Slot::B] {
                assert_eq!(fs::metadata(slot_file(&dir, s)).expect("m").len(), 64);
            }
        }
    }

    // Both damaged: Corrupt (Provision) and nothing is written.
    damage(&dir, Slot::A, Damage::FlipByte);
    damage(&dir, Slot::B, Damage::Truncate);
    let before = read_image(&dir);
    let mut store = open_store(&dir);
    let mut counted = Counting::new(&mut store);
    let error = refused(&mut counted, key(), unprotected(2));
    assert_eq!(error, OpenError::Corrupt);
    assert_eq!(error.remedy(), Remedy::Provision);
    assert_eq!(counted.calls(), (2, 0));
    assert_eq!(read_image(&dir), before);
}

// ---------------------------------------------------------------------------
// H6 exit_at_every_store_call
// ---------------------------------------------------------------------------

const H6_WINDOW: u32 = 2;
const H6_RESERVES: usize = 5;
/// open (2 reads) plus three commits of 6 calls: reserves 1, 3 and 5 commit.
const H6_CALLS: u32 = MAX_STORE_CALLS_PER_OPEN + 3 * MAX_STORE_CALLS_PER_RESERVE;

/// Values announced by a child that exits once `k` store calls completed:
/// reserves 1–2 return after call 8, reserves 3–4 after call 14, reserve 5 after 20.
fn h6_announced(k: u32) -> usize {
    [8, 8, 14, 14, 20].iter().filter(|&&done| done < k).count()
}

/// Exits the process with status 86 as soon as `exit_after` store calls have
/// completed (0: at the first call, before it runs).
struct ExitAfter<S> {
    inner: S,
    calls: u32,
    exit_after: Option<u32>,
}
impl<S> ExitAfter<S> {
    fn gate(&self) {
        if self.exit_after == Some(self.calls) {
            std::process::exit(FAULT_EXIT);
        }
    }
}
impl<S: ReservationStore> ReservationStore for ExitAfter<S> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.gate();
        let result = self.inner.read(slot, buf);
        self.calls += 1;
        self.gate();
        result
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.gate();
        let result = self.inner.write(slot, record);
        self.calls += 1;
        self.gate();
        result
    }
}

#[test]
fn h6_exit_at_every_store_call() {
    let scenario = ScenarioDir::new("h6");
    let dir = owner_only(scenario.root.join("store"));
    provision_dir(&dir, key(), 1);
    // Chronological history of every activated value, child and parent.
    let mut history: Vec<u128> = Vec::new();
    for k in 0..=H6_CALLS {
        let k_text = k.to_string();
        let env = [
            (DIR_ENV, scenario.root.as_os_str()),
            (EXIT_AFTER_ENV, OsStr::new(&k_text)),
        ];
        let (status, announced) = run_child("h6", &env);
        assert_fault_exit(&format!("h6 k={k}"), status);
        assert_eq!(announced.len(), h6_announced(k), "k={k}");
        history.extend(&announced);
        assert_strictly_increasing(&history);

        // The parent reopens and reserves a value above every NRX-GEN printed.
        let mut store = open_store(&dir);
        let mut reserver = open_reserver(&mut store, key(), unprotected(1));
        let g = take(&mut reserver);
        assert!(
            history.iter().all(|&printed| g > printed),
            "k={k}: {g:#x} not above {history:x?}"
        );
        history.push(g);
    }

    // A clean boot runs to completion.
    let (status, announced) = run_child("h6", &scenario.env());
    assert_success("h6 clean", status);
    assert_eq!(announced.len(), H6_RESERVES);
    history.extend(&announced);
    assert_strictly_increasing(&history);
}

fn child_h6(root: &Path) {
    let exit_after = std::env::var(EXIT_AFTER_ENV)
        .ok()
        .map(|k| k.parse::<u32>().expect("EXIT_AFTER"));
    let (mut adapter, trace) = adapter(true, None);
    let report = adapter.port(ExecutionMode::Live, t(0)).tick(None);
    assert_eq!(report.permission, PermissionStatus::Missing);
    let mut store = open_store(&root.join("store"));
    let wrapped = ExitAfter {
        inner: &mut store,
        calls: 0,
        exit_after,
    };
    let mut reserver = open_reserver(wrapped, key(), unprotected(H6_WINDOW));
    for i in 0..H6_RESERVES {
        let token = reserver.reserve().expect("reserve");
        let g = token.generation();
        announce(g.get());
        let now = t(10 * i as i128);
        let report = adapter
            .grant(reserved_permission(g, token), now)
            .expect("grant");
        assert_eq!(report.permission, PermissionStatus::Initialized);
    }
    assert!(only_safe_output(&trace));
    adapter.shutdown(t(100));
}

// ---------------------------------------------------------------------------
// H7 write_error_poisons
// ---------------------------------------------------------------------------

/// Makes write number `fail_write` fail inside the real store: a directory
/// occupies the temporary-file path; it cannot be unlinked, so the exclusive
/// create fails with EEXIST (for any user, root included) and the store poisons
/// itself.
struct Sabotage<'a> {
    inner: &'a mut FileReservationStore,
    dir: PathBuf,
    fail_write: u32,
    writes: u32,
}
impl ReservationStore for Sabotage<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.inner.read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let index = self.writes;
        self.writes += 1;
        if index != self.fail_write {
            return self.inner.write(slot, record);
        }
        let blocker = tmp_file(&self.dir, slot);
        fs::create_dir(&blocker).expect("block tmp path");
        let result = self.inner.write(slot, record);
        fs::remove_dir(&blocker).expect("unblock tmp path");
        result
    }
}

#[test]
fn h7_write_error_poisons() {
    let scenario = ScenarioDir::new("h7");
    let (status, generations) = run_child("h7", &scenario.env());
    assert_success("h7", status);
    // Per failing slot: 3 values before the failure, 3 from a fresh handle.
    let expected: Vec<u128> = [1, 2, 3, 4, 5, 6, 1, 2, 3, 7, 8, 9]
        .map(|c| value(1, c))
        .to_vec();
    assert_eq!(generations, expected);
}

fn child_h7(root: &Path) {
    // Write 0 and 1 are the first commit; 2 (slot A) or 3 (slot B) fails.
    for (fail_write, slot, fresh_first) in [(2, Slot::A, value(1, 4)), (3, Slot::B, value(1, 7))] {
        let dir = owner_only(root.join(format!("store-{slot:?}")));
        provision_dir(&dir, key(), 1);
        let mut store = open_store(&dir);
        let mut issued = Vec::new();
        {
            let sabotage = Sabotage {
                inner: &mut store,
                dir: dir.clone(),
                fail_write,
                writes: 0,
            };
            let mut reserver = open_reserver(sabotage, key(), unprotected(3));
            for _ in 0..3 {
                let g = take(&mut reserver);
                announce(g);
                issued.push(g);
            }
            let error = reserve_error(&mut reserver);
            assert_eq!(
                error,
                ReserveError::Uncertain(PoisonCause::WriteFailed(slot))
            );
            assert_eq!(error.remedy(), Remedy::ReopenStore);
            let poisoned = ReserveError::Poisoned(PoisonCause::WriteFailed(slot));
            assert_eq!(reserve_error(&mut reserver), poisoned);
            assert_eq!(
                reserver.reserve_from_window().map(|t| t.generation()),
                Err(poisoned)
            );
            assert_eq!(poisoned.remedy(), Remedy::ReopenStore);
            assert_eq!(
                reserver.status().poisoned,
                Some(PoisonCause::WriteFailed(slot))
            );
        }
        assert!(store.is_poisoned(), "{slot:?}");
        // The planted directory cannot be unlinked, so the exclusive create of
        // the temporary file refuses it (a symlink or file would be unlinked).
        assert_eq!(store.last_io_error(), Some(ErrorKind::AlreadyExists));

        // Reopen over the same handle: StoreUnavailable, 2 reads and 0 writes.
        let mut counted = Counting::new(&mut store);
        let error = refused(&mut counted, key(), unprotected(3));
        assert_eq!(error, OpenError::StoreUnavailable);
        assert_eq!(error.remedy(), Remedy::ReopenStore);
        assert_eq!(counted.calls(), (2, 0));
        drop(store);

        // A fresh handle opens and issues only greater values.
        let mut fresh = open_store(&dir);
        let mut reserver = open_reserver(&mut fresh, key(), unprotected(3));
        for i in 0..3 {
            let g = take(&mut reserver);
            announce(g);
            if i == 0 {
                assert_eq!(g, fresh_first, "{slot:?}");
            }
            assert!(g > *issued.last().expect("issued"), "{slot:?}");
            issued.push(g);
        }
    }
}

// ---------------------------------------------------------------------------
// H8 rollback_restore
// ---------------------------------------------------------------------------

#[test]
fn h8_rollback_restore() {
    let scenario = ScenarioDir::new("h8");
    let (status, generations) = run_child("h8", &scenario.env());
    assert_success("h8", status);
    // Values 1..=6, then the CHAR reissue of 3 under Unprotected, then epoch 2.
    let mut expected: Vec<u128> = (1..=6).map(|c| value(1, c)).collect();
    expected.push(value(1, 3));
    expected.push(value(2, 1));
    assert_eq!(generations, expected);
}

fn child_h8(root: &Path) {
    let dir = owner_only(root.join("store"));
    provision_dir(&dir, key(), 1);
    let mut issued = boot(&dir, unprotected(2), 2);
    let backup = read_image(&dir); // copy of both slot files at ceiling 2
    issued.extend(boot(&dir, unprotected(2), 4));
    // Highest generation handed out, recorded outside the reservation image.
    let witness = generation(*issued.last().expect("issued"));
    assert_eq!(witness.get(), value(1, 6));

    // CHAR (RB1, OBL): Unprotected does not detect a faithful restore and reissues.
    restore_image(&dir, &backup);
    let reissued = boot(&dir, unprotected(2), 1)[0];
    assert_eq!(reissued, value(1, 3));
    assert!(issued.contains(&reissued), "CHAR: a value is reissued");

    // Witness: RolledBack, 2 reads and 0 writes, files untouched.
    restore_image(&dir, &backup);
    let mut store = open_store(&dir);
    let mut counted = Counting::new(&mut store);
    let error = refused(
        &mut counted,
        key(),
        config(2, RollbackDefense::Witness(witness)),
    );
    assert_eq!(
        error,
        OpenError::RolledBack {
            witness,
            stored: value(1, 2)
        }
    );
    assert_eq!(error.remedy(), Remedy::Provision);
    assert_eq!(counted.calls(), (2, 0));
    assert_eq!(read_image(&dir), backup);

    // Recovery: advance the floor to epoch 2, then provision epoch 2 under it.
    let guards = ProvisionGuards {
        floor: Some(epoch(2)),
        prior_high_water: None,
    };
    let report = provision(&mut store, key(), epoch(2), guards).expect("reprovision");
    assert_eq!(report.replaced, Some(epoch(1)));
    drop(store);
    let floor = config(2, RollbackDefense::Floor(epoch(2)));
    let fresh = boot(&dir, floor, 1)[0];
    assert_eq!(fresh, value(2, 1));
    assert!(fresh > witness.get());

    // The old copy restored again now fails closed below the floor.
    restore_image(&dir, &backup);
    let mut store = open_store(&dir);
    let mut counted = Counting::new(&mut store);
    let error = refused(&mut counted, key(), floor);
    assert_eq!(
        error,
        OpenError::BelowFloor {
            stored: epoch(1),
            floor: epoch(2)
        }
    );
    assert_eq!(error.remedy(), Remedy::Provision);
    assert_eq!(counted.calls(), (2, 0));
    assert_eq!(read_image(&dir), backup);
}

// ---------------------------------------------------------------------------
// H9 cloned_dir_other_receiver
// ---------------------------------------------------------------------------

#[test]
fn h9_cloned_dir_other_receiver() {
    let scenario = ScenarioDir::new("h9");
    let (status, generations) = run_child("h9", &scenario.env());
    assert_success("h9", status);
    assert_eq!(generations, [value(1, 1)]);
}

fn child_h9(root: &Path) {
    let original = owner_only(root.join("store-1"));
    provision_dir(&original, key(), 1);
    boot(&original, unprotected(4), 1);
    let clone = owner_only(root.join("store-2"));
    for slot in [Slot::A, Slot::B] {
        fs::copy(slot_file(&original, slot), slot_file(&clone, slot)).expect("copy slot");
    }
    let image = read_image(&clone);

    // Another receiver (identity from host config, outside the image): refused.
    let mut store = open_store(&clone);
    let mut counted = Counting::new(&mut store);
    let error = refused(
        &mut counted,
        key_for(OTHER_RECEIVER, &binding()),
        unprotected(4),
    );
    assert_eq!(error, OpenError::ForeignReceiver);
    assert_eq!(error.remedy(), Remedy::RepairStore);
    assert_eq!(counted.calls(), (2, 0));

    // Same receiver, another binding (wiring): refused as ForeignBinding.
    let other = ActuatorBinding::new(HOLDER, CAPABILITY, "driver/two", ExecutionMode::Live)
        .expect("binding");
    let error = refused(&mut store, key_for(RECEIVER, &other), unprotected(4));
    assert_eq!(error, OpenError::ForeignBinding);
    assert_eq!(read_image(&clone), image, "refusals write nothing");

    // CHAR (OBL I3): a clone opened under the SAME receiver id is not detected,
    // so the clone and the original each issue the same next value (reuse).
    // These values are characterization only and are never activated.
    let from_clone = {
        let mut reserver = open_reserver(&mut store, key(), unprotected(4));
        assert_eq!(reserver.status().durable_ceiling, value(1, 4));
        take(&mut reserver)
    };
    drop(store);
    let mut original_store = open_store(&original);
    let from_original = take(&mut open_reserver(
        &mut original_store,
        key(),
        unprotected(4),
    ));
    assert_eq!(from_clone, value(1, 5));
    assert_eq!(from_original, from_clone, "CHAR: one value issued twice");
}

// ---------------------------------------------------------------------------
// H10 host_reserved_adapter_restart
// ---------------------------------------------------------------------------

#[test]
fn h10_host_reserved_adapter_restart() {
    let scenario = ScenarioDir::new("h10");
    let dir = owner_only(scenario.root.join("store"));
    provision_dir(&dir, key(), 1);

    // Boot 1 activates g1, runs to t = 10 s, then loses power while active.
    let (status, boot1) = run_child("h10_boot1", &scenario.env());
    assert_fault_exit("h10_boot1", status);
    assert_eq!(boot1, [value(1, 1)]);

    // Boot 2: clock restarts at 0; the replayed g1 is rejected, g2 accepted.
    let g1 = boot1[0].to_string();
    let env = [
        (DIR_ENV, scenario.root.as_os_str()),
        (REPLAY_ENV, OsStr::new(&g1)),
    ];
    let (status, boot2) = run_child("h10_boot2", &env);
    assert_success("h10_boot2", status);
    // Window 4: boot 1's unused values 2..=4 are skipped.
    assert_eq!(boot2, [value(1, 5)]);
}

fn child_h10_boot1(root: &Path) {
    let (mut adapter, trace) = adapter(true, None);
    adapter.port(ExecutionMode::Live, t(0)).tick(None);
    let mut store = open_store(&root.join("store"));
    let mut reserver = open_reserver(&mut store, key(), unprotected(4));
    let token = reserver.reserve().expect("reserve");
    let g1 = token.generation();
    announce(g1.get());
    adapter
        .grant(reserved_permission(g1, token), t(0))
        .expect("grant g1");
    // Sequence 0 at 0.1 s (after the grant's safe write), then every 2 s to 10 s.
    for sequence in 0..=5u64 {
        let ms = if sequence == 0 {
            100
        } else {
            2_000 * i128::from(sequence)
        };
        let report = adapter
            .port(ExecutionMode::Live, t(ms))
            .tick(Some(command(g1, sequence, ms, 0.25)));
        let decision = report.decision.expect("evaluated");
        assert_eq!(decision.outcome, Outcome::Accepted, "sequence {sequence}");
    }
    assert_eq!(adapter.last_accepted_at(), Some(t(10_000)));
    assert_eq!(trace.borrow().calls.last(), Some(&0.25));
    // Power loss while g1 is active: no shutdown, no destructors.
    std::process::exit(FAULT_EXIT);
}

fn child_h10_boot2(root: &Path) {
    let g1 = generation(
        std::env::var(REPLAY_ENV)
            .expect("replayed generation")
            .parse()
            .expect("decimal generation"),
    );
    let (mut adapter, trace) = adapter(true, None);
    let report = adapter.port(ExecutionMode::Live, t(0)).tick(None);
    assert_eq!(report.permission, PermissionStatus::Missing);
    let mut store = open_store(&root.join("store"));
    let mut reserver = open_reserver(&mut store, key(), unprotected(4));
    let token = reserver.reserve().expect("reserve");
    let g2 = token.generation();
    assert!(g2 > g1);
    announce(g2.get());
    // Boot 2's monotonic clock restarted at 0.
    adapter
        .grant(reserved_permission(g2, token), t(0))
        .expect("grant g2");

    // Replayed g1 commands carrying fresh boot-2 timestamps and sequences.
    for (sequence, ms) in [(5, 100), (6, 200), (0, 300)] {
        let report = adapter
            .port(ExecutionMode::Live, t(ms))
            .tick(Some(command(g1, sequence, ms, 0.9)));
        let decision = report.decision.expect("evaluated");
        assert_eq!(
            decision.outcome,
            Outcome::Rejected(RejectReason::GenerationMismatch)
        );
        assert_eq!(report.output, 0.0);
    }
    assert!(only_safe_output(&trace), "safe output only");
    assert_eq!(adapter.last_accepted_at(), None);

    let report = adapter
        .port(ExecutionMode::Live, t(400))
        .tick(Some(command(g2, 0, 400, 0.25)));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Accepted
    );
    assert_eq!(report.output, 0.25);
    adapter.shutdown(t(500));
}

// ---------------------------------------------------------------------------
// H11 host_mismatch_latch_precedence
// ---------------------------------------------------------------------------

#[test]
fn h11_host_mismatch_latch_precedence() {
    let (status, generations) = run_child("h11", &[]);
    assert_success("h11", status);
    assert!(generations.is_empty());
}

/// The first token of a freshly provisioned in-memory store keyed to `binding`.
fn first_token(
    store: &mut FaultStore,
    binding: &ActuatorBinding,
    epoch_value: u64,
) -> ReservedGeneration {
    let key = key_for(RECEIVER, binding);
    provisioned(store, key, epoch_value);
    let mut reserver = open_reserver(store, key, unprotected(4));
    reserver.reserve().expect("reserve")
}

fn child_h11() {
    let other_driver =
        ActuatorBinding::new(HOLDER, CAPABILITY, "driver/two", ExecutionMode::Live).expect("b");
    let simulation =
        ActuatorBinding::new(HOLDER, CAPABILITY, DRIVER, ExecutionMode::Simulation).expect("b");

    // ReservationMismatch: binding, mode and generation; the adapter is untouched.
    let (mut strict, trace) = adapter(true, None);
    for (label, token_binding) in [("binding", &other_driver), ("mode", &simulation)] {
        let mut store = FaultStore::erased(0xFF);
        let token = first_token(&mut store, token_binding, 1);
        let g = token.generation();
        assert_eq!(g.get(), value(1, 1));
        let error = DriverPermission::reserved(binding(), lease(g, 0, 60_000), token)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error, PermissionError::ReservationMismatch, "{label}");
    }
    let mut store = FaultStore::erased(0xFF);
    let token = first_token(&mut store, &binding(), 1);
    let other_generation = generation(token.generation().get() + 1);
    let error = DriverPermission::reserved(binding(), lease(other_generation, 0, 60_000), token)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error, PermissionError::ReservationMismatch, "generation");
    assert_eq!(driver_calls(&trace), 0);

    // ReservationRequired: a strict adapter refuses an unreserved permission.
    let unreserved = DriverPermission::new(binding(), lease(generation(value(1, 1)), 0, 60_000))
        .expect("permission");
    assert!(!unreserved.is_reserved());
    assert_eq!(
        strict.grant(unreserved, t(0)).map(|_| ()).unwrap_err(),
        PermissionError::ReservationRequired
    );
    assert_eq!(driver_calls(&trace), 0);
    assert!(strict.requires_reserved_generations());
    let report = strict.port(ExecutionMode::Live, t(1)).tick(None);
    assert_eq!(report.permission, PermissionStatus::Missing);
    assert_eq!(report.generation, None);

    // Precedence (as E19): each earlier check wins over ReservationRequired,
    // with 0 driver calls from the refused grant.
    let unreserved = || {
        DriverPermission::new(binding(), lease(generation(value(1, 1)), 0, 60_000))
            .expect("permission")
    };
    let (mut shut, trace) = adapter(true, None);
    shut.shutdown(t(0));
    let before = driver_calls(&trace);
    assert_eq!(
        shut.grant(unreserved(), t(1)).map(|_| ()).unwrap_err(),
        PermissionError::Shutdown
    );
    assert_eq!(driver_calls(&trace), before);

    let (mut faulted, trace) = adapter(true, Some(1));
    faulted.port(ExecutionMode::Live, t(0)).tick(None);
    assert!(faulted.driver_fault().is_some());
    let before = driver_calls(&trace);
    assert_eq!(
        faulted.grant(unreserved(), t(1)).map(|_| ()).unwrap_err(),
        PermissionError::DriverFault
    );
    assert_eq!(driver_calls(&trace), before);

    let (mut strict, trace) = adapter(true, None);
    let wrong_binding = DriverPermission::new(
        other_driver.clone(),
        lease(generation(value(1, 1)), 0, 60_000),
    )
    .expect("permission");
    assert_eq!(
        strict.grant(wrong_binding, t(0)).map(|_| ()).unwrap_err(),
        PermissionError::BindingMismatch
    );
    let simulated_now = Timestamp::new(ClockDomain::Simulation, 0);
    assert_eq!(
        strict
            .grant(unreserved(), simulated_now)
            .map(|_| ())
            .unwrap_err(),
        PermissionError::ModeMismatch
    );
    assert_eq!(driver_calls(&trace), 0);

    // The latch is not set by a failed reserved grant (as E20).
    let (mut legacy, trace) = adapter(false, None);
    let legacy_high = generation(value(2, 5));
    legacy
        .grant(
            DriverPermission::new(binding(), lease(legacy_high, 0, 60_000)).expect("p"),
            t(0),
        )
        .expect("legacy grant");
    let calls = driver_calls(&trace);
    let mut store = FaultStore::erased(0xFF);
    let token = first_token(&mut store, &binding(), 1);
    let g = token.generation();
    assert!(g < legacy_high);
    assert_eq!(
        legacy
            .grant(reserved_permission(g, token), t(1))
            .map(|_| ())
            .unwrap_err(),
        PermissionError::Session(SessionError::ReusedGeneration)
    );
    assert!(!legacy.requires_reserved_generations());
    assert_eq!(driver_calls(&trace), calls);
    let higher = generation(value(2, 6));
    legacy
        .grant(
            DriverPermission::new(binding(), lease(higher, 0, 60_000)).expect("p"),
            t(2),
        )
        .expect("unreserved higher grant still accepted");

    // A successful reserved grant latches a legacy adapter (as E6).
    let (mut legacy, trace) = adapter(false, None);
    let mut store = FaultStore::erased(0xFF);
    let token = first_token(&mut store, &binding(), 1);
    let g1 = token.generation();
    legacy
        .grant(reserved_permission(g1, token), t(0))
        .expect("reserved grant");
    assert!(legacy.requires_reserved_generations());
    let calls = driver_calls(&trace);
    let unreserved_higher =
        DriverPermission::new(binding(), lease(generation(value(1, 9)), 0, 60_000)).expect("p");
    assert_eq!(
        legacy
            .grant(unreserved_higher, t(1))
            .map(|_| ())
            .unwrap_err(),
        PermissionError::ReservationRequired
    );
    assert_eq!(driver_calls(&trace), calls);
    let report = legacy.port(ExecutionMode::Live, t(2)).tick(None);
    assert_eq!(report.generation, Some(g1));
    // The refused unreserved grant installed nothing: a command carrying its
    // generation is rejected and only the safe output is written (the check runs
    // before the lease is installed, as on the embedded boundary).
    let calls = driver_calls(&trace);
    let report = legacy.port(ExecutionMode::Live, t(3)).tick(Some(command(
        generation(value(1, 9)),
        0,
        3,
        0.9,
    )));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Rejected(RejectReason::GenerationMismatch)
    );
    assert_eq!(report.output, 0.0);
    assert_eq!(driver_calls(&trace), calls + 1);
    assert!(only_safe_output(&trace));

    // Check order (as E19): the reservation requirement precedes the
    // control-time check. After a reserved grant at t(50), an unreserved grant at
    // the regressed time t(40) is ReservationRequired, while a reserved token at
    // the same time reaches the control-time check.
    let (mut strict, trace) = adapter(true, None);
    let mut store = FaultStore::erased(0xFF);
    let token = first_token(&mut store, &binding(), 1);
    let g = token.generation();
    strict
        .grant(reserved_permission(g, token), t(50))
        .expect("reserved grant at t(50)");
    let calls = driver_calls(&trace);
    let late = DriverPermission::new(binding(), lease(generation(value(1, 9)), 0, 60_000))
        .expect("permission");
    assert_eq!(
        strict.grant(late, t(40)).map(|_| ()).unwrap_err(),
        PermissionError::ReservationRequired
    );
    let mut store = FaultStore::erased(0xFF);
    let token = first_token(&mut store, &binding(), 2);
    let later = token.generation();
    assert_eq!(
        strict
            .grant(reserved_permission(later, token), t(40))
            .map(|_| ())
            .unwrap_err(),
        PermissionError::Session(SessionError::InvalidEvaluationTime)
    );
    assert_eq!(driver_calls(&trace), calls);
    let report = strict.port(ExecutionMode::Live, t(60)).tick(None);
    assert_eq!(report.generation, Some(g));
}

// ---------------------------------------------------------------------------
// H12 host binding-key golden
// ---------------------------------------------------------------------------

#[test]
fn h12_host_binding_key_golden() {
    let (status, _) = run_child("h12", &[]);
    assert_success("h12", status);
}

fn child_h12() {
    let key = binding().reservation_key();
    assert_eq!(key.get(), 0x2249_4187_5563_01ab);
    assert_eq!(
        key,
        BindingKey::named(HOLDER, CAPABILITY, DRIVER, ExecutionMode::Live)
    );
    let keys = [
        ExecutionMode::Live,
        ExecutionMode::Simulation,
        ExecutionMode::Replay,
    ]
    .map(|mode| {
        ActuatorBinding::new(HOLDER, CAPABILITY, DRIVER, mode)
            .expect("binding")
            .reservation_key()
    });
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[1], keys[2]);
    assert_ne!(keys[0], keys[2]);
    let other = ActuatorBinding::new(HOLDER, CAPABILITY, "driver/two", ExecutionMode::Live)
        .expect("binding");
    assert_ne!(other.reservation_key(), key);
}

// ---------------------------------------------------------------------------
// H13 host_timer_rollover_reconstruction (as E22)
// ---------------------------------------------------------------------------

#[test]
fn h13_host_timer_rollover_reconstruction() {
    let scenario = ScenarioDir::new("h13");
    let (status, generations) = run_child("h13", &scenario.env());
    assert_success("h13", status);
    // g1 activated; g2 burned by the refused grant; g3 activated after reconstruction.
    assert_eq!(generations, [value(1, 1), value(1, 3)]);
}

fn child_h13(root: &Path) {
    const RUN_TO: i128 = 3_600_000; // one hour, in ms
    let dir = owner_only(root.join("store"));
    provision_dir(&dir, key(), 1);
    let mut store = open_store(&dir);
    let mut reserver = open_reserver(&mut store, key(), unprotected(4));

    let (mut first, _) = adapter(true, None);
    first.port(ExecutionMode::Live, t(0)).tick(None);
    let token = reserver.reserve().expect("reserve");
    let g1 = token.generation();
    announce(g1.get());
    first
        .grant(
            DriverPermission::reserved(binding(), lease(g1, 0, RUN_TO + 60_000), token)
                .expect("permission"),
            t(0),
        )
        .expect("grant g1");
    let report = first
        .port(ExecutionMode::Live, t(RUN_TO))
        .tick(Some(command(g1, 0, RUN_TO, 0.25)));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Accepted
    );

    // Timer rollover: evaluation time jumps back and the regression latches.
    let report = first.port(ExecutionMode::Live, t(5)).tick(None);
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Rejected(RejectReason::EvaluationTimeRegression)
    );
    assert_eq!(report.output, 0.0);
    let report = first
        .port(ExecutionMode::Live, t(RUN_TO + 100))
        .tick(Some(command(g1, 1, RUN_TO + 100, 0.25)));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Rejected(RejectReason::EvaluationTimeRegression)
    );
    let token = reserver.reserve_from_window().expect("window token");
    let g2 = token.generation();
    assert_eq!(
        first
            .grant(reserved_permission(g2, token), t(10))
            .map(|_| ())
            .unwrap_err(),
        PermissionError::Session(SessionError::InvalidEvaluationTime)
    );
    first.shutdown(t(10));
    drop(first);

    // Reconstruction: a new strict adapter and a fresh token.
    let (mut second, trace) = adapter(true, None);
    let report = second.port(ExecutionMode::Live, t(10)).tick(None);
    assert_eq!(report.permission, PermissionStatus::Missing);
    let token = reserver.reserve_from_window().expect("window token");
    let g3 = token.generation();
    assert!(g3 > g2 && g2 > g1);
    announce(g3.get());
    second
        .grant(reserved_permission(g3, token), t(10))
        .expect("grant g3");

    // An old-generation command with post-rollover timestamps: safe output only.
    let report = second
        .port(ExecutionMode::Live, t(20))
        .tick(Some(command(g1, 2, 20, 0.9)));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Rejected(RejectReason::GenerationMismatch)
    );
    assert_eq!(report.output, 0.0);
    assert!(only_safe_output(&trace));

    let report = second
        .port(ExecutionMode::Live, t(1_000))
        .tick(Some(command(g3, 0, 1_000, 0.25)));
    assert_eq!(
        report.decision.expect("evaluated").outcome,
        Outcome::Accepted
    );
    assert_eq!(report.output, 0.25);
    second.shutdown(t(1_100));
}

// ---------------------------------------------------------------------------
// H14 directory_replaced_after_open
// ---------------------------------------------------------------------------

/// Renames the state directory away and replaces it just before the first
/// write, i.e. after the commit's pre-commit reads.
struct ReplaceBeforeWrite<'a> {
    inner: &'a mut FileReservationStore,
    dir: PathBuf,
    moved: PathBuf,
    replaced: bool,
}
impl ReservationStore for ReplaceBeforeWrite<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.inner.read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        if !self.replaced {
            self.replaced = true;
            fs::rename(&self.dir, &self.moved).expect("rename state directory");
            owner_only(self.dir.clone());
        }
        self.inner.write(slot, record)
    }
}

#[test]
fn h14_directory_replaced_after_open() {
    let scenario = ScenarioDir::new("h14");
    let (status, generations) = run_child("h14", &scenario.env());
    assert_success("h14", status);
    assert_eq!(generations, [value(1, 1), value(1, 2)]);
}

fn child_h14(root: &Path) {
    // (a) Replaced between open and the next committing reserve. That reserve's
    // pre-commit read meets the identity check first, so the cause is
    // ReadFailed(A) (host_store: a read after replacement is Unavailable;
    // algorithm: any Unavailable pre-commit read poisons ReadFailed(slot)).
    // SPEC DEVIATION: tests.md H14 says `Uncertain(WriteFailed)`; that cause is
    // only reachable with timing (b) below. See the WP-A04.4 evidence notes.
    let dir = owner_only(root.join("store"));
    provision_dir(&dir, key(), 1);
    let before = read_image(&dir);
    let moved = root.join("store-moved");
    let mut store = open_store(&dir);
    let committed = {
        let mut reserver = open_reserver(&mut store, key(), unprotected(2));
        let g1 = take(&mut reserver);
        announce(g1);
        let committed = read_image(&dir);
        fs::rename(&dir, &moved).expect("rename state directory");
        owner_only(dir.clone());
        // Values inside the already committed window need no store call.
        let g2 = take(&mut reserver);
        announce(g2);
        let error = reserve_error(&mut reserver);
        assert_eq!(
            error,
            ReserveError::Uncertain(PoisonCause::ReadFailed(Slot::A))
        );
        assert_eq!(error.remedy(), Remedy::ReopenStore);
        assert_eq!(
            reserve_error(&mut reserver),
            ReserveError::Poisoned(PoisonCause::ReadFailed(Slot::A))
        );
        committed
    };
    assert_ne!(committed, before);
    assert!(store.is_poisoned());
    assert!(
        entries(&dir).is_empty(),
        "nothing written to the replacement"
    );
    assert_eq!(entries(&moved), STATE_FILES);
    assert_eq!(read_image(&moved), committed);
    drop(store);
    // Startup never recreates state: the empty replacement fails closed Blank.
    let mut replacement = open_store(&dir);
    assert_eq!(
        refused(&mut replacement, key(), unprotected(2)),
        OpenError::Blank
    );

    // (b) Replaced after the pre-commit reads, before the first write: the write
    // re-verifies the identity and fails, `Uncertain(WriteFailed(A))`.
    let dir = owner_only(root.join("store-b"));
    provision_dir(&dir, key(), 1);
    let before = read_image(&dir);
    let moved = root.join("store-b-moved");
    let mut store = open_store(&dir);
    {
        let replacing = ReplaceBeforeWrite {
            inner: &mut store,
            dir: dir.clone(),
            moved: moved.clone(),
            replaced: false,
        };
        let mut reserver = open_reserver(replacing, key(), unprotected(2));
        let error = reserve_error(&mut reserver);
        assert_eq!(
            error,
            ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::A))
        );
        assert_eq!(
            reserve_error(&mut reserver),
            ReserveError::Poisoned(PoisonCause::WriteFailed(Slot::A))
        );
    }
    assert!(store.is_poisoned());
    assert_eq!(store.last_io_error(), Some(ErrorKind::Other));
    let mut buf = [0u8; RECORD_BYTES];
    assert_eq!(store.read(Slot::A, &mut buf), Err(StoreError::Unavailable));
    assert!(
        entries(&dir).is_empty(),
        "nothing written to the replacement"
    );
    assert_eq!(
        entries(&moved),
        STATE_FILES,
        "no tmp in the original either"
    );
    assert_eq!(read_image(&moved), before);
}
