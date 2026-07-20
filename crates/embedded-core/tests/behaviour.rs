//! Behavioural tests for the embedded command gate, watchdog, lease and node.

use neuradix_embedded_core::{
    AuthorityLease, CommandGate, EmbeddedComponent, GateDecision, HealthState, Limits, NodeId,
    Outcome, PropulsionNode, SafeReason, Watchdog,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};

fn t(ns: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, ns)
}

/// A gate with a ±1.0 envelope, 0.5/step slew, a 10 s lease, a 100 ms watchdog
/// and a zero-thrust safe output.
fn gate() -> CommandGate {
    CommandGate::new(
        Limits::new(-1.0, 1.0, 0.5).unwrap(),
        AuthorityLease::until(t(10_000_000_000)),
        Watchdog::new(Duration::from_millis(100)),
        0.0,
    )
}

#[test]
fn limits_reject_invalid_envelopes() {
    assert!(Limits::new(1.0, -1.0, 0.5).is_none()); // min > max
    assert!(Limits::new(-1.0, 1.0, -0.1).is_none()); // negative slew
    assert!(Limits::new(f32::NAN, 1.0, 0.5).is_none()); // non-finite
    assert!(Limits::new(-1.0, 1.0, 0.5).is_some());
}

#[test]
fn lease_grants_only_before_expiry_and_within_domain() {
    let lease = AuthorityLease::until(t(1_000));
    assert!(lease.grants_at(t(0)));
    assert!(lease.grants_at(t(999)));
    assert!(!lease.grants_at(t(1_000))); // expiry is exclusive
    assert!(!lease.grants_at(t(2_000)));
    // A different clock domain never grants.
    assert!(!lease.grants_at(Timestamp::new(ClockDomain::Simulation, 0)));
}

#[test]
fn watchdog_starts_expired_and_is_fed() {
    let mut wd = Watchdog::new(Duration::from_millis(100));
    assert!(wd.is_expired(t(0)), "un-fed watchdog is expired");
    wd.feed(t(0));
    assert!(!wd.is_expired(t(50_000_000))); // 50 ms < 100 ms
    assert!(!wd.is_expired(t(100_000_000))); // exactly at timeout is still ok
    assert!(wd.is_expired(t(100_000_001))); // just past timeout
}

#[test]
fn first_command_is_range_limited_only_not_slewed() {
    let mut g = gate();
    // First command 0.8: clamped by range (ok), NOT slew-limited from 0.
    let d = g.evaluate(Some(0.8), t(0));
    assert_eq!(d.applied, 0.8);
    assert_eq!(d.outcome, Outcome::Accepted);
    assert!(!d.slew_limited);
}

#[test]
fn range_and_slew_limits_act() {
    let mut g = gate();
    g.evaluate(Some(0.0), t(0)); // establish last_applied = 0.0

    // Request 5.0: range-clamped to 1.0, then slew-limited to 0.0 + 0.5 = 0.5.
    let d = g.evaluate(Some(5.0), t(10_000_000));
    assert_eq!(d.applied, 0.5);
    assert!(d.range_clamped);
    assert!(d.slew_limited);
    assert_eq!(d.outcome, Outcome::Modified);

    // Next tick, request 5.0 again: slews another 0.5 to 1.0 (now at the range).
    let d = g.evaluate(Some(5.0), t(20_000_000));
    assert_eq!(d.applied, 1.0);
}

#[test]
fn lease_expiry_forces_the_safe_output() {
    let mut g = gate();
    assert_eq!(g.evaluate(Some(0.8), t(0)).applied, 0.8);
    // After the 10 s lease expires, even a fresh command yields the safe output.
    let d = g.evaluate(Some(0.8), t(10_000_000_000));
    assert_eq!(d.applied, 0.0);
    assert_eq!(d.outcome, Outcome::SafeState(SafeReason::LeaseExpired));
}

#[test]
fn link_loss_forces_the_safe_output() {
    let mut g = gate();
    assert_eq!(g.evaluate(Some(0.8), t(0)).applied, 0.8);
    // 200 ms with no command exceeds the 100 ms watchdog -> safe output.
    let d = g.evaluate(None, t(200_000_000));
    assert_eq!(d.applied, 0.0);
    assert_eq!(d.outcome, Outcome::SafeState(SafeReason::LinkLost));
}

#[test]
fn a_missing_single_tick_holds_the_last_value() {
    let mut g = gate();
    g.evaluate(Some(0.6), t(0));
    // 50 ms later, no command but within the watchdog: hold 0.6.
    let d = g.evaluate(None, t(50_000_000));
    assert_eq!(d.applied, 0.6);
    assert_eq!(d.outcome, Outcome::Accepted);
}

#[test]
fn a_non_finite_command_is_rejected_to_safe() {
    let mut g = gate();
    g.evaluate(Some(0.5), t(0));
    let d = g.evaluate(Some(f32::NAN), t(10_000_000));
    assert_eq!(d.applied, 0.0);
    assert_eq!(d.outcome, Outcome::SafeState(SafeReason::BadCommand));
}

#[test]
fn safe_output_is_clamped_into_the_envelope() {
    // A safe output outside the range is clamped so it is always applicable.
    let g = CommandGate::new(
        Limits::new(-1.0, 1.0, 0.5).unwrap(),
        AuthorityLease::until(t(1_000_000_000)),
        Watchdog::new(Duration::from_millis(100)),
        9.0, // absurd safe output
    );
    assert_eq!(g.safe_output(), 1.0);
}

#[test]
fn a_non_finite_safe_output_is_coerced_finite() {
    // A NaN safe output must never reach the actuator: it is coerced into the
    // envelope, and applying the safe state yields a finite value.
    let mut g = CommandGate::new(
        Limits::new(-1.0, 1.0, 0.5).unwrap(),
        AuthorityLease::until(t(1_000)),
        Watchdog::new(Duration::from_millis(100)),
        f32::NAN,
    );
    assert!(g.safe_output().is_finite());
    let d = g.evaluate(Some(0.5), t(2_000)); // lease already expired -> safe
    assert!(d.applied.is_finite());
    assert_eq!(d.outcome, Outcome::SafeState(SafeReason::LeaseExpired));
}

#[test]
fn propulsion_node_reports_health_and_enters_safe_state() {
    let mut node = PropulsionNode::new(NodeId::new("thruster"), gate());
    assert_eq!(node.id().as_str(), "thruster");
    assert_eq!(node.health(), HealthState::Unknown); // before first tick

    assert_eq!(node.tick(t(0), Some(0.8)), 0.8);
    assert_eq!(node.health(), HealthState::Healthy);
    assert!(!node.in_safe_state());

    // Link loss -> safe output, health degraded.
    let out = node.tick(t(300_000_000), None);
    assert_eq!(out, 0.0);
    assert!(node.in_safe_state());
    assert_eq!(node.health(), HealthState::Degraded);
    let d: GateDecision = node.last_decision().unwrap();
    assert_eq!(d.outcome, Outcome::SafeState(SafeReason::LinkLost));
}

#[test]
fn recovery_slews_from_the_safe_output() {
    // After a safe state, the next command slews from the safe output, not from
    // the pre-safe value — a conservative resumption.
    let mut g = gate();
    g.evaluate(Some(1.0), t(0)); // applied 1.0 (first command, range only)
    g.evaluate(None, t(300_000_000)); // link lost -> safe 0.0
    // Resume with a fresh command 1.0: slew-limited from 0.0 to 0.5.
    let d = g.evaluate(Some(1.0), t(310_000_000));
    assert_eq!(d.applied, 0.5);
    assert!(d.slew_limited);
}
