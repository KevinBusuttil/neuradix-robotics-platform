//! Deterministic generation-reservation tests R1–R29 (WP-A04.4).
//!
//! Codec goldens, the open/reserve/provision state machine, exact store-call
//! bounds, and characterization of the documented residuals. Characterization
//! tests carry a `_char` suffix and assert the unsafe (reuse or undetected)
//! outcome explicitly, so any change in that behaviour is visible in review.
//!
//! Every store is the shared fault-injecting `FaultStore`. A reserver either
//! borrows it (`&mut FaultStore`) or goes through [`Shared`], a deliberate second
//! handle (a C5 violation used only by tests) that lets a test read the call
//! counters, or model an external writer, while the reserver is alive.

#[path = "support/fault_store.rs"]
mod support;

use core::cell::{Cell, RefCell};
use core::num::NonZeroU32;

use neuradix_command_core::reservation::record::{self, RecordFields, SlotView};
use neuradix_command_core::reservation::{
    BindingKey, GenerationReserver, MAX_STORE_CALLS_PER_OPEN, MAX_STORE_CALLS_PER_PROVISION,
    MAX_STORE_CALLS_PER_RESERVE, NamespaceEpoch, OpenError, PoisonCause, ProvisionError,
    ProvisionGuards, ProvisionReport, RECORD_BYTES, RECORD_FORMAT, RECORD_MAGIC, ReceiverId,
    Remedy, ReservationKey, ReservationStore, ReserveError, ReservedGeneration, ReserverConfig,
    ReserverStatus, RollbackDefense, RollbackPosture, Slot, SlotCondition, StoreError, provision,
};
use neuradix_command_core::{ExecutionMode, Generation};
use support::{Fault, FaultStore, Media, OpKind, Tear, WeakPolicy, provisioned};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Golden receiver identity: bytes 01 02 … 10.
const RECEIVER: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
/// Deterministic non-record content (fails the magic and the CRC).
const JUNK: [u8; RECORD_BYTES] = [0xA5; RECORD_BYTES];
const READ_A: (OpKind, Slot) = (OpKind::Read, Slot::A);
const READ_B: (OpKind, Slot) = (OpKind::Read, Slot::B);
const WRITE_A: (OpKind, Slot) = (OpKind::Write, Slot::A);
const WRITE_B: (OpKind, Slot) = (OpKind::Write, Slot::B);

/// The golden key: receiver 01..10, binding `numeric(1, 2, 7, Live)`.
fn key() -> ReservationKey {
    ReservationKey::new(
        ReceiverId::new(RECEIVER).unwrap(),
        BindingKey::numeric(1, 2, 7, ExecutionMode::Live),
    )
}

fn key_for_receiver(bytes: [u8; 16]) -> ReservationKey {
    ReservationKey::new(ReceiverId::new(bytes).unwrap(), key().binding())
}

fn foreign_receiver_key() -> ReservationKey {
    key_for_receiver([0x11; 16])
}

fn foreign_binding_key() -> ReservationKey {
    ReservationKey::new(
        key().receiver(),
        BindingKey::numeric(1, 2, 8, ExecutionMode::Live),
    )
}

fn epoch(value: u64) -> NamespaceEpoch {
    NamespaceEpoch::new(value).unwrap()
}

/// `(epoch << 64) | counter`.
const fn value(epoch: u64, counter: u64) -> u128 {
    ((epoch as u128) << 64) | counter as u128
}

fn generation(value: u128) -> Generation {
    Generation::new(value).unwrap()
}

fn config(window: u32, rollback: RollbackDefense) -> ReserverConfig {
    ReserverConfig::new(NonZeroU32::new(window).unwrap(), rollback)
}

fn unprotected(window: u32) -> ReserverConfig {
    config(window, RollbackDefense::Unprotected)
}

fn guards(floor: Option<u64>, prior_high_water: Option<u128>) -> ProvisionGuards {
    ProvisionGuards {
        floor: floor.map(epoch),
        prior_high_water: prior_high_water.map(generation),
    }
}

fn fields(
    slot: Slot,
    key: ReservationKey,
    epoch_value: u64,
    high_water: u64,
    commits: u32,
) -> RecordFields {
    RecordFields {
        slot,
        key,
        epoch: epoch(epoch_value),
        high_water,
        commits,
    }
}

fn record_bytes(
    slot: Slot,
    key: ReservationKey,
    epoch_value: u64,
    high_water: u64,
    commits: u32,
) -> [u8; RECORD_BYTES] {
    record::encode(&fields(slot, key, epoch_value, high_water, commits))
}

/// Write a well-formed record pair directly (crafted state or external writer).
fn write_both(
    store: &mut FaultStore,
    key: ReservationKey,
    epoch_value: u64,
    high_water: u64,
    commits: u32,
) {
    for slot in [Slot::A, Slot::B] {
        store.set_raw(
            slot,
            record_bytes(slot, key, epoch_value, high_water, commits),
        );
    }
}

/// A 0xFF-erased store provisioned for [`key`] at `epoch_value` (6 store calls).
fn provisioned_store(epoch_value: u64) -> FaultStore {
    let mut store = FaultStore::erased(0xFF);
    provisioned(&mut store, key(), epoch_value);
    store
}

/// Schedule `faults` at offsets relative to the store's next call.
fn faulted(mut store: FaultStore, faults: &[(u64, Fault)]) -> FaultStore {
    let base = store.ops();
    for &(offset, fault) in faults {
        store.inject(base + offset, fault);
    }
    store
}

fn diff(before: (u64, u64), after: (u64, u64)) -> (u64, u64) {
    (after.0 - before.0, after.1 - before.1)
}

/// `(kind, slot)` of every call logged at or after global op `from`.
fn ops_since(store: &FaultStore, from: u64) -> Vec<(OpKind, Slot)> {
    store
        .log()
        .filter(|op| op.index >= from)
        .map(|op| (op.kind, op.slot))
        .collect()
}

fn writes_since(store: &FaultStore, from: u64) -> Vec<Slot> {
    store
        .log()
        .filter(|op| op.index >= from && op.kind == OpKind::Write)
        .map(|op| op.slot)
        .collect()
}

fn fault_fired(store: &FaultStore) -> bool {
    store.log().any(|op| op.faulted)
}

/// Every tear the fault model supports, at every byte length.
fn every_tear() -> impl Iterator<Item = Tear> {
    [Tear::Unchanged, Tear::Erased, Tear::Complete]
        .into_iter()
        .chain((0..=RECORD_BYTES).map(Tear::PrefixOverOld))
        .chain((0..=RECORD_BYTES).map(Tear::PrefixOverErased))
        .chain((0..=RECORD_BYTES).map(Tear::WeakTail))
}

/// Open over a borrowed store, mapping the failure to its error.
fn open(
    store: &mut FaultStore,
    config: ReserverConfig,
) -> Result<GenerationReserver<&mut FaultStore>, OpenError> {
    GenerationReserver::open(store, key(), config).map_err(|f| f.error)
}

/// An open that must refuse. Asserts the open bound: exactly 2 reads (A then B)
/// and 0 writes.
fn refused(store: &mut FaultStore, key: ReservationKey, config: ReserverConfig) -> OpenError {
    let before = store.calls();
    let from = store.ops();
    let error = match GenerationReserver::open(&mut *store, key, config) {
        Ok(reserver) => panic!("open unexpectedly succeeded: {reserver:?}"),
        Err(failure) => failure.error,
    };
    assert_eq!(
        diff(before, store.calls()),
        (2, 0),
        "{error:?}: open must be 2 reads and 0 writes"
    );
    assert_eq!(ops_since(store, from), [READ_A, READ_B], "{error:?}");
    error
}

/// An open that must succeed; returns its status. Asserts the open bound.
fn accepted(store: &mut FaultStore, key: ReservationKey, config: ReserverConfig) -> ReserverStatus {
    let before = store.calls();
    let from = store.ops();
    let status = match GenerationReserver::open(&mut *store, key, config) {
        Ok(reserver) => reserver.status(),
        Err(failure) => panic!("open refused: {failure:?}"),
    };
    assert_eq!(diff(before, store.calls()), (2, 0));
    assert_eq!(ops_since(store, from), [READ_A, READ_B]);
    status
}

/// A second handle onto one `FaultStore`. It deliberately violates C5 so a test
/// can read the call counters, and model an external writer, while a reserver
/// is alive.
struct Shared<'a>(&'a RefCell<FaultStore>);

impl ReservationStore for Shared<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().write(slot, record)
    }
}

fn new_rig(epoch_value: u64) -> RefCell<FaultStore> {
    RefCell::new(provisioned_store(epoch_value))
}

fn open_rig(
    rig: &RefCell<FaultStore>,
    config: ReserverConfig,
) -> Result<GenerationReserver<Shared<'_>>, OpenError> {
    GenerationReserver::open(Shared(rig), key(), config).map_err(|f| f.error)
}

/// Run `call` and return its result with the `(reads, writes)` it cost.
fn metered<T>(rig: &RefCell<FaultStore>, call: impl FnOnce() -> T) -> (T, (u64, u64)) {
    let before = rig.borrow().calls();
    let out = call();
    (out, diff(before, rig.borrow().calls()))
}

/// Schedule `fault` `offset` calls after the rig's next call.
fn inject_next(rig: &RefCell<FaultStore>, offset: u64, fault: Fault) {
    let at = rig.borrow().ops() + offset;
    rig.borrow_mut().inject(at, fault);
}

fn issued(result: Result<ReservedGeneration, ReserveError>) -> Result<u128, ReserveError> {
    result.map(|token| token.generation().get())
}

/// Every value a history activated; asserts I1 (strictly increasing) and the
/// reserved layout of each token.
#[derive(Debug, Default)]
struct Ledger {
    issued: Vec<u128>,
}

impl Ledger {
    fn take(&mut self, token: ReservedGeneration) -> u128 {
        let v = token.generation().get();
        assert_eq!(NamespaceEpoch::of(token.generation()), Some(token.epoch()));
        assert_ne!(v as u64, 0, "counter 0 is never issued");
        if let Some(&last) = self.issued.last() {
            assert!(v > last, "I1 violated: {v:#x} returned after {last:#x}");
        }
        self.issued.push(v);
        v
    }

    fn max(&self) -> u128 {
        self.issued.last().copied().unwrap_or(0)
    }
}

/// Outcome of the commit that follows a first issued value (see [`commit_run`]).
struct CommitRun {
    result: Result<u128, ReserveError>,
    cost: (u64, u64),
    ops: Vec<(OpKind, Slot)>,
    then: Result<u128, ReserveError>,
    then_cost: (u64, u64),
    status: ReserverStatus,
    store: FaultStore,
}

/// Provision epoch 1, open with window 1 and issue `value(1, 1)` (both slots are
/// then current). While the reserver is alive, apply `at_rest` to the medium and
/// inject `faults` relative to the next call; then reserve twice.
fn commit_run(at_rest: impl FnOnce(&mut FaultStore), faults: &[(u64, Fault)]) -> CommitRun {
    let rig = new_rig(1);
    let run = {
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 1)));
        at_rest(&mut rig.borrow_mut());
        let base = rig.borrow().ops();
        for &(offset, fault) in faults {
            rig.borrow_mut().inject(base + offset, fault);
        }
        let (result, cost) = metered(&rig, || issued(reserver.reserve()));
        let ops = ops_since(&rig.borrow(), base);
        let (then, then_cost) = metered(&rig, || issued(reserver.reserve()));
        (result, cost, ops, then, then_cost, reserver.status())
    };
    let (result, cost, ops, then, then_cost, status) = run;
    CommitRun {
        result,
        cost,
        ops,
        then,
        then_cost,
        status,
        store: rig.into_inner(),
    }
}

/// (case, store, epoch, slot written first, replaced epoch, commits written).
type ProvisionSuccess = (&'static str, FaultStore, u64, Slot, Option<u64>, u32);
/// (case, store, epoch, guards, expected error or `None` for Ok, (reads, writes)).
type ProvisionCase = (
    &'static str,
    FaultStore,
    u64,
    ProvisionGuards,
    Option<ProvisionError>,
    (u64, u64),
);

struct Golden {
    hex: &'static str,
    slot: Slot,
    epoch: u64,
    high_water: u64,
    commits: u32,
    crc: u32,
}

/// record_format.md golden vectors (receiver 01..10, binding numeric(1,2,7,Live)).
const GOLDEN: [Golden; 3] = [
    Golden {
        hex: "4e525847010000000102030405060708090a0b0c0d0e0f102497afd1b857f14b01000000000000000100000000000000010000000000000000000000db2299eb",
        slot: Slot::A,
        epoch: 1,
        high_water: 1,
        commits: 1,
        crc: 0xeb99_22db,
    },
    Golden {
        hex: "4e525847010100000102030405060708090a0b0c0d0e0f102497afd1b857f14b01000000000000000100000000000000010000000000000000000000cb919ac9",
        slot: Slot::B,
        epoch: 1,
        high_water: 1,
        commits: 1,
        crc: 0xc99a_91cb,
    },
    Golden {
        hex: "4e525847010000000102030405060708090a0b0c0d0e0f102497afd1b857f14bffffffffffffffffffffffffffffffff070000000000000000000000712ca9a0",
        slot: Slot::A,
        epoch: u64::MAX,
        high_water: u64::MAX,
        commits: 7,
        crc: 0xa0a9_2c71,
    },
];

fn hex(text: &str) -> [u8; RECORD_BYTES] {
    assert_eq!(text.len(), 2 * RECORD_BYTES);
    let mut out = [0u8; RECORD_BYTES];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn golden_a() -> [u8; RECORD_BYTES] {
    hex(GOLDEN[0].hex)
}

fn golden_b() -> [u8; RECORD_BYTES] {
    hex(GOLDEN[1].hex)
}

fn sealed(mut bytes: [u8; RECORD_BYTES]) -> [u8; RECORD_BYTES] {
    record::seal(&mut bytes);
    bytes
}

// ---------------------------------------------------------------------------
// R1–R5: codec and open classification
// ---------------------------------------------------------------------------

#[test]
fn r1_golden_records_v1() {
    assert_eq!(RECORD_BYTES, 64);
    assert_eq!(RECORD_MAGIC, *b"NRXG");
    assert_eq!(RECORD_FORMAT, 1);
    assert_eq!(key().binding().get(), 0x4bf1_57b8_d1af_9724);

    for golden in &GOLDEN {
        let bytes = hex(golden.hex);
        let expected = fields(
            golden.slot,
            key(),
            golden.epoch,
            golden.high_water,
            golden.commits,
        );
        assert_eq!(record::encode(&expected), bytes, "{:?}", golden.slot);
        assert_eq!(record::crc32(&bytes[..60]), golden.crc);
        assert_eq!(bytes[60..], golden.crc.to_le_bytes());
        // seal reproduces the CRC whatever the stale trailer held.
        for stale in [[0u8; 4], [0xFF; 4], [0xDE, 0xAD, 0xBE, 0xEF]] {
            let mut resealed = bytes;
            resealed[60..].copy_from_slice(&stale);
            record::seal(&mut resealed);
            assert_eq!(resealed, bytes);
        }
        // decode round-trips.
        assert_eq!(record::decode(&bytes), SlotView::Valid(expected));
        let SlotView::Valid(decoded) = record::decode(&bytes) else {
            unreachable!()
        };
        assert_eq!(record::encode(&decoded), bytes);
    }

    // The A and B copies differ only in the slot tag and the CRC.
    let (a, b) = (golden_a(), golden_b());
    let differing: Vec<usize> = (0..RECORD_BYTES).filter(|&i| a[i] != b[i]).collect();
    assert_eq!(differing, [5, 60, 61, 62, 63]);

    // Round trip across field extremes.
    for (slot, e, high_water, commits) in [
        (Slot::A, 1, 0, 1),
        (Slot::B, 42, 7, u32::MAX),
        (Slot::B, u64::MAX, u64::MAX - 1, 2),
    ] {
        let f = fields(slot, foreign_binding_key(), e, high_water, commits);
        assert_eq!(record::decode(&record::encode(&f)), SlotView::Valid(f));
    }
}

#[test]
fn r2_every_single_bit_flip_rejected() {
    let mut flips = 0;
    for golden in &GOLDEN {
        let good = hex(golden.hex);
        for bit in 0..RECORD_BYTES * 8 {
            let mut bad = good;
            bad[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(
                record::decode(&bad),
                SlotView::Corrupt,
                "bit {bit} of {:?}",
                golden.slot
            );
            flips += 1;
        }
    }
    assert_eq!(flips, 3 * 512);
    assert_eq!(record::decode(&[0xFF; RECORD_BYTES]), SlotView::Blank);
    assert_eq!(record::decode(&[0x00; RECORD_BYTES]), SlotView::Blank);

    // Through open: a flipped copy is tolerated beside the intact one; two
    // flipped copies fail closed as Corrupt.
    let (a, b) = (golden_a(), golden_b());
    for bit in 0..RECORD_BYTES * 8 {
        let flip = |mut bytes: [u8; RECORD_BYTES]| {
            bytes[bit / 8] ^= 1 << (bit % 8);
            bytes
        };
        for (raw_a, raw_b, slots) in [
            (flip(a), b, [SlotCondition::Corrupt, SlotCondition::Current]),
            (a, flip(b), [SlotCondition::Current, SlotCondition::Corrupt]),
        ] {
            let mut store = FaultStore::erased(0xFF);
            store.set_raw(Slot::A, raw_a);
            store.set_raw(Slot::B, raw_b);
            let status = accepted(&mut store, key(), unprotected(1));
            assert_eq!(status.slots, slots, "bit {bit}");
            assert_eq!(status.durable_ceiling, value(1, 1));
        }
        let mut store = FaultStore::erased(0xFF);
        store.set_raw(Slot::A, flip(a));
        store.set_raw(Slot::B, flip(b));
        assert_eq!(
            refused(&mut store, key(), unprotected(1)),
            OpenError::Corrupt
        );
    }
}

/// CRC-valid-after-sealing edits that a v1 reader must treat as an unknown format.
fn unsupported_variants(base: [u8; RECORD_BYTES]) -> Vec<(String, [u8; RECORD_BYTES])> {
    let mut out = Vec::new();
    for version in [0u8, 2, 0xFF] {
        let mut bytes = base;
        bytes[4] = version;
        out.push((format!("version {version}"), bytes));
    }
    for (lo, hi) in [(1u8, 0u8), (0, 0x80), (0xFF, 0xFF)] {
        let mut bytes = base;
        bytes[6] = lo;
        bytes[7] = hi;
        out.push((
            format!("flags {:#06x}", u16::from_le_bytes([lo, hi])),
            bytes,
        ));
    }
    for index in 52..60 {
        for set in [0x01u8, 0x80] {
            let mut bytes = base;
            bytes[index] = set;
            out.push((format!("reserved byte {index} = {set:#04x}"), bytes));
        }
    }
    out
}

#[test]
fn r3_unknown_format_fails_closed() {
    let mut cases = 0;
    for (slot, base, valid_other) in [
        (Slot::A, golden_a(), golden_b()),
        (Slot::B, golden_b(), golden_a()),
    ] {
        for (what, edited) in unsupported_variants(base) {
            // The CRC is checked first: an unsealed edit (a torn v1 record) is
            // Corrupt, never Unsupported.
            assert_eq!(record::decode(&edited), SlotView::Corrupt, "{what}");
            let unsupported = sealed(edited);
            assert_eq!(
                record::decode(&unsupported),
                SlotView::Unsupported,
                "{what}"
            );

            // One Unsupported slot beside a valid Mine slot fails the open closed.
            let mut store = FaultStore::erased(0xFF);
            store.set_raw(slot, unsupported);
            store.set_raw(slot.other(), valid_other);
            for rollback in [
                RollbackDefense::Unprotected,
                RollbackDefense::Floor(epoch(1)),
                RollbackDefense::Witness(generation(1)),
            ] {
                let error = refused(&mut store, key(), config(1, rollback));
                assert_eq!(error, OpenError::UnsupportedFormat, "{what} in {slot:?}");
                assert_eq!(error.remedy(), Remedy::RepairStore);
            }

            // Provision refuses too, with 2 reads and 0 writes.
            let image = store.snapshot();
            let before = store.calls();
            assert_eq!(
                provision(&mut store, key(), epoch(9), ProvisionGuards::default()),
                Err(ProvisionError::UnsupportedFormat),
                "{what} in {slot:?}"
            );
            assert_eq!(diff(before, store.calls()), (2, 0));
            assert_eq!(store.snapshot(), image);
            cases += 1;
        }
    }
    assert_eq!(cases, 2 * (3 + 3 + 16));

    // Precedence: Unsupported is reported before a foreign or aliased slot.
    let mut store = FaultStore::erased(0xFF);
    let mut v2 = golden_a();
    v2[4] = 2;
    store.set_raw(Slot::A, sealed(v2));
    store.set_raw(
        Slot::B,
        record_bytes(Slot::A, foreign_receiver_key(), 1, 1, 1),
    );
    assert_eq!(
        refused(&mut store, key(), unprotected(1)),
        OpenError::UnsupportedFormat
    );
}

/// CRC-valid records with field values v1 forbids.
fn malformed_variants(base: [u8; RECORD_BYTES]) -> Vec<(&'static str, [u8; RECORD_BYTES])> {
    let edit = |f: fn(&mut [u8; RECORD_BYTES])| {
        let mut bytes = base;
        f(&mut bytes);
        sealed(bytes)
    };
    vec![
        ("epoch 0", edit(|b| b[32..40].fill(0))),
        ("commits 0", edit(|b| b[48..52].fill(0))),
        ("slot tag 2", edit(|b| b[5] = 2)),
        ("slot tag 0xFF", edit(|b| b[5] = 0xFF)),
        ("receiver all 0x00", edit(|b| b[8..24].fill(0x00))),
        ("receiver all 0xFF", edit(|b| b[8..24].fill(0xFF))),
    ]
}

#[test]
fn r4_malformed_but_crc_valid_is_corrupt() {
    for (slot, base, valid_other) in [
        (Slot::A, golden_a(), golden_b()),
        (Slot::B, golden_b(), golden_a()),
    ] {
        for (what, bytes) in malformed_variants(base) {
            assert_eq!(record::decode(&bytes), SlotView::Corrupt, "{what}");

            // Tolerated beside a valid slot.
            let mut store = FaultStore::erased(0xFF);
            store.set_raw(slot, bytes);
            store.set_raw(slot.other(), valid_other);
            let status = accepted(&mut store, key(), unprotected(1));
            let mut expected = [SlotCondition::Current; 2];
            expected[slot.index()] = SlotCondition::Corrupt;
            assert_eq!(status.slots, expected, "{what} in {slot:?}");
            assert_eq!(status.durable_ceiling, value(1, 1));
            {
                let mut reserver = open(&mut store, unprotected(1)).unwrap();
                assert_eq!(issued(reserver.reserve()), Ok(value(1, 2)), "{what}");
            }
            // The malformed slot was rewritten by that commit.
            assert!(matches!(
                record::decode(&store.raw(slot)),
                SlotView::Valid(_)
            ));

            // Two malformed slots fail closed.
            let mut store = FaultStore::erased(0xFF);
            store.set_raw(slot, bytes);
            store.set_raw(slot.other(), bytes);
            assert_eq!(
                refused(&mut store, key(), unprotected(1)),
                OpenError::Corrupt,
                "{what}"
            );
        }
    }
}

#[test]
fn r5_blank_unreadable_unavailable() {
    let junk_a = || {
        let mut store = FaultStore::erased(0xFF);
        store.set_raw(Slot::A, JUNK);
        store
    };
    let junk_b = || {
        let mut store = FaultStore::erased(0x00);
        store.set_raw(Slot::B, JUNK);
        store
    };
    // "Any Unavailable" wins over every other slot view (open precedence 1).
    let beside = |slot: Slot, bytes: [u8; RECORD_BYTES]| {
        let mut store = FaultStore::erased(0xFF);
        store.set_raw(slot, bytes);
        faulted(store, &[(slot.other().index() as u64, Fault::Unavailable)])
    };
    let unsupported_a = {
        let mut bytes = golden_a();
        bytes[4] = 2;
        sealed(bytes)
    };
    let refusals: Vec<(&str, FaultStore, OpenError)> = vec![
        (
            "Unsupported + Unavailable",
            beside(Slot::A, unsupported_a),
            OpenError::StoreUnavailable,
        ),
        (
            "Unavailable + aliased",
            beside(Slot::B, golden_a()),
            OpenError::StoreUnavailable,
        ),
        (
            "foreign receiver + Unavailable",
            beside(
                Slot::A,
                record_bytes(Slot::A, foreign_receiver_key(), 1, 1, 1),
            ),
            OpenError::StoreUnavailable,
        ),
        ("erased 0xFF", FaultStore::erased(0xFF), OpenError::Blank),
        ("erased 0x00", FaultStore::erased(0x00), OpenError::Blank),
        (
            "both Io",
            faulted(
                provisioned_store(1),
                &[(0, Fault::ReadIo), (1, Fault::ReadIo)],
            ),
            OpenError::Unreadable,
        ),
        (
            "Io + Blank",
            faulted(FaultStore::erased(0xFF), &[(0, Fault::ReadIo)]),
            OpenError::Unreadable,
        ),
        (
            "Blank + Io",
            faulted(FaultStore::erased(0x00), &[(1, Fault::ReadIo)]),
            OpenError::Unreadable,
        ),
        (
            "Corrupt + Io",
            faulted(junk_a(), &[(1, Fault::ReadIo)]),
            OpenError::Unreadable,
        ),
        (
            "Unavailable + valid",
            faulted(provisioned_store(1), &[(0, Fault::Unavailable)]),
            OpenError::StoreUnavailable,
        ),
        (
            "valid + Unavailable",
            faulted(provisioned_store(1), &[(1, Fault::Unavailable)]),
            OpenError::StoreUnavailable,
        ),
        (
            "Io + Unavailable",
            faulted(
                provisioned_store(1),
                &[(0, Fault::ReadIo), (1, Fault::Unavailable)],
            ),
            OpenError::StoreUnavailable,
        ),
        (
            "Unavailable + Blank",
            faulted(FaultStore::erased(0xFF), &[(0, Fault::Unavailable)]),
            OpenError::StoreUnavailable,
        ),
        ("Corrupt + Blank", junk_a(), OpenError::Corrupt),
        ("Blank + Corrupt", junk_b(), OpenError::Corrupt),
        (
            "ReadCorrupt + Blank",
            faulted(FaultStore::erased(0xFF), &[(0, Fault::ReadCorrupt)]),
            OpenError::Corrupt,
        ),
        (
            "ReadCorrupt both",
            faulted(
                provisioned_store(1),
                &[(0, Fault::ReadCorrupt), (1, Fault::ReadCorrupt)],
            ),
            OpenError::Corrupt,
        ),
    ];
    for (what, mut store, expected) in refusals {
        let error = refused(&mut store, key(), unprotected(1));
        assert_eq!(error, expected, "{what}");
        let remedy = match expected {
            OpenError::Blank | OpenError::Corrupt => Remedy::Provision,
            OpenError::Unreadable => Remedy::RetryOpen,
            OpenError::StoreUnavailable => Remedy::ReopenStore,
            other => unreachable!("{other:?}"),
        };
        assert_eq!(error.remedy(), remedy, "{what}");
    }

    // RetryOpen: a transient Io clears on the same handle; the retry is read-only.
    let mut store = faulted(
        provisioned_store(1),
        &[(0, Fault::ReadIo), (1, Fault::ReadIo)],
    );
    assert_eq!(
        refused(&mut store, key(), unprotected(1)),
        OpenError::Unreadable
    );
    assert_eq!(
        accepted(&mut store, key(), unprotected(1)).durable_ceiling,
        value(1, 0)
    );

    // One failed read beside a valid slot is tolerated and recorded.
    for (what, faults, slots) in [
        (
            "Io + valid",
            [(0, Fault::ReadIo)],
            [SlotCondition::Unreadable, SlotCondition::Current],
        ),
        (
            "valid + Io",
            [(1, Fault::ReadIo)],
            [SlotCondition::Current, SlotCondition::Unreadable],
        ),
        (
            "ReadCorrupt + valid",
            [(0, Fault::ReadCorrupt)],
            [SlotCondition::Corrupt, SlotCondition::Current],
        ),
    ] {
        let mut store = faulted(provisioned_store(1), &faults);
        assert_eq!(
            accepted(&mut store, key(), unprotected(1)).slots,
            slots,
            "{what}"
        );
    }
    let mut store = provisioned_store(1);
    store.erase(Slot::B);
    assert_eq!(
        accepted(&mut store, key(), unprotected(1)).slots,
        [SlotCondition::Current, SlotCondition::Blank]
    );
}

// ---------------------------------------------------------------------------
// R6–R13: state machine, window, witness, floor, identity, exhaustion, provisioning
// ---------------------------------------------------------------------------

#[test]
fn r6_provision_then_monotonic() {
    for window in [1u32, 4] {
        let rig = new_rig(1);
        let mut ledger = Ledger::default();
        let mut ceiling = 0u64;
        let mut commits = 1u32; // provisioning wrote commit count 1
        for boot in 0..10 {
            rig.borrow_mut().reboot();
            let (opened, cost) = metered(&rig, || open_rig(&rig, unprotected(window)));
            let mut reserver = opened.unwrap();
            assert_eq!(cost, (2, 0), "open of boot {boot}");
            let status = reserver.status();
            assert_eq!(status.durable_ceiling, value(1, ceiling));
            assert_eq!(status.window_remaining, 0, "every startup commits first");
            assert_eq!(status.commits, commits);
            let mut next = ceiling + 1;
            for i in 0..10 {
                let commits_now = next > ceiling;
                let (token, cost) = metered(&rig, || reserver.reserve());
                assert_eq!(ledger.take(token.unwrap()), value(1, next));
                if commits_now {
                    ceiling = next + u64::from(window) - 1;
                    commits += 1;
                    assert_eq!(cost, (4, 2), "window {window}, boot {boot}, reserve {i}");
                } else {
                    assert_eq!(cost, (0, 0), "window {window}, boot {boot}, reserve {i}");
                }
                if i == 0 {
                    assert!(commits_now);
                    assert_eq!(cost.1, 2, "the first reserve of boot {boot} makes 2 writes");
                }
                next += 1;
                let status = reserver.status();
                assert_eq!(status.durable_ceiling, value(1, ceiling));
                assert_eq!(status.commits, commits);
            }
        }
        assert_eq!(ledger.issued.len(), 100);
        assert_eq!(ledger.issued[0], value(1, 1));
        if window == 1 {
            let contiguous: Vec<u128> = (1..=100).map(|c| value(1, c)).collect();
            assert_eq!(ledger.issued, contiguous);
        } else {
            // 3 commits of 4 per boot: each boot starts 12 above the previous one.
            assert_eq!(ledger.issued[10], value(1, 13));
            assert_eq!(ledger.issued[99], value(1, 9 * 12 + 10));
        }
    }
}

#[test]
fn r7_window_and_reserve_from_window() {
    let rig = new_rig(1);
    {
        // Window 4, 8 reserves: exactly 2 commits (12 calls), 0 calls inside.
        let mut reserver = open_rig(&rig, unprotected(4)).unwrap();
        let mut costs = Vec::new();
        for counter in 1..=8 {
            let (token, cost) = metered(&rig, || issued(reserver.reserve()));
            assert_eq!(token, Ok(value(1, counter)));
            costs.push(cost);
        }
        let inside = (0, 0);
        let commit = (4, 2);
        assert_eq!(
            costs,
            [
                commit, inside, inside, inside, commit, inside, inside, inside
            ]
        );
        let total: u64 = costs.iter().map(|(r, w)| r + w).sum();
        assert_eq!(total, 12);
        let status = reserver.status();
        assert_eq!(status.commits, 3);
        assert_eq!(status.window_remaining, 0);
        assert_eq!(status.durable_ceiling, value(1, 8));
    }

    rig.borrow_mut().reboot();
    {
        let mut reserver = open_rig(&rig, unprotected(4)).unwrap();
        // Right after open: CommitRequired, 0 calls, no state change, no poison.
        let opened = reserver.status();
        for _ in 0..2 {
            let (result, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
            assert_eq!(result, Err(ReserveError::CommitRequired));
            assert_eq!(
                ReserveError::CommitRequired.remedy(),
                Remedy::CommitWhenSafe
            );
            assert_eq!(cost, (0, 0));
            assert_eq!(reserver.status(), opened);
            assert_eq!(reserver.status().poisoned, None);
        }
        let (token, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!((token, cost), (Ok(value(1, 9)), (4, 2)));
        assert_eq!(reserver.status().window_remaining, 3);
        // Values inside the committed window, 0 calls each.
        for counter in 10..=12 {
            let (token, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
            assert_eq!((token, cost), (Ok(value(1, counter)), (0, 0)));
        }
        // Window end.
        let (result, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
        assert_eq!((result, cost), (Err(ReserveError::CommitRequired), (0, 0)));
        assert_eq!(reserver.status().window_remaining, 0);
        assert_eq!(reserver.status().poisoned, None);
        // reserve still commits after CommitRequired.
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 13)));
        assert_eq!(issued(reserver.reserve_from_window()), Ok(value(1, 14)));
        assert_eq!(reserver.status().window_remaining, 2);
        assert_eq!(reserver.status().durable_ceiling, value(1, 16));
    }

    // After reboot the unused window values (15, 16) are skipped.
    rig.borrow_mut().reboot();
    let mut reserver = open_rig(&rig, unprotected(4)).unwrap();
    assert_eq!(reserver.status().durable_ceiling, value(1, 16));
    assert_eq!(
        issued(reserver.reserve_from_window()),
        Err(ReserveError::CommitRequired)
    );
    assert_eq!(issued(reserver.reserve()), Ok(value(1, 17)));
}

#[test]
fn r8_witness_at_open() {
    // History: window 1 issues to counter 100; a snapshot is taken at ceiling 50.
    let rig = new_rig(1);
    let mut ledger = Ledger::default();
    let mut at_50 = None;
    {
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        for counter in 1..=100 {
            assert_eq!(ledger.take(reserver.reserve().unwrap()), value(1, counter));
            if counter == 50 {
                at_50 = Some(rig.borrow().snapshot());
            }
        }
    }
    let at_50 = at_50.unwrap();
    let mut store = rig.into_inner();
    store.reboot();

    // witness ≤ stored → Ok.
    for witness in [value(1, 100), value(1, 99), value(1, 1), value(1, 0)] {
        let status = accepted(
            &mut store,
            key(),
            config(64, RollbackDefense::Witness(generation(witness))),
        );
        assert_eq!(status.durable_ceiling, value(1, 100));
        assert_eq!(status.rollback, RollbackPosture::Witness);
    }
    // A witness above the stored ceiling refuses (an untrusted high witness is a DoS).
    let high = generation(value(1, 101));
    assert_eq!(
        refused(&mut store, key(), config(1, RollbackDefense::Witness(high))),
        OpenError::RolledBack {
            witness: high,
            stored: value(1, 100)
        }
    );

    // The reviewer's case: restore the ceiling-50 snapshot, open with the
    // witness of the last value handed out and window 64.
    store.restore(at_50);
    store.reboot();
    let witness = generation(value(1, 100));
    for rollback in [
        RollbackDefense::Witness(witness),
        RollbackDefense::FloorAndWitness {
            floor: epoch(1),
            witness,
        },
    ] {
        let error = refused(&mut store, key(), config(64, rollback));
        assert_eq!(
            error,
            OpenError::RolledBack {
                witness,
                stored: value(1, 50)
            }
        );
        assert_eq!(error.remedy(), Remedy::Provision);
        assert_eq!(store.snapshot(), at_50, "open wrote nothing");
    }
    // Nothing was issued: the last activated value is still 100.
    assert_eq!(ledger.max(), value(1, 100));

    // A legacy witness (< 2^64) never triggers, even against a fresh epoch.
    let mut fresh = provisioned_store(1);
    for legacy in [1u128, 9, u128::from(u64::MAX)] {
        let status = accepted(
            &mut fresh,
            key(),
            config(1, RollbackDefense::Witness(generation(legacy))),
        );
        assert_eq!(status.durable_ceiling, value(1, 0));
    }

    // No API supplies a witness after open: it enters only through the
    // ReserverConfig given to `open`, and the status exposes only the posture
    // (exhaustive destructuring: no field can carry a witness value).
    let reserver = open(
        &mut fresh,
        config(1, RollbackDefense::Witness(generation(value(1, 0)))),
    )
    .unwrap();
    let ReserverStatus {
        key: status_key,
        epoch: status_epoch,
        durable_ceiling,
        window_remaining,
        slots,
        commits,
        poisoned,
        rollback,
    } = reserver.status();
    assert_eq!(status_key, key());
    assert_eq!(status_epoch, epoch(1));
    assert_eq!(durable_ceiling, value(1, 0));
    assert_eq!(window_remaining, 0);
    assert_eq!(slots, [SlotCondition::Current; 2]);
    assert_eq!(commits, 1);
    assert_eq!(poisoned, None);
    assert_eq!(rollback, RollbackPosture::Witness);
    assert!(rollback.detects_restore_within_epoch());
    assert_eq!(
        RollbackDefense::Witness(witness).posture(),
        RollbackPosture::Witness
    );
    assert_eq!(RollbackDefense::Witness(witness).witness(), Some(witness));
}

#[test]
fn r9_floor() {
    let (e1, e2, e3) = (epoch(1), epoch(2), epoch(3));
    let mut store = provisioned_store(1);
    for rollback in [
        RollbackDefense::Floor(e2),
        RollbackDefense::FloorAndWitness {
            floor: e2,
            witness: generation(1),
        },
        // The floor is checked before the witness.
        RollbackDefense::FloorAndWitness {
            floor: e2,
            witness: generation(value(9, 9)),
        },
    ] {
        let error = refused(&mut store, key(), config(1, rollback));
        assert_eq!(
            error,
            OpenError::BelowFloor {
                stored: e1,
                floor: e2
            },
            "{rollback:?}"
        );
        assert_eq!(error.remedy(), Remedy::Provision);
    }
    // The floor is inclusive.
    let status = accepted(&mut store, key(), config(1, RollbackDefense::Floor(e1)));
    assert_eq!(status.rollback, RollbackPosture::Floor);
    assert!(!status.rollback.detects_restore_within_epoch());

    // Provision E = 1 with floor 2 → BelowFloor, 0 writes.
    let mut blank = FaultStore::erased(0xFF);
    let before = blank.calls();
    assert_eq!(
        provision(&mut blank, key(), e1, guards(Some(2), None)),
        Err(ProvisionError::BelowFloor { floor: e2 })
    );
    assert_eq!(diff(before, blank.calls()), (2, 0));
    assert_eq!(refused(&mut blank, key(), unprotected(1)), OpenError::Blank);

    let image = store.snapshot();
    let before = store.calls();
    assert_eq!(
        provision(&mut store, key(), e2, guards(Some(3), None)),
        Err(ProvisionError::BelowFloor { floor: e3 })
    );
    assert_eq!(diff(before, store.calls()), (2, 0));
    assert_eq!(store.snapshot(), image);

    // At the floor: provisioning and open succeed.
    let report = provision(&mut store, key(), e2, guards(Some(2), None)).unwrap();
    assert_eq!(report.epoch, e2);
    assert_eq!(report.replaced, Some(e1));
    let status = accepted(&mut store, key(), config(1, RollbackDefense::Floor(e2)));
    assert_eq!(status.epoch, e2);
}

#[test]
fn r10_foreign() {
    let rx = foreign_receiver_key();
    let bind = foreign_binding_key();
    let mine = key();
    let cases = [
        (
            "receiver alone",
            Some(rx),
            Some(rx),
            OpenError::ForeignReceiver,
        ),
        (
            "receiver + blank",
            Some(rx),
            None,
            OpenError::ForeignReceiver,
        ),
        (
            "binding alone",
            Some(bind),
            Some(bind),
            OpenError::ForeignBinding,
        ),
        (
            "blank + binding",
            None,
            Some(bind),
            OpenError::ForeignBinding,
        ),
        (
            "mine + receiver",
            Some(mine),
            Some(rx),
            OpenError::ForeignReceiver,
        ),
        (
            "receiver + mine",
            Some(rx),
            Some(mine),
            OpenError::ForeignReceiver,
        ),
        (
            "mine + binding",
            Some(mine),
            Some(bind),
            OpenError::ForeignBinding,
        ),
        (
            "binding + mine",
            Some(bind),
            Some(mine),
            OpenError::ForeignBinding,
        ),
        (
            "binding + receiver",
            Some(bind),
            Some(rx),
            OpenError::ForeignReceiver,
        ),
    ];
    for (what, a, b, expected) in cases {
        let mut store = FaultStore::erased(0xFF);
        for (slot, owner) in [(Slot::A, a), (Slot::B, b)] {
            if let Some(owner) = owner {
                store.set_raw(slot, record_bytes(slot, owner, 3, 5, 2));
            }
        }
        for rollback in [
            RollbackDefense::Unprotected,
            RollbackDefense::Floor(epoch(1)),
        ] {
            let error = refused(&mut store, mine, config(1, rollback));
            assert_eq!(error, expected, "{what}");
            assert_eq!(error.remedy(), Remedy::RepairStore);
        }
    }

    // Provisioning a greater epoch over a foreign image succeeds; open then succeeds.
    for (foreign, seen_by_foreign) in [
        (rx, OpenError::ForeignReceiver),
        (bind, OpenError::ForeignBinding),
    ] {
        let mut store = FaultStore::erased(0xFF);
        provisioned(&mut store, foreign, 3);
        // Epoch ordering spans every key.
        for e in [1, 3] {
            let before = store.calls();
            assert_eq!(
                provision(&mut store, mine, epoch(e), ProvisionGuards::default()),
                Err(ProvisionError::EpochNotNewer { highest: epoch(3) })
            );
            assert_eq!(diff(before, store.calls()), (2, 0));
        }
        let report = provision(&mut store, mine, epoch(4), ProvisionGuards::default()).unwrap();
        assert_eq!(
            report,
            ProvisionReport {
                epoch: epoch(4),
                replaced: None,
                commits: 1
            }
        );
        {
            let mut reserver = open(&mut store, unprotected(1)).unwrap();
            assert_eq!(issued(reserver.reserve()), Ok(value(4, 1)));
        }
        assert_eq!(
            refused(&mut store, foreign, unprotected(1)),
            seen_by_foreign
        );
    }
}

/// Maps both slots onto physical slot A (a wiring or driver bug; violates C2).
struct Aliasing<'a> {
    inner: &'a mut FaultStore,
}

impl ReservationStore for Aliasing<'_> {
    fn read(&mut self, _slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.inner.read(Slot::A, buf)
    }
    fn write(&mut self, _slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.inner.write(Slot::A, record)
    }
}

/// Aliases reads and/or writes of slot B onto A while the flags are set.
struct LateAliasing<'a> {
    rig: &'a RefCell<FaultStore>,
    reads: &'a Cell<bool>,
    writes: &'a Cell<bool>,
}

impl ReservationStore for LateAliasing<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let slot = if self.reads.get() { Slot::A } else { slot };
        self.rig.borrow_mut().read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let slot = if self.writes.get() { Slot::A } else { slot };
        self.rig.borrow_mut().write(slot, record)
    }
}

#[test]
fn r11_slot_aliasing() {
    // Provisioned correctly, then read through an aliasing store.
    let mut store = provisioned_store(1);
    let image = store.snapshot();
    let before = store.calls();
    let error =
        match GenerationReserver::open(Aliasing { inner: &mut store }, key(), unprotected(1)) {
            Ok(reserver) => panic!("aliasing store accepted: {reserver:?}"),
            Err(failure) => failure.error,
        };
    assert_eq!(error, OpenError::SlotAliasing);
    assert_eq!(error.remedy(), Remedy::RepairStore);
    assert_eq!(diff(before, store.calls()), (2, 0));

    // Provision refuses through the alias once a record exists: 0 writes.
    let before = store.calls();
    assert_eq!(
        provision(
            &mut Aliasing { inner: &mut store },
            key(),
            epoch(2),
            ProvisionGuards::default()
        ),
        Err(ProvisionError::SlotAliasing)
    );
    assert_eq!(diff(before, store.calls()), (2, 0));
    assert_eq!(store.snapshot(), image);

    // From blank the alias is invisible to provision (both reads decode Blank);
    // its two copies land in one location and the next open fails closed.
    let mut blank = FaultStore::erased(0xFF);
    assert!(
        provision(
            &mut Aliasing { inner: &mut blank },
            key(),
            epoch(1),
            ProvisionGuards::default()
        )
        .is_ok()
    );
    assert_eq!(blank.raw(Slot::B), [0xFF; RECORD_BYTES]);
    let error = GenerationReserver::open(Aliasing { inner: &mut blank }, key(), unprotected(1))
        .map(|reserver| reserver.status())
        .map_err(|f| f.error);
    assert_eq!(error, Err(OpenError::SlotAliasing));
    assert_eq!(
        refused(&mut blank, key(), unprotected(1)),
        OpenError::SlotAliasing
    );

    // Open precedence: an aliased slot is reported before a foreign one.
    for (a, b) in [
        (
            record_bytes(Slot::A, foreign_receiver_key(), 1, 1, 1),
            golden_a(),
        ),
        (
            golden_b(),
            record_bytes(Slot::B, foreign_binding_key(), 1, 1, 1),
        ),
    ] {
        let mut store = FaultStore::erased(0xFF);
        store.set_raw(Slot::A, a);
        store.set_raw(Slot::B, b);
        assert_eq!(
            refused(&mut store, key(), unprotected(1)),
            OpenError::SlotAliasing
        );
    }

    // Aliasing that appears under a live reserver is caught at the next commit:
    // read aliasing → MediaChanged (0 writes); write aliasing → VerifyFailed(B).
    for (alias_reads, alias_writes, cause, cost) in [
        (true, true, PoisonCause::MediaChanged, (2, 0)),
        (true, false, PoisonCause::MediaChanged, (2, 0)),
        (false, true, PoisonCause::VerifyFailed(Slot::B), (4, 2)),
    ] {
        let rig = new_rig(1);
        let (reads, writes) = (Cell::new(false), Cell::new(false));
        let store = LateAliasing {
            rig: &rig,
            reads: &reads,
            writes: &writes,
        };
        let mut reserver = GenerationReserver::open(store, key(), unprotected(1))
            .map_err(|f| f.error)
            .unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 1)));
        reads.set(alias_reads);
        writes.set(alias_writes);
        let (result, spent) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!(result, Err(ReserveError::Uncertain(cause)));
        assert_eq!(spent, cost);
        assert_eq!(
            issued(reserver.reserve()),
            Err(ReserveError::Poisoned(cause))
        );
    }
}

#[test]
fn r12_exhaustion() {
    // Counter u64::MAX − 1: returns u64::MAX, then Exhausted with 0 calls.
    let rig = RefCell::new(FaultStore::erased(0xFF));
    write_both(&mut rig.borrow_mut(), key(), 1, u64::MAX - 1, 5);
    {
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        let (token, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!((token, cost), (Ok(value(1, u64::MAX)), (4, 2)));
        assert_eq!(reserver.status().durable_ceiling, value(1, u64::MAX));
        for _ in 0..3 {
            let (result, cost) = metered(&rig, || issued(reserver.reserve()));
            assert_eq!((result, cost), (Err(ReserveError::Exhausted), (0, 0)));
            let (result, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
            assert_eq!((result, cost), (Err(ReserveError::Exhausted), (0, 0)));
        }
        assert_eq!(ReserveError::Exhausted.remedy(), Remedy::Provision);
        assert_eq!(
            reserver.status().poisoned,
            None,
            "exhaustion is not a poison"
        );
        assert_eq!(reserver.status().window_remaining, 0);
    }
    let mut store = rig.into_inner();
    store.reboot();
    let error = refused(&mut store, key(), unprotected(1));
    assert_eq!(error, OpenError::Exhausted);
    assert_eq!(error.remedy(), Remedy::Provision);
    // Provision E + 1 recovers at ((E + 1) << 64) | 1.
    let report = provision(&mut store, key(), epoch(2), ProvisionGuards::default()).unwrap();
    assert_eq!(report.replaced, Some(epoch(1)));
    assert_eq!(report.commits, 7);
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(2, 1)));
    }

    // Window 5 near the end of the epoch clamps at u64::MAX and never crosses it.
    let rig = RefCell::new(FaultStore::erased(0xFF));
    write_both(&mut rig.borrow_mut(), key(), 1, u64::MAX - 3, 1);
    {
        let mut reserver = open_rig(&rig, unprotected(5)).unwrap();
        let mut costs = Vec::new();
        for counter in [u64::MAX - 2, u64::MAX - 1, u64::MAX] {
            let (token, cost) = metered(&rig, || issued(reserver.reserve()));
            assert_eq!(token, Ok(value(1, counter)));
            costs.push(cost);
        }
        assert_eq!(costs, [(4, 2), (0, 0), (0, 0)]);
        assert_eq!(reserver.status().durable_ceiling, value(1, u64::MAX));
        let (result, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!((result, cost), (Err(ReserveError::Exhausted), (0, 0)));
    }
    for slot in [Slot::A, Slot::B] {
        assert_eq!(
            record::decode(&rig.borrow().raw(slot)),
            SlotView::Valid(fields(slot, key(), 1, u64::MAX, 2))
        );
    }

    // Epoch u64::MAX, counter u64::MAX − 1: u128::MAX exactly once, then Exhausted.
    let rig = RefCell::new(FaultStore::erased(0x00));
    write_both(&mut rig.borrow_mut(), key(), u64::MAX, u64::MAX - 1, 1);
    {
        let mut reserver = open_rig(&rig, unprotected(3)).unwrap();
        let mut ledger = Ledger::default();
        assert_eq!(ledger.take(reserver.reserve().unwrap()), u128::MAX);
        for _ in 0..3 {
            let (result, cost) = metered(&rig, || issued(reserver.reserve()));
            assert_eq!((result, cost), (Err(ReserveError::Exhausted), (0, 0)));
        }
    }
    let mut store = rig.into_inner();
    store.reboot();
    assert_eq!(
        refused(&mut store, key(), unprotected(1)),
        OpenError::Exhausted
    );
    // The key is terminal: no epoch is newer than u64::MAX.
    for e in [u64::MAX, 1] {
        let before = store.calls();
        assert_eq!(
            provision(&mut store, key(), epoch(e), ProvisionGuards::default()),
            Err(ProvisionError::EpochNotNewer {
                highest: epoch(u64::MAX)
            })
        );
        assert_eq!(diff(before, store.calls()), (2, 0));
    }
}

#[test]
fn r13_provisioning_rules() {
    // Refusals: each 2 reads, 0 writes, image unchanged.
    let refusals: Vec<(&str, FaultStore, u64, ProvisionGuards, ProvisionError)> = vec![
        (
            "same epoch",
            provisioned_store(3),
            3,
            ProvisionGuards::default(),
            ProvisionError::EpochNotNewer { highest: epoch(3) },
        ),
        (
            "older epoch",
            provisioned_store(3),
            2,
            ProvisionGuards::default(),
            ProvisionError::EpochNotNewer { highest: epoch(3) },
        ),
        (
            "Io on A",
            faulted(provisioned_store(1), &[(0, Fault::ReadIo)]),
            2,
            ProvisionGuards::default(),
            ProvisionError::Store(StoreError::Io),
        ),
        (
            "Io on B over blank",
            faulted(FaultStore::erased(0xFF), &[(1, Fault::ReadIo)]),
            2,
            ProvisionGuards::default(),
            ProvisionError::Store(StoreError::Io),
        ),
        (
            "Unavailable on A",
            faulted(provisioned_store(1), &[(0, Fault::Unavailable)]),
            2,
            ProvisionGuards::default(),
            ProvisionError::Store(StoreError::Unavailable),
        ),
        (
            "Unavailable on B",
            faulted(provisioned_store(1), &[(1, Fault::Unavailable)]),
            2,
            ProvisionGuards::default(),
            ProvisionError::Store(StoreError::Unavailable),
        ),
        (
            "below floor over blank",
            FaultStore::erased(0xFF),
            2,
            guards(Some(3), None),
            ProvisionError::BelowFloor { floor: epoch(3) },
        ),
        (
            "below floor over E1",
            provisioned_store(1),
            2,
            guards(Some(3), None),
            ProvisionError::BelowFloor { floor: epoch(3) },
        ),
        (
            "prior history",
            provisioned_store(1),
            2,
            guards(None, Some(value(2, 1))),
            ProvisionError::PriorHistoryNotExceeded {
                prior: generation(value(2, 1)),
            },
        ),
    ];
    for (what, mut store, e, provision_guards, expected) in refusals {
        let image = store.snapshot();
        let before = store.calls();
        let from = store.ops();
        assert_eq!(
            provision(&mut store, key(), epoch(e), provision_guards),
            Err(expected),
            "{what}"
        );
        assert_eq!(diff(before, store.calls()), (2, 0), "{what}");
        assert_eq!(ops_since(&store, from), [READ_A, READ_B], "{what}");
        assert_eq!(store.snapshot(), image, "{what}");
    }

    // Success: the slot not holding this key's best record is written first,
    // each write is read back, and `replaced` reports the previous best epoch.
    let mut best_a = FaultStore::erased(0xFF);
    best_a.set_raw(Slot::A, record_bytes(Slot::A, key(), 1, 5, 4));
    best_a.set_raw(Slot::B, record_bytes(Slot::B, key(), 1, 3, 2));
    let mut best_b = FaultStore::erased(0xFF);
    best_b.set_raw(Slot::A, record_bytes(Slot::A, key(), 1, 3, 2));
    best_b.set_raw(Slot::B, record_bytes(Slot::B, key(), 1, 5, 4));
    let mut only_a = provisioned_store(1);
    only_a.erase(Slot::B);
    let mut junk_a = provisioned_store(1);
    junk_a.set_raw(Slot::A, JUNK);
    let mut mine_and_foreign = FaultStore::erased(0xFF);
    mine_and_foreign.set_raw(Slot::A, record_bytes(Slot::A, key(), 1, 9, 6));
    mine_and_foreign.set_raw(
        Slot::B,
        record_bytes(Slot::B, foreign_receiver_key(), 3, 0, 1),
    );
    let successes: Vec<ProvisionSuccess> = vec![
        ("blank", FaultStore::erased(0xFF), 1, Slot::A, None, 1),
        ("blank 0x00", FaultStore::erased(0x00), 1, Slot::A, None, 1),
        ("both equal", provisioned_store(1), 2, Slot::A, Some(1), 2),
        ("A best, B stale", best_a, 2, Slot::B, Some(1), 5),
        ("B best, A stale", best_b, 2, Slot::A, Some(1), 5),
        ("A only, B blank", only_a, 2, Slot::B, Some(1), 2),
        ("A junk, B only", junk_a, 2, Slot::A, Some(1), 2),
        (
            "A mine, B foreign",
            mine_and_foreign,
            4,
            Slot::B,
            Some(1),
            7,
        ),
        // A read returning Err(Corrupt) (ECC-flagged slot) is acceptable, unlike
        // Io or Unavailable: that slot does not hold the best record.
        (
            "A Err(Corrupt), B only",
            faulted(provisioned_store(1), &[(0, Fault::ReadCorrupt)]),
            2,
            Slot::A,
            Some(1),
            2,
        ),
        (
            "A only, B Err(Corrupt)",
            faulted(provisioned_store(1), &[(1, Fault::ReadCorrupt)]),
            2,
            Slot::B,
            Some(1),
            2,
        ),
        (
            "both Err(Corrupt) over blank",
            faulted(
                FaultStore::erased(0xFF),
                &[(0, Fault::ReadCorrupt), (1, Fault::ReadCorrupt)],
            ),
            1,
            Slot::A,
            None,
            1,
        ),
    ];
    for (what, mut store, e, first, replaced, commits) in successes {
        let before = store.calls();
        let from = store.ops();
        let report = provision(&mut store, key(), epoch(e), ProvisionGuards::default());
        assert_eq!(
            report,
            Ok(ProvisionReport {
                epoch: epoch(e),
                replaced: replaced.map(epoch),
                commits
            }),
            "{what}"
        );
        assert_eq!(diff(before, store.calls()), (4, 2), "{what}");
        let second = first.other();
        assert_eq!(
            ops_since(&store, from),
            [
                READ_A,
                READ_B,
                (OpKind::Write, first),
                (OpKind::Read, first),
                (OpKind::Write, second),
                (OpKind::Read, second),
            ],
            "{what}"
        );
        assert_eq!(writes_since(&store, from), [first, second], "{what}");
        for slot in [Slot::A, Slot::B] {
            assert_eq!(
                record::decode(&store.raw(slot)),
                SlotView::Valid(fields(slot, key(), e, 0, commits)),
                "{what}"
            );
        }
        // Nothing is issued by provisioning: the first value needs open + commit.
        let status = accepted(&mut store, key(), unprotected(1));
        assert_eq!(status.durable_ceiling, value(e, 0), "{what}");
        assert_eq!(status.window_remaining, 0, "{what}");
    }
}

// ---------------------------------------------------------------------------
// R14–R16: interrupted provisioning, erase-unit loss, exact bounds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Before {
    Blank,
    Counter,
    Foreign,
    OneErased(Slot),
}

fn r14_setup(before: Before, erased: u8) -> FaultStore {
    let mut store = FaultStore::erased(erased);
    match before {
        Before::Blank => {}
        Before::Counter => write_both(&mut store, key(), 1, 7, 8),
        Before::Foreign => write_both(&mut store, foreign_receiver_key(), 3, 7, 8),
        Before::OneErased(slot) => {
            write_both(&mut store, key(), 1, 7, 8);
            store.erase(slot);
        }
    }
    store
}

/// R14b (review finding): the only valid record is a lone FOREIGN record beside a
/// blank or damaged slot. Provisioning must write the other slot first, so an
/// interrupted provision leaves the old state (the foreign record, failing
/// closed) or the new one, never a store with no valid record.
#[test]
fn r14b_provision_preserves_lone_foreign_record() {
    let mut cases = 0u32;
    for foreign_slot in [Slot::A, Slot::B] {
        for other in [[0xFF; RECORD_BYTES], [0x00; RECORD_BYTES], JUNK] {
            let setup = || {
                let mut store = FaultStore::erased(0xFF);
                store.set_raw(
                    foreign_slot,
                    record_bytes(foreign_slot, foreign_receiver_key(), 3, 7, 8),
                );
                store.set_raw(foreign_slot.other(), other);
                store
            };
            // Clean run: the non-foreign slot is written first.
            let mut store = setup();
            provision(&mut store, key(), epoch(4), ProvisionGuards::default()).unwrap();
            let order: Vec<Slot> = store.write_order().collect();
            assert_eq!(order, vec![foreign_slot.other(), foreign_slot]);

            // Power loss or write error at either write, with every tear.
            for offset in [2u64, 4] {
                for tear in every_tear() {
                    for fault in [Fault::Crash(tear), Fault::WriteErr(tear)] {
                        let what = format!("foreign in {foreign_slot:?}, op {offset}, {fault:?}");
                        let foreign_bytes = setup().raw(foreign_slot);
                        let mut store = faulted(setup(), &[(offset, fault)]);
                        assert!(
                            provision(&mut store, key(), epoch(4), ProvisionGuards::default())
                                .is_err(),
                            "{what}"
                        );
                        store.clear_faults();
                        store.reboot();
                        match open(&mut store, unprotected(1)) {
                            Ok(reserver) => {
                                assert_eq!(reserver.status().epoch, epoch(4), "{what}");
                            }
                            Err(OpenError::ForeignReceiver) => {
                                assert_eq!(store.raw(foreign_slot), foreign_bytes, "{what}");
                            }
                            Err(other) => panic!("{what}: old record lost, open {other:?}"),
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2 * 3 * 2 * (3 + 3 * (RECORD_BYTES as u32 + 1)) * 2);
}

#[test]
fn r14_provision_crash_every_byte() {
    let old_ceiling = value(1, 7);
    let mut cases = 0u32;
    for erased in [0xFF, 0x00] {
        for before in [
            Before::Blank,
            Before::Counter,
            Before::Foreign,
            Before::OneErased(Slot::A),
            Before::OneErased(Slot::B),
        ] {
            let new_epoch = if matches!(before, Before::Foreign) {
                4
            } else {
                2
            };
            let new_ceiling = value(new_epoch, 0);
            // The slot not holding this key's best record is written first.
            let first = if matches!(before, Before::OneErased(Slot::B)) {
                Slot::B
            } else {
                Slot::A
            };
            // Provision ops: 0 read A, 1 read B, 2 write first, 3 read-back,
            // 4 write second, 5 read-back.
            for offset in 0..6u64 {
                let faults: Vec<Fault> = if offset == 2 || offset == 4 {
                    every_tear()
                        .flat_map(|tear| [Fault::Crash(tear), Fault::WriteErr(tear)])
                        .collect()
                } else {
                    vec![Fault::Crash(Tear::Unchanged), Fault::ReadIo]
                };
                for fault in faults {
                    let what = format!("{before:?} erased {erased:#04x} op {offset} {fault:?}");
                    let mut store = faulted(r14_setup(before, erased), &[(offset, fault)]);
                    let image = store.snapshot();
                    let result = provision(
                        &mut store,
                        key(),
                        epoch(new_epoch),
                        ProvisionGuards::default(),
                    );
                    let expected = match (offset, fault) {
                        (0 | 1, Fault::ReadIo) => ProvisionError::Store(StoreError::Io),
                        (0 | 1, _) => ProvisionError::Store(StoreError::Unavailable),
                        (2, _) => ProvisionError::Uncertain(PoisonCause::WriteFailed(first)),
                        (3, _) => ProvisionError::Uncertain(PoisonCause::VerifyFailed(first)),
                        (4, _) => {
                            ProvisionError::Uncertain(PoisonCause::WriteFailed(first.other()))
                        }
                        _ => ProvisionError::Uncertain(PoisonCause::VerifyFailed(first.other())),
                    };
                    assert_eq!(result, Err(expected), "{what}");
                    if offset < 2 {
                        assert_eq!(store.snapshot(), image, "{what}: refusal wrote");
                    }
                    // From op 3 on, the first slot holds the new record (verified
                    // write), so the next open can only find the new state (or,
                    // over a foreign image, still fail closed on the other slot).
                    let first_landed = offset >= 3;
                    store.clear_faults();
                    store.reboot();

                    let outcome = open(&mut store, unprotected(1)).map(|mut reserver| {
                        let status = reserver.status();
                        (status, issued(reserver.reserve()))
                    });
                    let is_new = |status: &ReserverStatus, next: &Result<u128, ReserveError>| {
                        status.epoch == epoch(new_epoch)
                            && status.durable_ceiling == new_ceiling
                            && *next == Ok(value(new_epoch, 1))
                    };
                    match (before, outcome) {
                        (Before::Blank, Err(OpenError::Blank | OpenError::Corrupt)) => {
                            assert!(!first_landed, "{what}: verified new record lost");
                        }
                        (Before::Blank, Ok((status, next))) => {
                            assert!(is_new(&status, &next), "{what}: {status:?} {next:?}");
                        }
                        (Before::Foreign, Err(OpenError::ForeignReceiver)) => {
                            // After op 5 both slots were written and the first verified.
                            assert!(offset < 5, "{what}: both slots rewritten");
                        }
                        (Before::Foreign, Ok((status, next))) => {
                            assert!(is_new(&status, &next), "{what}: {status:?} {next:?}");
                        }
                        (Before::Counter | Before::OneErased(_), Ok((status, next))) => {
                            // Never lower than the old state, and exactly old or new.
                            assert!(status.durable_ceiling >= old_ceiling, "{what}");
                            let is_old = status.epoch == epoch(1)
                                && status.durable_ceiling == old_ceiling
                                && next == Ok(value(1, 8));
                            assert!(
                                is_old || is_new(&status, &next),
                                "{what}: {status:?} {next:?}"
                            );
                            if offset < 2 {
                                assert!(is_old, "{what}: refused provision changed state");
                            }
                            if first_landed {
                                assert!(
                                    is_new(&status, &next),
                                    "{what}: verified new record not selected"
                                );
                            }
                        }
                        (_, other) => panic!("{what}: {other:?}"),
                    }
                    cases += 1;
                }
            }
        }
    }
    let tears = 3 + 3 * 65;
    assert_eq!(cases, 2 * 5 * (2 * 2 * tears + 4 * 2));
    println!("R14: {cases} interrupted provisions");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Damage {
    Erase,
    Junk,
}

/// Applies one single-slot damage event (erase-unit loss or junk) just before
/// global operation `at`, i.e. at rest between two store calls. The store sits
/// in a `RefCell` so the timeline can read the op counter while a reserver owns
/// this wrapper.
struct Damaging<'a> {
    rig: &'a RefCell<FaultStore>,
    at: u64,
    slot: Slot,
    damage: Damage,
    fired: &'a Cell<bool>,
}

impl Damaging<'_> {
    fn arm(&self) {
        let mut store = self.rig.borrow_mut();
        if !self.fired.get() && store.ops() == self.at {
            self.fired.set(true);
            match self.damage {
                Damage::Erase => store.erase(self.slot),
                Damage::Junk => store.set_raw(self.slot, JUNK),
            }
        }
    }
}

impl ReservationStore for Damaging<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.arm();
        self.rig.borrow_mut().read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.arm();
        self.rig.borrow_mut().write(slot, record)
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Boot(u32),
    Provision(u64),
}

const R15_WINDOW: u32 = 2;
const R15_SCRIPT: [Step; 4] = [
    Step::Boot(3),
    Step::Boot(2),
    Step::Provision(2),
    Step::Boot(2),
];

/// What one R15 timeline did.
struct R15Run {
    store: FaultStore,
    /// Op index at which the scripted part ended (the recovery boot's first op).
    script_end: u64,
    /// The planned power loss fired.
    crashed: bool,
    /// The slot damage was applied.
    damaged: bool,
}

/// One timeline after a clean provision of epoch 1: the script, stopped at the
/// first failed call, then a recovery boot.
fn r15_timeline(damage: Option<(u64, Slot, Damage)>, fault: Option<(u64, Fault)>) -> R15Run {
    let rig = RefCell::new(provisioned_store(1));
    if let Some((at, planned)) = fault {
        rig.borrow_mut().inject(at, planned);
    }
    let (at, slot, kind) = damage.unwrap_or((u64::MAX, Slot::A, Damage::Erase));
    let fired = Cell::new(false);
    let io = || Damaging {
        rig: &rig,
        at,
        slot,
        damage: kind,
        fired: &fired,
    };
    let ops = || rig.borrow().ops();
    // Messages name the case lazily (formatted only on failure).
    let mut ledger = Ledger::default();
    let mut max_committed = value(1, 0);
    // First op of the call (open, reserve or provision) that failed, if any.
    let mut failed_call: Option<u64> = None;
    'script: for step in R15_SCRIPT {
        match step {
            Step::Provision(e) => {
                let call = ops();
                if let Err(error) =
                    provision(&mut io(), key(), epoch(e), ProvisionGuards::default())
                {
                    // A crash on a read refuses (Store); on a write it burns the
                    // epoch (Uncertain).
                    assert!(
                        matches!(
                            error,
                            ProvisionError::Uncertain(_) | ProvisionError::Store(_)
                        ),
                        "damage {damage:?}, fault {fault:?}: {error:?}"
                    );
                    failed_call = Some(call);
                    break 'script;
                }
                max_committed = max_committed.max(value(e, 0));
            }
            Step::Boot(reserves) => {
                rig.borrow_mut().reboot();
                let call = ops();
                let mut reserver =
                    match GenerationReserver::open(io(), key(), unprotected(R15_WINDOW)) {
                        Ok(reserver) => reserver,
                        Err(failure) => {
                            assert_eq!(
                                failure.error,
                                OpenError::StoreUnavailable,
                                "damage {damage:?}, fault {fault:?}"
                            );
                            failed_call = Some(call);
                            break 'script;
                        }
                    };
                for _ in 0..reserves {
                    let call = ops();
                    match reserver.reserve() {
                        Ok(token) => {
                            ledger.take(token);
                            max_committed = max_committed.max(reserver.status().durable_ceiling);
                        }
                        Err(error) => {
                            assert!(
                                matches!(error, ReserveError::Uncertain(_)),
                                "damage {damage:?}, fault {fault:?}: {error:?}"
                            );
                            failed_call = Some(call);
                            break 'script;
                        }
                    }
                }
            }
        }
    }
    let crashed_in_script = fault_fired(&rig.borrow());
    if failed_call.is_some() {
        assert!(
            fired.get() || crashed_in_script,
            "damage {damage:?}, fault {fault:?}: a call failed without any injected event"
        );
    }
    if damage.is_none() && fault.is_none() {
        assert_eq!(failed_call, None);
    }
    // The crash interrupted a call that had already observed the damaged slot
    // (the damage fired at rest before that call's first read).
    let damage_seen_by_crashed_call =
        crashed_in_script && failed_call.is_some_and(|call| fired.get() && at <= call);
    let script_end = ops();

    // Recovery boot: faults stop; an armed damage event may still fire.
    rig.borrow_mut().clear_faults();
    rig.borrow_mut().reboot();
    let crashed = fault_fired(&rig.borrow());
    let damaged_before_recovery = fired.get();
    let issued_before_recovery = ledger.max();
    let recovery = match GenerationReserver::open(io(), key(), unprotected(R15_WINDOW)) {
        Ok(mut reserver) => {
            let ceiling = reserver.status().durable_ceiling;
            Ok((ceiling, reserver.reserve().map(|token| ledger.take(token))))
        }
        Err(failure) => Err(failure.error),
    };
    match recovery {
        Ok((ceiling, reserved)) => {
            assert!(
                ceiling >= max_committed,
                "damage {damage:?}, fault {fault:?}: selected {ceiling:#x} below committed {max_committed:#x}"
            );
            assert!(
                ceiling >= issued_before_recovery,
                "damage {damage:?}, fault {fault:?}"
            );
            if let Err(error) = reserved {
                assert!(
                    matches!(error, ReserveError::Uncertain(_)),
                    "damage {damage:?}, fault {fault:?}"
                );
                assert!(
                    !damaged_before_recovery && fired.get(),
                    "damage {damage:?}, fault {fault:?}: recovery commit failed without a new event: {error:?}"
                );
            }
        }
        Err(error) => {
            // No record left: only an interrupted write combined with the loss
            // of the other slot during that same call can do this. It fails
            // closed (remedy Provision).
            assert!(
                matches!(error, OpenError::Blank | OpenError::Corrupt),
                "damage {damage:?}, fault {fault:?}: {error:?}"
            );
            assert!(
                fired.get() && crashed,
                "damage {damage:?}, fault {fault:?}: {error:?} after a single event"
            );
            // A call that saw the damaged slot writes it first, so its sole
            // current copy survives any tear of that write.
            assert!(
                !damage_seen_by_crashed_call,
                "damage {damage:?}, fault {fault:?}: {error:?}: the only current copy was written first"
            );
        }
    }
    R15Run {
        store: rig.into_inner(),
        script_end,
        crashed,
        damaged: fired.get(),
    }
}

#[test]
fn r15_erase_unit_loss_never_lowers() {
    let first = provisioned_store(1).ops();
    let clean = r15_timeline(None, None);
    assert!(!clean.crashed && !clean.damaged);
    let script_end = clean.script_end;
    // Recovery boot: open (2 reads) + one commit (4 reads, 2 writes).
    const RECOVERY_OPS: u64 = 8;
    assert_eq!(clean.store.ops(), script_end + RECOVERY_OPS);
    let script: Vec<(u64, OpKind)> = clean
        .store
        .log()
        .filter(|op| (first..script_end).contains(&op.index))
        .map(|op| (op.index, op.kind))
        .collect();
    assert_eq!(script.len() as u64, script_end - first);
    let script_writes = script.iter().filter(|(_, k)| *k == OpKind::Write).count();
    assert_eq!(script_writes, 2 * 4 + 2, "4 commits and 1 provision");

    // A power loss interrupting every commit and provision write (the F4 tear
    // set). A crash on a read leaves the same durable state as Crash(Unchanged)
    // or Crash(Complete) on the adjacent write, so reads add no new state.
    let mut faults = vec![None];
    for &(index, kind) in &script {
        if kind == OpKind::Write {
            for tear in [Tear::Erased, Tear::PrefixOverOld(31), Tear::Complete] {
                faults.push(Some((index, Fault::Crash(tear))));
            }
        }
    }
    assert_eq!(faults.len(), 1 + 3 * script_writes);
    // One slot erased or junk before every op of the timeline, including the
    // recovery boot's ops.
    let mut damages = vec![None];
    for at in first..script_end + RECOVERY_OPS {
        for slot in [Slot::A, Slot::B] {
            for kind in [Damage::Erase, Damage::Junk] {
                damages.push(Some((at, slot, kind)));
            }
        }
    }
    let mut cases = 0usize;
    // Per planned power loss: timelines in which it fired together with damage.
    let mut combined = vec![0usize; faults.len()];
    for &damage in &damages {
        for (i, &fault) in faults.iter().enumerate() {
            let run = r15_timeline(damage, fault);
            // No planned event is a no-op: alone, each one fires.
            if damage.is_none() {
                assert_eq!(run.crashed, fault.is_some(), "fault {fault:?}");
            }
            if fault.is_none() {
                assert_eq!(run.damaged, damage.is_some(), "damage {damage:?}");
            }
            combined[i] += usize::from(run.crashed && run.damaged);
            cases += 1;
        }
    }
    assert_eq!(cases, damages.len() * faults.len());
    // Every power loss is combined with slot damage at many points (both
    // before it and during its recovery), not only run on its own.
    for (fault, &count) in faults.iter().zip(&combined).skip(1) {
        assert!(
            count >= 4 * 8,
            "{fault:?} combined with damage only {count} times"
        );
    }
    let total: usize = combined.iter().sum();
    println!("R15: {cases} timelines, {total} with both a power loss and slot damage");
}

/// Exhaustive over `OpenError`: adding a variant fails to compile here.
fn open_error_index(error: OpenError) -> usize {
    match error {
        OpenError::StoreUnavailable => 0,
        OpenError::UnsupportedFormat => 1,
        OpenError::SlotAliasing => 2,
        OpenError::ForeignReceiver => 3,
        OpenError::ForeignBinding => 4,
        OpenError::Unreadable => 5,
        OpenError::Blank => 6,
        OpenError::Corrupt => 7,
        OpenError::BelowFloor { .. } => 8,
        OpenError::RolledBack { .. } => 9,
        OpenError::Exhausted => 10,
    }
}

/// Exhaustive over `ProvisionError`.
fn provision_error_index(error: ProvisionError) -> usize {
    match error {
        ProvisionError::Store(_) => 0,
        ProvisionError::UnsupportedFormat => 1,
        ProvisionError::SlotAliasing => 2,
        ProvisionError::EpochNotNewer { .. } => 3,
        ProvisionError::BelowFloor { .. } => 4,
        ProvisionError::PriorHistoryNotExceeded { .. } => 5,
        ProvisionError::Uncertain(_) => 6,
    }
}

/// Exhaustive over `PoisonCause`.
fn poison_index(cause: PoisonCause) -> usize {
    match cause {
        PoisonCause::WriteFailed(_) => 0,
        PoisonCause::VerifyFailed(_) => 1,
        PoisonCause::ReadFailed(_) => 2,
        PoisonCause::MediaChanged => 3,
    }
}

#[test]
fn r16_bounds_and_write_counts() {
    assert_eq!(MAX_STORE_CALLS_PER_OPEN, 2);
    assert_eq!(MAX_STORE_CALLS_PER_RESERVE, 6);
    assert_eq!(MAX_STORE_CALLS_PER_PROVISION, 6);

    // --- open: exactly 2 reads (A then B) and 0 writes on every outcome.
    let mut unsupported = provisioned_store(1);
    let mut v2 = unsupported.raw(Slot::A);
    v2[4] = 2;
    unsupported.set_raw(Slot::A, sealed(v2));
    let mut aliased = provisioned_store(1);
    aliased.set_raw(Slot::B, record_bytes(Slot::A, key(), 1, 0, 1));
    let mut junk = FaultStore::erased(0xFF);
    junk.set_raw(Slot::A, JUNK);
    junk.set_raw(Slot::B, JUNK);
    let mut exhausted = FaultStore::erased(0xFF);
    write_both(&mut exhausted, key(), 1, u64::MAX, 3);
    let mut one_corrupt = provisioned_store(1);
    one_corrupt.set_raw(Slot::B, JUNK);
    let unprotected1 = unprotected(1);
    let open_cases: Vec<(&str, FaultStore, ReserverConfig, Option<OpenError>)> = vec![
        ("ok", provisioned_store(1), unprotected1, None),
        ("ok, B corrupt", one_corrupt, unprotected1, None),
        (
            "ok, A unreadable",
            faulted(provisioned_store(1), &[(0, Fault::ReadIo)]),
            unprotected1,
            None,
        ),
        (
            "store unavailable",
            faulted(provisioned_store(1), &[(1, Fault::Unavailable)]),
            unprotected1,
            Some(OpenError::StoreUnavailable),
        ),
        (
            "unsupported",
            unsupported,
            unprotected1,
            Some(OpenError::UnsupportedFormat),
        ),
        (
            "aliasing",
            aliased,
            unprotected1,
            Some(OpenError::SlotAliasing),
        ),
        (
            "foreign receiver",
            {
                let mut s = FaultStore::erased(0xFF);
                provisioned(&mut s, foreign_receiver_key(), 1);
                s
            },
            unprotected1,
            Some(OpenError::ForeignReceiver),
        ),
        (
            "foreign binding",
            {
                let mut s = FaultStore::erased(0xFF);
                provisioned(&mut s, foreign_binding_key(), 1);
                s
            },
            unprotected1,
            Some(OpenError::ForeignBinding),
        ),
        (
            "unreadable",
            faulted(
                provisioned_store(1),
                &[(0, Fault::ReadIo), (1, Fault::ReadIo)],
            ),
            unprotected1,
            Some(OpenError::Unreadable),
        ),
        (
            "blank",
            FaultStore::erased(0xFF),
            unprotected1,
            Some(OpenError::Blank),
        ),
        ("corrupt", junk, unprotected1, Some(OpenError::Corrupt)),
        (
            "below floor",
            provisioned_store(1),
            config(1, RollbackDefense::Floor(epoch(2))),
            Some(OpenError::BelowFloor {
                stored: epoch(1),
                floor: epoch(2),
            }),
        ),
        (
            "rolled back",
            provisioned_store(1),
            config(1, RollbackDefense::Witness(generation(value(1, 1)))),
            Some(OpenError::RolledBack {
                witness: generation(value(1, 1)),
                stored: value(1, 0),
            }),
        ),
        (
            "exhausted",
            exhausted,
            unprotected1,
            Some(OpenError::Exhausted),
        ),
    ];
    let mut seen = [false; 11];
    for (what, mut store, open_config, expected) in open_cases {
        match expected {
            None => {
                accepted(&mut store, key(), open_config);
            }
            Some(expected) => {
                let error = refused(&mut store, key(), open_config);
                assert_eq!(error, expected, "{what}");
                seen[open_error_index(error)] = true;
            }
        }
    }
    assert_eq!(seen, [true; 11], "every OpenError variant exercised");

    // --- reserve.
    let rig = new_rig(1);
    {
        let mut reserver = open_rig(&rig, unprotected(3)).unwrap();
        let checks: [(&str, (u64, u64)); 3] = [
            (
                "status",
                metered(&rig, || {
                    let _status = reserver.status();
                })
                .1,
            ),
            (
                "debug",
                metered(&rig, || {
                    let _text = format!("{reserver:?}");
                })
                .1,
            ),
            (
                "from window after open",
                metered(&rig, || issued(reserver.reserve_from_window())).1,
            ),
        ];
        for (what, cost) in checks {
            assert_eq!(cost, (0, 0), "{what}");
        }
        let (token, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!((token, cost), (Ok(value(1, 1)), (4, 2)), "commit");
        let (token, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!((token, cost), (Ok(value(1, 2)), (0, 0)), "window hit");
        let (token, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
        assert_eq!((token, cost), (Ok(value(1, 3)), (0, 0)), "window hit");
        let (result, cost) = metered(&rig, || issued(reserver.reserve_from_window()));
        assert_eq!(
            (result, cost),
            (Err(ReserveError::CommitRequired), (0, 0)),
            "window end"
        );
    }

    // Each committing failure path: typed cause, exact cost, then Poisoned for free.
    let a_junk = |s: &mut FaultStore| s.set_raw(Slot::A, JUNK);
    let older = |s: &mut FaultStore| write_both(s, key(), 1, 0, 1);
    let nothing = |_: &mut FaultStore| {};
    let unchanged = Fault::WriteErr(Tear::Unchanged);
    let failures: Vec<(&str, CommitRun, PoisonCause, (u64, u64))> = vec![
        (
            "pre-commit Unavailable",
            commit_run(nothing, &[(0, Fault::Unavailable)]),
            PoisonCause::ReadFailed(Slot::A),
            (2, 0),
        ),
        (
            "pre-commit contradiction",
            commit_run(older, &[]),
            PoisonCause::MediaChanged,
            (2, 0),
        ),
        (
            "pre-commit Io on the sole current copy",
            commit_run(a_junk, &[(1, Fault::ReadIo)]),
            PoisonCause::ReadFailed(Slot::B),
            (2, 0),
        ),
        (
            "first write",
            commit_run(nothing, &[(2, unchanged)]),
            PoisonCause::WriteFailed(Slot::A),
            (2, 1),
        ),
        (
            "first read-back",
            commit_run(nothing, &[(3, Fault::ReadIo)]),
            PoisonCause::VerifyFailed(Slot::A),
            (3, 1),
        ),
        (
            "second write",
            commit_run(nothing, &[(4, unchanged)]),
            PoisonCause::WriteFailed(Slot::B),
            (3, 2),
        ),
        (
            "second read-back",
            commit_run(nothing, &[(5, Fault::ReadIo)]),
            PoisonCause::VerifyFailed(Slot::B),
            (4, 2),
        ),
    ];
    let mut causes = [false; 4];
    for (what, run, cause, cost) in failures {
        assert_eq!(run.result, Err(ReserveError::Uncertain(cause)), "{what}");
        assert_eq!(run.cost, cost, "{what}");
        assert!(run.cost.0 + run.cost.1 <= u64::from(MAX_STORE_CALLS_PER_RESERVE));
        assert!(run.cost.1 <= 2);
        assert_eq!(run.then, Err(ReserveError::Poisoned(cause)), "{what}");
        assert_eq!(run.then_cost, (0, 0), "{what}");
        assert_eq!(run.status.poisoned, Some(cause), "{what}");
        causes[poison_index(cause)] = true;
    }
    assert_eq!(causes, [true; 4], "every PoisonCause exercised");
    let clean = commit_run(|_| {}, &[]);
    assert_eq!((clean.result, clean.cost), (Ok(value(1, 2)), (4, 2)));
    assert_eq!(
        clean.ops,
        [READ_A, READ_B, WRITE_A, READ_A, WRITE_B, READ_B]
    );
    assert_eq!((clean.then, clean.then_cost), (Ok(value(1, 3)), (4, 2)));
    // provision (4 reads, 2 writes) + open (2 reads) + three commits (4, 2).
    assert_eq!(clean.store.calls(), (4 + 2 + 4 * 3, 2 + 2 * 3));

    // --- provision: success ≤ 6 calls; every refusal 2 reads and 0 writes.
    let mut unsupported = provisioned_store(1);
    let mut v2 = unsupported.raw(Slot::B);
    v2[4] = 2;
    unsupported.set_raw(Slot::B, sealed(v2));
    let mut aliased = provisioned_store(1);
    aliased.set_raw(Slot::A, record_bytes(Slot::B, key(), 1, 0, 1));
    let default = ProvisionGuards::default();
    let provision_cases: Vec<ProvisionCase> = vec![
        ("blank", FaultStore::erased(0xFF), 1, default, None, (4, 2)),
        ("over E1", provisioned_store(1), 2, default, None, (4, 2)),
        (
            "Io",
            faulted(provisioned_store(1), &[(0, Fault::ReadIo)]),
            2,
            default,
            Some(ProvisionError::Store(StoreError::Io)),
            (2, 0),
        ),
        (
            "unsupported",
            unsupported,
            2,
            default,
            Some(ProvisionError::UnsupportedFormat),
            (2, 0),
        ),
        (
            "aliasing",
            aliased,
            2,
            default,
            Some(ProvisionError::SlotAliasing),
            (2, 0),
        ),
        (
            "not newer",
            provisioned_store(2),
            2,
            default,
            Some(ProvisionError::EpochNotNewer { highest: epoch(2) }),
            (2, 0),
        ),
        (
            "below floor",
            provisioned_store(1),
            2,
            guards(Some(5), None),
            Some(ProvisionError::BelowFloor { floor: epoch(5) }),
            (2, 0),
        ),
        (
            "prior history",
            provisioned_store(1),
            2,
            guards(None, Some(value(2, 1))),
            Some(ProvisionError::PriorHistoryNotExceeded {
                prior: generation(value(2, 1)),
            }),
            (2, 0),
        ),
        (
            "first write",
            faulted(provisioned_store(1), &[(2, unchanged)]),
            2,
            default,
            Some(ProvisionError::Uncertain(PoisonCause::WriteFailed(Slot::A))),
            (2, 1),
        ),
        (
            "first read-back",
            faulted(provisioned_store(1), &[(3, Fault::ReadIo)]),
            2,
            default,
            Some(ProvisionError::Uncertain(PoisonCause::VerifyFailed(
                Slot::A,
            ))),
            (3, 1),
        ),
        (
            "second write",
            faulted(provisioned_store(1), &[(4, unchanged)]),
            2,
            default,
            Some(ProvisionError::Uncertain(PoisonCause::WriteFailed(Slot::B))),
            (3, 2),
        ),
        (
            "second read-back",
            faulted(provisioned_store(1), &[(5, Fault::ReadIo)]),
            2,
            default,
            Some(ProvisionError::Uncertain(PoisonCause::VerifyFailed(
                Slot::B,
            ))),
            (4, 2),
        ),
    ];
    let mut seen = [false; 7];
    for (what, mut store, e, provision_guards, expected, cost) in provision_cases {
        let before = store.calls();
        let result = provision(&mut store, key(), epoch(e), provision_guards);
        let spent = diff(before, store.calls());
        assert_eq!(spent, cost, "{what}");
        assert!(spent.0 + spent.1 <= u64::from(MAX_STORE_CALLS_PER_PROVISION));
        match expected {
            None => assert!(result.is_ok(), "{what}: {result:?}"),
            Some(expected) => {
                assert_eq!(result, Err(expected), "{what}");
                seen[provision_error_index(expected)] = true;
            }
        }
    }
    assert_eq!(seen, [true; 7], "every ProvisionError variant exercised");
}

// ---------------------------------------------------------------------------
// R17–R21: poisoning, media changes, runtime rot, lying and dropped writes
// ---------------------------------------------------------------------------

#[test]
fn r17_poison_and_reopen() {
    let mut cases = 0;
    for tear in [Tear::Unchanged, Tear::Complete, Tear::PrefixOverOld(31)] {
        // (B damaged at rest before the commit, fault offset, faulted slot)
        for (b_damaged, offset, slot) in [
            (false, 2, Slot::A),
            (false, 4, Slot::B),
            (true, 2, Slot::B),
            (true, 4, Slot::A),
        ] {
            for after_reboot in [false, true] {
                let what = format!("{tear:?}, B damaged {b_damaged}, op {offset}");
                let rig = new_rig(1);
                let mut ledger = Ledger::default();
                {
                    let mut reserver = open_rig(&rig, unprotected(2)).unwrap();
                    for _ in 0..4 {
                        ledger.take(reserver.reserve().unwrap());
                    }
                    if b_damaged {
                        rig.borrow_mut().set_raw(Slot::B, JUNK);
                    }
                    inject_next(&rig, offset, Fault::WriteErr(tear));
                    let (result, cost) = metered(&rig, || issued(reserver.reserve()));
                    let cause = PoisonCause::WriteFailed(slot);
                    assert_eq!(result, Err(ReserveError::Uncertain(cause)), "{what}");
                    assert_eq!(ReserveError::Uncertain(cause).remedy(), Remedy::ReopenStore);
                    let expected_cost = if offset == 2 { (2, 1) } else { (3, 2) };
                    assert_eq!(cost, expected_cost, "{what}");
                    for _ in 0..3 {
                        let (again, cost) = metered(&rig, || issued(reserver.reserve()));
                        assert_eq!(again, Err(ReserveError::Poisoned(cause)), "{what}");
                        assert_eq!(cost, (0, 0));
                        let (again, cost) =
                            metered(&rig, || issued(reserver.reserve_from_window()));
                        assert_eq!(again, Err(ReserveError::Poisoned(cause)), "{what}");
                        assert_eq!(cost, (0, 0));
                    }
                    assert_eq!(ReserveError::Poisoned(cause).remedy(), Remedy::ReopenStore);
                    assert_eq!(reserver.status().poisoned, Some(cause));
                }
                if after_reboot {
                    rig.borrow_mut().reboot();
                }
                let mut reserver = open_rig(&rig, unprotected(2)).unwrap();
                for _ in 0..3 {
                    let v = ledger.take(reserver.reserve().unwrap());
                    assert!(v > value(1, 4), "{what}");
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 3 * 4 * 2);
}

#[derive(Debug, Clone, Copy)]
enum External {
    RestoreOlder,
    HigherBoth,
    HigherOne(Slot),
    ForeignReceiverIn(Slot),
    ForeignBindingIn(Slot),
    UnsupportedIn(Slot),
}

#[test]
fn r18_media_changed() {
    let changes = [
        External::RestoreOlder,
        External::HigherBoth,
        External::HigherOne(Slot::A),
        External::HigherOne(Slot::B),
        External::ForeignReceiverIn(Slot::A),
        External::ForeignReceiverIn(Slot::B),
        External::ForeignBindingIn(Slot::A),
        External::UnsupportedIn(Slot::B),
    ];
    for change in changes {
        let rig = new_rig(1);
        let older = rig.borrow().snapshot();
        let mut ledger = Ledger::default();
        let mut reserver = open_rig(&rig, unprotected(4)).unwrap();
        ledger.take(reserver.reserve().unwrap());
        {
            let mut store = rig.borrow_mut();
            match change {
                External::RestoreOlder => store.restore(older),
                External::HigherBoth => write_both(&mut store, key(), 1, 100, 9),
                External::HigherOne(slot) => {
                    store.set_raw(slot, record_bytes(slot, key(), 1, 100, 9));
                }
                External::ForeignReceiverIn(slot) => {
                    store.set_raw(slot, record_bytes(slot, foreign_receiver_key(), 1, 4, 2));
                }
                External::ForeignBindingIn(slot) => {
                    store.set_raw(slot, record_bytes(slot, foreign_binding_key(), 1, 4, 2));
                }
                External::UnsupportedIn(slot) => {
                    let mut bytes = record_bytes(slot, key(), 1, 4, 2);
                    bytes[6] = 1;
                    store.set_raw(slot, sealed(bytes));
                }
            }
        }
        // Values already committed inside the window are unaffected.
        for counter in 2..=4 {
            let (token, cost) = metered(&rig, || reserver.reserve());
            assert_eq!(ledger.take(token.unwrap()), value(1, counter), "{change:?}");
            assert_eq!(cost, (0, 0));
        }
        // Window exhausted: the next commit detects the change and issues nothing.
        let image = rig.borrow().snapshot();
        let (result, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!(
            result,
            Err(ReserveError::Uncertain(PoisonCause::MediaChanged)),
            "{change:?}"
        );
        assert_eq!(cost, (2, 0), "{change:?}");
        assert_eq!(
            rig.borrow().snapshot(),
            image,
            "{change:?}: nothing written"
        );
        for _ in 0..2 {
            assert_eq!(
                issued(reserver.reserve()),
                Err(ReserveError::Poisoned(PoisonCause::MediaChanged))
            );
            assert_eq!(
                issued(reserver.reserve_from_window()),
                Err(ReserveError::Poisoned(PoisonCause::MediaChanged))
            );
        }
        assert_eq!(ledger.max(), value(1, 4));
    }
}

#[test]
fn r19_runtime_rot_sole_copy() {
    // Rot of B after a commit makes B the first slot written by the next commit.
    for (byte, mask) in [(0usize, 0x01u8), (40, 0x80), (63, 0x10)] {
        let run = commit_run(|s| s.rot(Slot::B, byte, mask), &[]);
        assert_eq!(run.result, Ok(value(1, 2)));
        assert_eq!(run.cost, (4, 2));
        assert_eq!(run.ops, [READ_A, READ_B, WRITE_B, READ_B, WRITE_A, READ_A]);
        assert_eq!(run.status.slots, [SlotCondition::Current; 2]);
    }

    // Tearing that first write at every length: A (the sole current copy) is
    // untouched, so the next open succeeds with a ceiling ≥ everything issued.
    let mut cases = 0;
    for tear in every_tear() {
        for fault in [Fault::Crash(tear), Fault::WriteErr(tear)] {
            let rig = new_rig(1);
            let mut ledger = Ledger::default();
            {
                let mut reserver = open_rig(&rig, unprotected(2)).unwrap();
                ledger.take(reserver.reserve().unwrap());
                ledger.take(reserver.reserve().unwrap());
                rig.borrow_mut().rot(Slot::B, 40, 0x01);
                let base = rig.borrow().ops();
                rig.borrow_mut().inject(base + 2, fault);
                assert_eq!(
                    issued(reserver.reserve()),
                    Err(ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::B))),
                    "{fault:?}"
                );
                assert_eq!(writes_since(&rig.borrow(), base), [Slot::B]);
            }
            rig.borrow_mut().reboot();
            let mut reserver = open_rig(&rig, unprotected(2))
                .unwrap_or_else(|error| panic!("{fault:?}: {error:?}"));
            let status = reserver.status();
            assert!(status.durable_ceiling >= ledger.max(), "{fault:?}");
            // A was never touched: still a valid record (Current, or Stale when
            // B's write landed completely and holds the higher ceiling).
            assert!(
                matches!(
                    status.slots[0],
                    SlotCondition::Current | SlotCondition::Stale
                ),
                "{fault:?}: {:?}",
                status.slots
            );
            ledger.take(reserver.reserve().unwrap());
            cases += 1;
        }
    }
    assert_eq!(cases, 2 * (3 + 3 * 65));
}

/// CHAR (W4): a write acknowledged from a volatile cache, whose read-back also
/// hits the cache, is accepted; after power loss the durable image is older and
/// the value is issued again. Outside C1/C3; this pins the reuse.
#[test]
fn r20_lying_cache_write_reuses_char() {
    let rig = new_rig(1);
    let mut first_boot = Vec::new();
    {
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        first_boot.push(issued(reserver.reserve()).unwrap());
        inject_next(&rig, 2, Fault::Lie);
        inject_next(&rig, 4, Fault::Lie);
        let (lied, cost) = metered(&rig, || issued(reserver.reserve()));
        assert_eq!(
            (lied, cost),
            (Ok(value(1, 2)), (4, 2)),
            "read-back matched the cache"
        );
        first_boot.push(lied.unwrap());
    }
    rig.borrow_mut().reboot();
    let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
    assert_eq!(reserver.status().durable_ceiling, value(1, 1));
    let reissued = issued(reserver.reserve()).unwrap();
    assert_eq!(reissued, value(1, 2));
    assert!(
        first_boot.contains(&reissued),
        "CHAR: lying acknowledgement reissues {reissued:#x}"
    );

    // Contrast: a lie on one slot only is covered by the other durable copy.
    let rig = new_rig(1);
    let mut ledger = Ledger::default();
    {
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        ledger.take(reserver.reserve().unwrap());
        inject_next(&rig, 2, Fault::Lie);
        ledger.take(reserver.reserve().unwrap());
    }
    rig.borrow_mut().reboot();
    let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
    assert_eq!(ledger.take(reserver.reserve().unwrap()), value(1, 3));
}

#[test]
fn r21_silent_drop_detected() {
    for (offset, slot, cost) in [(2, Slot::A, (3, 1)), (4, Slot::B, (4, 2))] {
        let run = commit_run(|_| {}, &[(offset, Fault::SilentDrop)]);
        let cause = PoisonCause::VerifyFailed(slot);
        assert_eq!(run.result, Err(ReserveError::Uncertain(cause)));
        assert_eq!(run.cost, cost);
        assert_eq!(run.then, Err(ReserveError::Poisoned(cause)));
        assert_eq!(run.then_cost, (0, 0));
        // Reopen: the next value is greater than everything returned (value(1, 1)).
        let mut store = run.store;
        store.reboot();
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        let next = issued(reserver.reserve()).unwrap();
        assert!(next > value(1, 1));
    }
    // Provisioning detects a dropped write too (and burns the epoch).
    for (offset, slot) in [(2, Slot::A), (4, Slot::B)] {
        let mut store = faulted(provisioned_store(1), &[(offset, Fault::SilentDrop)]);
        assert_eq!(
            provision(&mut store, key(), epoch(2), ProvisionGuards::default()),
            Err(ProvisionError::Uncertain(PoisonCause::VerifyFailed(slot)))
        );
    }
}

// ---------------------------------------------------------------------------
// R22–R29: rollback, clones, recovery, ECC refusal, legacy history, read causes
// ---------------------------------------------------------------------------

/// Epoch 1, window 1: value 1 issued, snapshot at ceiling 1, values 2 and 3
/// issued, then the snapshot is restored (a faithful two-slot rollback).
fn rolled_back_within_epoch() -> (FaultStore, Vec<u128>) {
    let mut store = provisioned_store(1);
    let mut history = Vec::new();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        history.push(issued(reserver.reserve()).unwrap());
    }
    let image = store.snapshot();
    store.reboot();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        for _ in 0..2 {
            history.push(issued(reserver.reserve()).unwrap());
        }
    }
    assert_eq!(history, [value(1, 1), value(1, 2), value(1, 3)]);
    store.restore(image);
    store.reboot();
    (store, history)
}

#[test]
fn r22_rollback_within_epoch_unprotected_char() {
    let (mut store, history) = rolled_back_within_epoch();
    let mut reserver = open(&mut store, unprotected(1)).unwrap();
    let status = reserver.status();
    assert_eq!(status.rollback, RollbackPosture::Unprotected);
    assert!(!status.rollback.detects_restore_within_epoch());
    assert_eq!(status.durable_ceiling, value(1, 1));
    let reissued = issued(reserver.reserve()).unwrap();
    assert_eq!(reissued, value(1, 2));
    assert!(history.contains(&reissued), "CHAR: Unprotected reissues");
}

#[test]
fn r22_rollback_within_epoch_floor_char() {
    let (mut store, history) = rolled_back_within_epoch();
    let mut reserver = open(&mut store, config(1, RollbackDefense::Floor(epoch(1)))).unwrap();
    let status = reserver.status();
    assert_eq!(status.rollback, RollbackPosture::Floor);
    assert!(!status.rollback.detects_restore_within_epoch());
    let reissued = issued(reserver.reserve()).unwrap();
    assert_eq!(reissued, value(1, 2));
    assert!(
        history.contains(&reissued),
        "CHAR: the floor is cross-epoch only"
    );
}

#[test]
fn r22_rollback_postures() {
    // Within an epoch, a witness detects the restore.
    let (mut store, history) = rolled_back_within_epoch();
    let witness = generation(*history.last().unwrap());
    for rollback in [
        RollbackDefense::Witness(witness),
        RollbackDefense::FloorAndWitness {
            floor: epoch(1),
            witness,
        },
    ] {
        assert!(rollback.posture().detects_restore_within_epoch());
        assert_eq!(
            refused(&mut store, key(), config(1, rollback)),
            OpenError::RolledBack {
                witness,
                stored: value(1, 1)
            }
        );
    }

    // Across an epoch, the floor detects the restore.
    let mut store = provisioned_store(1);
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 1)));
    }
    let epoch1_image = store.snapshot();
    provision(&mut store, key(), epoch(2), guards(Some(2), None)).unwrap();
    {
        let mut reserver = open(&mut store, config(1, RollbackDefense::Floor(epoch(2)))).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(2, 1)));
    }
    store.restore(epoch1_image);
    store.reboot();
    let error = refused(
        &mut store,
        key(),
        config(1, RollbackDefense::Floor(epoch(2))),
    );
    assert_eq!(
        error,
        OpenError::BelowFloor {
            stored: epoch(1),
            floor: epoch(2)
        }
    );
    assert_eq!(error.remedy(), Remedy::Provision);
    let witness = generation(value(2, 1));
    assert_eq!(
        refused(
            &mut store,
            key(),
            config(1, RollbackDefense::Witness(witness))
        ),
        OpenError::RolledBack {
            witness,
            stored: value(1, 1)
        }
    );
}

/// CHAR (I3): an image cloned onto a device with the same ReceiverId is not
/// detected; both devices issue the same generation.
#[test]
fn r23_clone_same_receiver_char() {
    let mut original = provisioned_store(1);
    {
        let mut reserver = open(&mut original, unprotected(4)).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 1)));
    }
    let mut clone = original.clone_image();
    original.reboot();
    clone.reboot();
    let on_original = {
        let mut reserver = open(&mut original, unprotected(4)).unwrap();
        issued(reserver.reserve()).unwrap()
    };
    let on_clone = {
        let mut reserver = open(&mut clone, unprotected(4)).unwrap();
        issued(reserver.reserve()).unwrap()
    };
    assert_eq!(on_original, value(1, 5));
    assert_eq!(
        on_clone, on_original,
        "CHAR: same-id clone issues the same generation"
    );
}

#[test]
fn r23_clone_distinct_receiver() {
    let mut original = provisioned_store(1);
    {
        let mut reserver = open(&mut original, unprotected(4)).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(1, 1)));
    }
    let mut clone = original.clone_image();
    let other_device = key_for_receiver([0x22; 16]);
    let error = refused(&mut clone, other_device, unprotected(4));
    assert_eq!(error, OpenError::ForeignReceiver);
    assert_eq!(error.remedy(), Remedy::RepairStore);
}

/// Tears that leave the first provisioning write non-valid.
const STALE_TEARS: [Tear; 6] = [
    Tear::Unchanged,
    Tear::Erased,
    Tear::PrefixOverOld(31),
    Tear::PrefixOverErased(40),
    Tear::PrefixOverOld(63),
    Tear::WeakTail(8),
];

/// Epoch 1 issues 1..=5; a known rollback restores the ceiling-1 image; the
/// recovery provisioning of epoch 2 (floor 2) is interrupted on its first write.
fn interrupted_recovery(tear: Tear) -> (FaultStore, Vec<u128>) {
    let mut store = provisioned_store(1);
    let mut history = Vec::new();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        history.push(issued(reserver.reserve()).unwrap());
    }
    let image = store.snapshot();
    store.reboot();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        for _ in 0..4 {
            history.push(issued(reserver.reserve()).unwrap());
        }
    }
    store.restore(image);
    let mut store = faulted(store, &[(2, Fault::Crash(tear))]);
    assert_eq!(
        provision(&mut store, key(), epoch(2), guards(Some(2), None)),
        Err(ProvisionError::Uncertain(PoisonCause::WriteFailed(Slot::A))),
        "{tear:?}"
    );
    store.reboot();
    (store, history)
}

#[test]
fn r24_interrupted_recover_without_floor_char() {
    for tear in STALE_TEARS {
        let (mut store, history) = interrupted_recovery(tear);
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        let status = reserver.status();
        assert_eq!(status.epoch, epoch(1), "{tear:?}: stale namespace in force");
        assert_eq!(status.durable_ceiling, value(1, 1));
        let reissued = issued(reserver.reserve()).unwrap();
        assert_eq!(reissued, value(1, 2));
        assert!(
            history.contains(&reissued),
            "CHAR {tear:?}: the stale namespace reissues"
        );
    }
}

#[test]
fn r24_interrupted_recover_with_floor() {
    for tear in STALE_TEARS {
        let (mut store, history) = interrupted_recovery(tear);
        let error = refused(
            &mut store,
            key(),
            config(1, RollbackDefense::Floor(epoch(2))),
        );
        assert_eq!(
            error,
            OpenError::BelowFloor {
                stored: epoch(1),
                floor: epoch(2)
            },
            "{tear:?}"
        );
        assert_eq!(error.remedy(), Remedy::Provision);
        let witness = generation(*history.last().unwrap());
        assert!(matches!(
            refused(
                &mut store,
                key(),
                config(1, RollbackDefense::Witness(witness))
            ),
            OpenError::RolledBack { .. }
        ));
        // The provisioner retries with a NEW epoch until Ok.
        provision(&mut store, key(), epoch(3), guards(Some(2), None)).unwrap();
        let mut reserver = open(&mut store, config(1, RollbackDefense::Floor(epoch(2)))).unwrap();
        let next = issued(reserver.reserve()).unwrap();
        assert_eq!(next, value(3, 1));
        assert!(next > *history.last().unwrap());
    }
}

#[test]
fn r25_ecc_refuse_characterized() {
    let mut store = FaultStore::erased(0xFF).with_media(Media::EccRefuse, WeakPolicy::AlwaysErased);
    provisioned(&mut store, key(), 1);
    let mut ledger = Ledger::default();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        ledger.take(reserver.reserve().unwrap());
    }
    // A power loss leaves slot A marginal: it now reads as erased and refuses programming.
    store.reboot();
    // Offsets: open reads 0 and 1, pre-commit reads 2 and 3, first write (A) 4.
    let mut store = faulted(store, &[(4, Fault::Crash(Tear::WeakTail(31)))]);
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        assert_eq!(
            issued(reserver.reserve()),
            Err(ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::A)))
        );
    }
    for boot in 0..5 {
        store.reboot();
        for _ in 0..2 {
            let before = store.calls();
            let (status, result, then) = {
                let mut reserver = open(&mut store, unprotected(1)).unwrap();
                let status = reserver.status();
                let result = reserver.reserve().map(|token| ledger.take(token));
                (status, result, issued(reserver.reserve()))
            };
            assert_eq!(
                status.slots,
                [SlotCondition::Blank, SlotCondition::Current],
                "boot {boot}"
            );
            assert_eq!(status.durable_ceiling, value(1, 1), "never lowered");
            let cause = PoisonCause::WriteFailed(Slot::A);
            assert_eq!(result, Err(ReserveError::Uncertain(cause)), "boot {boot}");
            assert_eq!(then, Err(ReserveError::Poisoned(cause)));
            // open 2 reads + pre-commit 2 reads + 1 refused write.
            assert_eq!(diff(before, store.calls()), (4, 1));
        }
    }
    // AVAIL: persistent WriteFailed, and nothing reused (only value 1 was ever issued).
    assert_eq!(ledger.issued, [value(1, 1)]);
    // Trusted provisioning cannot program the refusing slot either (RepairStore).
    assert_eq!(
        provision(&mut store, key(), epoch(2), ProvisionGuards::default()),
        Err(ProvisionError::Uncertain(PoisonCause::WriteFailed(Slot::A)))
    );
}

#[test]
fn r26_legacy_history_at_or_above_2_64() {
    let prior = generation(value(5, 9));
    let legacy = guards(None, Some(prior.get()));
    let mut store = FaultStore::erased(0xFF);
    let before = store.calls();
    assert_eq!(
        provision(&mut store, key(), epoch(5), legacy),
        Err(ProvisionError::PriorHistoryNotExceeded { prior })
    );
    assert_eq!(diff(before, store.calls()), (2, 0));
    assert_eq!(refused(&mut store, key(), unprotected(1)), OpenError::Blank);
    // The epoch's first value must strictly exceed the prior history.
    for (p, allowed) in [
        (value(5, 1), false),
        (value(5, 0), true),
        (value(4, u64::MAX), true),
    ] {
        let mut fresh = FaultStore::erased(0xFF);
        let result = provision(&mut fresh, key(), epoch(5), guards(None, Some(p)));
        assert_eq!(result.is_ok(), allowed, "prior {p:#x}: {result:?}");
    }

    // E = 6 → Ok; the first reserved boot opens with Witness(legacy max) and
    // issues above it.
    let report = provision(&mut store, key(), epoch(6), legacy).unwrap();
    assert_eq!(report.epoch, epoch(6));
    {
        let mut reserver = open(&mut store, config(1, RollbackDefense::Witness(prior))).unwrap();
        let first = issued(reserver.reserve()).unwrap();
        assert_eq!(first, value(6, 1));
        assert!(first > prior.get());
    }

    // Provisioned at E = 5 without guards, the witness refuses the open.
    let mut unguarded = provisioned_store(5);
    let error = refused(
        &mut unguarded,
        key(),
        config(1, RollbackDefense::Witness(prior)),
    );
    assert_eq!(
        error,
        OpenError::RolledBack {
            witness: prior,
            stored: value(5, 0)
        }
    );
    assert_eq!(error.remedy(), Remedy::Provision);
}

/// Epoch 1, window 2: values 1..=3 (ceiling 4), a programmer dump of the
/// region, values 5..=7 (ceiling 8), then the dump is written back to both slots.
fn reflashed() -> (FaultStore, [[u8; RECORD_BYTES]; 2], Vec<u128>) {
    let mut store = provisioned_store(1);
    let mut history = Vec::new();
    {
        let mut reserver = open(&mut store, unprotected(2)).unwrap();
        for _ in 0..3 {
            history.push(issued(reserver.reserve()).unwrap());
        }
    }
    let dump = [store.raw(Slot::A), store.raw(Slot::B)];
    store.reboot();
    {
        let mut reserver = open(&mut store, unprotected(2)).unwrap();
        for _ in 0..3 {
            history.push(issued(reserver.reserve()).unwrap());
        }
    }
    assert_eq!(history, [1, 2, 3, 5, 6, 7].map(|c| value(1, c)));
    store.set_raw(Slot::A, dump[0]);
    store.set_raw(Slot::B, dump[1]);
    store.reboot();
    (store, dump, history)
}

#[test]
fn r27_reflash_from_old_dump_unprotected_char() {
    let (mut store, _dump, history) = reflashed();
    let mut reserver = open(&mut store, unprotected(2)).unwrap();
    assert_eq!(reserver.status().durable_ceiling, value(1, 4));
    let reissued = issued(reserver.reserve()).unwrap();
    assert_eq!(reissued, value(1, 5));
    assert!(
        history.contains(&reissued),
        "CHAR: reflash reissues under Unprotected"
    );
}

#[test]
fn r27_reflash_from_old_dump() {
    let (mut store, dump, history) = reflashed();
    let witness = generation(*history.last().unwrap());
    assert_eq!(
        refused(
            &mut store,
            key(),
            config(2, RollbackDefense::Witness(witness))
        ),
        OpenError::RolledBack {
            witness,
            stored: value(1, 4)
        }
    );

    // Maintenance erases the region: Blank; provision a new epoch with the floor
    // advanced; a later reflash of the old dump is then below the floor.
    store.erase(Slot::A);
    store.erase(Slot::B);
    let error = refused(&mut store, key(), unprotected(2));
    assert_eq!(error, OpenError::Blank);
    assert_eq!(error.remedy(), Remedy::Provision);
    provision(&mut store, key(), epoch(2), guards(Some(2), None)).unwrap();
    {
        let mut reserver = open(&mut store, config(2, RollbackDefense::Floor(epoch(2)))).unwrap();
        assert_eq!(issued(reserver.reserve()), Ok(value(2, 1)));
    }
    store.set_raw(Slot::A, dump[0]);
    store.set_raw(Slot::B, dump[1]);
    store.reboot();
    assert_eq!(
        refused(
            &mut store,
            key(),
            config(2, RollbackDefense::Floor(epoch(2)))
        ),
        OpenError::BelowFloor {
            stored: epoch(1),
            floor: epoch(2)
        }
    );
}

/// Epoch 1, window 1: value 1 (ceiling 1); B's bytes are kept; values 2 and 3
/// (ceiling 3); then B alone is rolled back to ceiling 1.
fn single_slot_rollback() -> (FaultStore, Vec<u128>) {
    let mut store = provisioned_store(1);
    let mut history = Vec::new();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        history.push(issued(reserver.reserve()).unwrap());
    }
    let old_b = store.raw(Slot::B);
    store.reboot();
    {
        let mut reserver = open(&mut store, unprotected(1)).unwrap();
        for _ in 0..2 {
            history.push(issued(reserver.reserve()).unwrap());
        }
    }
    store.set_raw(Slot::B, old_b);
    store.reboot();
    (store, history)
}

/// CHAR (W5b): slot A unreadable at open (and still at the pre-commit read)
/// while B was rolled back: B's older ceiling is selected and values reissued.
#[test]
fn r28_unreadable_plus_single_slot_rollback_char() {
    let (store, history) = single_slot_rollback();
    let mut store = faulted(store, &[(0, Fault::ReadIo), (2, Fault::ReadIo)]);
    let mut reserver = open(&mut store, unprotected(1)).unwrap();
    let status = reserver.status();
    assert_eq!(
        status.slots,
        [SlotCondition::Unreadable, SlotCondition::Current]
    );
    assert_eq!(status.durable_ceiling, value(1, 1));
    let reissued = issued(reserver.reserve()).unwrap();
    assert_eq!(reissued, value(1, 2));
    assert!(
        history.contains(&reissued),
        "CHAR: unreadable A + rolled-back B reissues"
    );
}

/// CHAR (widened residual, review finding): the same double fault with slot A
/// ECC-flagged, erased or rotted instead of unreadable. No slot order can close
/// it; the named residual is "single-slot rollback plus any loss of the other
/// slot before the next complete commit". A witness detects every variant.
#[test]
fn r28b_single_slot_rollback_plus_other_slot_loss_char() {
    enum Loss {
        EccFlagged,
        Erased,
        Rotted,
    }
    for loss in [Loss::EccFlagged, Loss::Erased, Loss::Rotted] {
        let (store, history) = single_slot_rollback();
        let witness = generation(*history.last().unwrap());
        let prepare = |mut store: FaultStore| match loss {
            Loss::EccFlagged => faulted(store, &[(0, Fault::ReadCorrupt), (2, Fault::ReadCorrupt)]),
            Loss::Erased => {
                store.erase(Slot::A);
                store
            }
            Loss::Rotted => {
                store.rot(Slot::A, 40, 0x01);
                store
            }
        };
        let mut unprotected_store = prepare(store);
        let reissued = {
            let mut reserver = open(&mut unprotected_store, unprotected(1)).unwrap();
            assert_eq!(reserver.status().durable_ceiling, value(1, 1));
            issued(reserver.reserve()).unwrap()
        };
        assert_eq!(reissued, value(1, 2));
        assert!(
            history.contains(&reissued),
            "CHAR: rolled-back B plus lost A reissues"
        );

        let (store, _history) = single_slot_rollback();
        let mut witnessed = prepare(store);
        assert_eq!(
            refused(
                &mut witnessed,
                key(),
                config(1, RollbackDefense::Witness(witness))
            ),
            OpenError::RolledBack {
                witness,
                stored: value(1, 1)
            }
        );
    }
}

#[test]
fn r28_unreadable_plus_single_slot_rollback_witness() {
    let (store, history) = single_slot_rollback();
    let witness = generation(*history.last().unwrap());
    let mut store = faulted(store, &[(0, Fault::ReadIo)]);
    assert_eq!(
        refused(
            &mut store,
            key(),
            config(1, RollbackDefense::Witness(witness))
        ),
        OpenError::RolledBack {
            witness,
            stored: value(1, 1)
        }
    );

    // Without a witness, a transient Io (A readable again at the pre-commit
    // read) is caught as MediaChanged before anything is issued.
    let (store, _history) = single_slot_rollback();
    let mut store = faulted(store, &[(0, Fault::ReadIo)]);
    let mut reserver = open(&mut store, unprotected(1)).unwrap();
    assert_eq!(
        issued(reserver.reserve()),
        Err(ReserveError::Uncertain(PoisonCause::MediaChanged))
    );
}

#[test]
fn r29_read_failed_cause() {
    // Open tolerates Corrupt A; a pre-commit read error on B (the sole current
    // copy) is ReadFailed(B), not MediaChanged; reopen is fine.
    for fault in [Fault::ReadIo, Fault::ReadCorrupt] {
        let rig = new_rig(1);
        let mut ledger = Ledger::default();
        {
            let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
            ledger.take(reserver.reserve().unwrap());
        }
        rig.borrow_mut().set_raw(Slot::A, JUNK);
        rig.borrow_mut().reboot();
        {
            let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
            assert_eq!(
                reserver.status().slots,
                [SlotCondition::Corrupt, SlotCondition::Current]
            );
            inject_next(&rig, 1, fault);
            let (result, cost) = metered(&rig, || issued(reserver.reserve()));
            let cause = PoisonCause::ReadFailed(Slot::B);
            assert_eq!(result, Err(ReserveError::Uncertain(cause)), "{fault:?}");
            assert_eq!(cost, (2, 0));
            assert_eq!(
                issued(reserver.reserve()),
                Err(ReserveError::Poisoned(cause))
            );
            assert_eq!(ReserveError::Poisoned(cause).remedy(), Remedy::ReopenStore);
        }
        let mut reserver = open_rig(&rig, unprotected(1)).unwrap();
        let next = ledger.take(reserver.reserve().unwrap());
        assert_eq!(next, value(1, 2));
    }

    // No current copy is confirmed and BOTH pre-commit reads fail: the cause
    // names the first failing slot (A).
    for fault_a in [Fault::ReadIo, Fault::ReadCorrupt] {
        for fault_b in [Fault::ReadIo, Fault::ReadCorrupt] {
            let run = commit_run(|_| {}, &[(0, fault_a), (1, fault_b)]);
            let cause = PoisonCause::ReadFailed(Slot::A);
            assert_eq!(
                run.result,
                Err(ReserveError::Uncertain(cause)),
                "{fault_a:?} + {fault_b:?}"
            );
            assert_eq!(run.cost, (2, 0));
            assert_eq!(run.then, Err(ReserveError::Poisoned(cause)));
        }
    }

    // A pre-commit read error on a non-current slot while the other is current:
    // the commit proceeds and writes the errored slot first.
    let b_junk = |s: &mut FaultStore| s.set_raw(Slot::B, JUNK);
    for fault in [Fault::ReadIo, Fault::ReadCorrupt] {
        let run = commit_run(b_junk, &[(1, fault)]);
        assert_eq!(run.result, Ok(value(1, 2)), "{fault:?}");
        assert_eq!(run.ops, [READ_A, READ_B, WRITE_B, READ_B, WRITE_A, READ_A]);
        for (offset, first) in [(0, Slot::A), (1, Slot::B)] {
            let run = commit_run(|_| {}, &[(offset, fault)]);
            assert_eq!(run.result, Ok(value(1, 2)), "{fault:?} at {offset}");
            let writes: Vec<Slot> = run
                .ops
                .iter()
                .filter(|(kind, _)| *kind == OpKind::Write)
                .map(|&(_, slot)| slot)
                .collect();
            assert_eq!(writes, [first, first.other()], "{fault:?} at {offset}");
            assert_eq!(run.status.poisoned, None);
        }
    }

    // Any Unavailable is ReadFailed, even with a current copy in the other slot.
    for (at_rest_b_junk, faults, slot) in [
        (false, vec![(0, Fault::Unavailable)], Slot::A),
        (false, vec![(1, Fault::Unavailable)], Slot::B),
        (
            false,
            vec![(0, Fault::Unavailable), (1, Fault::Unavailable)],
            Slot::A,
        ),
        (true, vec![(0, Fault::Unavailable)], Slot::A),
        (
            true,
            vec![(0, Fault::ReadIo), (1, Fault::Unavailable)],
            Slot::B,
        ),
    ] {
        let run = commit_run(
            |s| {
                if at_rest_b_junk {
                    s.set_raw(Slot::B, JUNK);
                }
            },
            &faults,
        );
        let cause = PoisonCause::ReadFailed(slot);
        assert_eq!(
            run.result,
            Err(ReserveError::Uncertain(cause)),
            "{faults:?}"
        );
        assert_eq!(run.cost, (2, 0), "{faults:?}");
        assert_eq!(run.then, Err(ReserveError::Poisoned(cause)));
    }
}
