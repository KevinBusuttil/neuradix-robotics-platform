//! The embedded actuator boundary performs no heap allocation at runtime.
//!
//! `neuradix-embedded-core` is `no_std` without `alloc`, so heap use cannot
//! compile. This test additionally counts allocations on the current thread
//! across construction, grant, dispatch, fault, revocation and shutdown, and
//! across the WP-A04.4 reserved startup: provision, open (refused and
//! accepted), reserve, `reserve_from_window`, `DriverPermission::reserved`, a
//! strict grant, a regrant from the window and their refusals.

#[path = "../../command-core/tests/support/fault_store.rs"]
mod support;

use core::num::NonZeroU32;
use neuradix_embedded_core::reservation::{
    GenerationReserver, NamespaceEpoch, OpenError, ProvisionError, ProvisionGuards, ReceiverId,
    ReservationKey, ReserveError, ReserverConfig, RollbackDefense, provision,
};
use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
    CommandPolicy, DriverError, DriverPermission, ExecutionMode, Generation, Limits,
    PermissionError, PermissionStatus, SessionConfig, SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use support::FaultStore;

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

struct Driver {
    writes: u32,
}
impl ActuatorDriver for Driver {
    fn endpoint(&self) -> u64 {
        7
    }
    fn mode(&self) -> ExecutionMode {
        ExecutionMode::Live
    }
    fn write(&mut self, _value: f32) -> Result<(), DriverError> {
        self.writes += 1;
        if self.writes == 6 {
            Err(DriverError::UnknownOutcome)
        } else {
            Ok(())
        }
    }
}

fn t(ms: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000)
}

fn scenario() -> u32 {
    let binding = ActuatorBinding::new(1, 2, 7, ExecutionMode::Live);
    let mut adapter = ActuatorAdapter::new(
        binding,
        Limits::with_slew_rate(-1.0, 1.0, 10.0).unwrap(),
        0.0,
        Driver { writes: 0 },
    )
    .unwrap();
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    let grant = |g| {
        let generation = Generation::new(g).unwrap();
        let lease = AuthorityLease::new(
            1,
            2,
            SessionConfig::new(generation, t(0), t(1000), policy).unwrap(),
        );
        DriverPermission::new(binding, lease).unwrap()
    };
    let command = |g, sequence, ms| Command {
        holder: 1,
        capability: 2,
        value: 0.7,
        meta: CommandMeta {
            generation: Generation::new(g).unwrap(),
            sequence,
            source_at: t(ms),
            deadline: t(ms + 200),
            timeline: 1,
        },
    };
    let mut observed = 0u32;
    adapter.port(ExecutionMode::Live, t(0)).tick(None);
    adapter.grant(grant(1), t(1)).unwrap();
    for i in 0..4u64 {
        let ms = 2 + i as i128 * 10;
        let r = adapter
            .port(ExecutionMode::Live, t(ms))
            .tick(Some(command(1, i, ms)));
        observed += r.driver_calls() as u32;
    }
    adapter.port(ExecutionMode::Replay, t(50)).tick(None);
    adapter.renew(t(2000), t(51)).ok();
    adapter.revoke(t(52));
    adapter.grant(grant(2), t(53)).ok();
    adapter.shutdown(t(54));
    observed
}

/// Never-failing driver for the reserved flow.
struct Pwm {
    writes: u32,
}
impl ActuatorDriver for Pwm {
    fn endpoint(&self) -> u64 {
        7
    }
    fn mode(&self) -> ExecutionMode {
        ExecutionMode::Live
    }
    fn write(&mut self, _value: f32) -> Result<(), DriverError> {
        self.writes += 1;
        Ok(())
    }
}

/// Reserved startup and replacement over the fixed-array `FaultStore`.
/// Returns the driver calls and the store `(reads, writes)` observed.
fn reserved_scenario() -> (u64, (u64, u64)) {
    let binding = ActuatorBinding::new(1, 2, 7, ExecutionMode::Live);
    let key = ReservationKey::new(
        ReceiverId::new([0x5A; 16]).unwrap(),
        binding.reservation_key(),
    );
    let epoch = NamespaceEpoch::new(1).unwrap();
    let config = ReserverConfig::new(NonZeroU32::new(2).unwrap(), RollbackDefense::Unprotected);
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    let lease = |generation: Generation| {
        AuthorityLease::new(
            1,
            2,
            SessionConfig::new(generation, t(0), t(1000), policy).unwrap(),
        )
    };
    let command = |generation: Generation, sequence, ms| Command {
        holder: 1,
        capability: 2,
        value: 0.7,
        meta: CommandMeta {
            generation,
            sequence,
            source_at: t(ms),
            deadline: t(ms + 200),
            timeline: 1,
        },
    };

    let mut adapter = ActuatorAdapter::new_reserved(
        binding,
        Limits::with_slew_rate(-1.0, 1.0, 10.0).unwrap(),
        0.0,
        Pwm { writes: 0 },
    )
    .unwrap();
    let mut observed = 0u64;
    let mut store = FaultStore::erased(0xFF);

    // Fail-closed paths: a blank store refuses open; safe ticks only.
    let blank = GenerationReserver::open(&mut store, key, config).map_err(|f| f.error);
    assert_eq!(blank.err(), Some(OpenError::Blank));
    let idle = adapter.port(ExecutionMode::Live, t(0)).tick(None);
    assert_eq!(idle.permission, PermissionStatus::Missing);
    observed += idle.driver_calls() as u64;

    // Trusted provisioning, then a refused re-provision of the same epoch.
    provision(&mut store, key, epoch, ProvisionGuards::default()).unwrap();
    assert_eq!(
        provision(&mut store, key, epoch, ProvisionGuards::default()).err(),
        Some(ProvisionError::EpochNotNewer { highest: epoch })
    );

    let mut reserver = GenerationReserver::open(&mut store, key, config)
        .map_err(|f| f.error)
        .unwrap();
    // Nothing committed yet: the window is empty.
    assert_eq!(
        reserver.reserve_from_window().err(),
        Some(ReserveError::CommitRequired)
    );

    // The strict adapter refuses an unreserved permission.
    let plain = DriverPermission::new(binding, lease(Generation::new(1).unwrap())).unwrap();
    assert_eq!(
        adapter.grant(plain, t(1)).err(),
        Some(PermissionError::ReservationRequired)
    );

    // Reserve (commit) then strict grant, then traffic.
    let token = reserver.reserve().unwrap();
    let g1 = token.generation();
    let init = adapter
        .grant(
            DriverPermission::reserved(binding, lease(g1), token).unwrap(),
            t(2),
        )
        .unwrap();
    observed += init.driver_calls() as u64;
    for i in 0..3u64 {
        let ms = 10 + i as i128 * 10;
        let r = adapter
            .port(ExecutionMode::Live, t(ms))
            .tick(Some(command(g1, i, ms)));
        observed += r.driver_calls() as u64;
    }

    // Regrant from the window while granted (0 store calls).
    let spare = reserver.reserve_from_window().unwrap();
    let g2 = spare.generation();
    let init = adapter
        .grant(
            DriverPermission::reserved(binding, lease(g2), spare).unwrap(),
            t(50),
        )
        .unwrap();
    observed += init.driver_calls() as u64;
    let r = adapter
        .port(ExecutionMode::Live, t(60))
        .tick(Some(command(g2, 0, 60)));
    observed += r.driver_calls() as u64;

    // Window exhausted: CommitRequired; revoke first, then commit and grant.
    assert_eq!(
        reserver.reserve_from_window().err(),
        Some(ReserveError::CommitRequired)
    );
    observed += adapter.revoke(t(70)).driver_calls() as u64;
    let token = reserver.reserve().unwrap();
    let g3 = token.generation();
    observed += adapter
        .grant(
            DriverPermission::reserved(binding, lease(g3), token).unwrap(),
            t(71),
        )
        .unwrap()
        .driver_calls() as u64;

    // Token refusal: the lease generation differs from the token's.
    let token = reserver.reserve_from_window().unwrap();
    assert_eq!(
        DriverPermission::reserved(binding, lease(g3), token).err(),
        Some(PermissionError::ReservationMismatch)
    );
    let status = reserver.status();
    assert_eq!(status.window_remaining, 0);
    assert_eq!(status.poisoned, None);
    let store_calls = reserver.into_store().calls();

    assert!(adapter.requires_reserved_generations());
    observed += adapter.shutdown(t(80)).driver_calls() as u64;
    (observed, store_calls)
}

#[test]
fn actuator_boundary_allocates_nothing() {
    COUNTING.with(|on| on.set(true));
    let observed = std::hint::black_box(scenario());
    let (driver_calls, store_calls) = std::hint::black_box(reserved_scenario());
    COUNTING.with(|on| on.set(false));
    assert!(observed > 0);
    // Driver: 1 idle + 1 grant + 3 commands + 1 regrant + 1 command + 1 revoke
    // + 1 grant + 1 shutdown = 10.
    assert_eq!(driver_calls, 10);
    // Store: blank open (2, 0), provision (4, 2), refused provision (2, 0),
    // open (2, 0), two commits (8, 4); the window regrant, `reserve_from_window`
    // refusals, `status` and the token refusal make none.
    assert_eq!(store_calls, (18, 6));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
