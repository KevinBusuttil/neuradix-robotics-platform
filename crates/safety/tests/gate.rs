//! Behaviour and determinism tests for the safety authority + constraint gate.

use neuradix_runtime::run_lockstep;
use neuradix_safety::{
    AuthorityDenial, AuthorityLease, Capability, CommandEnvelope, CommandRequest, Constraint,
    Identity, LeaseTable, Outcome, RejectReason, SafetyGate,
};
use neuradix_time::{ClockDomain, ManualClock, Timestamp};

fn ts(nanos: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Simulation, nanos)
}

fn holder() -> Identity {
    Identity::new("depth-controller")
}

fn cap() -> Capability {
    Capability::new("propulsion/vertical-thrust")
}

/// A lease valid for `[0, 1s)` with an optional envelope.
fn lease(envelope: Option<CommandEnvelope>) -> AuthorityLease {
    AuthorityLease {
        holder: holder(),
        capability: cap(),
        priority: 10,
        issued: ts(0),
        expires: ts(1_000_000_000),
        envelope,
    }
}

fn gate_with(constraints: Vec<Constraint>, envelope: Option<CommandEnvelope>) -> SafetyGate {
    let mut leases = LeaseTable::new();
    leases.grant(lease(envelope));
    SafetyGate::new(leases, constraints, 0.0)
}

fn request(value: f64, at_nanos: i128) -> CommandRequest {
    CommandRequest::new(holder(), cap(), value, ts(at_nanos))
}

#[test]
fn authorized_and_within_range_is_accepted() {
    let mut gate = gate_with(vec![Constraint::range("range", -4.0, 4.0).unwrap()], None);
    let d = gate.evaluate(request(2.0, 10));
    assert_eq!(d.outcome, Outcome::Accepted);
    assert_eq!(d.applied, 2.0);
    assert!(d.acted_rules.is_empty());
}

#[test]
fn range_constraint_clamps_and_names_the_rule() {
    let mut gate = gate_with(
        vec![Constraint::range("thrust-range", -4.0, 4.0).unwrap()],
        None,
    );
    let d = gate.evaluate(request(9.0, 10));
    assert_eq!(d.outcome, Outcome::Modified);
    assert_eq!(d.applied, 4.0);
    assert_eq!(d.acted_rules, vec!["thrust-range"]);
}

#[test]
fn slew_rate_limits_change_from_previous_applied() {
    // 2 units/second. First command establishes 0.0 at t=0.
    let mut gate = gate_with(vec![Constraint::slew_rate("slew", 2.0).unwrap()], None);
    let first = gate.evaluate(request(0.0, 0));
    assert_eq!(first.applied, 0.0);
    // 0.5s later, request 10.0: max change = 2.0 * 0.5 = 1.0 -> applied 1.0.
    let second = gate.evaluate(request(10.0, 500_000_000));
    assert_eq!(second.outcome, Outcome::Modified);
    assert_eq!(second.applied, 1.0);
    assert_eq!(second.acted_rules, vec!["slew"]);
}

#[test]
fn first_command_respects_range_despite_slew_ordering() {
    // Regression: on the first command the slew limiter has no previous applied
    // value and must not undo the hard range clamp.
    let mut gate = gate_with(
        vec![
            Constraint::range("range", -0.8, 0.8).unwrap(),
            Constraint::slew_rate("slew", 50.0).unwrap(),
        ],
        None,
    );
    let d = gate.evaluate(request(1.0, 50_000_000));
    assert_eq!(d.applied, 0.8, "range must bound the first command");
    assert_eq!(d.outcome, Outcome::Modified);
    assert_eq!(d.acted_rules, vec!["range"]);
}

#[test]
fn no_lease_is_rejected_with_safe_output() {
    let gate_leases = LeaseTable::new();
    let mut gate = SafetyGate::new(gate_leases, vec![], 0.0);
    let d = gate.evaluate(request(3.0, 10));
    assert!(d.is_rejected());
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::NoLease))
    );
    assert_eq!(d.applied, 0.0, "rejection applies the fail-safe value");
}

#[test]
fn expired_lease_is_rejected() {
    let mut gate = gate_with(vec![], None);
    // t = 2s, lease expired at 1s.
    let d = gate.evaluate(request(3.0, 2_000_000_000));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::Expired))
    );
    assert_eq!(d.applied, 0.0);
}

#[test]
fn out_of_envelope_is_rejected() {
    let mut gate = gate_with(
        vec![],
        Some(CommandEnvelope {
            min: -5.0,
            max: 5.0,
        }),
    );
    let d = gate.evaluate(request(6.0, 10));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::OutOfEnvelope))
    );
}

#[test]
fn decisions_replay_identically_through_the_executor() {
    let inputs: Vec<(Timestamp, CommandRequest)> = (0..6)
        .map(|i| {
            let at = i as i128 * 100_000_000;
            (ts(at), request((i as f64) * 3.0 - 6.0, at))
        })
        .collect();

    let constraints = || {
        vec![
            Constraint::range("range", -4.0, 4.0).unwrap(),
            Constraint::slew_rate("slew", 20.0).unwrap(),
        ]
    };

    let clock_a = ManualClock::new(ts(0));
    let mut gate_a = gate_with(constraints(), None);
    let out_a = run_lockstep(&clock_a, &mut gate_a, inputs.clone()).unwrap();

    let clock_b = ManualClock::new(ts(0));
    let mut gate_b = gate_with(constraints(), None);
    let out_b = run_lockstep(&clock_b, &mut gate_b, inputs).unwrap();

    assert_eq!(
        out_a, out_b,
        "safety decisions must be deterministic and replayable"
    );
    assert_eq!(out_a.len(), 6);
}
