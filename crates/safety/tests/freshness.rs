//! Identical validity scenarios through the real host and embedded gates.
use neuradix_embedded_core::{self as mcu, CommandGate, Limits};
use neuradix_safety::{
    AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Constraint, Generation,
    Identity, LeaseTable, Outcome, RejectReason as R, SafetyGate, SessionConfig, SessionError,
    SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};

fn t(n: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, n)
}
fn config(generation: u128, age: i128, watchdog: i128) -> SessionConfig {
    SessionConfig::new(
        Generation::new(generation).unwrap(),
        t(0),
        t(1000),
        CommandPolicy::new(
            SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
            Duration::from_nanos(age),
            Duration::from_nanos(10),
            Duration::from_nanos(watchdog),
        )
        .unwrap(),
    )
    .unwrap()
}
fn meta(sequence: u64, source: i128, deadline: i128) -> CommandMeta {
    CommandMeta {
        generation: Generation::new(7).unwrap(),
        sequence,
        source_at: t(source),
        deadline: t(deadline),
        timeline: 1,
    }
}
struct Pair {
    host: SafetyGate,
    embedded: CommandGate,
}
impl Pair {
    fn new(config: SessionConfig) -> Self {
        let mut table = LeaseTable::new();
        table
            .grant(AuthorityLease::new(
                Identity::new("controller"),
                Capability::new("thrust"),
                config,
                None,
            ))
            .unwrap();
        Self {
            host: SafetyGate::new(
                table,
                vec![Constraint::range("range", -1.0, 1.0).unwrap()],
                0.0,
            )
            .unwrap(),
            embedded: CommandGate::new(
                Limits::new(-1.0, 1.0, 2.0).unwrap(),
                mcu::AuthorityLease::new(1, 2, config),
                0.0,
            )
            .unwrap(),
        }
    }
    fn step(
        &mut self,
        input: Option<(CommandMeta, f32, bool)>,
        now: Timestamp,
        expected: Option<R>,
    ) {
        let host = input.map(|(meta, value, known)| {
            CommandRequest::new(
                Identity::new(if known { "controller" } else { "stranger" }),
                Capability::new("thrust"),
                value as f64,
                meta,
            )
        });
        let embedded = input.map(|(meta, value, known)| mcu::Command {
            holder: if known { 1 } else { 9 },
            capability: 2,
            value,
            meta,
        });
        let h = self.host.evaluate(host, now);
        let e = self.embedded.evaluate(embedded, now);
        let hr = match h.outcome {
            Outcome::Rejected(reason) => Some(reason),
            _ => None,
        };
        let er = match e.outcome {
            mcu::Outcome::SafeState(reason) => Some(reason),
            _ => None,
        };
        assert_eq!(hr, expected, "host at {now}");
        assert_eq!(er, expected, "embedded at {now}");
        assert_eq!(h.applied, e.applied as f64);
        assert_eq!(h.at, now);
        assert_eq!(e.at, now);
        assert!(h.applied.is_finite() && (-1.0..=1.0).contains(&h.applied));
        if expected.is_some() {
            assert_eq!(h.applied, 0.0);
        }
    }
    fn send(&mut self, meta: CommandMeta, now: i128, expected: Option<R>) {
        self.step(Some((meta, 0.5, true)), t(now), expected);
    }
    fn replace(&mut self, config: SessionConfig) -> Result<(), SessionError> {
        let h = self.host.leases_mut().grant(AuthorityLease::new(
            Identity::new("controller"),
            Capability::new("thrust"),
            config,
            None,
        ));
        let e = self
            .embedded
            .replace_lease(mcu::AuthorityLease::new(1, 2, config));
        assert_eq!(h, e);
        h
    }
    fn renew(&mut self, expires: i128, now: i128) -> Result<(), SessionError> {
        let h = self.host.renew_lease(
            &Identity::new("controller"),
            &Capability::new("thrust"),
            t(expires),
            t(now),
        );
        let e = self.embedded.renew_lease(t(expires), t(now));
        assert_eq!(h, e);
        h
    }
    fn accepted_at(&self, expected: Option<Timestamp>) {
        assert_eq!(
            self.host
                .leases()
                .last_accepted_at(&Identity::new("controller"), &Capability::new("thrust")),
            expected
        );
        assert_eq!(self.embedded.last_accepted_at(), expected);
    }
}

#[test]
fn exact_age_skew_deadline_and_lease_boundaries_match() {
    for (input, now, reason) in [
        (meta(0, 100, 400), 200, None),
        (meta(0, 100, 400), 201, Some(R::StaleCommand)),
        (meta(0, 110, 300), 100, None),
        (meta(0, 111, 300), 100, Some(R::FutureCommand)),
        (meta(0, 100, 200), 199, None),
        (meta(0, 100, 200), 200, Some(R::DeadlineExpired)),
        (meta(0, 990, 1100), 999, None),
        (meta(0, 990, 1100), 1000, Some(R::LeaseExpired)),
        (meta(0, 0, 100), 0, None),
        (meta(0, 0, 100), -1, Some(R::LeaseNotYetValid)),
        (meta(0, 100, 100), 100, Some(R::InvalidDeadline)),
        (meta(0, 100, 99), 100, Some(R::InvalidDeadline)),
    ] {
        Pair::new(config(7, 100, 50)).send(input, now, reason);
    }
}

#[test]
fn valid_progression_gaps_duplicates_order_and_exhaustion_match() {
    let mut p = Pair::new(config(7, 100, 50));
    p.send(meta(10, 100, 300), 100, None);
    p.send(meta(10, 110, 300), 110, Some(R::Duplicate));
    p.send(meta(9, 120, 300), 120, Some(R::OutOfOrder));
    p.send(meta(20, 130, 300), 130, None); // gaps allowed
    p.send(meta(u64::MAX, 140, 300), 140, None);
    p.step(None, t(141), None); // MAX does not invalidate its own held command
    p.send(meta(0, 150, 300), 150, Some(R::SequenceExhausted));
    p.send(meta(u64::MAX, 160, 300), 160, Some(R::SequenceExhausted));
    p.accepted_at(Some(t(140)));
}

#[test]
fn rejected_packets_do_not_feed_watchdog_or_consume_sequence() {
    let mut p = Pair::new(config(7, 1000, 50));
    p.send(meta(1, 100, 900), 100, None);
    for now in [110, 120, 130, 140, 150] {
        p.send(meta(1, now, 900), now, Some(R::Duplicate));
        p.accepted_at(Some(t(100)));
    }
    p.step(None, t(151), Some(R::WatchdogExpired));
    p.step(
        Some((meta(2, 152, 900), f32::NAN, true)),
        t(152),
        Some(R::NonFiniteCommand),
    );
    p.accepted_at(Some(t(100)));
    p.send(meta(2, 153, 900), 153, None); // failed numeric check did not consume 2
    p.accepted_at(Some(t(153)));
}

#[test]
fn all_temporally_rejected_packets_leave_liveness_unchanged() {
    for expected in [
        R::StaleCommand,
        R::FutureCommand,
        R::DeadlineExpired,
        R::GenerationMismatch,
        R::UnsupportedClockRelationship,
        R::UnknownBinding,
    ] {
        let mut p = Pair::new(config(7, 100, 50));
        p.send(meta(0, 100, 900), 100, None);
        let mut m = meta(u64::MAX, 140, 900);
        match expected {
            R::StaleCommand => m.source_at = t(0),
            R::FutureCommand => m.source_at = t(200),
            R::DeadlineExpired => {
                m.source_at = t(100);
                m.deadline = t(140);
            }
            R::GenerationMismatch => m.generation = Generation::new(8).unwrap(),
            R::UnsupportedClockRelationship => m.timeline = 2,
            _ => {}
        }
        p.step(
            Some((m, 0.5, expected != R::UnknownBinding)),
            t(140),
            Some(expected),
        );
        p.accepted_at(Some(t(100)));
        p.step(None, t(151), Some(R::WatchdogExpired));
        p.send(meta(1, 152, 900), 152, None); // rejected MAX never poisons sequence state
    }
}

#[test]
fn expiry_is_enforced_without_new_input() {
    for (cfg, source, deadline, edge, reason) in [
        (config(7, 1000, 50), 100, 900, 151, R::WatchdogExpired),
        (config(7, 100, 1000), 100, 900, 201, R::StaleCommand),
        (config(7, 1000, 1000), 100, 150, 150, R::DeadlineExpired),
        (config(7, 1000, 1000), 990, 1100, 1000, R::LeaseExpired),
    ] {
        let mut p = Pair::new(cfg);
        p.send(meta(0, source, deadline), source, None);
        p.step(None, t(edge - 1), None);
        p.step(None, t(edge), Some(reason));
    }
}

#[test]
fn renewal_preserves_sequence_and_deadline_and_cannot_revive_expired_lease() {
    let mut p = Pair::new(config(7, 1000, 1000));
    p.send(meta(4, 100, 180), 100, None);
    p.renew(2000, 140).unwrap();
    p.send(meta(4, 150, 900), 150, Some(R::Duplicate));
    p.step(None, t(180), Some(R::DeadlineExpired));
    assert_eq!(
        p.replace(config(7, 1000, 1000)),
        Err(SessionError::ReusedGeneration)
    );
    assert_eq!(
        p.replace(config(6, 1000, 1000)),
        Err(SessionError::ReusedGeneration)
    );
    let mut expired = Pair::new(config(7, 100, 50));
    assert_eq!(expired.renew(2000, 1000), Err(SessionError::InactiveLease));
}

#[test]
fn renewal_does_not_extend_source_age_or_watchdog() {
    for (age, watchdog, edge, reason) in [
        (100, 1000, 201, R::StaleCommand),
        (1000, 50, 151, R::WatchdogExpired),
    ] {
        let mut p = Pair::new(config(7, age, watchdog));
        p.send(meta(0, 100, 900), 100, None);
        p.renew(2000, 120).unwrap();
        p.step(None, t(edge), Some(reason));
    }
}

#[test]
fn replacement_revocation_and_restart_reject_old_generations() {
    let mut p = Pair::new(config(7, 100, 50));
    p.send(meta(u64::MAX, 100, 500), 100, None);
    p.replace(config(8, 100, 50)).unwrap();
    p.step(None, t(101), Some(R::GenerationMismatch));
    p.send(meta(0, 102, 500), 102, Some(R::GenerationMismatch));
    let mut fresh = meta(0, 103, 500);
    fresh.generation = Generation::new(8).unwrap();
    p.send(fresh, 103, None);
    p.host
        .leases_mut()
        .revoke(&Identity::new("controller"), &Capability::new("thrust"));
    p.embedded.revoke_lease();
    p.step(None, t(104), Some(R::LeaseRevoked));
    assert_eq!(p.renew(2000, 105), Err(SessionError::RevokedSession));
    assert_eq!(
        p.replace(config(8, 100, 50)),
        Err(SessionError::ReusedGeneration)
    );

    // Simulate durable startup having reserved a newer generation BEFORE ingress.
    let mut restarted = Pair::new(config(9, 100, 50));
    restarted.send(fresh, 104, Some(R::GenerationMismatch));
    fresh.generation = Generation::new(9).unwrap();
    fresh.source_at = t(105);
    restarted.send(fresh, 105, None);
}

#[test]
fn source_clock_relationships_are_explicit_and_rejection_is_not_clock_reset() {
    for field in 0..3 {
        let mut p = Pair::new(config(7, 100, 50));
        let mut m = meta(0, 100, 300);
        match field {
            0 => m.timeline = 99,
            1 => m.source_at = Timestamp::new(ClockDomain::Utc, 100),
            _ => m.deadline = Timestamp::new(ClockDomain::Sensor, 300),
        }
        p.send(m, 100, Some(R::UnsupportedClockRelationship));
        p.send(meta(0, 101, 300), 101, None);
    }
}

#[test]
fn gate_clock_faults_latch_across_session_changes() {
    for (bad, reason) in [
        (t(99), R::EvaluationTimeRegression),
        (
            Timestamp::new(ClockDomain::Simulation, 100),
            R::EvaluationClockMismatch,
        ),
    ] {
        let mut p = Pair::new(config(7, 100, 50));
        p.send(meta(0, 100, 300), 100, None);
        p.step(None, bad, Some(reason));
        p.replace(config(8, 100, 50)).unwrap();
        let mut m = meta(0, 101, 300);
        m.generation = Generation::new(8).unwrap();
        p.send(m, 101, Some(reason));
    }
}

#[test]
fn extreme_timestamp_arithmetic_is_checked_on_both_paths() {
    let wide = SessionConfig::new(
        Generation::new(7).unwrap(),
        t(i128::MIN),
        t(i128::MAX),
        config(7, 100, 50).policy(),
    )
    .unwrap();
    let mut p = Pair::new(wide);
    p.step(
        Some((meta(0, i128::MIN, i128::MIN + 100), 0.5, true)),
        t(i128::MIN),
        None,
    );
    p.step(None, t(i128::MAX - 1), Some(R::EvaluationTimeOverflow));
    p.step(None, t(i128::MAX - 1), Some(R::EvaluationTimeOverflow));
    let mut p = Pair::new(wide);
    p.step(
        Some((meta(0, i128::MIN, i128::MAX), 0.5, true)),
        t(i128::MAX - 1),
        Some(R::CommandTimeOverflow),
    );
    let mut p = Pair::new(wide);
    p.step(
        Some((meta(0, 0, i128::MAX), 0.5, true)),
        t(i128::MIN),
        Some(R::FutureCommand),
    );
}

#[test]
fn unknown_payload_identities_do_not_allocate_tracking_slots() {
    let mut p = Pair::new(config(7, 100, 50));
    for i in 0..1000 {
        let input = CommandRequest::new(
            Identity::new(format!("unknown-{i}")),
            Capability::new("thrust"),
            0.5,
            meta(0, 100, 300),
        );
        assert_eq!(
            p.host.evaluate(Some(input), t(100)).outcome,
            Outcome::Rejected(R::UnknownBinding)
        );
    }
    assert_eq!(p.host.leases().binding_count(), 1);
    p.send(meta(0, 100, 300), 100, None);
}

#[test]
fn rejected_output_stays_safe_until_new_valid_input() {
    let mut p = Pair::new(config(7, 100, 50));
    p.send(meta(0, 100, 300), 100, None);
    p.send(meta(0, 101, 300), 101, Some(R::Duplicate));
    p.step(None, t(102), Some(R::Duplicate));
    p.send(meta(1, 103, 300), 103, None);
    p.step(None, t(104), None);
}

#[test]
fn capability_binding_is_checked_before_validity_or_numeric_input() {
    let mut p = Pair::new(config(7, 100, 50));
    let m = meta(0, 100, 300);
    let h = CommandRequest::new(
        Identity::new("controller"),
        Capability::new("different-actuator"),
        f64::NAN,
        m,
    );
    let e = mcu::Command {
        holder: 1,
        capability: 99,
        value: f32::NAN,
        meta: m,
    };
    let h = p.host.evaluate(Some(h), t(100));
    let e = p.embedded.evaluate(Some(e), t(100));
    assert_eq!(h.outcome, Outcome::Rejected(R::UnknownBinding));
    assert_eq!(e.outcome, mcu::Outcome::SafeState(R::UnknownBinding));
    assert_eq!((h.applied, e.applied), (0.0, 0.0));
    p.accepted_at(None);
    p.send(m, 101, None);
}

#[test]
fn initial_idle_and_unsupported_evaluation_domain_do_not_grant_authority() {
    let mut p = Pair::new(config(7, 100, 50));
    p.step(None, t(0), Some(R::NoCommand));
    p.send(meta(0, 1, 300), 1, None);

    let mut p = Pair::new(config(7, 100, 50));
    p.step(
        Some((meta(0, 100, 300), 0.5, true)),
        Timestamp::new(ClockDomain::Utc, 100),
        Some(R::UnsupportedClockRelationship),
    );
    p.accepted_at(None);
    // A payload rejection cannot erase the observed runtime clock domain.
    p.send(meta(0, 101, 300), 101, Some(R::EvaluationClockMismatch));
    p.step(None, t(102), Some(R::EvaluationClockMismatch));
}

#[test]
fn delayed_renewal_cannot_revive_observed_expiry_on_either_gate() {
    let mut p = Pair::new(config(7, 1000, 1000));
    p.send(meta(0, 990, 2000), 990, None);
    p.step(None, t(1000), Some(R::LeaseExpired));
    assert_eq!(p.renew(2000, 999), Err(SessionError::InvalidEvaluationTime));
    assert_eq!(p.renew(2000, 1000), Err(SessionError::InactiveLease));
    p.send(meta(1, 1001, 2000), 1001, Some(R::LeaseExpired));
    p.accepted_at(Some(t(990)));
}

#[test]
fn renewal_uses_all_runtime_ticks_and_cannot_clear_clock_faults() {
    let mut p = Pair::new(config(7, 100, 50));
    p.step(None, t(1000), Some(R::NoCommand));
    assert_eq!(p.renew(2000, 999), Err(SessionError::InvalidEvaluationTime));
    let mut p = Pair::new(config(7, 100, 50));
    p.send(meta(0, 100, 300), 100, None);
    assert_eq!(p.renew(2000, 99), Err(SessionError::InvalidEvaluationTime));
    p.send(meta(1, 101, 300), 101, None); // rejected renewal does not fault the gate
    p.step(None, t(100), Some(R::EvaluationTimeRegression));
    assert_eq!(p.renew(2000, 102), Err(SessionError::InvalidEvaluationTime));
}
