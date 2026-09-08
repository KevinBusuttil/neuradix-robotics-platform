//! A04.1 numeric and runtime regressions, using A04.2 source metadata.
use neuradix_runtime::{run_lockstep, Processor, TickContext};
use neuradix_safety::{AuthorityLease, Capability, CommandEnvelope, CommandMeta, CommandPolicy, CommandRequest, Constraint, Generation, Identity, LeaseTable, Outcome, RejectReason as R, SafetyDecision, SafetyError, SafetyGate, SessionConfig, SharedTimeline};
use neuradix_time::{ClockDomain, Duration, ManualClock, Timestamp};
fn t(n: i128) -> Timestamp { Timestamp::new(ClockDomain::Simulation, n) }
fn config() -> SessionConfig {
    SessionConfig::new(Generation::new(1).unwrap(), t(0), t(10_000_000_000), CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Simulation).unwrap(), Duration::from_millis(100), Duration::ZERO, Duration::from_millis(100)).unwrap()).unwrap()
}
fn request(value: f64, seq: u64, now: i128) -> CommandRequest {
    CommandRequest::new(Identity::new("controller"), Capability::new("thrust"), value, CommandMeta { generation: Generation::new(1).unwrap(), sequence: seq, source_at: t(now), deadline: t(now + 100_000_000), timeline: 1 })
}
fn gate(constraints: Vec<Constraint>, envelope: Option<CommandEnvelope>) -> SafetyGate {
    let mut leases = LeaseTable::new();
    leases.grant(AuthorityLease::new(Identity::new("controller"), Capability::new("thrust"), config(), envelope)).unwrap();
    SafetyGate::new(leases, constraints, 0.0).unwrap()
}
fn evaluate(gate: &mut SafetyGate, value: f64, seq: u64, now: i128) -> SafetyDecision { gate.evaluate(Some(request(value, seq, now)), t(now)) }

#[test]
fn authorized_and_within_range_is_accepted() {
    let mut g = gate(vec![Constraint::range("range", -4.0, 4.0).unwrap()], None);
    let d = evaluate(&mut g, 2.0, 0, 10);
    assert_eq!(d.outcome, Outcome::Accepted); assert_eq!(d.applied, 2.0);
}
#[test]
fn range_constraint_clamps_and_names_the_rule() {
    let mut g = gate(vec![Constraint::range("range", -4.0, 4.0).unwrap()], None);
    let d = evaluate(&mut g, 9.0, 0, 10);
    assert_eq!(d.outcome, Outcome::Modified); assert_eq!(d.applied, 4.0);
    assert_eq!(d.acted_rules, vec!["range"]);
}
#[test]
fn slew_uses_runtime_elapsed_time_and_equal_time_holds() {
    let mut g = gate(vec![Constraint::slew_rate("slew", 2.0).unwrap()], None);
    evaluate(&mut g, 0.0, 0, 0);
    let mut input = request(10.0, 1, 450_000_000);
    input.meta.deadline = t(600_000_000);
    assert_eq!(g.evaluate(Some(input), t(500_000_000)).applied, 1.0);
    assert_eq!(evaluate(&mut g, -10.0, 2, 500_000_000).applied, 1.0);
}
#[test]
fn first_command_respects_range_despite_slew_ordering() {
    let mut g = gate(vec![Constraint::range("range", -0.8, 0.8).unwrap(), Constraint::slew_rate("slew", 50.0).unwrap()], None);
    assert_eq!(evaluate(&mut g, 1.0, 0, 50_000_000).applied, 0.8);
}
#[test]
fn no_lease_is_rejected_with_safe_output() {
    let mut g = SafetyGate::new(LeaseTable::new(), vec![], 0.0).unwrap();
    let d = evaluate(&mut g, 1.0, 0, 0);
    assert_eq!(d.outcome, Outcome::Rejected(R::UnknownBinding)); assert_eq!(d.applied, 0.0);
}
#[test]
fn processor_does_not_authorize_using_old_source_time() {
    let mut g = gate(vec![], None);
    let ctx = TickContext { now: t(10_000_000_000), sequence: 0 };
    let d = g.process(&ctx, Some(request(0.5, 0, 0))).unwrap().remove(0);
    assert_eq!(d.outcome, Outcome::Rejected(R::LeaseExpired));
    assert_eq!(d.at, ctx.now); assert_eq!(d.request.unwrap().meta.source_at, t(0));
}
#[test]
fn out_of_envelope_does_not_consume_sequence_or_feed_watchdog() {
    let mut g = gate(vec![], Some(CommandEnvelope::new(-1.0, 1.0).unwrap()));
    assert_eq!(evaluate(&mut g, 2.0, 0, 0).outcome, Outcome::Rejected(R::OutOfEnvelope));
    assert_eq!(g.leases().last_accepted_at(&Identity::new("controller"), &Capability::new("thrust")), None);
    assert_eq!(evaluate(&mut g, 0.5, 0, 1).outcome, Outcome::Accepted);
}
#[test]
fn decisions_replay_identically_including_idle_expiry() {
    let inputs = vec![(t(0), Some(request(0.5, 0, 0))), (t(50_000_000), None), (t(100_000_000), None)];
    let mut a = gate(vec![], None); let mut b = gate(vec![], None);
    let out = run_lockstep(&ManualClock::new(t(0)), &mut a, inputs.clone()).unwrap();
    assert_eq!(out, run_lockstep(&ManualClock::new(t(0)), &mut b, inputs).unwrap());
    assert_eq!(out[2].outcome, Outcome::Rejected(R::DeadlineExpired));
}
#[test]
fn invalid_numeric_configuration_cannot_create_usable_gate() {
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
    assert!(Constraint::slew_rate("hold", 0.0).is_ok());
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.1, 1.1] {
        assert!(matches!(SafetyGate::new(LeaseTable::new(), vec![Constraint::range("range", -1.0, 1.0).unwrap()], bad), Err(SafetyError::InvalidSafeOutput)));
    }
    assert!(SafetyGate::new(LeaseTable::new(), vec![Constraint::range("left", -2.0, -1.0).unwrap(), Constraint::range("right", 1.0, 2.0).unwrap()], 0.0).is_err());
}
#[test]
fn non_finite_commands_fall_back_and_recovery_slews_from_safe() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut g = gate(vec![Constraint::range("range", -1.0, 1.0).unwrap(), Constraint::slew_rate("slew", 2.0).unwrap()], None);
        evaluate(&mut g, 0.8, 0, 0);
        let d = evaluate(&mut g, bad, 1, 100_000_000);
        assert_eq!(d.outcome, Outcome::Rejected(R::NonFiniteCommand)); assert_eq!(d.applied, 0.0);
        assert!((evaluate(&mut g, 1.0, 1, 200_000_000).applied - 0.2).abs() < 1e-12);
    }
}
#[test]
fn constraint_order_and_extreme_finite_values_preserve_all_hard_bounds() {
    for reverse in [false, true] {
        let mut constraints = vec![Constraint::range("wide", -1.0, 1.0).unwrap(), Constraint::slew_rate("slew", 2.0).unwrap(), Constraint::range("narrow", -0.5, 0.5).unwrap()];
        if reverse { constraints.reverse(); }
        let mut g = gate(constraints, None);
        for (i, value) in [f64::MAX, -f64::MAX, 0.25, -0.25, 0.5].into_iter().enumerate() {
            let d = evaluate(&mut g, value, i as u64, i as i128 * 100_000_000);
            assert!(d.applied.is_finite() && (-0.5..=0.5).contains(&d.applied)); assert!(!d.is_rejected());
        }
    }
}
#[test]
fn overflowing_slew_arithmetic_does_not_commit_validity_state() {
    for (first, elapsed) in [(0.0, 3_000_000_000), (f64::MAX, 500_000_000), (-f64::MAX, 500_000_000)] {
        let mut g = gate(vec![Constraint::range("range", -f64::MAX, f64::MAX).unwrap(), Constraint::slew_rate("slew", f64::MAX).unwrap()], None);
        assert_eq!(evaluate(&mut g, first, 0, 0).applied, first);
        let d = evaluate(&mut g, 0.0, 1, elapsed);
        assert_eq!(d.outcome, Outcome::Rejected(R::InvalidOutput)); assert_eq!(d.applied, 0.0);
        assert_eq!(g.leases().last_accepted_at(&Identity::new("controller"), &Capability::new("thrust")), Some(t(0)));
    }
}
#[test]
fn trusted_binding_table_is_bounded_and_retains_revoked_generation() {
    let mut table = LeaseTable::new();
    for i in 0..32 {
        table.grant(AuthorityLease::new(Identity::new(format!("h{i}")), Capability::new("c"), config(), None)).unwrap();
    }
    assert_eq!(table.grant(AuthorityLease::new(Identity::new("overflow"), Capability::new("c"), config(), None)), Err(neuradix_safety::SessionError::CapacityExceeded));
    table.revoke(&Identity::new("h0"), &Capability::new("c"));
    assert_eq!(table.binding_count(), 32);
    assert_eq!(table.grant(AuthorityLease::new(Identity::new("h0"), Capability::new("c"), config(), None)), Err(neuradix_safety::SessionError::ReusedGeneration));
}
