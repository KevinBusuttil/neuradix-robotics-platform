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
    SafetyGate::new(leases, constraints, 0.0).unwrap()
}

fn request(value: f64, at_nanos: i128) -> CommandRequest {
    CommandRequest::new(holder(), cap(), value, ts(at_nanos))
}

#[test]
fn authorized_and_within_range_is_accepted() {
    let mut gate = gate_with(vec![Constraint::range("range", -4.0, 4.0).unwrap()], None);
    let d = gate.evaluate(request(2.0, 10), ts(10));
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
    let d = gate.evaluate(request(9.0, 10), ts(10));
    assert_eq!(d.outcome, Outcome::Modified);
    assert_eq!(d.applied, 4.0);
    assert_eq!(d.acted_rules, vec!["thrust-range"]);
}

#[test]
fn slew_rate_limits_change_from_previous_applied() {
    // 2 units/second. First command establishes 0.0 at t=0.
    let mut gate = gate_with(vec![Constraint::slew_rate("slew", 2.0).unwrap()], None);
    let first = gate.evaluate(request(0.0, 0), ts(0));
    assert_eq!(first.applied, 0.0);
    // 0.5s later, request 10.0: max change = 2.0 * 0.5 = 1.0 -> applied 1.0.
    let second = gate.evaluate(request(10.0, 500_000_000), ts(500_000_000));
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
    let d = gate.evaluate(request(1.0, 50_000_000), ts(50_000_000));
    assert_eq!(d.applied, 0.8, "range must bound the first command");
    assert_eq!(d.outcome, Outcome::Modified);
    assert_eq!(d.acted_rules, vec!["range"]);
}

#[test]
fn no_lease_is_rejected_with_safe_output() {
    let gate_leases = LeaseTable::new();
    let mut gate = SafetyGate::new(gate_leases, vec![], 0.0).unwrap();
    let d = gate.evaluate(request(3.0, 10), ts(10));
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
    let d = gate.evaluate(request(3.0, 2_000_000_000), ts(2_000_000_000));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::Expired))
    );
    assert_eq!(d.applied, 0.0);
}

#[test]
fn out_of_envelope_is_rejected() {
    let mut gate = gate_with(vec![], Some(CommandEnvelope::new(-5.0, 5.0).unwrap()));
    let d = gate.evaluate(request(6.0, 10), ts(10));
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

#[test]
fn source_time_inside_expired_lease_cannot_authorize_at_evaluation_time() {
    let mut gate = gate_with(vec![], None);
    let d = gate.evaluate(request(0.5, 500_000_000), ts(1_000_000_000));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::Expired))
    );
    assert_eq!(d.applied, 0.0);
    assert_eq!(d.at, ts(1_000_000_000));
    assert_eq!(d.request.at, ts(500_000_000));
}

#[test]
fn processor_uses_tick_context_for_authority_and_decision_time() {
    use neuradix_runtime::{Processor, TickContext};
    let mut gate = gate_with(vec![], None);
    let ctx = TickContext {
        now: ts(2_000_000_000),
        sequence: 0,
    };
    let decisions = gate.process(&ctx, request(0.5, 0)).unwrap();
    assert_eq!(
        decisions[0].outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::Expired))
    );
    assert_eq!(decisions[0].at, ctx.now);
    assert_eq!(decisions[0].request.at, ts(0));
}

#[test]
fn slew_uses_evaluation_elapsed_time_and_equal_time_holds_output() {
    let mut gate = gate_with(vec![Constraint::slew_rate("slew", 2.0).unwrap()], None);
    assert_eq!(gate.evaluate(request(0.0, 900_000_000), ts(0)).applied, 0.0);
    // Source time regresses; evaluation time advances by 0.25 s.
    assert_eq!(
        gate.evaluate(request(10.0, 0), ts(250_000_000)).applied,
        0.5
    );
    assert_eq!(
        gate.evaluate(request(-10.0, i128::MAX), ts(250_000_000))
            .applied,
        0.5
    );
}

#[test]
fn source_clock_is_diagnostic_but_lease_clock_must_match_evaluation() {
    let mut gate = gate_with(vec![], None);
    let mut input = request(0.5, 0);
    input.at = Timestamp::new(ClockDomain::Utc, i128::MAX);
    let d = gate.evaluate(input.clone(), ts(10));
    assert_eq!(d.outcome, Outcome::Accepted);
    assert_eq!(d.request, input);
    assert_eq!(d.at, ts(10));

    let mut gate = gate_with(vec![], None);
    let d = gate.evaluate(request(0.5, 0), Timestamp::new(ClockDomain::Monotonic, 10));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::Authority(
            AuthorityDenial::ClockDomainMismatch
        ))
    );
    assert_eq!(d.applied, 0.0);
}

#[test]
fn evaluation_clock_faults_latch_even_after_time_appears_valid() {
    for (bad_time, reason) in [
        (ts(9), RejectReason::EvaluationTimeRegression),
        (
            Timestamp::new(ClockDomain::Utc, 10),
            RejectReason::EvaluationClockMismatch,
        ),
    ] {
        let mut gate = gate_with(vec![], None);
        assert_eq!(
            gate.evaluate(request(0.5, 0), ts(10)).outcome,
            Outcome::Accepted
        );
        for now in [bad_time, ts(11)] {
            let d = gate.evaluate(request(0.5, 0), now);
            assert_eq!(d.outcome, Outcome::Rejected(reason));
            assert_eq!(d.applied, 0.0);
            assert_eq!(d.at, now);
            assert_eq!(gate.last_applied(), Some(0.0));
        }
    }
}

#[test]
fn regression_after_expiry_cannot_resurrect_the_lease() {
    let mut gate = gate_with(vec![], None);
    assert!(
        gate.evaluate(request(0.5, 0), ts(2_000_000_000))
            .is_rejected()
    );
    let d = gate.evaluate(request(0.5, 0), ts(500_000_000));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::EvaluationTimeRegression)
    );
    assert_eq!(d.applied, 0.0);
}

#[test]
fn elapsed_time_overflow_latches_a_safe_decision() {
    let mut leases = LeaseTable::new();
    let mut wide_lease = lease(None);
    wide_lease.issued = ts(i128::MIN);
    wide_lease.expires = ts(i128::MAX);
    leases.grant(wide_lease);
    let mut gate = SafetyGate::new(leases, vec![], 0.0).unwrap();
    assert_eq!(
        gate.evaluate(request(0.5, 0), ts(i128::MIN)).outcome,
        Outcome::Accepted
    );
    let d = gate.evaluate(request(0.5, 0), ts(i128::MAX - 1));
    assert_eq!(
        d.outcome,
        Outcome::Rejected(RejectReason::EvaluationTimeOverflow)
    );
    assert_eq!(d.applied, 0.0);
}

#[test]
fn invalid_numeric_configuration_is_rejected() {
    use neuradix_safety::SafetyError;
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(CommandEnvelope::new(bad, 1.0).is_err());
        assert!(CommandEnvelope::new(-1.0, bad).is_err());
        assert!(Constraint::range("range", bad, 1.0).is_err());
        assert!(Constraint::range("range", -1.0, bad).is_err());
        assert!(Constraint::slew_rate("slew", bad).is_err());
    }
    assert!(CommandEnvelope::new(1.0, -1.0).is_err());
    assert!(Constraint::range("range", 1.0, -1.0).is_err());
    assert!(Constraint::slew_rate("slew", -0.1).is_err());
    assert!(CommandEnvelope::new(1.0, 1.0).unwrap().permits(1.0));
    assert!(Constraint::range("point", 1.0, 1.0).is_ok());
    assert!(Constraint::slew_rate("hold", 0.0).is_ok());
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.1, 1.1] {
        assert!(matches!(
            SafetyGate::new(
                LeaseTable::new(),
                vec![Constraint::range("range", -1.0, 1.0).unwrap()],
                bad,
            ),
            Err(SafetyError::InvalidSafeOutput)
        ));
    }
    assert!(
        SafetyGate::new(
            LeaseTable::new(),
            vec![
                Constraint::range("left", -2.0, -1.0).unwrap(),
                Constraint::range("right", 1.0, 2.0).unwrap(),
            ],
            0.0
        )
        .is_err()
    );
}

#[test]
fn non_finite_commands_fall_back_locally_and_recovery_slews_from_safe() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut gate = gate_with(
            vec![
                Constraint::range("range", -1.0, 1.0).unwrap(),
                Constraint::slew_rate("slew", 2.0).unwrap(),
            ],
            None,
        );
        assert_eq!(gate.evaluate(request(0.8, 0), ts(0)).applied, 0.8);
        let d = gate.evaluate(request(bad, 0), ts(100_000_000));
        assert_eq!(d.outcome, Outcome::Rejected(RejectReason::NonFiniteCommand));
        assert_eq!(d.applied, 0.0);
        assert_eq!(d.at, ts(100_000_000));
        let recovered = gate.evaluate(request(1.0, 0), ts(200_000_000));
        assert!((recovered.applied - 0.2).abs() < 1e-12);
    }
}

#[test]
fn constraint_order_and_extreme_finite_values_preserve_all_hard_bounds() {
    for reverse in [false, true] {
        let mut constraints = vec![
            Constraint::range("wide", -1.0, 1.0).unwrap(),
            Constraint::slew_rate("slew", 2.0).unwrap(),
            Constraint::range("narrow", -0.5, 0.5).unwrap(),
        ];
        if reverse {
            constraints.reverse();
        }
        let mut gate = gate_with(constraints, None);
        for (i, value) in [f64::MAX, -f64::MAX, 0.25, -0.25, 0.5]
            .into_iter()
            .enumerate()
        {
            let d = gate.evaluate(request(value, 0), ts(i as i128 * 100_000_000));
            assert!(d.applied.is_finite());
            assert!((-0.5..=0.5).contains(&d.applied));
            assert!(!d.is_rejected());
        }
    }
}

#[test]
fn overflowing_slew_arithmetic_rejects_to_valid_output() {
    for (rate, first, second, elapsed) in [
        (f64::MAX, 0.0, 1.0, 3_000_000_000),     // rate * dt overflow
        (f64::MAX, f64::MAX, 0.0, 500_000_000),  // previous + delta overflow
        (f64::MAX, -f64::MAX, 0.0, 500_000_000), // previous - delta overflow
    ] {
        let mut leases = LeaseTable::new();
        let mut long_lease = lease(None);
        long_lease.expires = ts(10_000_000_000);
        leases.grant(long_lease);
        let mut gate = SafetyGate::new(
            leases,
            vec![
                Constraint::range("range", -f64::MAX, f64::MAX).unwrap(),
                Constraint::slew_rate("slew", rate).unwrap(),
            ],
            0.0,
        )
        .unwrap();
        assert_eq!(gate.evaluate(request(first, 0), ts(0)).applied, first);
        let d = gate.evaluate(request(second, 0), ts(elapsed));
        assert_eq!(d.outcome, Outcome::Rejected(RejectReason::InvalidOutput));
        assert_eq!(d.applied, 0.0);
    }
}

#[test]
fn lease_issue_boundary_and_zero_rate_remain_valid() {
    let mut leases = LeaseTable::new();
    let mut future = lease(None);
    future.issued = ts(10);
    leases.grant(future);
    let mut gate = SafetyGate::new(
        leases,
        vec![Constraint::slew_rate("hold", 0.0).unwrap()],
        0.0,
    )
    .unwrap();
    assert_eq!(
        gate.evaluate(request(0.5, 0), ts(9)).outcome,
        Outcome::Rejected(RejectReason::Authority(AuthorityDenial::NotYetValid))
    );
    let d = gate.evaluate(request(0.5, 0), ts(10));
    assert_eq!(d.outcome, Outcome::Modified);
    assert_eq!(d.applied, 0.0); // zero rate holds the previous safe output
}
