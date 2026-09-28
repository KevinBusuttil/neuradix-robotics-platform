//! The embedded actuator boundary performs no heap allocation at runtime.
//!
//! `neuradix-embedded-core` is `no_std` without `alloc`, so heap use cannot
//! compile. This test additionally counts allocations on the current thread
//! across construction, grant, dispatch, fault, revocation and shutdown.
use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
    CommandPolicy, DriverError, DriverPermission, ExecutionMode, Generation, Limits, SessionConfig,
    SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

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

#[test]
fn actuator_boundary_allocates_nothing() {
    COUNTING.with(|on| on.set(true));
    let observed = std::hint::black_box(scenario());
    COUNTING.with(|on| on.set(false));
    assert!(observed > 0);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
