//! WP-A04.4 embedded reserved-generation startup, E1–E23.
//!
//! Trusted setup on the `no_std` actuator boundary: a fixed-capacity
//! instrumented driver (`Trace`, the `actuator.rs` pattern) and the shared
//! fault-injecting `FaultStore`. Every test counts driver calls and, where the
//! scheduling rule matters, store calls. Characterization tests (E18 first
//! half, E21, E23) carry a `_char` suffix and assert the documented residual
//! behaviour explicitly, so any change is visible in review.
//!
//! Tokens are single-use: a token passed to `DriverPermission::reserved` is
//! consumed even when the permission is refused, and a dropped token is burned.

// Explicit drops document that adapter Drop performs no driver I/O.
#![allow(clippy::drop_non_drop)]

#[path = "../../command-core/tests/support/fault_store.rs"]
mod support;

use core::num::NonZeroU32;
use std::cell::RefCell;

use neuradix_embedded_core::reservation::{
    GenerationReserver, NamespaceEpoch, OpenError, PoisonCause, ProvisionGuards, RECORD_BYTES,
    ReceiverId, Remedy, ReservationKey, ReservationStore, ReserveError, ReservedGeneration,
    ReserverConfig, RollbackDefense, RollbackPosture, Slot, StoreError, provision,
};
use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
    CommandPolicy, DispatchReport, DriverError, DriverPermission, ExecutionMode, Generation,
    Limits, Outcome, PermissionError, PermissionStatus, SafeReason as R, SessionConfig,
    SessionError, SharedTimeline,
};
use neuradix_time::{Duration, Timestamp};
use support::{FaultStore, OpKind, Tear, provisioned};

// ---------------------------------------------------------------------------
// Fixed-capacity instrumented driver (no allocation in the driver itself)
// ---------------------------------------------------------------------------

struct Trace {
    calls: [f32; 64],
    len: usize,
    /// 1-based call indices that fail.
    fail: [usize; 4],
    error: DriverError,
}
impl Trace {
    fn new() -> RefCell<Self> {
        RefCell::new(Self {
            calls: [0.0; 64],
            len: 0,
            fail: [0; 4],
            error: DriverError::UnknownOutcome,
        })
    }
}
fn calls(trace: &RefCell<Trace>) -> Vec<f32> {
    let t = trace.borrow();
    t.calls[..t.len].to_vec()
}
fn call_count(trace: &RefCell<Trace>) -> usize {
    trace.borrow().len
}
/// Fail the driver call `offset` calls after the calls made so far (1 = next).
fn fail_next(trace: &RefCell<Trace>, offset: usize, error: DriverError) {
    let mut t = trace.borrow_mut();
    t.fail[0] = t.len + offset;
    t.error = error;
}

struct Driver<'a> {
    trace: &'a RefCell<Trace>,
    endpoint: u64,
    mode: ExecutionMode,
}
impl ActuatorDriver for Driver<'_> {
    fn endpoint(&self) -> u64 {
        self.endpoint
    }
    fn mode(&self) -> ExecutionMode {
        self.mode
    }
    fn write(&mut self, value: f32) -> Result<(), DriverError> {
        let mut t = self.trace.borrow_mut();
        let index = t.len;
        t.calls[index] = value;
        t.len += 1;
        if t.fail.contains(&t.len) {
            Err(t.error)
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const HOLDER: u64 = 1;
const CAPABILITY: u64 = 2;
const ENDPOINT: u64 = 7;
const LIVE: ExecutionMode = ExecutionMode::Live;
/// Receiver identity from outside the storage image (MCU UID/OTP stand-in).
const RECEIVER: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
/// Default lease expiry for fixtures, in ms.
const LEASE_MS: i128 = 60_000;
/// Boundary tear lengths K_b (tests.md).
const K_B: [usize; 14] = [0, 1, 4, 8, 24, 31, 32, 40, 48, 52, 59, 60, 63, 64];

fn at(mode: ExecutionMode, ms: i128) -> Timestamp {
    Timestamp::new(mode.clock_domain(), ms * 1_000_000)
}
fn t(ms: i128) -> Timestamp {
    at(LIVE, ms)
}
fn binding(mode: ExecutionMode) -> ActuatorBinding {
    ActuatorBinding::new(HOLDER, CAPABILITY, ENDPOINT, mode)
}
fn limits() -> Limits {
    Limits::with_slew_rate(-1.0, 1.0, 1000.0).unwrap()
}
fn driver(trace: &RefCell<Trace>) -> Driver<'_> {
    Driver {
        trace,
        endpoint: ENDPOINT,
        mode: LIVE,
    }
}
/// Strict adapter: every grant must carry a reserved permission.
fn strict(trace: &RefCell<Trace>) -> ActuatorAdapter<Driver<'_>> {
    ActuatorAdapter::new_reserved(binding(LIVE), limits(), 0.0, driver(trace)).unwrap()
}
/// Legacy adapter (additive API; unreserved grants accepted until the latch).
fn legacy(trace: &RefCell<Trace>) -> ActuatorAdapter<Driver<'_>> {
    ActuatorAdapter::new(binding(LIVE), limits(), 0.0, driver(trace)).unwrap()
}

/// `(epoch << 64) | counter`.
const fn value(epoch: u64, counter: u64) -> u128 {
    ((epoch as u128) << 64) | counter as u128
}
fn generation(value: u128) -> Generation {
    Generation::new(value).unwrap()
}
fn epoch(value: u64) -> NamespaceEpoch {
    NamespaceEpoch::new(value).unwrap()
}

fn lease_in(
    mode: ExecutionMode,
    g: Generation,
    issued_ms: i128,
    expires_ms: i128,
) -> AuthorityLease {
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, mode.clock_domain()).unwrap(),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    AuthorityLease::new(
        HOLDER,
        CAPABILITY,
        SessionConfig::new(g, at(mode, issued_ms), at(mode, expires_ms), policy).unwrap(),
    )
}
fn lease(g: Generation) -> AuthorityLease {
    lease_in(LIVE, g, 0, LEASE_MS)
}
/// Reserved permission for the fixed live binding, lease generation = token.
fn reserved(token: ReservedGeneration) -> DriverPermission {
    let g = token.generation();
    DriverPermission::reserved(binding(LIVE), lease(g), token).unwrap()
}
/// Unreserved (legacy) permission for the fixed live binding.
fn unreserved(g: u128) -> DriverPermission {
    DriverPermission::new(binding(LIVE), lease(generation(g))).unwrap()
}

fn cmd(value: f32, g: u128, seq: u64, source_ms: i128) -> Command {
    Command {
        holder: HOLDER,
        capability: CAPABILITY,
        value,
        meta: CommandMeta {
            generation: generation(g),
            sequence: seq,
            source_at: t(source_ms),
            deadline: t(source_ms + 400),
            timeline: 1,
        },
    }
}
fn live(a: &mut ActuatorAdapter<Driver<'_>>, ms: i128, input: Option<Command>) -> DispatchReport {
    a.port(LIVE, t(ms)).tick(input)
}
fn reason(report: &DispatchReport) -> Option<R> {
    match report.decision?.outcome {
        Outcome::SafeState(r) => Some(r),
        _ => None,
    }
}
/// A valid command was accepted and its value written unchanged.
fn assert_accepted(report: &DispatchReport, value: f32) {
    assert_eq!(report.permission, PermissionStatus::Granted);
    assert_eq!(report.decision.map(|d| d.outcome), Some(Outcome::Accepted));
    assert_eq!(report.output, value);
    assert_eq!(report.write_result, Some(Ok(())));
}
/// A report selected the safe output only: exactly one successful safe write.
fn assert_safe_only(report: &DispatchReport) {
    assert_eq!(report.output, 0.0);
    assert_eq!(report.write_result, Some(Ok(())));
    assert_eq!(report.fallback_result, None);
    assert_eq!(report.driver_calls(), 1);
}
/// A grant installed the permission and wrote the safe output once.
fn assert_initialized(report: &DispatchReport, g: Generation) {
    assert_eq!(report.permission, PermissionStatus::Initialized);
    assert_eq!(report.generation, Some(g));
    assert_safe_only(report);
}

/// The golden key: receiver 01..10, binding `numeric(1, 2, 7, Live)`.
fn key() -> ReservationKey {
    ReservationKey::new(
        ReceiverId::new(RECEIVER).unwrap(),
        binding(LIVE).reservation_key(),
    )
}
fn config(window: u32) -> ReserverConfig {
    ReserverConfig::new(
        NonZeroU32::new(window).unwrap(),
        RollbackDefense::Unprotected,
    )
}
fn provisioned_store(key: ReservationKey, epoch_value: u64) -> FaultStore {
    let mut store = FaultStore::erased(0xFF);
    provisioned(&mut store, key, epoch_value);
    store
}
fn open(
    store: &mut FaultStore,
    key: ReservationKey,
    config: ReserverConfig,
) -> GenerationReserver<&mut FaultStore> {
    GenerationReserver::open(store, key, config)
        .map_err(|f| f.error)
        .unwrap()
}

/// Deliberate second handle over one `FaultStore` (a C5 violation used only by
/// tests) so a test can read the store call counters while a reserver is alive.
struct Shared<'a>(&'a RefCell<FaultStore>);
impl ReservationStore for Shared<'_> {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        self.0.borrow_mut().write(slot, record)
    }
}
fn rig(window: u32) -> (RefCell<FaultStore>, ReserverConfig) {
    (RefCell::new(provisioned_store(key(), 1)), config(window))
}
fn open_rig(rig: &RefCell<FaultStore>, config: ReserverConfig) -> GenerationReserver<Shared<'_>> {
    GenerationReserver::open(Shared(rig), key(), config)
        .map_err(|f| f.error)
        .unwrap()
}
/// Run `call` and return its result with the `(reads, writes)` it cost.
fn metered<T>(rig: &RefCell<FaultStore>, call: impl FnOnce() -> T) -> (T, (u64, u64)) {
    let before = rig.borrow().calls();
    let out = call();
    let after = rig.borrow().calls();
    (out, (after.0 - before.0, after.1 - before.1))
}
/// A committing reserve: 2 pre-commit reads, 2 writes, 2 read-backs.
const COMMIT: (u64, u64) = (4, 2);
const NO_CALLS: (u64, u64) = (0, 0);

/// Startup failure routed to the fail-closed path (never holds a store).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Startup {
    Open(OpenError),
    Reserve(ReserveError),
    Permission(PermissionError),
}

/// Every activated generation across a multi-boot history; asserts strictly
/// increasing activation (I1 at the gate) on every push.
#[derive(Default)]
struct Activated(Vec<u128>);
impl Activated {
    fn record(&mut self, g: Generation) {
        if let Some(&last) = self.0.last() {
            assert!(
                g.get() > last,
                "activated {:#x} not above earlier {last:#x}",
                g.get()
            );
        }
        self.0.push(g.get());
    }
    fn max(&self) -> u128 {
        self.0.last().copied().unwrap_or(0)
    }
}

/// Reserved grant of `token` at `ms`, recorded in `history`.
fn activate(
    adapter: &mut ActuatorAdapter<Driver<'_>>,
    token: ReservedGeneration,
    ms: i128,
    history: &mut Activated,
) -> Result<Generation, Startup> {
    let g = token.generation();
    let permission =
        DriverPermission::reserved(binding(LIVE), lease(g), token).map_err(Startup::Permission)?;
    let report = adapter
        .grant(permission, t(ms))
        .map_err(Startup::Permission)?;
    assert_initialized(&report, g);
    history.record(g);
    Ok(g)
}

/// One trusted startup in the normative order: construct (no I/O), safe tick,
/// open, reserve, grant, then `regrants` replacements from the window. Every
/// error after construction routes to the fail-closed path, checked here: the
/// adapter was never granted and a valid-looking command gets the safe output.
fn startup(
    store: &mut FaultStore,
    config: ReserverConfig,
    regrants: u32,
    history: &mut Activated,
) -> Result<(), Startup> {
    let trace = Trace::new();
    let mut adapter = strict(&trace);
    assert_eq!(call_count(&trace), 0, "construction performs no I/O");
    let first = live(&mut adapter, 0, None);
    assert_eq!(first.permission, PermissionStatus::Missing);
    assert_safe_only(&first);
    let result = open_reserve_grant(&mut adapter, store, config, regrants, history);
    if result.is_err() {
        let refused = live(&mut adapter, 100, Some(cmd(0.9, value(1, 1), 0, 100)));
        assert_eq!(refused.permission, PermissionStatus::Missing);
        assert_eq!(refused.decision, None);
        assert_safe_only(&refused);
        assert_eq!(adapter.generation(), None);
        assert!(calls(&trace).iter().all(|v| *v == 0.0), "safe output only");
    }
    result
}

fn open_reserve_grant(
    adapter: &mut ActuatorAdapter<Driver<'_>>,
    store: &mut FaultStore,
    config: ReserverConfig,
    regrants: u32,
    history: &mut Activated,
) -> Result<(), Startup> {
    let mut reserver =
        GenerationReserver::open(store, key(), config).map_err(|f| Startup::Open(f.error))?;
    assert_eq!(reserver.status().rollback, config.rollback().posture());
    let token = reserver.reserve().map_err(Startup::Reserve)?;
    activate(adapter, token, 1, history)?;
    for i in 0..regrants {
        let token = reserver.reserve_from_window().map_err(Startup::Reserve)?;
        activate(adapter, token, 2 + i128::from(i), history)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// E1–E10: strict adapter, lifecycle, token checks, latch, regrant, reconstruction
// ---------------------------------------------------------------------------

#[test]
fn e1_strict_adapter_refuses_unreserved_permission() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    assert!(a.requires_reserved_generations());
    let permission = unreserved(1);
    assert!(!permission.is_reserved());
    assert_eq!(
        a.grant(permission, t(0)).err(),
        Some(PermissionError::ReservationRequired)
    );
    assert_eq!(
        call_count(&trace),
        0,
        "a refused grant makes 0 driver calls"
    );
    assert_eq!(a.status(), PermissionStatus::Missing);
    assert_eq!(a.generation(), None);
    // A refused grant installs nothing: a valid-looking command is still Missing.
    let report = live(&mut a, 1, Some(cmd(0.9, 1, 0, 1)));
    assert_eq!(report.permission, PermissionStatus::Missing);
    assert_eq!(report.decision, None);
    assert_safe_only(&report);
    // Also for a legacy-space generation above anything reserved.
    assert_eq!(
        a.grant(unreserved(u128::MAX), t(2)).err(),
        Some(PermissionError::ReservationRequired)
    );
    assert_eq!(calls(&trace), vec![0.0]);
    assert!(a.requires_reserved_generations());
}

#[test]
fn e2_reserved_lifecycle_provision_open_reserve_grant() {
    let trace = Trace::new();
    let mut store = FaultStore::erased(0xFF);
    // Trusted, out-of-band provisioning with a registry epoch.
    let report = provision(&mut store, key(), epoch(1), ProvisionGuards::default()).unwrap();
    assert_eq!(report.epoch, epoch(1));
    assert_eq!(report.replaced, None);
    assert_eq!(
        store.calls(),
        (4, 2),
        "provision: 2 reads + 2 verified writes"
    );

    let mut a = strict(&trace);
    let idle = live(&mut a, 0, None);
    assert_eq!(idle.permission, PermissionStatus::Missing);
    assert_safe_only(&idle);

    let mut reserver = open(&mut store, key(), config(4));
    let status = reserver.status();
    assert_eq!(status.durable_ceiling, value(1, 0));
    assert_eq!(status.window_remaining, 0, "first reserve always commits");
    assert_eq!(status.rollback, RollbackPosture::Unprotected);
    let token = reserver.reserve().unwrap();
    assert_eq!(token.generation().get(), value(1, 1));
    assert_eq!(token.key(), key());
    assert_eq!(token.epoch(), epoch(1));
    assert_eq!(reserver.status().durable_ceiling, value(1, 4));

    let g = token.generation();
    let permission = DriverPermission::reserved(binding(LIVE), lease(g), token).unwrap();
    assert!(permission.is_reserved());
    assert_eq!(permission.generation(), g);
    let init = a.grant(permission, t(1)).unwrap();
    assert_initialized(&init, g);
    assert_eq!(a.status(), PermissionStatus::Granted);
    assert_eq!(a.generation(), Some(g));

    let report = live(&mut a, 10, Some(cmd(0.5, g.get(), 0, 10)));
    assert_accepted(&report, 0.5);
    assert_eq!(report.generation, Some(g));
    assert_eq!(calls(&trace), vec![0.0, 0.0, 0.5]);
    assert_eq!(a.last_accepted_at(), Some(t(10)));

    drop(reserver);
    // provision (4, 2) + open (2, 0) + one commit (4, 2).
    assert_eq!(store.calls(), (10, 4));
}

#[test]
fn e3_token_generation_differs_from_lease_generation() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(4));
    let token = reserver.reserve().unwrap();
    assert_eq!(token.generation().get(), value(1, 1));
    // Lease one above the token.
    assert_eq!(
        DriverPermission::reserved(binding(LIVE), lease(generation(value(1, 2))), token).err(),
        Some(PermissionError::ReservationMismatch)
    );
    // Lease below the token, including a legacy-space generation.
    for lower in [value(1, 1), 1] {
        let token = reserver.reserve().unwrap();
        assert!(token.generation().get() > lower);
        assert_eq!(
            DriverPermission::reserved(binding(LIVE), lease(generation(lower)), token).err(),
            Some(PermissionError::ReservationMismatch)
        );
    }
    // Tokens were burned; nothing changed on the adapter and no driver I/O.
    assert_eq!(a.status(), PermissionStatus::Missing);
    assert_eq!(a.generation(), None);
    assert_eq!(call_count(&trace), 0);
    // Burned values are never reissued: the next token is above all three.
    let next = reserver.reserve().unwrap();
    assert_eq!(next.generation().get(), value(1, 4));
    let g = next.generation();
    assert_initialized(&a.grant(reserved(next), t(0)).unwrap(), g);
}

#[test]
fn e4_token_for_another_endpoint_is_mismatched() {
    let endpoint8 = ActuatorBinding::new(HOLDER, CAPABILITY, 8, LIVE);
    assert_ne!(endpoint8.reservation_key(), binding(LIVE).reservation_key());
    let key8 = ReservationKey::new(key().receiver(), endpoint8.reservation_key());
    let mut store = provisioned_store(key8, 1);
    let mut reserver = open(&mut store, key8, config(4));
    let token = reserver.reserve().unwrap();
    assert_eq!(token.key().binding(), endpoint8.reservation_key());
    let g = token.generation();
    // Same holder, capability, mode and generation: only the endpoint differs.
    assert_eq!(
        DriverPermission::reserved(binding(LIVE), lease(g), token).err(),
        Some(PermissionError::ReservationMismatch)
    );
    // Control: the endpoint-8 binding accepts its own token.
    let token = reserver.reserve().unwrap();
    let g = token.generation();
    assert!(
        DriverPermission::reserved(endpoint8, lease(g), token)
            .unwrap()
            .is_reserved()
    );
}

#[test]
fn e5_simulation_token_with_live_binding_is_mismatched() {
    let sim = binding(ExecutionMode::Simulation);
    assert_ne!(sim.reservation_key(), binding(LIVE).reservation_key());
    let sim_key = ReservationKey::new(key().receiver(), sim.reservation_key());
    let mut store = provisioned_store(sim_key, 1);
    let mut reserver = open(&mut store, sim_key, config(4));
    let token = reserver.reserve().unwrap();
    let g = token.generation();
    // The lease is on the live timeline, so `new`'s ModeMismatch check passes
    // and only the reservation key differs.
    assert_eq!(
        DriverPermission::reserved(binding(LIVE), lease(g), token).err(),
        Some(PermissionError::ReservationMismatch)
    );
    // Control: a simulation binding with a simulation lease accepts it.
    let token = reserver.reserve().unwrap();
    let g = token.generation();
    let sim_lease = lease_in(ExecutionMode::Simulation, g, 0, LEASE_MS);
    assert!(
        DriverPermission::reserved(sim, sim_lease, token)
            .unwrap()
            .is_reserved()
    );
}

#[test]
fn e6_latch_after_first_reserved_grant() {
    let trace = Trace::new();
    let mut a = legacy(&trace);
    assert!(!a.requires_reserved_generations());
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(4));
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    assert_initialized(&a.grant(reserved(token), t(0)).unwrap(), g1);
    assert!(
        a.requires_reserved_generations(),
        "latched by the reserved grant"
    );

    let token = reserver.reserve_from_window().unwrap();
    let g2 = token.generation();
    assert!(g2 > g1);
    let before = call_count(&trace);
    assert_eq!(
        a.grant(unreserved(g2.get()), t(1)).err(),
        Some(PermissionError::ReservationRequired)
    );
    assert_eq!(call_count(&trace), before, "0 driver calls");
    assert_eq!(a.generation(), Some(g1));
    assert_eq!(a.status(), PermissionStatus::Granted);
    assert_initialized(&a.grant(reserved(token), t(2)).unwrap(), g2);
    assert!(a.requires_reserved_generations());
}

#[test]
fn e7_legacy_adapter_accepts_reserved_permission() {
    let trace = Trace::new();
    let mut a = legacy(&trace);
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(1));
    let token = reserver.reserve().unwrap();
    let g = token.generation();
    let init = a.grant(reserved(token), t(0)).unwrap();
    assert_initialized(&init, g);
    assert_accepted(&live(&mut a, 5, Some(cmd(0.4, g.get(), 0, 5))), 0.4);
    assert_eq!(calls(&trace), vec![0.0, 0.4]);
}

#[test]
fn e8_revoke_then_regrant_with_new_token_rejects_old_generation() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let (rig, config) = rig(4);
    let mut reserver = open_rig(&rig, config);
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    a.grant(reserved(token), t(0)).unwrap();
    assert_accepted(&live(&mut a, 10, Some(cmd(0.5, g1.get(), 0, 10))), 0.5);

    let revoked = a.revoke(t(20));
    assert_eq!(revoked.permission, PermissionStatus::Revoked);
    assert_eq!(revoked.generation, Some(g1), "watermark retained");
    assert_safe_only(&revoked);
    let denied = live(&mut a, 30, Some(cmd(0.9, g1.get(), 1, 30)));
    assert_eq!(denied.permission, PermissionStatus::Revoked);
    assert_safe_only(&denied);
    // Regrant needs a new token; an unreserved permission is refused.
    assert_eq!(
        a.grant(unreserved(value(1, 2)), t(35)).err(),
        Some(PermissionError::ReservationRequired)
    );
    let (token, cost) = metered(&rig, || reserver.reserve());
    assert_eq!(cost, NO_CALLS, "inside the window");
    let token = token.unwrap();
    let g2 = token.generation();
    assert!(g2 > g1);
    assert_initialized(&a.grant(reserved(token), t(40)).unwrap(), g2);

    // The old generation with a fresh sequence is not resurrected.
    let replay = live(&mut a, 50, Some(cmd(0.9, g1.get(), 2, 50)));
    assert_eq!(reason(&replay), Some(R::GenerationMismatch));
    assert_safe_only(&replay);
    assert_accepted(&live(&mut a, 60, Some(cmd(0.3, g2.get(), 0, 60))), 0.3);
    assert!(!calls(&trace).contains(&0.9));
}

#[test]
fn e9_renewal_needs_no_token() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let (rig, config) = rig(1);
    let mut reserver = open_rig(&rig, config);
    let token = reserver.reserve().unwrap();
    let g = token.generation();
    let permission =
        DriverPermission::reserved(binding(LIVE), lease_in(LIVE, g, 0, 1000), token).unwrap();
    a.grant(permission, t(0)).unwrap();
    assert_accepted(&live(&mut a, 10, Some(cmd(0.5, g.get(), 0, 10))), 0.5);

    let status_before = reserver.status();
    let driver_before = call_count(&trace);
    let (renewed, cost) = metered(&rig, || a.renew(t(5000), t(500)));
    assert_eq!(renewed, Ok(()));
    assert_eq!(cost, NO_CALLS);
    assert_eq!(call_count(&trace), driver_before, "renewal performs no I/O");
    assert_eq!(reserver.status(), status_before, "no reservation consumed");
    assert_eq!(a.generation(), Some(g));
    assert!(a.requires_reserved_generations());
    // Past the original expiry (1000 ms) the renewed lease still grants.
    assert_accepted(&live(&mut a, 1500, Some(cmd(0.2, g.get(), 1, 1500))), 0.2);
}

#[test]
fn e10_driver_fault_reconstruction_uses_greater_token() {
    let (rig, config) = rig(4);
    let mut reserver = open_rig(&rig, config);
    let trace_a = Trace::new();
    let mut a = strict(&trace_a);
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    a.grant(reserved(token), t(0)).unwrap();
    assert_accepted(&live(&mut a, 5, Some(cmd(0.5, g1.get(), 0, 5))), 0.5);
    let token = reserver.reserve_from_window().unwrap();
    let g2 = token.generation();
    assert_initialized(&a.grant(reserved(token), t(10)).unwrap(), g2);

    // Driver fault on the next requested write: one safe fallback, then latched.
    fail_next(&trace_a, 1, DriverError::UnknownOutcome);
    let faulted = live(&mut a, 20, Some(cmd(0.6, g2.get(), 0, 20)));
    assert_eq!(faulted.write_result, Some(Err(DriverError::UnknownOutcome)));
    assert_eq!(faulted.fallback_result, Some(Ok(())));
    assert_eq!(a.driver_fault(), Some(DriverError::UnknownOutcome));
    let before = call_count(&trace_a);
    assert_eq!(
        a.grant(unreserved(value(9, 9)), t(30)).err(),
        Some(PermissionError::DriverFault)
    );
    assert_eq!(call_count(&trace_a), before);
    drop(a);
    assert_eq!(call_count(&trace_a), before, "drop performs no I/O");

    // Trusted reconstruction: a new strict adapter and a fresh, greater token.
    let trace_b = Trace::new();
    let mut b = strict(&trace_b);
    assert_eq!(live(&mut b, 40, None).permission, PermissionStatus::Missing);
    let token = reserver.reserve_from_window().unwrap();
    let g3 = token.generation();
    assert!(g3 > g2 && g2 > g1);
    assert_initialized(&b.grant(reserved(token), t(41)).unwrap(), g3);
    let stale = live(&mut b, 50, Some(cmd(0.9, g2.get(), 1, 50)));
    assert_eq!(reason(&stale), Some(R::GenerationMismatch));
    assert_safe_only(&stale);
    assert_accepted(&live(&mut b, 60, Some(cmd(0.4, g3.get(), 0, 60))), 0.4);
    assert_eq!(b.driver_fault(), None);
}

// ---------------------------------------------------------------------------
// E11–E14: restart, burned values, crash during startup reserve, blank store
// ---------------------------------------------------------------------------

#[test]
fn e11_acc05_restart_rejects_replayed_old_generation() {
    let config = config(4);
    let mut store = provisioned_store(key(), 1);

    // Boot 1: runs to t = 10 s with g1.
    let trace1 = Trace::new();
    let g1 = {
        let mut a = strict(&trace1);
        assert_eq!(live(&mut a, 0, None).permission, PermissionStatus::Missing);
        let mut reserver = open(&mut store, key(), config);
        let token = reserver.reserve().unwrap();
        let g1 = token.generation();
        assert_initialized(&a.grant(reserved(token), t(0)).unwrap(), g1);
        for seq in 0..=5u64 {
            let ms = if seq == 0 { 10 } else { 2000 * i128::from(seq) };
            // Sequence 0..=5 (including 5, the one later replayed) accepted.
            assert_accepted(&live(&mut a, ms, Some(cmd(0.5, g1.get(), seq, ms))), 0.5);
        }
        assert_eq!(a.last_accepted_at(), Some(t(10_000)));
        let written = call_count(&trace1);
        drop(a);
        assert_eq!(call_count(&trace1), written, "adapter drop performs no I/O");
        g1
    };
    store.reboot();

    // Boot 2: the clock restarts at 0; the new generation is greater.
    let trace2 = Trace::new();
    let mut b = strict(&trace2);
    assert_eq!(live(&mut b, 0, None).permission, PermissionStatus::Missing);
    let mut reserver = open(&mut store, key(), config);
    let token = reserver.reserve().unwrap();
    let g2 = token.generation();
    assert!(g2 > g1);
    assert_eq!(g2.get(), value(1, 5), "unused boot-1 window values skipped");
    assert_initialized(&b.grant(reserved(token), t(1)).unwrap(), g2);

    // Replayed g1 traffic with fresh boot-2 timestamps: a fresh sequence, and
    // the exact last accepted sequence.
    for (ms, seq) in [(20, 6), (30, 5)] {
        let before = call_count(&trace2);
        let replay = live(&mut b, ms, Some(cmd(0.9, g1.get(), seq, ms)));
        assert_eq!(reason(&replay), Some(R::GenerationMismatch));
        assert_safe_only(&replay);
        assert_eq!(calls(&trace2)[before..], [0.0]);
    }
    assert_accepted(&live(&mut b, 40, Some(cmd(0.5, g2.get(), 0, 40))), 0.5);
    assert!(!calls(&trace2).contains(&0.9));
}

#[test]
fn e12_crash_between_reserve_and_grant_burns_value() {
    for window in [1u32, 4] {
        let config = config(window);
        let mut store = provisioned_store(key(), 1);
        // Boot 1: reserve, then power loss before the grant.
        let g1 = {
            let trace = Trace::new();
            let mut a = strict(&trace);
            live(&mut a, 0, None);
            let mut reserver = open(&mut store, key(), config);
            let token = reserver.reserve().unwrap();
            let g1 = token.generation();
            assert_eq!(g1.get(), value(1, 1));
            let _burned = token;
            drop(reserver);
            assert_eq!(a.generation(), None, "never granted");
            assert_eq!(calls(&trace), vec![0.0]);
            g1
        };
        // provision (4, 2) + open (2, 0) + the committing reserve (4, 2): no
        // store I/O happened after `reserve` returned (token and reserver drop
        // perform none), so the crash point is strictly after the commit.
        assert_eq!(store.calls(), (10, 4), "no store I/O after reserve");
        store.reboot();

        // Boot 2: the burned value is never reissued.
        let mut history = Activated::default();
        startup(&mut store, config, 0, &mut history).unwrap();
        assert!(history.max() > g1.get());
        assert_eq!(
            history.max(),
            value(1, u64::from(window) + 1),
            "window {window}"
        );
    }
}

#[test]
fn e13_crash_at_every_store_op_of_startup_reserve() {
    // Store ops of one startup: open reads A, B; pre-commit reads A, B; write,
    // read-back of the first slot; write, read-back of the second.
    const OPS: u64 = 8;
    let reference_kinds: Vec<OpKind> = {
        let mut store = provisioned_store(key(), 1);
        let mut history = Activated::default();
        startup(&mut store, config(1), 0, &mut history).unwrap();
        store.reboot();
        let base = store.ops();
        startup(&mut store, config(1), 0, &mut history).unwrap();
        assert_eq!(store.ops() - base, OPS, "open (2) + committing reserve (6)");
        store
            .log()
            .filter(|op| op.index >= base)
            .map(|op| op.kind)
            .collect()
    };
    use OpKind::{Read, Write};
    assert_eq!(
        reference_kinds,
        [Read, Read, Read, Read, Write, Read, Write, Read]
    );
    // Both slots are current after a clean boot, so A is written first.
    let expected = |k: u64| match k {
        0 | 1 => Startup::Open(OpenError::StoreUnavailable),
        2 => Startup::Reserve(ReserveError::Uncertain(PoisonCause::ReadFailed(Slot::A))),
        3 => Startup::Reserve(ReserveError::Uncertain(PoisonCause::ReadFailed(Slot::B))),
        4 => Startup::Reserve(ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::A))),
        5 => Startup::Reserve(ReserveError::Uncertain(PoisonCause::VerifyFailed(Slot::A))),
        6 => Startup::Reserve(ReserveError::Uncertain(PoisonCause::WriteFailed(Slot::B))),
        _ => Startup::Reserve(ReserveError::Uncertain(PoisonCause::VerifyFailed(Slot::B))),
    };
    let mut write_tears = vec![Tear::Unchanged, Tear::Erased, Tear::Complete];
    for k in K_B {
        write_tears.push(Tear::PrefixOverOld(k));
        write_tears.push(Tear::PrefixOverErased(k));
    }

    let mut cases = 0u32;
    for window in [1u32, 4] {
        let regrants = window - 1;
        let config = config(window);
        for (k, kind) in (0..OPS).zip(&reference_kinds) {
            // The tear only applies to writes; a read crash has one case.
            let tears: &[Tear] = match kind {
                Write => &write_tears,
                Read => &[Tear::Unchanged],
            };
            for &tear in tears {
                cases += 1;
                let mut store = provisioned_store(key(), 1);
                let mut history = Activated::default();
                // Boot 1: clean, activations up to the window.
                startup(&mut store, config, regrants, &mut history).unwrap();
                store.reboot();
                // Boot 2: power loss at store op k of the startup.
                let base = store.ops();
                store.inject(base + k, support::Fault::Crash(tear));
                let crashed = startup(&mut store, config, regrants, &mut history);
                assert_eq!(crashed, Err(expected(k)), "window {window} op {k} {tear:?}");
                assert!(store.dead(), "crash fired at op {k}");
                let before = history.max();
                // Boots 3 and 4: open succeeds (I3) and activation is greater (I1).
                for boot in [3, 4] {
                    store.reboot();
                    let result = startup(&mut store, config, regrants, &mut history);
                    assert_eq!(
                        result,
                        Ok(()),
                        "boot {boot} window {window} op {k} {tear:?}"
                    );
                }
                assert!(history.max() > before);
                assert_eq!(history.0.len(), 3 * window as usize);
            }
        }
    }
    // 2 windows × (6 read ops + 2 write ops × 31 tears).
    assert_eq!(cases, 136);
}

#[test]
fn e14_unprovisioned_store_fails_closed() {
    for erased in [0xFF, 0x00] {
        let mut store = FaultStore::erased(erased);
        for attempt in 1..=2u64 {
            let failure = GenerationReserver::open(&mut store, key(), config(4)).unwrap_err();
            assert_eq!(failure.error, OpenError::Blank);
            assert_eq!(failure.error.remedy(), Remedy::Provision);
            let (_error, returned) = failure.into_parts();
            assert_eq!(returned.calls(), (2 * attempt, 0), "2 reads, 0 writes");
        }
        assert_eq!(store.raw(Slot::A), [erased; RECORD_BYTES]);
        assert_eq!(store.raw(Slot::B), [erased; RECORD_BYTES]);

        let trace = Trace::new();
        let mut a = strict(&trace);
        let report = live(&mut a, 0, Some(cmd(0.9, value(1, 1), 0, 0)));
        assert_eq!(report.permission, PermissionStatus::Missing);
        assert_eq!(report.decision, None);
        assert_safe_only(&report);
        let later = live(&mut a, 10, Some(cmd(0.9, value(1, 1), 1, 10)));
        assert_eq!(later.permission, PermissionStatus::Missing);
        assert_safe_only(&later);
        assert_eq!(calls(&trace), vec![0.0, 0.0]);
        assert_eq!(a.generation(), None);
    }
}

// ---------------------------------------------------------------------------
// E15–E18: window regrants, spares, replacement while granted, spare rule
// ---------------------------------------------------------------------------

#[test]
fn e15_window_regrants_make_no_store_calls() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let (rig, config) = rig(4);
    let mut reserver = open_rig(&rig, config);
    let (token, cost) = metered(&rig, || reserver.reserve());
    assert_eq!(cost, COMMIT);
    let token = token.unwrap();
    assert_eq!(token.generation().get(), value(1, 1));
    a.grant(reserved(token), t(0)).unwrap();
    assert_eq!(reserver.status().window_remaining, 3);

    for i in 1..4u64 {
        let ms = 10 * i128::from(i);
        if i == 2 {
            // Revocation, then regrant.
            assert_safe_only(&a.revoke(t(ms - 1)));
        }
        let (token, cost) = metered(&rig, || reserver.reserve());
        assert_eq!(cost, NO_CALLS, "regrant {i} inside the window");
        let token = token.unwrap();
        let g = token.generation();
        assert_eq!(g.get(), value(1, 1 + i));
        let (report, cost) = metered(&rig, || a.grant(reserved(token), t(ms)));
        assert_eq!(cost, NO_CALLS);
        assert_initialized(&report.unwrap(), g);
        assert_accepted(
            &live(&mut a, ms + 1, Some(cmd(0.1, g.get(), 0, ms + 1))),
            0.1,
        );
    }
    assert_eq!(reserver.status().window_remaining, 0);
    assert_eq!(reserver.status().commits, 2, "provision + one commit");
    let (refused, cost) = metered(&rig, || reserver.reserve_from_window().err());
    assert_eq!(refused, Some(ReserveError::CommitRequired));
    assert_eq!(cost, NO_CALLS);
}

#[test]
fn e16_spare_taken_after_activation_is_used_with_no_store_calls() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let (rig, config) = rig(4);
    let mut reserver = open_rig(&rig, config);
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    a.grant(reserved(token), t(0)).unwrap();
    // Spare taken right after activation: issued after g1, 0 store calls.
    let (spare, cost) = metered(&rig, || reserver.reserve_from_window());
    assert_eq!(cost, NO_CALLS);
    let spare = spare.unwrap();
    let g2 = spare.generation();
    assert!(g2 > g1);
    assert_accepted(&live(&mut a, 10, Some(cmd(0.5, g1.get(), 0, 10))), 0.5);
    assert_accepted(&live(&mut a, 20, Some(cmd(0.6, g1.get(), 1, 20))), 0.6);

    // Later replacement uses the spare: no store calls at all.
    let (report, cost) = metered(&rig, || a.grant(reserved(spare), t(500)));
    assert_eq!(cost, NO_CALLS);
    assert_initialized(&report.unwrap(), g2);
    let old = live(&mut a, 510, Some(cmd(0.9, g1.get(), 2, 510)));
    assert_eq!(reason(&old), Some(R::GenerationMismatch));
    assert_safe_only(&old);
    assert_accepted(&live(&mut a, 520, Some(cmd(0.3, g2.get(), 0, 520))), 0.3);
}

#[test]
fn e17_replacement_while_granted() {
    let trace = Trace::new();
    let mut a = strict(&trace);
    let (rig, config) = rig(2);
    let mut reserver = open_rig(&rig, config);
    let (token, cost) = metered(&rig, || reserver.reserve());
    assert_eq!(cost, COMMIT);
    let token = token.unwrap();
    let g1 = token.generation();
    a.grant(reserved(token), t(0)).unwrap();
    assert_accepted(&live(&mut a, 10, Some(cmd(0.5, g1.get(), 0, 10))), 0.5);
    assert_eq!(a.status(), PermissionStatus::Granted);

    // With a Granted lease: reserve_from_window + grant make 0 store calls.
    let (replacement, cost) = metered(&rig, || {
        let token = reserver.reserve_from_window().unwrap();
        let g = token.generation();
        (g, a.grant(reserved(token), t(20)))
    });
    assert_eq!(cost, NO_CALLS);
    let (g2, report) = replacement;
    assert!(g2 > g1);
    assert_initialized(&report.unwrap(), g2);
    assert_accepted(&live(&mut a, 30, Some(cmd(0.4, g2.get(), 0, 30))), 0.4);

    // Window empty: CommitRequired with 0 calls; not a poison; still Granted.
    let (refused, cost) = metered(&rig, || reserver.reserve_from_window().err());
    assert_eq!(refused, Some(ReserveError::CommitRequired));
    assert_eq!(cost, NO_CALLS);
    assert_eq!(refused.unwrap().remedy(), Remedy::CommitWhenSafe);
    assert_eq!(reserver.status().poisoned, None);
    assert_eq!(a.status(), PermissionStatus::Granted);
    assert_eq!(a.generation(), Some(g2));

    // Setup revokes first (safe write observed), then commits, then grants.
    let before = call_count(&trace);
    let revoked = a.revoke(t(40));
    assert_eq!(revoked.permission, PermissionStatus::Revoked);
    assert_safe_only(&revoked);
    assert_eq!(calls(&trace)[before..], [0.0]);
    assert_eq!(a.status(), PermissionStatus::Revoked);
    let (token, cost) = metered(&rig, || reserver.reserve());
    assert_eq!(cost, COMMIT);
    let token = token.unwrap();
    let g3 = token.generation();
    assert!(g3 > g2);
    assert_initialized(&a.grant(reserved(token), t(50)).unwrap(), g3);
    assert_accepted(&live(&mut a, 60, Some(cmd(0.2, g3.get(), 0, 60))), 0.2);
}

/// Reference trusted-setup helper implementing the spare-token rule: a spare is
/// taken only after an activation, and every activation drops any older spare.
#[derive(Default)]
struct SpareSetup {
    spare: Option<ReservedGeneration>,
    last_activated: Option<Generation>,
}
impl SpareSetup {
    /// Next token for activation: the spare, else the window, else a commit
    /// (only legal before ingress or after revoke; see the scheduling rule).
    fn next_token<S: ReservationStore>(
        &mut self,
        reserver: &mut GenerationReserver<S>,
    ) -> Result<ReservedGeneration, ReserveError> {
        if let Some(spare) = self.spare.take() {
            return Ok(spare);
        }
        match reserver.reserve_from_window() {
            Err(ReserveError::CommitRequired) => reserver.reserve(),
            other => other,
        }
    }
    fn activate(
        &mut self,
        adapter: &mut ActuatorAdapter<Driver<'_>>,
        token: ReservedGeneration,
        ms: i128,
    ) -> Result<DispatchReport, PermissionError> {
        let g = token.generation();
        let report = adapter.grant(
            DriverPermission::reserved(binding(LIVE), lease(g), token)?,
            t(ms),
        )?;
        self.last_activated = Some(g);
        if self.spare.as_ref().is_some_and(|s| s.generation() < g) {
            let _burned = self.spare.take();
        }
        Ok(report)
    }
    fn take_spare<S: ReservationStore>(&mut self, reserver: &mut GenerationReserver<S>) {
        self.spare = reserver.reserve_from_window().ok();
    }
}

#[test]
fn e18_spare_rule_and_reconstruction() {
    // CHAR: violating the spare rule. A fresh strict adapter accepts a spare
    // older than the last activated token (the first grant has no generation
    // comparison); commands for the last activated token are not replayable.
    {
        let mut store = provisioned_store(key(), 1);
        let mut reserver = open(&mut store, key(), config(4));
        let trace_a = Trace::new();
        let mut a = strict(&trace_a);
        let token = reserver.reserve().unwrap();
        a.grant(reserved(token), t(0)).unwrap();
        let spare = reserver.reserve_from_window().unwrap();
        let g_spare = spare.generation();
        let token = reserver.reserve_from_window().unwrap();
        let g_last = token.generation();
        assert!(g_last > g_spare);
        a.grant(reserved(token), t(10)).unwrap(); // spare NOT dropped (rule violated)
        assert_accepted(&live(&mut a, 20, Some(cmd(0.5, g_last.get(), 0, 20))), 0.5);
        fail_next(&trace_a, 1, DriverError::Unavailable);
        live(&mut a, 30, Some(cmd(0.5, g_last.get(), 1, 30)));
        assert_eq!(a.driver_fault(), Some(DriverError::Unavailable));
        drop(a);

        let trace_b = Trace::new();
        let mut b = strict(&trace_b);
        let init = b.grant(reserved(spare), t(40)).unwrap();
        assert_initialized(&init, g_spare);
        assert!(
            b.generation().unwrap() < g_last,
            "CHAR: older spare accepted"
        );
        let replay = live(&mut b, 50, Some(cmd(0.9, g_last.get(), 2, 50)));
        assert_eq!(reason(&replay), Some(R::GenerationMismatch));
        assert_safe_only(&replay);
        assert!(!calls(&trace_b).contains(&0.9));
    }

    // The reference helper drops older spares on activation, so the
    // reconstructed adapter receives only a token greater than the last one.
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(4));
    let mut setup = SpareSetup::default();
    let trace_a = Trace::new();
    let mut a = strict(&trace_a);
    let token = setup.next_token(&mut reserver).unwrap();
    assert_eq!(token.generation().get(), value(1, 1));
    setup.activate(&mut a, token, 0).unwrap();
    setup.take_spare(&mut reserver);
    let g_spare = setup.spare.as_ref().unwrap().generation();
    assert!(g_spare > setup.last_activated.unwrap());
    // A replacement that bypasses the spare activates a newer token.
    let token = reserver.reserve_from_window().unwrap();
    let g_last = token.generation();
    assert!(g_last > g_spare);
    setup.activate(&mut a, token, 10).unwrap();
    assert!(setup.spare.is_none(), "older spare dropped on activation");
    assert_eq!(setup.last_activated, Some(g_last));
    fail_next(&trace_a, 1, DriverError::Unavailable);
    live(&mut a, 20, None);
    assert!(a.driver_fault().is_some());
    drop(a);

    let trace_b = Trace::new();
    let mut b = strict(&trace_b);
    let token = setup.next_token(&mut reserver).unwrap();
    let g_new = token.generation();
    assert!(
        g_new > g_last,
        "reconstruction receives only a greater token"
    );
    assert_initialized(&setup.activate(&mut b, token, 30).unwrap(), g_new);
    let replay = live(&mut b, 40, Some(cmd(0.9, g_last.get(), 3, 40)));
    assert_eq!(reason(&replay), Some(R::GenerationMismatch));
    assert_safe_only(&replay);
    assert_accepted(&live(&mut b, 50, Some(cmd(0.5, g_new.get(), 0, 50))), 0.5);
}

// ---------------------------------------------------------------------------
// E19–E23: grant precedence, latch, legacy space, rollover, receiver
// ---------------------------------------------------------------------------

#[test]
fn e19_grant_precedence_before_reservation_required() {
    // Control: a clean strict adapter refuses the unreserved permission.
    let trace = Trace::new();
    let mut control = strict(&trace);
    assert_eq!(
        control.grant(unreserved(1), t(0)).err(),
        Some(PermissionError::ReservationRequired)
    );
    assert_eq!(call_count(&trace), 0);

    // Shutdown.
    let trace = Trace::new();
    let mut a = strict(&trace);
    a.shutdown(t(0));
    let before = call_count(&trace);
    assert_eq!(
        a.grant(unreserved(1), t(1)).err(),
        Some(PermissionError::Shutdown)
    );
    assert_eq!(call_count(&trace), before, "Shutdown: 0 driver calls");

    // DriverFault, latched by a failed safe write before any grant.
    let trace = Trace::new();
    let mut a = strict(&trace);
    fail_next(&trace, 1, DriverError::Rejected);
    let faulted = live(&mut a, 0, None);
    assert_eq!(faulted.write_result, Some(Err(DriverError::Rejected)));
    assert_eq!(a.driver_fault(), Some(DriverError::Rejected));
    let before = call_count(&trace);
    assert_eq!(
        a.grant(unreserved(1), t(1)).err(),
        Some(PermissionError::DriverFault)
    );
    assert_eq!(call_count(&trace), before, "DriverFault: 0 driver calls");

    // BindingMismatch.
    let trace = Trace::new();
    let mut a = strict(&trace);
    let other = ActuatorBinding::new(HOLDER, CAPABILITY, 8, LIVE);
    let permission = DriverPermission::new(other, lease(generation(1))).unwrap();
    assert_eq!(
        a.grant(permission, t(0)).err(),
        Some(PermissionError::BindingMismatch)
    );
    // ModeMismatch: control time in another clock domain.
    assert_eq!(
        a.grant(unreserved(1), at(ExecutionMode::Simulation, 0))
            .err(),
        Some(PermissionError::ModeMismatch)
    );
    assert_eq!(
        call_count(&trace),
        0,
        "Binding/ModeMismatch: 0 driver calls"
    );
    assert_eq!(a.status(), PermissionStatus::Missing);
    assert!(a.requires_reserved_generations());

    // ReservationRequired precedes the control-time and generation checks.
    let trace = Trace::new();
    let mut a = strict(&trace);
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(1));
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    a.grant(reserved(token), t(50)).unwrap();
    let before = call_count(&trace);
    // Reused generation and a control time before the gate clock.
    assert_eq!(
        a.grant(unreserved(g1.get()), t(40)).err(),
        Some(PermissionError::ReservationRequired)
    );
    assert_eq!(call_count(&trace), before);
    // Control: the later checks do fire once the reservation requirement is
    // met, so ReservationRequired above really took precedence over them.
    let token = reserver.reserve().unwrap();
    assert!(token.generation() > g1);
    assert_eq!(
        a.grant(reserved(token), t(40)).err(),
        Some(PermissionError::Session(
            SessionError::InvalidEvaluationTime
        ))
    );
    assert_eq!(call_count(&trace), before);
    assert_eq!(a.generation(), Some(g1));
    assert!(a.requires_reserved_generations());
}

#[test]
fn e20_latch_not_set_on_failed_grant() {
    let trace = Trace::new();
    let mut a = legacy(&trace);
    let high = value(2, 5);
    assert_initialized(&a.grant(unreserved(high), t(0)).unwrap(), generation(high));
    assert!(!a.requires_reserved_generations());

    let mut store = provisioned_store(key(), 2);
    let mut reserver = open(&mut store, key(), config(1));
    let token = reserver.reserve().unwrap();
    assert_eq!(token.generation().get(), value(2, 1));
    let before = call_count(&trace);
    assert_eq!(
        a.grant(reserved(token), t(1)).err(),
        Some(PermissionError::Session(SessionError::ReusedGeneration))
    );
    assert_eq!(call_count(&trace), before, "0 driver calls");
    assert!(
        !a.requires_reserved_generations(),
        "latch set only on success"
    );
    assert_eq!(a.generation(), Some(generation(high)));
    assert_eq!(a.status(), PermissionStatus::Granted);

    let higher = value(2, 6);
    assert_initialized(
        &a.grant(unreserved(higher), t(2)).unwrap(),
        generation(higher),
    );
    assert!(!a.requires_reserved_generations());
}

#[test]
fn e21_legacy_ge_2_64_blocks_reserved_char() {
    let trace = Trace::new();
    let mut a = legacy(&trace);
    let legacy_g = 5u128 << 64;
    a.grant(unreserved(legacy_g), t(0)).unwrap();
    assert_accepted(&live(&mut a, 5, Some(cmd(0.5, legacy_g, 0, 5))), 0.5);

    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(4));
    let token = reserver.reserve().unwrap();
    assert_eq!(token.generation().get(), value(1, 1));
    let before = call_count(&trace);
    // CHAR: the reserved grant is below the legacy watermark and is refused.
    assert_eq!(
        a.grant(reserved(token), t(10)).err(),
        Some(PermissionError::Session(SessionError::ReusedGeneration))
    );
    assert_eq!(call_count(&trace), before);
    assert!(!a.requires_reserved_generations());
    assert_eq!(a.generation(), Some(generation(legacy_g)));

    // Migration note: such an adapter is reconstructed, not replaced. The new
    // strict adapter never accepts the legacy generation's traffic.
    drop(a);
    let trace_b = Trace::new();
    let mut b = strict(&trace_b);
    let token = reserver.reserve_from_window().unwrap();
    let g = token.generation();
    assert_initialized(&b.grant(reserved(token), t(20)).unwrap(), g);
    let replay = live(&mut b, 30, Some(cmd(0.9, legacy_g, 1, 30)));
    assert_eq!(reason(&replay), Some(R::GenerationMismatch));
    assert_safe_only(&replay);
}

#[test]
fn e22_timer_rollover_reconstruction() {
    // A 32-bit millisecond timer wraps after 2^32 ms (about 49.7 days).
    let large: i128 = 1 << 32;
    let mut store = provisioned_store(key(), 1);
    let mut reserver = open(&mut store, key(), config(4));
    let trace_a = Trace::new();
    let mut a = strict(&trace_a);
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    let permission = DriverPermission::reserved(
        binding(LIVE),
        lease_in(LIVE, g1, 0, large + LEASE_MS),
        token,
    )
    .unwrap();
    a.grant(permission, t(0)).unwrap();
    assert_accepted(
        &live(
            &mut a,
            large - 1000,
            Some(cmd(0.5, g1.get(), 0, large - 1000)),
        ),
        0.5,
    );
    assert_accepted(
        &live(&mut a, large, Some(cmd(0.5, g1.get(), 1, large))),
        0.5,
    );

    // Rollover: evaluation time regresses; the fault latches.
    let regressed = live(&mut a, 3, Some(cmd(0.6, g1.get(), 2, 3)));
    assert_eq!(reason(&regressed), Some(R::EvaluationTimeRegression));
    assert_safe_only(&regressed);
    let latched = live(&mut a, large + 10, Some(cmd(0.6, g1.get(), 3, large + 10)));
    assert_eq!(reason(&latched), Some(R::EvaluationTimeRegression));
    assert_safe_only(&latched);
    // A grant cannot clear it (each token is burned by its refused grant): at a
    // post-rollover control time, and at a forward control time that only the
    // latched fault refuses (a greater reserved generation would otherwise pass).
    for control_ms in [5, large + 20] {
        let token = reserver.reserve_from_window().unwrap();
        assert!(token.generation() > g1);
        let before = call_count(&trace_a);
        assert_eq!(
            a.grant(reserved(token), t(control_ms)).err(),
            Some(PermissionError::Session(
                SessionError::InvalidEvaluationTime
            )),
            "control time {control_ms} ms"
        );
        assert_eq!(call_count(&trace_a), before);
        assert_eq!(a.generation(), Some(g1));
    }
    let still = live(&mut a, large + 30, None);
    assert_eq!(reason(&still), Some(R::EvaluationTimeRegression));
    assert_safe_only(&still);
    drop(a);

    // Reconstruction with a strict adapter and a fresh token.
    let trace_b = Trace::new();
    let mut b = strict(&trace_b);
    assert_eq!(live(&mut b, 5, None).permission, PermissionStatus::Missing);
    let token = reserver.reserve_from_window().unwrap();
    let g3 = token.generation();
    assert_eq!(g3.get(), value(1, 4), "the burned values are skipped");
    assert_initialized(&b.grant(reserved(token), t(6)).unwrap(), g3);
    // Old-generation traffic with post-rollover timestamps.
    for (ms, seq) in [(10, 4), (20, 0)] {
        let replay = live(&mut b, ms, Some(cmd(0.9, g1.get(), seq, ms)));
        assert_eq!(reason(&replay), Some(R::GenerationMismatch));
        assert_safe_only(&replay);
    }
    assert_accepted(&live(&mut b, 30, Some(cmd(0.5, g3.get(), 0, 30))), 0.5);
    assert!(!calls(&trace_b).contains(&0.9));
}

#[test]
fn e23_receiver_agnostic_token_char() {
    let other = ReservationKey::new(
        ReceiverId::new([0x22; 16]).unwrap(),
        binding(LIVE).reservation_key(),
    );
    assert_ne!(other.receiver(), key().receiver());
    assert_eq!(other.binding(), key().binding());
    let mut store = provisioned_store(other, 1);
    // The store-side defense: this receiver's key refuses the other image.
    let refused = GenerationReserver::open(&mut store, key(), config(1)).unwrap_err();
    assert_eq!(refused.error, OpenError::ForeignReceiver);

    let mut reserver = open(&mut store, other, config(1));
    let token = reserver.reserve().unwrap();
    assert_eq!(token.key(), other);
    let g = token.generation();
    // CHAR: the adapter holds no ReceiverId, so the token is accepted.
    let permission = DriverPermission::reserved(binding(LIVE), lease(g), token).unwrap();
    assert!(permission.is_reserved());
    let trace = Trace::new();
    let mut a = strict(&trace);
    assert_initialized(&a.grant(permission, t(0)).unwrap(), g);
}
