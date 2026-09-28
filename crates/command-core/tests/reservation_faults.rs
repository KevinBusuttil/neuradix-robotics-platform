//! WP-A04.4 fault-injection sweeps for `GenerationReserver` (tests.md F1–F8).
//!
//! A multi-boot timeline driver runs trusted provisioning and then boots that
//! each `open` a reserver and reserve generations over the shared `FaultStore`.
//! Faults are injected by global store-operation index. Every store call goes
//! through a [`Metered`] wrapper so call counts, power state and fired faults
//! stay observable while a reserver owns the store.
//!
//! On every call of every sweep the driver asserts:
//! - **I1 no reuse**: generations activated (returned while the store is
//!   powered) are strictly increasing across the whole multi-boot history, each
//!   lies at or below the reserver's durable ceiling, and `open` never selects a
//!   ceiling below an activated generation;
//! - **bounds**: `open` is exactly 2 reads and 0 writes on every outcome;
//!   `reserve` is 0 calls inside the window, otherwise at most 6 calls with at
//!   most 2 writes (exactly 4 reads and 2 writes when it succeeds);
//!   `reserve_from_window` and `status` make 0 calls; `provision` is at most 6
//!   calls with at most 2 writes, and every refusal is exactly 2 reads, 0 writes;
//!   a successful commit or provision writes both slots, and every `Uncertain`
//!   one has the exact call counts of its cause (a pre-commit poison: 2 reads,
//!   0 writes; a write or read-back failure: stops there and names that slot);
//! - **typed failures**: every refusal is a variant the injected fault set can
//!   cause, is routed by its `remedy()`, and a poison is sticky with 0 calls.
//!
//! The driver re-provisions with a fresh registry epoch whenever
//! `remedy() == Provision` (or a provision attempt failed) and reacquires the
//! store on `ReopenStore`/`RetryOpen`. Sweeps additionally assert I2
//! (liveness), I3 (availability) and a never-decreasing selected ceiling where
//! tests.md names them. Sweeps without marginal slots (F1, F4, F7 and the
//! non-`WeakTail` F6 cases) also check proof step 2 on every commit write: the
//! slot not being written holds a valid record for the key at or above every
//! activated generation (this is what catches mutant M1 with a single fault).
//! The default sweeps F1–F5 run in debug; F6–F8 are the `#[ignore]` release
//! campaign (`cargo test --release -p neuradix-command-core --test
//! reservation_faults -- --ignored --nocapture`).

use std::cell::{Cell, RefCell};
use std::fmt;
use std::num::NonZeroU32;

use neuradix_command_core::ExecutionMode;
use neuradix_command_core::reservation::{
    BindingKey, GenerationReserver, MAX_STORE_CALLS_PER_OPEN, MAX_STORE_CALLS_PER_PROVISION,
    MAX_STORE_CALLS_PER_RESERVE, NamespaceEpoch, OpenError, PoisonCause, ProvisionError,
    ProvisionGuards, RECORD_BYTES, ReceiverId, Remedy, ReservationKey, ReservationStore,
    ReserveError, ReservedGeneration, ReserverConfig, ReserverStatus, RollbackDefense,
    RollbackPosture, Slot, StoreError, provision,
    record::{self, SlotView},
};

#[path = "support/fault_store.rs"]
mod support;
use support::{Fault, FaultStore, Media, OpKind, Tear, WeakPolicy};

/// Boundary tear lengths used in debug (tests.md K_b).
const K_B: [usize; 14] = [0, 1, 4, 8, 24, 31, 32, 40, 48, 52, 59, 60, 63, 64];
/// Marginal-tail lengths for the weak-bit sweeps F2, F2b and F3.
const WEAK_K: [usize; 5] = [0, 1, 31, 59, 63];
/// The four per-boot weak-bit policies.
const PER_BOOT: [WeakPolicy; 4] = [
    WeakPolicy::AlwaysErased,
    WeakPolicy::AlwaysNew,
    WeakPolicy::AlternateEven,
    WeakPolicy::AlternateOdd,
];
/// SAFE-set faults applicable to a read: Crash-on-read (the tear is irrelevant
/// on a read), ReadIo, ReadCorrupt and Unavailable.
const READ_FAULTS: [Fault; 4] = [
    Fault::Crash(Tear::Unchanged),
    Fault::ReadIo,
    Fault::ReadCorrupt,
    Fault::Unavailable,
];
/// F4 power losses: on a read the device dies before reading (one behaviour);
/// on a write, three tears.
const CRASH_ON_READ: [Fault; 1] = [Fault::Crash(Tear::Unchanged)];
const CRASH_ON_WRITE: [Fault; 3] = [
    Fault::Crash(Tear::Erased),
    Fault::Crash(Tear::PrefixOverOld(31)),
    Fault::Crash(Tear::Complete),
];
/// F7 reduced kind set D, split by the operation it applies to. The Crash tears
/// other than Unchanged are inapplicable on a read (the device dies before
/// reading, so they would duplicate Crash-on-read); WriteErr only applies to a
/// write and ReadIo only to a read.
const D_ON_READ: [Fault; 2] = [Fault::Crash(Tear::Unchanged), Fault::ReadIo];
const D_ON_WRITE: [Fault; 5] = [
    Fault::Crash(Tear::Unchanged),
    Fault::Crash(Tear::Erased),
    Fault::Crash(Tear::PrefixOverOld(31)),
    Fault::Crash(Tear::Complete),
    Fault::WriteErr(Tear::Complete),
];
/// Open/provision/reserve attempts per boot, including reacquisitions.
const MAX_ATTEMPTS: u32 = 4;
/// F5/F8 campaign seed (tests.md), overridable with `NEURADIX_RESERVATION_SEED`.
const DEFAULT_SEED: u64 = 0x5EED_A04A_0804_0001;
const SEED_VAR: &str = "NEURADIX_RESERVATION_SEED";

fn key() -> ReservationKey {
    let mut id = [0u8; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = i as u8 + 1;
    }
    ReservationKey::new(
        ReceiverId::new(id).expect("programmed receiver id"),
        BindingKey::numeric(1, 2, 7, ExecutionMode::Live),
    )
}

fn delta(now: (u64, u64), before: (u64, u64)) -> (u64, u64) {
    (now.0 - before.0, now.1 - before.1)
}

/// A successful commit or provision wrote both slots, one each.
fn check_both_written(last_writes: [Option<Slot>; 2], case: &Case) {
    let [Some(first), Some(second)] = last_writes else {
        panic!("a successful commit or provision must write two slots: {last_writes:?} in {case:?}")
    };
    assert_eq!(
        second,
        first.other(),
        "a successful commit or provision must write BOTH slots in {case:?}"
    );
}

/// Exact call counts and addressed slot of a failed commit or provision
/// (algorithm.md RESERVE 3a/3c, PROVISION 7): a pre-commit poison is decided
/// by the 2 reads with nothing written; after that, verified writes are
/// write + read-back pairs that stop at the first failure, never retry and
/// never touch the second slot after a first-slot failure.
fn check_uncertain(
    cause: PoisonCause,
    (reads, writes): (u64, u64),
    last_writes: [Option<Slot>; 2],
    case: &Case,
) {
    match cause {
        PoisonCause::ReadFailed(_) | PoisonCause::MediaChanged => assert_eq!(
            (reads, writes),
            (2, 0),
            "{cause:?} must be decided by the pre-commit reads with nothing written in {case:?}"
        ),
        PoisonCause::WriteFailed(slot) | PoisonCause::VerifyFailed(slot) => {
            // 2 pre-commit reads plus a read-back for every write that
            // completed: `writes + 1` for a write failure, `writes + 2` for a
            // failed read-back.
            let extra_reads = match cause {
                PoisonCause::WriteFailed(_) => 1,
                _ => 2,
            };
            assert!(
                (1..=2).contains(&writes),
                "{cause:?} after {writes} writes in {case:?}"
            );
            assert_eq!(
                reads,
                writes + extra_reads,
                "{cause:?}: {reads} reads for {writes} writes in {case:?}"
            );
            assert_eq!(
                last_writes[1],
                Some(slot),
                "{cause:?} must name the slot written last in {case:?}"
            );
            if writes == 2 {
                assert_eq!(
                    last_writes[0],
                    Some(slot.other()),
                    "the second write must address the other slot in {case:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Store metering
// ---------------------------------------------------------------------------

/// Store-call accounting shared with a [`Metered`] wrapper; readable while a
/// reserver owns the wrapper.
#[derive(Default)]
struct Meter {
    reads: Cell<u64>,
    writes: Cell<u64>,
    dead: Cell<bool>,
    /// Planned fault indices, mirroring the store's plan.
    plan: Cell<[Option<u64>; 2]>,
    /// Planned faults that fired (their operation ran while powered).
    fired: Cell<u32>,
    /// `(kind, powered)` per operation index, when tracing.
    trace: Option<RefCell<Vec<(OpKind, bool)>>>,
    /// Proof step 2 is checked on commit writes when set (see [`Checks`]).
    key: Option<ReservationKey>,
    in_commit: Cell<bool>,
    /// Greatest activated generation, mirrored from the driver state.
    activated_max: Cell<Option<u128>>,
    /// First commit write whose untouched slot did not hold a valid record for
    /// the key at or above every activated generation: `(op, written, other)`.
    order_violation: Cell<Option<(u64, Slot, SlotView)>>,
    /// Slots addressed by the two most recent writes, oldest first.
    last_writes: Cell<[Option<Slot>; 2]>,
}

impl Meter {
    fn calls(&self) -> (u64, u64) {
        (self.reads.get(), self.writes.get())
    }

    fn observe(&self, store: &FaultStore, index: u64, kind: OpKind, powered: bool) {
        let (reads, writes) = store.calls();
        self.reads.set(reads);
        self.writes.set(writes);
        self.dead.set(store.dead());
        if powered && self.plan.get().contains(&Some(index)) {
            self.fired.set(self.fired.get() + 1);
        }
        if let Some(trace) = &self.trace {
            trace.borrow_mut().push((kind, powered));
        }
    }

    /// Proof step 2: while a commit writes `slot`, the slot NOT being written
    /// holds a valid record for this key at or above every activated value.
    /// Checked on the durable bytes, so only for media without marginal slots.
    fn check_untouched(&self, store: &FaultStore, slot: Slot) {
        let Some(key) = self.key else { return };
        if !self.in_commit.get() || store.dead() || self.order_violation.get().is_some() {
            return;
        }
        let other = record::decode(&store.raw(slot.other()));
        let holds = match other {
            SlotView::Valid(f) if f.key == key && f.slot == slot.other() => {
                let value = (u128::from(f.epoch.get()) << 64) | u128::from(f.high_water);
                self.activated_max.get().is_none_or(|max| value >= max)
            }
            _ => false,
        };
        if !holds {
            self.order_violation.set(Some((store.ops(), slot, other)));
        }
    }
}

/// A fresh "handle" over the modelled medium. Reacquiring the store after
/// `ReopenStore` builds a new wrapper; the medium keeps its state.
struct Metered<'a> {
    store: &'a mut FaultStore,
    meter: &'a Meter,
}

impl ReservationStore for Metered<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let (index, powered) = (self.store.ops(), !self.store.dead());
        let result = self.store.read(slot, buf);
        self.meter.observe(self.store, index, OpKind::Read, powered);
        result
    }

    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let (index, powered) = (self.store.ops(), !self.store.dead());
        self.meter.check_untouched(self.store, slot);
        let result = self.store.write(slot, record);
        self.meter
            .observe(self.store, index, OpKind::Write, powered);
        let [_, previous] = self.meter.last_writes.get();
        self.meter.last_writes.set([previous, Some(slot)]);
        result
    }
}

// ---------------------------------------------------------------------------
// Tallies (printed per sweep)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Ev {
    Provisioned,
    ProvisionRefused,
    ProvisionUncertain,
    Opened,
    OpenUnavailable,
    OpenUnreadable,
    OpenBlank,
    OpenCorrupt,
    Commit,
    WindowHit,
    Activated,
    WriteFailed,
    VerifyFailed,
    ReadFailed,
    MediaChanged,
    PowerLost,
    UnavailableAtEnd,
}
const EVENTS: [Ev; 17] = [
    Ev::Provisioned,
    Ev::ProvisionRefused,
    Ev::ProvisionUncertain,
    Ev::Opened,
    Ev::OpenUnavailable,
    Ev::OpenUnreadable,
    Ev::OpenBlank,
    Ev::OpenCorrupt,
    Ev::Commit,
    Ev::WindowHit,
    Ev::Activated,
    Ev::WriteFailed,
    Ev::VerifyFailed,
    Ev::ReadFailed,
    Ev::MediaChanged,
    Ev::PowerLost,
    Ev::UnavailableAtEnd,
];

#[derive(Debug, Default, Clone, Copy)]
struct Tally([u64; EVENTS.len()]);

impl Tally {
    fn bump(&mut self, event: Ev) {
        self.0[event as usize] += 1;
    }
    fn get(&self, event: Ev) -> u64 {
        self.0[event as usize]
    }
    fn absorb(&mut self, other: &Tally) {
        for (mine, theirs) in self.0.iter_mut().zip(other.0) {
            *mine += theirs;
        }
    }
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for event in EVENTS {
            let n = self.get(event);
            if n > 0 {
                write!(f, "{}{event:?}={n}", if first { "" } else { " " })?;
                first = false;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Timeline driver
// ---------------------------------------------------------------------------

/// Invariants asserted in addition to I1, the bounds and typed failures.
#[derive(Debug, Default, Clone, Copy)]
struct Checks {
    /// I2: every boot after the faults stop opens first time and activates
    /// every planned value.
    i2: bool,
    /// I3: after the first successful provision an open fails only with
    /// `StoreUnavailable`/`Unreadable` caused by a fault firing during that very
    /// open (so the next fault-free open succeeds); and a reserve fails only in
    /// a call during which the fault fired.
    i3: bool,
    /// The ceiling selected at each successful open never decreases.
    monotone: bool,
    /// F2b: a reserve failure not caused by the injected write itself is
    /// `MediaChanged`, `ReadFailed` or `VerifyFailed`.
    per_read_typed: bool,
    /// F3 (R25 characterization): every failed commit or provision on an
    /// ECC-refuse medium is `Uncertain(WriteFailed)`.
    refuse_typed: bool,
    /// Proof step 2 on every commit write (the untouched slot holds a valid
    /// record for the key at or above every activated value). Only for runs
    /// without marginal (`WeakTail`) slots, whose durable bytes differ from
    /// what they read as.
    write_order: bool,
}

#[derive(Debug, Clone, Copy)]
struct Timeline {
    sweep: &'static str,
    erased: u8,
    media: Media,
    policy: WeakPolicy,
    window: u32,
    reserves: u32,
    /// Boots (after the provisioning session) whose operations are fault
    /// positions.
    fault_boots: u32,
    /// Fault-free boots after the fault window.
    later_boots: u32,
    checks: Checks,
}

/// Identifies the running case in assertion messages (formatted only on failure).
#[derive(Clone, Copy)]
struct Case {
    sweep: &'static str,
    timeline: Option<Timeline>,
    faults: [Option<(u64, Fault)>; 2],
    seed: Option<u64>,
    boot: u64,
}

impl fmt::Debug for Case {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} boot {}", self.sweep, self.boot)?;
        if let Some(seed) = self.seed {
            write!(f, " seed {seed:#018x}")?;
        }
        for (op, fault) in self.faults.iter().flatten() {
            write!(f, " fault {fault:?}@op{op}")?;
        }
        if let Some(t) = &self.timeline {
            write!(f, " {t:?}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct BootPlan {
    window: u32,
    reserves: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct BootReport {
    activated: u32,
    open_failures: u32,
    reserve_failures: u32,
}

enum Next {
    Done,
    Reacquire,
    EndBoot,
}

enum Step {
    Activated,
    Reacquire,
    PowerLost,
}

/// Driver state that is not the store, so it stays usable while a reserver
/// borrows the store.
struct State {
    key: ReservationKey,
    checks: Checks,
    case: Case,
    /// Next epoch of the modelled registry: durable, strictly increasing,
    /// single-use (a failed provision burns its epoch).
    next_epoch: u64,
    needs_provision: bool,
    provisioned_once: bool,
    /// Last (therefore greatest) activated generation.
    last: Option<u128>,
    max_selected: Option<u128>,
    tally: Tally,
}

struct Driver {
    store: FaultStore,
    meter: Meter,
    st: State,
}

impl Driver {
    fn new(store: FaultStore, checks: Checks, case: Case, trace: bool) -> Self {
        let meter = Meter {
            trace: trace.then(|| RefCell::new(Vec::new())),
            key: checks.write_order.then(key),
            ..Meter::default()
        };
        Self {
            store,
            meter,
            st: State {
                key: key(),
                checks,
                case,
                next_epoch: 1,
                needs_provision: true,
                provisioned_once: false,
                last: None,
                max_selected: None,
                tally: Tally::default(),
            },
        }
    }

    fn inject(&mut self, faults: &[(u64, Fault)]) {
        let mut plan = self.meter.plan.get();
        for &(op, fault) in faults {
            self.store.inject(op, fault);
            let free = plan
                .iter()
                .position(Option::is_none)
                .expect("at most two planned faults");
            plan[free] = Some(op);
            self.st.case.faults[free] = Some((op, fault));
        }
        self.meter.plan.set(plan);
    }

    fn clear_faults(&mut self) {
        self.store.clear_faults();
        self.meter.plan.set([None; 2]);
        self.st.case.faults = [None; 2];
    }

    /// Change media behaviour in place (campaign weak-bit events).
    fn set_media(&mut self, media: Media, policy: WeakPolicy) {
        let store = std::mem::replace(&mut self.store, FaultStore::erased(0xFF));
        self.store = store.with_media(media, policy);
    }

    /// The trusted provisioning session before the first boot: retry with a
    /// fresh epoch until Ok, or until the power is lost.
    fn provision_session(&mut self) {
        for _ in 0..MAX_ATTEMPTS {
            if self.store.dead() || self.provision_once() {
                break;
            }
        }
    }

    /// One trusted provision with the next registry epoch. Returns success.
    fn provision_once(&mut self) -> bool {
        let Driver { store, meter, st } = self;
        let meter: &Meter = meter;
        let epoch = NamespaceEpoch::new(st.next_epoch).expect("nonzero epoch");
        st.next_epoch += 1;
        let calls = meter.calls();
        let result = provision(
            &mut Metered { store, meter },
            st.key,
            epoch,
            ProvisionGuards::default(),
        );
        let (reads, writes) = delta(meter.calls(), calls);
        assert!(
            reads + writes <= u64::from(MAX_STORE_CALLS_PER_PROVISION) && writes <= 2,
            "provision made {reads} reads and {writes} writes in {:?}",
            st.case
        );
        match result {
            Ok(report) => {
                assert_eq!((reads, writes), (4, 2), "provision in {:?}", st.case);
                check_both_written(meter.last_writes.get(), &st.case);
                assert_eq!(report.epoch, epoch);
                assert!(report.replaced < Some(epoch), "{report:?} in {:?}", st.case);
                st.needs_provision = false;
                st.provisioned_once = true;
                st.tally.bump(Ev::Provisioned);
                true
            }
            Err(ProvisionError::Uncertain(cause)) => {
                // Provision only fails uncertainly in a verified write.
                assert!(
                    matches!(
                        cause,
                        PoisonCause::WriteFailed(_) | PoisonCause::VerifyFailed(_)
                    ),
                    "provision Uncertain({cause:?}) in {:?}",
                    st.case
                );
                check_uncertain(cause, (reads, writes), meter.last_writes.get(), &st.case);
                if st.checks.refuse_typed && !meter.dead.get() {
                    assert!(
                        matches!(cause, PoisonCause::WriteFailed(_)),
                        "ECC-refuse provision failed with {cause:?} in {:?}",
                        st.case
                    );
                }
                st.tally.bump(Ev::ProvisionUncertain);
                false
            }
            Err(ProvisionError::Store(StoreError::Io | StoreError::Unavailable)) => {
                assert_eq!(
                    (reads, writes),
                    (2, 0),
                    "a provision refusal must be 2 reads and 0 writes in {:?}",
                    st.case
                );
                st.tally.bump(Ev::ProvisionRefused);
                false
            }
            Err(other) => panic!("untyped provision failure {other:?} in {:?}", st.case),
        }
    }

    /// Power cycle, then provision if required, open and reserve, reacquiring
    /// the store on `ReopenStore`/`RetryOpen` (bounded).
    fn boot(&mut self, plan: BootPlan) -> BootReport {
        self.store.reboot();
        self.meter.dead.set(self.store.dead());
        let mut report = BootReport::default();
        for _ in 0..MAX_ATTEMPTS {
            if self.store.dead() {
                break;
            }
            if self.st.needs_provision && !self.provision_once() {
                continue;
            }
            match self.open_and_reserve(plan, &mut report) {
                Next::Done | Next::EndBoot => break,
                Next::Reacquire => {}
            }
        }
        report
    }

    fn open_and_reserve(&mut self, plan: BootPlan, report: &mut BootReport) -> Next {
        let Driver { store, meter, st } = self;
        let meter: &Meter = meter;
        let config = ReserverConfig::new(
            NonZeroU32::new(plan.window).expect("nonzero window"),
            RollbackDefense::Unprotected,
        );
        let calls = meter.calls();
        let fired = meter.fired.get();
        let opened = GenerationReserver::open(Metered { store, meter }, st.key, config);
        assert_eq!(
            delta(meter.calls(), calls),
            (u64::from(MAX_STORE_CALLS_PER_OPEN), 0),
            "open must make exactly 2 reads and 0 writes in {:?}",
            st.case
        );
        let mut reserver = match opened {
            Ok(reserver) => reserver,
            Err(failure) => {
                report.open_failures += 1;
                let fired_in_open = meter.fired.get() != fired;
                return st.open_failed(failure.error, fired_in_open, meter.dead.get());
            }
        };
        let after_open = meter.calls();
        st.opened(&reserver.status());
        assert_eq!(meter.calls(), after_open, "status must make 0 store calls");
        for i in 0..plan.reserves {
            match st.reserve_step(&mut reserver, meter, i) {
                Step::Activated => report.activated += 1,
                Step::Reacquire => {
                    report.reserve_failures += 1;
                    return Next::Reacquire;
                }
                Step::PowerLost => {
                    report.reserve_failures += 1;
                    return Next::EndBoot;
                }
            }
        }
        Next::Done
    }
}

impl State {
    fn open_failed(&mut self, error: OpenError, fired_in_open: bool, dead: bool) -> Next {
        let (event, remedy) = match error {
            OpenError::StoreUnavailable => (Ev::OpenUnavailable, Remedy::ReopenStore),
            OpenError::Unreadable => (Ev::OpenUnreadable, Remedy::RetryOpen),
            OpenError::Blank => (Ev::OpenBlank, Remedy::Provision),
            OpenError::Corrupt => (Ev::OpenCorrupt, Remedy::Provision),
            other => panic!(
                "untyped open failure {other:?} (not producible by the injected faults) in {:?}",
                self.case
            ),
        };
        assert_eq!(error.remedy(), remedy, "{error:?}");
        self.tally.bump(event);
        if self.checks.i3 && self.provisioned_once {
            assert!(
                fired_in_open && remedy != Remedy::Provision,
                "I3 violated: open -> {error:?} after the first successful provision \
                 (fault fired during this open: {fired_in_open}) in {:?}",
                self.case
            );
        }
        if remedy == Remedy::Provision {
            // Trusted, out-of-band provisioning with a fresh registry epoch.
            self.needs_provision = true;
        }
        if dead { Next::EndBoot } else { Next::Reacquire }
    }

    fn opened(&mut self, status: &ReserverStatus) {
        assert_eq!(status.key, self.key);
        assert_eq!(status.poisoned, None);
        assert_eq!(
            status.window_remaining, 0,
            "the first reserve after open must commit"
        );
        assert_eq!(status.rollback, RollbackPosture::Unprotected);
        let selected = status.durable_ceiling;
        if let Some(last) = self.last {
            assert!(
                selected >= last,
                "I1 violated: open selected ceiling {selected:#x} below activated {last:#x} in {:?}",
                self.case
            );
        }
        if self.checks.monotone
            && let Some(max) = self.max_selected
        {
            assert!(
                selected >= max,
                "selected ceiling decreased from {max:#x} to {selected:#x} in {:?}",
                self.case
            );
        }
        self.max_selected = self.max_selected.max(Some(selected));
        self.tally.bump(Ev::Opened);
    }

    fn reserve_step(
        &mut self,
        reserver: &mut GenerationReserver<Metered<'_>>,
        meter: &Meter,
        i: u32,
    ) -> Step {
        let before = reserver.status();
        let calls = meter.calls();
        let fired = meter.fired.get();
        assert_eq!(before.poisoned, None);
        let result = if before.window_remaining > 0 {
            // Window hit: alternate `reserve` and `reserve_from_window`.
            let result = if i.is_multiple_of(2) {
                reserver.reserve()
            } else {
                reserver.reserve_from_window()
            };
            assert_eq!(
                meter.calls(),
                calls,
                "a window hit must make 0 store calls in {:?}",
                self.case
            );
            assert!(
                result.is_ok(),
                "window hit failed: {result:?} in {:?}",
                self.case
            );
            self.tally.bump(Ev::WindowHit);
            result
        } else {
            let spare = reserver.reserve_from_window();
            assert!(
                matches!(spare, Err(ReserveError::CommitRequired)),
                "reserve_from_window with an empty window: {spare:?} in {:?}",
                self.case
            );
            assert_eq!(meter.calls(), calls, "reserve_from_window made store calls");
            assert_eq!(reserver.status(), before, "CommitRequired changed state");
            meter.in_commit.set(true);
            let result = reserver.reserve();
            meter.in_commit.set(false);
            if let Some((op, written, other)) = meter.order_violation.get() {
                panic!(
                    "proof step 2 violated: op {op} wrote {written:?} while the untouched slot \
                     was {other:?} (activated max {:#x?}) in {:?}",
                    self.last, self.case
                );
            }
            let (reads, writes) = delta(meter.calls(), calls);
            assert!(
                reads + writes <= u64::from(MAX_STORE_CALLS_PER_RESERVE) && writes <= 2,
                "reserve made {reads} reads and {writes} writes in {:?}",
                self.case
            );
            match &result {
                Ok(_) => {
                    assert_eq!(
                        (reads, writes),
                        (4, 2),
                        "a successful commit in {:?}",
                        self.case
                    );
                    check_both_written(meter.last_writes.get(), &self.case);
                }
                // The first failure of a fresh reserver is always `Uncertain`,
                // also when the power was lost during the call.
                Err(ReserveError::Uncertain(cause)) => {
                    check_uncertain(*cause, (reads, writes), meter.last_writes.get(), &self.case);
                }
                Err(error) => {
                    panic!("untyped first reserve failure {error:?} in {:?}", self.case)
                }
            }
            self.tally.bump(Ev::Commit);
            result
        };
        match result {
            Ok(token) => {
                assert!(
                    !meter.dead.get(),
                    "a generation was returned after power loss in {:?}",
                    self.case
                );
                self.activate(token, &reserver.status());
                meter.activated_max.set(self.last);
                Step::Activated
            }
            Err(_) if meter.dead.get() => {
                // Power lost during the call: nobody observes the result.
                self.tally.bump(Ev::PowerLost);
                Step::PowerLost
            }
            Err(error) => {
                let ReserveError::Uncertain(cause) = error else {
                    panic!("untyped first reserve failure {error:?} in {:?}", self.case)
                };
                assert_eq!(error.remedy(), Remedy::ReopenStore);
                let fired_here = meter.fired.get() != fired;
                self.tally.bump(match cause {
                    PoisonCause::WriteFailed(_) => Ev::WriteFailed,
                    PoisonCause::VerifyFailed(_) => Ev::VerifyFailed,
                    PoisonCause::ReadFailed(_) => Ev::ReadFailed,
                    PoisonCause::MediaChanged => Ev::MediaChanged,
                });
                if self.checks.i3 {
                    assert!(
                        fired_here,
                        "availability: reserve -> {error:?} without a fault in that call, in {:?}",
                        self.case
                    );
                }
                if self.checks.per_read_typed {
                    // W9b: per-read weak bits fail closed only as MediaChanged,
                    // ReadFailed or VerifyFailed. The only other failure is
                    // the injected `WriteErr` itself, in the call it fired in.
                    assert!(
                        if fired_here {
                            matches!(cause, PoisonCause::WriteFailed(_))
                        } else {
                            matches!(
                                cause,
                                PoisonCause::MediaChanged
                                    | PoisonCause::ReadFailed(_)
                                    | PoisonCause::VerifyFailed(_)
                            )
                        },
                        "per-read weak bits gave {cause:?} (injected fault fired in this call: \
                         {fired_here}) in {:?}",
                        self.case
                    );
                }
                if self.checks.refuse_typed {
                    assert!(
                        matches!(cause, PoisonCause::WriteFailed(_)),
                        "ECC-refuse commit failed with {cause:?} in {:?}",
                        self.case
                    );
                }
                // The poison is sticky and costs no store calls.
                let calls = meter.calls();
                let again = reserver.reserve();
                assert!(
                    matches!(again, Err(ReserveError::Poisoned(c)) if c == cause),
                    "after Uncertain({cause:?}): {again:?}"
                );
                let spare = reserver.reserve_from_window();
                assert!(
                    matches!(spare, Err(ReserveError::Poisoned(c)) if c == cause),
                    "after Uncertain({cause:?}): {spare:?}"
                );
                assert_eq!(reserver.status().poisoned, Some(cause));
                assert_eq!(meter.calls(), calls, "a poisoned reserver made store calls");
                Step::Reacquire
            }
        }
    }

    fn activate(&mut self, token: ReservedGeneration, status: &ReserverStatus) {
        let value = token.generation().get();
        assert_eq!(token.key(), self.key);
        assert_eq!(token.epoch(), status.epoch);
        assert_eq!(value >> 64, u128::from(status.epoch.get()));
        assert_ne!(value as u64, 0, "reserved counters start at 1");
        assert!(
            value <= status.durable_ceiling,
            "issued {value:#x} above the durable ceiling {:#x} in {:?}",
            status.durable_ceiling,
            self.case
        );
        assert_eq!(
            status.durable_ceiling - value,
            u128::from(status.window_remaining)
        );
        if let Some(last) = self.last {
            assert!(
                value > last,
                "I1 violated: activated {value:#x} after {last:#x} in {:?}",
                self.case
            );
        }
        self.last = Some(value);
        self.tally.bump(Ev::Activated);
    }
}

// ---------------------------------------------------------------------------
// Timelines
// ---------------------------------------------------------------------------

struct RunResult {
    tally: Tally,
    /// Global operation count at the end of the fault window.
    window_end: u64,
    trace: Vec<(OpKind, bool)>,
    available: bool,
}

/// Provisioning session, `fault_boots` boots with `faults` planned, then
/// `later_boots` fault-free boots (I2 is asserted on each of them if enabled).
fn run(t: &Timeline, faults: &[(u64, Fault)], trace: bool) -> RunResult {
    let case = Case {
        sweep: t.sweep,
        timeline: Some(*t),
        faults: [None; 2],
        seed: None,
        boot: 0,
    };
    let store = FaultStore::erased(t.erased).with_media(t.media, t.policy);
    let mut d = Driver::new(store, t.checks, case, trace);
    d.inject(faults);
    d.provision_session();
    let plan = BootPlan {
        window: t.window,
        reserves: t.reserves,
    };
    for boot in 1..=u64::from(t.fault_boots) {
        d.st.case.boot = boot;
        d.boot(plan);
    }
    let window_end = d.store.ops();
    assert_eq!(
        d.meter.fired.get() as usize,
        faults.len(),
        "every planned fault must fire inside the fault window: {:?}",
        d.st.case
    );
    d.clear_faults();
    let mut available = false;
    for boot in 1..=u64::from(t.later_boots) {
        d.st.case.boot = u64::from(t.fault_boots) + boot;
        let report = d.boot(plan);
        if t.checks.i2 {
            assert!(
                report.open_failures == 0
                    && report.reserve_failures == 0
                    && report.activated == t.reserves,
                "I2 violated: fault-free boot {report:?} in {:?}",
                d.st.case
            );
        }
        available = report.activated > 0;
    }
    if !available {
        d.st.tally.bump(Ev::UnavailableAtEnd);
    }
    RunResult {
        tally: d.st.tally,
        window_end,
        trace: d
            .meter
            .trace
            .take()
            .map(RefCell::into_inner)
            .unwrap_or_default(),
        available,
    }
}

fn count(kinds: &[OpKind]) -> (u64, u64) {
    let reads = kinds.iter().filter(|k| **k == OpKind::Read).count() as u64;
    (reads, kinds.len() as u64 - reads)
}

/// Operation kinds of the fault-free fault window, cross-checked against the
/// normative call counts: provision = 4 reads + 2 writes, open = 2 reads, and
/// every commit = 4 reads + 2 writes, with ceil(reserves / window) commits per
/// boot (the first reserve of every boot commits).
fn reference(t: &Timeline) -> Vec<OpKind> {
    let result = run(t, &[], true);
    let window = &result.trace[..result.window_end as usize];
    assert!(window.iter().all(|&(_, powered)| powered));
    let kinds: Vec<OpKind> = window.iter().map(|&(kind, _)| kind).collect();
    let commits = u64::from(t.fault_boots) * u64::from(t.reserves.div_ceil(t.window));
    assert_eq!(
        count(&kinds),
        (
            4 + 2 * u64::from(t.fault_boots) + 4 * commits,
            2 + 2 * commits
        ),
        "fault-free timeline shape for {t:?}"
    );
    assert_eq!(
        result.tally.get(Ev::Activated),
        u64::from((t.fault_boots + t.later_boots) * t.reserves)
    );
    kinds
}

fn boundary_tears() -> Vec<Tear> {
    let mut tears = vec![Tear::Unchanged, Tear::Erased, Tear::Complete];
    tears.extend(K_B.iter().map(|&k| Tear::PrefixOverOld(k)));
    tears.extend(K_B.iter().map(|&k| Tear::PrefixOverErased(k)));
    tears
}

/// SAFE-set faults applicable to a write: Crash and WriteErr with each tear,
/// SilentDrop and Unavailable.
fn write_faults(tears: &[Tear]) -> Vec<Fault> {
    let mut faults: Vec<Fault> = tears
        .iter()
        .flat_map(|&tear| [Fault::Crash(tear), Fault::WriteErr(tear)])
        .collect();
    faults.extend([Fault::SilentDrop, Fault::Unavailable]);
    faults
}

fn i123() -> Checks {
    Checks {
        i2: true,
        i3: true,
        monotone: true,
        ..Checks::default()
    }
}

/// [`i123`] plus the proof-step-2 write-order check (no marginal slots).
fn i123_ordered() -> Checks {
    Checks {
        write_order: true,
        ..i123()
    }
}

// ---------------------------------------------------------------------------
// Default sweeps (debug)
// ---------------------------------------------------------------------------

/// F1: window {1, 3} × reserves {1, 4}; erased 0xFF; Plain; provision + 3
/// boots; a single SAFE-set fault at every applicable op index.
#[test]
fn f1_single_fault_boundary() {
    let writes = write_faults(&boundary_tears());
    assert_eq!(writes.len(), 64);
    let (mut cases, mut expected, mut total) = (0u64, 0u64, Tally::default());
    for window in [1, 3] {
        for reserves in [1, 4] {
            let t = Timeline {
                sweep: "F1",
                erased: 0xFF,
                media: Media::Plain,
                policy: WeakPolicy::AlwaysErased,
                window,
                reserves,
                fault_boots: 3,
                later_boots: 1,
                checks: i123_ordered(),
            };
            let kinds = reference(&t);
            let (r, w) = count(&kinds);
            expected += r * READ_FAULTS.len() as u64 + w * writes.len() as u64;
            for (op, &kind) in kinds.iter().enumerate() {
                let faults: &[Fault] = match kind {
                    OpKind::Read => &READ_FAULTS,
                    OpKind::Write => &writes,
                };
                for &fault in faults {
                    total.absorb(&run(&t, &[(op as u64, fault)], false).tally);
                    cases += 1;
                }
            }
        }
    }
    println!("F1 single_fault_boundary: {cases} cases; {total}");
    assert_eq!(cases, expected);
    assert_eq!(cases, 4_128);
    assert_eq!(total.get(Ev::UnavailableAtEnd), 0);
}

/// Crash and WriteErr leaving a marginal tail `WeakTail(k)` at every write of
/// provision + 3 boots (window 1, one reserve per boot), followed by 4 later
/// boots, for each erased value, medium and policy.
fn weak_sweep(
    sweep: &'static str,
    media: &[Media],
    policies: &[WeakPolicy],
    checks: Checks,
) -> (u64, Tally) {
    let (mut cases, mut total) = (0u64, Tally::default());
    for erased in [0xFF, 0x00] {
        for &medium in media {
            for &policy in policies {
                let t = Timeline {
                    sweep,
                    erased,
                    media: medium,
                    policy,
                    window: 1,
                    reserves: 1,
                    fault_boots: 3,
                    later_boots: 4,
                    checks,
                };
                let kinds = reference(&t);
                for (op, _) in kinds
                    .iter()
                    .enumerate()
                    .filter(|(_, k)| **k == OpKind::Write)
                {
                    for k in WEAK_K {
                        let tear = Tear::WeakTail(k);
                        for fault in [Fault::Crash(tear), Fault::WriteErr(tear)] {
                            total.absorb(&run(&t, &[(op as u64, fault)], false).tally);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    (cases, total)
}

/// F2: WeakTail(k ∈ {0, 1, 31, 59, 63}) × 4 per-boot policies × {Plain,
/// EccFlag} × erased {0xFF, 0x00} at every write; 4 later boots. Asserts I1,
/// I3 and a never-decreasing selected maximum (and I2 on the later boots).
#[test]
fn f2_weak_bits_per_boot() {
    let (cases, total) = weak_sweep("F2", &[Media::Plain, Media::EccFlag], &PER_BOOT, i123());
    println!("F2 weak_bits_per_boot: {cases} cases; {total}");
    // 8 writes × {Crash, WriteErr} × 5 k × 4 policies × 2 media × 2 erased.
    assert_eq!(cases, 1_280);
    assert_eq!(total.get(Ev::UnavailableAtEnd), 0);
}

/// F2b: as F2 with the PerRead policy. I1 only; every non-Ok result is typed.
#[test]
fn f2b_weak_bits_per_read() {
    let checks = Checks {
        per_read_typed: true,
        ..Checks::default()
    };
    let (cases, total) = weak_sweep(
        "F2b",
        &[Media::Plain, Media::EccFlag],
        &[WeakPolicy::PerRead],
        checks,
    );
    println!(
        "F2b weak_bits_per_read: {cases} cases; {total} (MediaChanged/ReadFailed/VerifyFailed \
         are the typed availability losses of W9b)"
    );
    assert_eq!(cases, 320);
}

/// F3: ECC-refuse medium, a marginal tail at every write index. Asserts I1;
/// characterizes availability (R25/W10: persistent `Uncertain(WriteFailed)`).
#[test]
fn f3_ecc_refuse() {
    let checks = Checks {
        refuse_typed: true,
        ..Checks::default()
    };
    let (cases, total) = weak_sweep(
        "F3",
        &[Media::EccRefuse],
        &[WeakPolicy::AlwaysErased],
        checks,
    );
    let unavailable = total.get(Ev::UnavailableAtEnd);
    println!(
        "F3 ecc_refuse: {cases} cases; availability lost in {unavailable}/{cases} \
         (every failed commit or provision is Uncertain(WriteFailed)); {total}"
    );
    assert_eq!(cases, 160);
    // CHAR (W10): a marginal slot that refuses programming is never rewritten,
    // so every case ends unavailable (remedy RepairStore), with no reuse.
    assert_eq!(unavailable, cases);
}

/// Every ordered pair (i < j) of op indices ≡ 0 (mod `stride`), with the
/// faults `faults_for` applicable to the operation at each index. The kinds at
/// j come from the run with only the first fault, so they reflect the diverged
/// timeline; an index executed while the device is off cannot host a fault and
/// is skipped (not counted), like every inapplicable pair.
fn pair_sweep(
    t: &Timeline,
    faults_for: fn(OpKind) -> &'static [Fault],
    stride: usize,
    total: &mut Tally,
) -> u64 {
    let kinds = reference(t);
    let mut cases = 0;
    for (i, &kind_i) in kinds.iter().enumerate().step_by(stride) {
        for &first in faults_for(kind_i) {
            let single = run(t, &[(i as u64, first)], true);
            for j in (i + 1..single.window_end as usize).filter(|j| j % stride == 0) {
                let (kind_j, powered) = single.trace[j];
                if !powered {
                    continue;
                }
                for &second in faults_for(kind_j) {
                    let faults = [(i as u64, first), (j as u64, second)];
                    total.absorb(&run(t, &faults, false).tally);
                    cases += 1;
                }
            }
        }
    }
    cases
}

fn crash_faults(kind: OpKind) -> &'static [Fault] {
    match kind {
        OpKind::Read => &CRASH_ON_READ,
        OpKind::Write => &CRASH_ON_WRITE,
    }
}

/// F4: every (i, j) pair of power losses with tears {Erased, PrefixOverOld(31),
/// Complete} (Crash-on-read on reads), window {1, 3} × reserves {1, 4}.
/// Asserts I1 and I3 (and I2 after the faults stop).
#[test]
fn f4_paired_power_losses() {
    let (mut cases, mut total) = (0u64, Tally::default());
    for window in [1, 3] {
        for reserves in [1, 4] {
            let t = Timeline {
                sweep: "F4",
                erased: 0xFF,
                media: Media::Plain,
                policy: WeakPolicy::AlwaysErased,
                window,
                reserves,
                fault_boots: 3,
                later_boots: 1,
                checks: i123_ordered(),
            };
            cases += pair_sweep(&t, crash_faults, 1, &mut total);
        }
    }
    println!("F4 paired_power_losses: {cases} cases; {total}");
    assert_eq!(cases, F4_CASES);
    assert_eq!(total.get(Ev::UnavailableAtEnd), 0);
}
const F4_CASES: u64 = 10_648;

// ---------------------------------------------------------------------------
// Seeded campaign (F5 default, F8 release)
// ---------------------------------------------------------------------------

struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

fn campaign_seed() -> u64 {
    let Ok(text) = std::env::var(SEED_VAR) else {
        return DEFAULT_SEED;
    };
    let digits = text.trim().replace('_', "");
    let parsed = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => digits.parse(),
    };
    let seed = parsed.unwrap_or_else(|e| panic!("{SEED_VAR}={text:?} is not a u64: {e}"));
    assert_ne!(seed, 0, "xorshift64 needs a nonzero seed");
    seed
}

#[derive(Debug, Clone, Copy)]
enum Event {
    Clean,
    SafeFault,
    Rot,
    WeakBits,
}

fn safe_fault(rng: &mut XorShift64, tears: &[Tear]) -> Fault {
    let tear = rng.pick(tears);
    match rng.below(6) {
        0 => Fault::Crash(tear),
        1 => Fault::WriteErr(tear),
        2 => Fault::SilentDrop,
        3 => Fault::ReadIo,
        4 => Fault::ReadCorrupt,
        _ => Fault::Unavailable,
    }
}

/// Each boot picks: a clean boot; one SAFE-set fault at a random op of the boot
/// (tears from K_b); single-slot rot at rest; or weak bits (Plain or EccFlag,
/// a per-boot policy, a marginal tail left at a random op). The driver
/// re-provisions with a fresh registry epoch whenever `remedy() == Provision`
/// and reacquires the store on `ReopenStore`. Lie, snapshot/restore, reflash,
/// clone and EccRefuse are excluded. Asserts I1 and typed failures throughout,
/// then I2 once the faults stop.
fn campaign(sweep: &'static str, boots: u64) -> (u64, Tally) {
    let seed = campaign_seed();
    println!("{sweep}: xorshift64 seed {seed:#018x} (override with {SEED_VAR}); {boots} boots");
    let mut rng = XorShift64(seed);
    let tears = boundary_tears();
    let case = Case {
        sweep,
        timeline: None,
        faults: [None; 2],
        seed: Some(seed),
        boot: 0,
    };
    let mut d = Driver::new(FaultStore::erased(0xFF), Checks::default(), case, false);
    d.provision_session();
    let mut events = [0u64; 4];
    let mut run_boots = 0u64;
    for boot in 1..=boots {
        d.clear_faults();
        d.st.case.boot = boot;
        let plan = BootPlan {
            window: rng.pick(&[1, 2, 3, 16]),
            reserves: 1 + rng.below(4) as u32,
        };
        // Operations the boot makes before any fault fires: provisioning when
        // required (6), open (2) and 6 per commit (the first reserve of the
        // boot and every window end), so the fault lands on a reached op.
        let commits = u64::from(plan.reserves.div_ceil(plan.window));
        let span = 2 + 6 * commits + if d.st.needs_provision { 6 } else { 0 };
        let event = match rng.below(10) {
            0..=3 => Event::Clean,
            4..=6 => Event::SafeFault,
            7 => Event::Rot,
            _ => Event::WeakBits,
        };
        match event {
            Event::Clean => {}
            Event::SafeFault => {
                let at = d.store.ops() + rng.below(span);
                let fault = safe_fault(&mut rng, &tears);
                d.inject(&[(at, fault)]);
            }
            Event::Rot => {
                let slot = rng.pick(&[Slot::A, Slot::B]);
                let byte = rng.below(RECORD_BYTES as u64) as usize;
                let mask = 1 + rng.below(255) as u8;
                d.store.rot(slot, byte, mask);
            }
            Event::WeakBits => {
                let media = rng.pick(&[Media::Plain, Media::EccFlag]);
                let policy = rng.pick(&PER_BOOT);
                d.set_media(media, policy);
                let tear = Tear::WeakTail(rng.pick(&K_B));
                let fault = rng.pick(&[Fault::Crash(tear), Fault::WriteErr(tear)]);
                let at = d.store.ops() + rng.below(span);
                d.inject(&[(at, fault)]);
            }
        }
        events[event as usize] += 1;
        d.boot(plan);
        run_boots += 1;
    }
    // Non-vacuity: injected faults reached powered operations and produced
    // power losses, write and verify failures and unavailable opens. At 2,000
    // boots each count is expected in the tens, for any seed.
    let fired = u64::from(d.meter.fired.get());
    let tally = d.st.tally;
    println!(
        "{sweep}: {fired} of {} injected faults fired on a powered operation",
        events[Event::SafeFault as usize] + events[Event::WeakBits as usize]
    );
    for event in [
        Ev::PowerLost,
        Ev::WriteFailed,
        Ev::VerifyFailed,
        Ev::OpenUnavailable,
    ] {
        assert!(
            tally.get(event) > 0,
            "campaign never produced {event:?}: {tally}"
        );
    }
    assert!(fired > 0);
    // I2: after the faults stop, a clean boot (with trusted re-provisioning if
    // a remedy asked for it) opens and activates values.
    d.clear_faults();
    let before = d.st.tally.get(Ev::Activated);
    let plan = BootPlan {
        window: 3,
        reserves: 2,
    };
    let report = d.boot(plan);
    assert_eq!(
        report.activated, plan.reserves,
        "I2 violated after the campaign: {report:?} in {:?}",
        d.st.case
    );
    assert_eq!(d.st.tally.get(Ev::Activated), before + 2);
    let total = d.st.tally;
    println!(
        "{sweep}: {run_boots} boots (clean={} safe_fault={} rot={} weak={}); epochs used={}; {total}",
        events[0],
        events[1],
        events[2],
        events[3],
        d.st.next_epoch - 1
    );
    (run_boots, total)
}

/// F5: 2,000-boot seeded campaign.
#[test]
fn f5_seeded_campaign() {
    let (boots, total) = campaign("F5 seeded_campaign", 2_000);
    assert_eq!(boots, 2_000);
    assert!(total.get(Ev::Activated) > 0);
}

// ---------------------------------------------------------------------------
// Release sweeps (#[ignore]; CI job `reservation-faults`)
// ---------------------------------------------------------------------------

/// F6: windows {1, 3, 16} × reserves {1, 4} × erased {0xFF, 0x00} × media
/// {Plain, EccFlag, EccRefuse} × (Crash, WriteErr) × tears
/// PrefixOver{Old, Erased}(0..=64) + Unchanged/Erased/Complete, plus
/// WeakTail(0..=64) × 4 per-boot policies, plus SilentDrop and Unavailable on
/// writes; Crash-on-read, ReadIo, ReadCorrupt and Unavailable on reads. I1
/// always; I2 and I3 except under EccRefuse.
#[test]
#[ignore = "release fault sweep (CI job reservation-faults)"]
fn f6_single_fault_full() {
    let mut tears = vec![Tear::Unchanged, Tear::Erased, Tear::Complete];
    tears.extend((0..=RECORD_BYTES).map(Tear::PrefixOverOld));
    tears.extend((0..=RECORD_BYTES).map(Tear::PrefixOverErased));
    assert_eq!(tears.len(), 133);
    let mut on_write: Vec<(Fault, WeakPolicy)> = write_faults(&tears)
        .into_iter()
        .map(|fault| (fault, WeakPolicy::AlwaysErased))
        .collect();
    for k in 0..=RECORD_BYTES {
        for policy in PER_BOOT {
            let tear = Tear::WeakTail(k);
            on_write.push((Fault::Crash(tear), policy));
            on_write.push((Fault::WriteErr(tear), policy));
        }
    }
    assert_eq!(on_write.len(), 788);
    let (mut cases, mut expected, mut total) = (0u64, 0u64, Tally::default());
    let mut refuse_unavailable = 0u64;
    for window in [1, 3, 16] {
        for reserves in [1, 4] {
            let base = Timeline {
                sweep: "F6",
                erased: 0xFF,
                media: Media::Plain,
                policy: WeakPolicy::AlwaysErased,
                window,
                reserves,
                fault_boots: 3,
                later_boots: 1,
                checks: i123(),
            };
            let kinds = reference(&base);
            let (r, w) = count(&kinds);
            for erased in [0xFF, 0x00] {
                for media in [Media::Plain, Media::EccFlag, Media::EccRefuse] {
                    expected += r * READ_FAULTS.len() as u64 + w * on_write.len() as u64;
                    let checks = if media == Media::EccRefuse {
                        Checks::default()
                    } else {
                        i123()
                    };
                    for (op, &kind) in kinds.iter().enumerate() {
                        let variants: Vec<(Fault, WeakPolicy)> = match kind {
                            OpKind::Read => READ_FAULTS
                                .iter()
                                .map(|&f| (f, WeakPolicy::AlwaysErased))
                                .collect(),
                            OpKind::Write => on_write.clone(),
                        };
                        for (fault, policy) in variants {
                            let weak = matches!(
                                fault,
                                Fault::Crash(Tear::WeakTail(_))
                                    | Fault::WriteErr(Tear::WeakTail(_))
                            );
                            let t = Timeline {
                                erased,
                                media,
                                policy,
                                checks: Checks {
                                    write_order: !weak,
                                    ..checks
                                },
                                ..base
                            };
                            let result = run(&t, &[(op as u64, fault)], false);
                            if media == Media::EccRefuse {
                                // CHAR (W10/R25): availability is lost exactly
                                // when a marginal tail is left (WeakTail(k < 64));
                                // every other SAFE fault recovers as on Plain.
                                let marginal = matches!(
                                    fault,
                                    Fault::Crash(Tear::WeakTail(k))
                                        | Fault::WriteErr(Tear::WeakTail(k)) if k < RECORD_BYTES
                                );
                                assert_eq!(
                                    !result.available, marginal,
                                    "EccRefuse availability for {fault:?}@op{op} in {t:?}"
                                );
                                refuse_unavailable += u64::from(!result.available);
                            }
                            total.absorb(&result.tally);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "F6 single_fault_full: {cases} cases; EccRefuse availability lost in \
         {refuse_unavailable} cases; {total}"
    );
    assert_eq!(cases, expected);
    assert_eq!(cases, 344_736);
    // WeakTail(k < 64) × 4 policies × {Crash, WriteErr} at each of the 72
    // writes of the six timelines, for both erased values.
    assert_eq!(refuse_unavailable, 73_728);
}

fn d_faults(kind: OpKind) -> &'static [Fault] {
    match kind {
        OpKind::Read => &D_ON_READ,
        OpKind::Write => &D_ON_WRITE,
    }
}

/// F7: ordered pairs (i < j) of op indices ≡ 0 (mod 3) × D × D over window
/// {1, 3} × reserves {1, 4}; inapplicable pairs skipped; I1 only.
#[test]
#[ignore = "release fault sweep (CI job reservation-faults)"]
fn f7_double_fault_sampled() {
    let (mut cases, mut total) = (0u64, Tally::default());
    for window in [1, 3] {
        for reserves in [1, 4] {
            let t = Timeline {
                sweep: "F7",
                erased: 0xFF,
                media: Media::Plain,
                policy: WeakPolicy::AlwaysErased,
                window,
                reserves,
                fault_boots: 3,
                later_boots: 1,
                checks: Checks {
                    write_order: true,
                    ..Checks::default()
                },
            };
            cases += pair_sweep(&t, d_faults, 3, &mut total);
        }
    }
    println!("F7 double_fault_sampled: {cases} cases; {total}");
    assert_eq!(cases, F7_CASES);
}
const F7_CASES: u64 = 4_273;

/// F8: 100,000-boot seeded campaign with F5's event set.
#[test]
#[ignore = "release fault sweep (CI job reservation-faults)"]
fn f8_seeded_campaign_long() {
    let (boots, total) = campaign("F8 seeded_campaign_long", 100_000);
    assert_eq!(boots, 100_000);
    assert!(total.get(Ev::Activated) > 0);
}
