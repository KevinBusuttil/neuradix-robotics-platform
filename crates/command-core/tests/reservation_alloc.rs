//! The generation-reservation paths perform no heap allocation (WP-A04.4).
//!
//! `neuradix-command-core` is `no_std` without `alloc`, so heap use cannot
//! compile into it. This test additionally counts allocations on the current
//! thread across provisioning (success and every refusal), `open` (every
//! outcome), `reserve` (commit, window hit, each poison cause, poisoned and
//! exhausted), `reserve_from_window`, `status`, `remedy` and `Debug`/`Display`
//! formatting into a fixed buffer.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::fmt::{self, Write as _};
use std::num::NonZeroU32;

use neuradix_command_core::reservation::record::{self, RecordFields};
use neuradix_command_core::reservation::{
    BindingKey, GenerationReserver, NamespaceEpoch, OpenError, PoisonCause, ProvisionError,
    ProvisionGuards, RECORD_BYTES, ReceiverId, Remedy, ReservationKey, ReservationStore,
    ReserveError, ReserverConfig, RollbackDefense, Slot, SlotCondition, StoreError, provision,
};
use neuradix_command_core::{ExecutionMode, Generation};

#[path = "support/fault_store.rs"]
mod support;
use support::{Fault, FaultStore, Tear};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                ALLOCATIONS.with(|n| n.set(n.get() + 1));
            }
        });
        // SAFETY: forwards the caller's layout contract to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr/layout come from the matching System allocation above.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

/// A store handle over a shared medium, so the test can change the medium
/// between reserver calls (external writer) and inject faults at any time.
struct Shared<'a>(&'a RefCell<FaultStore>);
impl ReservationStore for Shared<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().write(slot, record)
    }
}

/// Fixed-capacity formatting sink; overflow is counted and discarded.
struct Sink {
    buf: [u8; 256],
    len: usize,
    total: usize,
}
impl fmt::Write for Sink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        self.total += s.len();
        Ok(())
    }
}

fn receiver(first: u8) -> ReceiverId {
    let mut id = [0u8; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = first + i as u8;
    }
    ReceiverId::new(id).expect("programmed receiver id")
}

fn epoch(value: u64) -> NamespaceEpoch {
    NamespaceEpoch::new(value).expect("nonzero epoch")
}

fn unprotected(window: u32) -> ReserverConfig {
    config(window, RollbackDefense::Unprotected)
}

fn config(window: u32, rollback: RollbackDefense) -> ReserverConfig {
    ReserverConfig::new(NonZeroU32::new(window).expect("nonzero window"), rollback)
}

fn record_for(slot: Slot, key: ReservationKey, epoch_value: u64, high_water: u64) -> [u8; 64] {
    record::encode(&RecordFields {
        slot,
        key,
        epoch: epoch(epoch_value),
        high_water,
        commits: 1,
    })
}

/// Write both slots directly (crafted or external records).
fn install(cell: &RefCell<FaultStore>, a: [u8; RECORD_BYTES], b: [u8; RECORD_BYTES]) {
    let mut store = cell.borrow_mut();
    store.set_raw(Slot::A, a);
    store.set_raw(Slot::B, b);
}

/// Both slots hold this key's record at `(epoch, high_water)`.
fn mine(cell: &RefCell<FaultStore>, key: ReservationKey, epoch_value: u64, high_water: u64) {
    install(
        cell,
        record_for(Slot::A, key, epoch_value, high_water),
        record_for(Slot::B, key, epoch_value, high_water),
    );
}

fn inject(cell: &RefCell<FaultStore>, after: u64, fault: Fault) {
    let mut store = cell.borrow_mut();
    let at = store.ops() + after;
    store.inject(at, fault);
}

fn calls(cell: &RefCell<FaultStore>) -> (u64, u64) {
    cell.borrow().calls()
}

fn open_error(
    cell: &RefCell<FaultStore>,
    key: ReservationKey,
    config: ReserverConfig,
) -> OpenError {
    let before = calls(cell);
    let error = match GenerationReserver::open(Shared(cell), key, config) {
        Ok(_) => panic!("open unexpectedly succeeded"),
        Err(failure) => failure.into_parts().0,
    };
    let after = calls(cell);
    assert_eq!((after.0 - before.0, after.1 - before.1), (2, 0));
    error
}

fn provision_error(
    cell: &RefCell<FaultStore>,
    key: ReservationKey,
    epoch_value: u64,
    guards: ProvisionGuards,
) -> ProvisionError {
    let before = calls(cell);
    let error = match provision(&mut Shared(cell), key, epoch(epoch_value), guards) {
        Ok(_) => panic!("provision unexpectedly succeeded"),
        Err(error) => error,
    };
    if !matches!(error, ProvisionError::Uncertain(_)) {
        let after = calls(cell);
        assert_eq!((after.0 - before.0, after.1 - before.1), (2, 0));
    }
    error
}

/// With both slots current (so the commit writes A first), open (window 2,
/// Unprotected), then reserve once with `fault` injected `offset` operations
/// after the open, expecting `cause`.
fn poison(
    cell: &RefCell<FaultStore>,
    key: ReservationKey,
    offset: u64,
    fault: Fault,
    cause: PoisonCause,
    sink: &mut Sink,
) {
    mine(cell, key, 2, 10);
    inject(cell, 2 + offset, fault);
    let mut reserver = GenerationReserver::open(Shared(cell), key, unprotected(2)).expect("open");
    let error = reserver.reserve().expect_err("poisoned commit");
    assert_eq!(error, ReserveError::Uncertain(cause));
    assert_eq!(error.remedy(), Remedy::ReopenStore);
    let before = calls(cell);
    assert_eq!(
        reserver.reserve().expect_err("sticky"),
        ReserveError::Poisoned(cause)
    );
    assert_eq!(
        reserver.reserve_from_window().expect_err("sticky"),
        ReserveError::Poisoned(cause)
    );
    assert_eq!(reserver.status().poisoned, Some(cause));
    assert_eq!(calls(cell), before);
    write!(sink, "{reserver:?} {error} {error:?}").expect("sink");
}

fn scenario() -> u32 {
    let mut paths = 0u32;
    let mut sink = Sink {
        buf: [0; 256],
        len: 0,
        total: 0,
    };
    let binding = BindingKey::numeric(1, 2, 7, ExecutionMode::Live);
    let key = ReservationKey::new(receiver(1), binding);
    let foreign_receiver = ReservationKey::new(receiver(0x40), binding);
    let foreign_binding = ReservationKey::new(
        receiver(1),
        BindingKey::numeric(1, 2, 8, ExecutionMode::Live),
    );
    let cell = RefCell::new(FaultStore::erased(0xFF));

    // ---- A blank store fails closed; every provisioning refusal. ----
    let blank = open_error(&cell, key, unprotected(4));
    assert_eq!(blank, OpenError::Blank);
    assert_eq!(blank.remedy(), Remedy::Provision);
    paths += 1;
    inject(&cell, 0, Fault::ReadIo);
    let io = provision_error(&cell, key, 1, ProvisionGuards::default());
    assert_eq!(io, ProvisionError::Store(StoreError::Io));
    inject(&cell, 1, Fault::Unavailable);
    let unavailable = provision_error(&cell, key, 1, ProvisionGuards::default());
    assert_eq!(unavailable, ProvisionError::Store(StoreError::Unavailable));
    let floor = ProvisionGuards {
        floor: Some(epoch(5)),
        prior_high_water: None,
    };
    let below = provision_error(&cell, key, 1, floor);
    assert_eq!(below, ProvisionError::BelowFloor { floor: epoch(5) });
    let prior = Generation::new((1u128 << 64) | 9).expect("nonzero");
    let history = ProvisionGuards {
        floor: None,
        prior_high_water: Some(prior),
    };
    let not_exceeded = provision_error(&cell, key, 1, history);
    assert_eq!(
        not_exceeded,
        ProvisionError::PriorHistoryNotExceeded { prior }
    );
    // A failed write burns the epoch (Unchanged tear: the slot stays blank).
    inject(&cell, 2, Fault::WriteErr(Tear::Unchanged));
    let uncertain = provision_error(&cell, key, 1, ProvisionGuards::default());
    assert_eq!(
        uncertain,
        ProvisionError::Uncertain(PoisonCause::WriteFailed(Slot::A))
    );
    write!(sink, "{uncertain} {uncertain:?}").expect("sink");
    paths += 5;

    // ---- Provision, then open, commit, window hits, CommitRequired. ----
    let report = provision(
        &mut Shared(&cell),
        key,
        epoch(2),
        ProvisionGuards::default(),
    )
    .expect("provision");
    assert_eq!(report.epoch, epoch(2));
    assert_eq!(report.replaced, None);
    let again = provision_error(&cell, key, 2, ProvisionGuards::default());
    assert_eq!(again, ProvisionError::EpochNotNewer { highest: epoch(2) });
    paths += 2;

    let mut reserver = GenerationReserver::open(Shared(&cell), key, unprotected(3)).expect("open");
    let status = reserver.status();
    assert_eq!(status.window_remaining, 0);
    assert_eq!(status.slots, [SlotCondition::Current; 2]);
    assert_eq!(
        reserver.reserve_from_window().expect_err("empty window"),
        ReserveError::CommitRequired
    );
    let before = calls(&cell);
    let first = reserver.reserve().expect("commit");
    assert_eq!(calls(&cell), (before.0 + 4, before.1 + 2));
    assert_eq!(first.generation().get(), (2u128 << 64) | 1);
    let hit = reserver.reserve().expect("window hit");
    let spare = reserver.reserve_from_window().expect("window hit");
    assert_eq!(calls(&cell), (before.0 + 4, before.1 + 2));
    assert!(first.generation() < hit.generation() && hit.generation() < spare.generation());
    let end = reserver.reserve_from_window().expect_err("window end");
    assert_eq!(end, ReserveError::CommitRequired);
    assert_eq!(end.remedy(), Remedy::CommitWhenSafe);
    let next = reserver.reserve().expect("second commit");
    assert_eq!(next.generation().get(), (2u128 << 64) | 4);
    assert_eq!(next.key(), key);
    assert_eq!(next.epoch(), epoch(2));
    let status = reserver.status();
    assert_eq!(status.window_remaining, 2);
    write!(sink, "{reserver:?} {status:?} {next:?}").expect("sink");
    let _burned = (first, hit, spare, next);
    let _store = reserver.into_store();
    paths += 4;

    // ---- Each poison cause (window 2, both slots current: A is written first). ----
    poison(
        &cell,
        key,
        2,
        Fault::WriteErr(Tear::Complete),
        PoisonCause::WriteFailed(Slot::A),
        &mut sink,
    );
    poison(
        &cell,
        key,
        2,
        Fault::SilentDrop,
        PoisonCause::VerifyFailed(Slot::A),
        &mut sink,
    );
    poison(
        &cell,
        key,
        0,
        Fault::Unavailable,
        PoisonCause::ReadFailed(Slot::A),
        &mut sink,
    );
    // MediaChanged: an external writer raises the record under a live reserver.
    let mut reserver = GenerationReserver::open(Shared(&cell), key, unprotected(1)).expect("open");
    let token = reserver.reserve().expect("commit");
    let ceiling = token.generation().get() as u64;
    mine(&cell, key, 2, ceiling + 50);
    let changed = reserver.reserve().expect_err("media changed");
    assert_eq!(changed, ReserveError::Uncertain(PoisonCause::MediaChanged));
    assert_eq!(
        reserver.reserve().expect_err("sticky"),
        ReserveError::Poisoned(PoisonCause::MediaChanged)
    );
    write!(sink, "{reserver:?} {changed} {token:?}").expect("sink");
    drop(token);
    paths += 4;

    // ---- Every other open outcome, with the provisioning format refusals. ----
    inject(&cell, 0, Fault::Unavailable);
    let unavailable = open_error(&cell, key, unprotected(1));
    assert_eq!(unavailable, OpenError::StoreUnavailable);
    assert_eq!(unavailable.remedy(), Remedy::ReopenStore);
    inject(&cell, 0, Fault::ReadIo);
    inject(&cell, 1, Fault::ReadIo);
    let unreadable = open_error(&cell, key, unprotected(1));
    assert_eq!(unreadable, OpenError::Unreadable);
    assert_eq!(unreadable.remedy(), Remedy::RetryOpen);
    {
        let mut store = cell.borrow_mut();
        store.rot(Slot::A, 40, 0x01);
        store.rot(Slot::B, 3, 0x80);
    }
    assert_eq!(open_error(&cell, key, unprotected(1)), OpenError::Corrupt);
    paths += 3;

    let mut unsupported = record_for(Slot::A, key, 3, 7);
    unsupported[4] = 2;
    record::seal(&mut unsupported);
    install(&cell, unsupported, record_for(Slot::B, key, 3, 7));
    assert_eq!(
        open_error(&cell, key, unprotected(1)),
        OpenError::UnsupportedFormat
    );
    assert_eq!(
        provision_error(&cell, key, 9, ProvisionGuards::default()),
        ProvisionError::UnsupportedFormat
    );
    install(
        &cell,
        record_for(Slot::B, key, 3, 7),
        record_for(Slot::B, key, 3, 7),
    );
    let aliasing = open_error(&cell, key, unprotected(1));
    assert_eq!(aliasing, OpenError::SlotAliasing);
    assert_eq!(aliasing.remedy(), Remedy::RepairStore);
    assert_eq!(
        provision_error(&cell, key, 9, ProvisionGuards::default()),
        ProvisionError::SlotAliasing
    );
    mine(&cell, foreign_receiver, 3, 7);
    assert_eq!(
        open_error(&cell, key, unprotected(1)),
        OpenError::ForeignReceiver
    );
    mine(&cell, foreign_binding, 3, 7);
    assert_eq!(
        open_error(&cell, key, unprotected(1)),
        OpenError::ForeignBinding
    );
    paths += 6;

    mine(&cell, key, 3, 7);
    let below = open_error(&cell, key, config(1, RollbackDefense::Floor(epoch(4))));
    assert_eq!(
        below,
        OpenError::BelowFloor {
            stored: epoch(3),
            floor: epoch(4)
        }
    );
    let witness = Generation::new((3u128 << 64) | 100).expect("nonzero");
    let rolled_back = open_error(&cell, key, config(1, RollbackDefense::Witness(witness)));
    assert_eq!(
        rolled_back,
        OpenError::RolledBack {
            witness,
            stored: (3u128 << 64) | 7
        }
    );
    mine(&cell, key, 3, u64::MAX);
    let exhausted = open_error(&cell, key, unprotected(1));
    assert_eq!(exhausted, OpenError::Exhausted);
    let failure = match GenerationReserver::open(Shared(&cell), key, unprotected(1)) {
        Ok(_) => panic!("open unexpectedly succeeded"),
        Err(failure) => failure,
    };
    write!(sink, "{failure} {failure:?} {below} {rolled_back:?}").expect("sink");
    paths += 4;

    // ---- Reserve at the end of the counter space: one value, then Exhausted. ----
    mine(&cell, key, 3, u64::MAX - 1);
    let mut reserver = GenerationReserver::open(Shared(&cell), key, unprotected(5))
        .map_err(|f| f.error)
        .expect("open");
    let last = reserver.reserve().expect("last value");
    assert_eq!(
        last.generation().get(),
        (3u128 << 64) | u128::from(u64::MAX)
    );
    let before = calls(&cell);
    let done = reserver.reserve().expect_err("exhausted");
    assert_eq!(done, ReserveError::Exhausted);
    assert_eq!(done.remedy(), Remedy::Provision);
    assert_eq!(
        reserver.reserve_from_window().expect_err("exhausted"),
        ReserveError::Exhausted
    );
    assert_eq!(reserver.status().window_remaining, 0);
    assert_eq!(calls(&cell), before);
    write!(sink, "{reserver:?} {done}").expect("sink");
    let _burned = last;
    paths += 1;

    assert!(sink.total > sink.len, "the formatting sink saw output");
    paths
}

#[test]
fn reservation_paths_allocate_nothing() {
    // The counter is live on this thread, so the zero below is not vacuous.
    COUNTING.with(|on| on.set(true));
    drop(std::hint::black_box(Box::new(0u64)));
    COUNTING.with(|on| on.set(false));
    assert_eq!(ALLOCATIONS.with(|n| n.replace(0)), 1);

    COUNTING.with(|on| on.set(true));
    let observed = std::hint::black_box(scenario());
    COUNTING.with(|on| on.set(false));
    assert_eq!(observed, 30);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
