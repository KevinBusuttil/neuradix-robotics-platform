//! FDIR fault-mode state-machine tests.

use neuradix_runtime::{HealthState, run_lockstep};
use neuradix_safety::{FaultMode, FdirMonitor, FdirPolicy};
use neuradix_time::{ClockDomain, ManualClock, Timestamp};

fn ts(nanos: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Simulation, nanos)
}

/// confirm=2, recovery=2, budget=1.
fn monitor() -> FdirMonitor {
    FdirMonitor::new(FdirPolicy::new(2, 2, 1))
}

#[test]
fn transient_fault_is_debounced() {
    let mut m = monitor();
    // A single degraded report is below the confirm threshold: no escalation.
    assert!(m.observe(HealthState::Degraded, ts(0)).is_none());
    assert_eq!(m.mode(), FaultMode::Nominal);
    // Recovering health clears the fault counter.
    assert!(m.observe(HealthState::Healthy, ts(1)).is_none());
    assert!(m.observe(HealthState::Degraded, ts(2)).is_none());
    assert_eq!(
        m.mode(),
        FaultMode::Nominal,
        "an isolated glitch must not escalate"
    );
}

#[test]
fn confirmed_soft_fault_enters_degraded() {
    let mut m = monitor();
    assert!(m.observe(HealthState::Degraded, ts(0)).is_none());
    let t = m.observe(HealthState::Degraded, ts(1)).expect("confirmed");
    assert_eq!((t.from, t.to), (FaultMode::Nominal, FaultMode::Degraded));
    assert_eq!(m.mode(), FaultMode::Degraded);
}

#[test]
fn confirmed_hard_fault_enters_safe() {
    let mut m = monitor();
    assert!(m.observe(HealthState::Unhealthy, ts(0)).is_none());
    let t = m.observe(HealthState::Unhealthy, ts(1)).expect("confirmed");
    assert_eq!((t.from, t.to), (FaultMode::Nominal, FaultMode::Safe));
    assert_eq!(m.mode(), FaultMode::Safe);
}

#[test]
fn degraded_recovers_after_healthy_streak() {
    let mut m = monitor();
    m.observe(HealthState::Degraded, ts(0));
    m.observe(HealthState::Degraded, ts(1)); // -> Degraded
    assert_eq!(m.mode(), FaultMode::Degraded);
    assert!(m.observe(HealthState::Healthy, ts(2)).is_none()); // 1 healthy, below threshold
    let t = m.observe(HealthState::Healthy, ts(3)).expect("recovered");
    assert_eq!((t.from, t.to), (FaultMode::Degraded, FaultMode::Nominal));
}

#[test]
fn restart_storm_latches_safe_when_budget_exhausted() {
    // budget = 1: the first Degraded->Nominal recovery is allowed; a second
    // degrade/recover cycle exhausts the budget and latches Safe.
    let mut m = monitor();
    // Cycle 1: degrade then recover (uses the one recovery credit).
    m.observe(HealthState::Degraded, ts(0));
    m.observe(HealthState::Degraded, ts(1));
    m.observe(HealthState::Healthy, ts(2));
    m.observe(HealthState::Healthy, ts(3)); // recovered -> Nominal
    assert_eq!(m.mode(), FaultMode::Nominal);
    // Cycle 2: degrade again, then a healthy streak — budget spent -> latch Safe.
    m.observe(HealthState::Degraded, ts(4));
    m.observe(HealthState::Degraded, ts(5)); // -> Degraded
    m.observe(HealthState::Healthy, ts(6));
    let t = m.observe(HealthState::Healthy, ts(7)).expect("latched");
    assert_eq!(t.to, FaultMode::Safe);
    assert_eq!(m.mode(), FaultMode::Safe);
}

#[test]
fn safe_requires_operator_reset_to_return_to_service() {
    let mut m = monitor();
    m.observe(HealthState::Unhealthy, ts(0));
    m.observe(HealthState::Unhealthy, ts(1)); // -> Safe
    // Healthy reports alone do not leave Safe.
    assert!(m.observe(HealthState::Healthy, ts(2)).is_none());
    assert!(m.observe(HealthState::Healthy, ts(3)).is_none());
    assert_eq!(m.mode(), FaultMode::Safe);
    // An explicit reset returns to service.
    let t = m.reset(ts(4)).expect("reset");
    assert_eq!((t.from, t.to), (FaultMode::Safe, FaultMode::Nominal));
    assert_eq!(m.mode(), FaultMode::Nominal);
    // reset() is a no-op when not in Safe.
    assert!(m.reset(ts(5)).is_none());
}

#[test]
fn fdir_replays_identically_through_the_executor() {
    let inputs: Vec<(Timestamp, HealthState)> = [
        HealthState::Healthy,
        HealthState::Degraded,
        HealthState::Degraded, // -> Degraded
        HealthState::Unhealthy,
        HealthState::Unhealthy, // -> Safe
    ]
    .into_iter()
    .enumerate()
    .map(|(i, h)| (ts(i as i128 * 1_000), h))
    .collect();

    let clock_a = ManualClock::new(ts(0));
    let out_a = run_lockstep(&clock_a, &mut monitor(), inputs.clone()).unwrap();
    let clock_b = ManualClock::new(ts(0));
    let out_b = run_lockstep(&clock_b, &mut monitor(), inputs).unwrap();

    assert_eq!(
        out_a, out_b,
        "FDIR transitions must be deterministic and replayable"
    );
    // Exactly two transitions: Nominal->Degraded and Degraded->Safe.
    assert_eq!(out_a.len(), 2);
    assert_eq!(out_a[0].to, FaultMode::Degraded);
    assert_eq!(out_a[1].to, FaultMode::Safe);
}
