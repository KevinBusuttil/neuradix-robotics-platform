//! Host simulation of the embedded AUV propulsion node behind the guarded
//! actuator boundary.
//!
//! The same `#![no_std]`, allocation-free `ActuatorAdapter` that a board package
//! would build is driven here by a deterministic host static loop (no board, no
//! sleeping, no ambient clock) in **Simulation** mode with an in-memory driver.
//! Trusted setup owns the adapter and installs permission; the commander only
//! reaches a single-use port per tick. It exercises:
//!
//! * trusted permission installation, which first writes the safe output;
//! * a commanded phase (slew-limited from the safe reference);
//! * **link loss** — the commander goes silent and the idle watchdog safes;
//! * **lease expiry** — even once the link returns, an expired lease keeps safe;
//! * a live-mode port against this simulation endpoint, which is refused;
//! * explicit terminal shutdown, after which no driver I/O occurs.
//!
//! Each row separates the gate-selected output from the driver acknowledgement.
//! Neither is physical thrust. The fixed generation is a simulation fixture; live
//! startup reserves one durably first (a portable allocator exists, WP-A04.4:
//! `neuradix_embedded_core::reservation` with `ActuatorAdapter::new_reserved`).
//! Board integration, a board reservation store and physical timing/reset
//! validation remain separate requirements.
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
    CommandPolicy, DispatchReport, DriverError, DriverPermission, ExecutionMode, Generation,
    Limits, Outcome, PermissionStatus, SafeReason, SessionConfig, SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};

/// 50 Hz control loop.
const TICK: Duration = Duration::from_millis(20);
/// The authority lease lasts 1.0 s.
const LEASE_NANOS: i128 = 1_000_000_000;
/// The accepted-command watchdog: 100 ms without an accepted command trips it.
const WATCHDOG: Duration = Duration::from_millis(100);
const HOLDER: u64 = 1;
const CAPABILITY: u64 = 2;
const ENDPOINT: u64 = 0x7468_7275; // "thru"
const MODE: ExecutionMode = ExecutionMode::Simulation;

/// In-memory simulation driver; records acknowledged writes only.
struct SimThruster {
    writes: Rc<RefCell<Vec<f32>>>,
}
impl ActuatorDriver for SimThruster {
    fn endpoint(&self) -> u64 {
        ENDPOINT
    }
    fn mode(&self) -> ExecutionMode {
        MODE
    }
    fn write(&mut self, value: f32) -> Result<(), DriverError> {
        self.writes.borrow_mut().push(value);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("Neuradix — embedded AUV propulsion node (guarded, host simulation)");
    println!(
        "  lease: {} ms, watchdog: {} ms, tick: {} ms, mode: {MODE:?}\n",
        LEASE_NANOS / 1_000_000,
        WATCHDOG.as_nanos() / 1_000_000,
        TICK.as_nanos() / 1_000_000,
    );
    let at = |nanos| Timestamp::new(MODE.clock_domain(), nanos);

    // Trusted setup: own the driver, then install a permission. The generation
    // is simulation-only; live startup requires a durable non-reused value.
    let writes = Rc::new(RefCell::new(Vec::new()));
    let binding = ActuatorBinding::new(HOLDER, CAPABILITY, ENDPOINT, MODE);
    let mut adapter = ActuatorAdapter::new(
        binding,
        Limits::with_slew_rate(-1.0, 1.0, 10.0).ok_or("invalid limits")?,
        0.0, // safe output: zero thrust
        SimThruster {
            writes: Rc::clone(&writes),
        },
    )?;
    let generation = Generation::new(1).ok_or("generation")?;
    let lease = AuthorityLease::new(
        HOLDER,
        CAPABILITY,
        SessionConfig::new(
            generation,
            at(0),
            at(LEASE_NANOS),
            CommandPolicy::new(
                SharedTimeline::new(1, ClockDomain::Simulation)?,
                Duration::from_secs(1),
                Duration::ZERO,
                WATCHDOG,
            )?,
        )?,
    );
    let init = adapter.grant(DriverPermission::new(binding, lease)?, at(0))?;
    println!(
        "  install: {:?}, safe output {:.3}, driver ack {:?}\n",
        init.permission, init.output, init.write_result
    );

    let mut now = at(0);
    let mut saw_link_loss = false;
    let mut saw_lease_expiry = false;
    let mut last_safe = None;

    println!("  t(ms)  request  selected  driver  outcome");
    for step in 1..70u32 {
        now = now.checked_add(TICK)?;
        // Commander model: rising thrust for the first 500 ms, silent from
        // 500-800 ms (link loss), then resumes (but the lease expires at 1000 ms).
        let ms = now.as_nanos() / 1_000_000;
        let request = if ms < 500 {
            Some((0.05 * step as f32).min(1.0))
        } else if ms < 800 {
            None
        } else {
            Some(0.6)
        };
        // Source and receiver share the SAME simulated clock. Never stamp a
        // received packet with arrival time as a substitute for source freshness.
        let command = request.map(|value| Command {
            holder: HOLDER,
            capability: CAPABILITY,
            value,
            meta: CommandMeta {
                generation,
                sequence: u64::from(step),
                source_at: now,
                deadline: now.checked_add(Duration::from_secs(1)).unwrap(),
                timeline: 1,
            },
        });
        // Trusted scheduling fixes mode and time; the component submits input only.
        let report = adapter.port(MODE, now).tick(command);
        let safe = safe_reason(&report);
        saw_link_loss |= safe == Some(SafeReason::WatchdogExpired);
        saw_lease_expiry |= safe == Some(SafeReason::LeaseExpired);
        // Print representative rows and every safe-state transition.
        if step % 7 == 0 || safe != last_safe {
            print_row(ms, request, &report);
        }
        last_safe = safe;
    }

    // A live-scheduled port cannot drive this simulation endpoint.
    now = now.checked_add(TICK)?;
    let crossed = adapter.port(ExecutionMode::Live, now).tick(None);
    let mode_refused = crossed.permission == PermissionStatus::ModeMismatch;

    now = now.checked_add(TICK)?;
    let stop = adapter.shutdown(now);
    let before = writes.borrow().len();
    now = now.checked_add(TICK)?;
    let after_shutdown = adapter.port(MODE, now).tick(None);
    let silent = after_shutdown.driver_calls() == 0 && writes.borrow().len() == before;

    println!("\nsafety checks (gate selection and driver acknowledgement, not physical thrust)");
    let status = |ok| if ok { "observed" } else { "MISSING" };
    println!("  link loss -> safe output   : {}", status(saw_link_loss));
    println!(
        "  lease expiry -> safe output: {}",
        status(saw_lease_expiry)
    );
    println!("  live port on sim endpoint  : {}", status(mode_refused));
    println!(
        "  shutdown safe write {:?}; later I/O suppressed: {}",
        stop.write_result,
        status(silent)
    );
    println!("  driver writes acknowledged : {}", writes.borrow().len());

    if !(saw_link_loss && saw_lease_expiry && mode_refused && silent) {
        return Err("expected link-loss, lease-expiry, mode and shutdown behaviour".into());
    }
    println!("\nadapter selected its local safe output on link loss and lease expiry.");
    Ok(())
}

fn safe_reason(report: &DispatchReport) -> Option<SafeReason> {
    match report.decision?.outcome {
        Outcome::SafeState(reason) => Some(reason),
        _ => None,
    }
}

fn print_row(ms: i128, request: Option<f32>, report: &DispatchReport) {
    let outcome = match report.decision.map(|d| d.outcome) {
        Some(Outcome::Accepted) => "accepted".to_owned(),
        Some(Outcome::Modified) => "modified (limited)".to_owned(),
        Some(Outcome::SafeState(reason)) => format!("SAFE: {reason}"),
        None => format!("inhibited: {:?}", report.permission),
    };
    let ack = match report.write_result {
        Some(Ok(())) => "ack",
        Some(Err(_)) => "FAILED",
        None => "none",
    };
    println!(
        "  {ms:>5}  {:>7}  {:>8.3}  {ack:>6}  {outcome}",
        request.map_or_else(|| "--".to_owned(), |r| format!("{r:.2}")),
        report.output,
    );
}
