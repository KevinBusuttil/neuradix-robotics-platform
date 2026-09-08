//! A04.1 numeric and local-state behavior with validated A04.2 metadata.
use neuradix_embedded_core::{
    AuthorityLease, Command, CommandGate, CommandMeta, CommandPolicy, EmbeddedComponent,
    Generation, HealthState, Limits, NodeId, Outcome, PropulsionNode, SafeReason as R,
    SessionConfig, SharedTimeline, Watchdog,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
fn t(n: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, n)
}
fn lease() -> AuthorityLease {
    AuthorityLease::new(
        1,
        2,
        SessionConfig::new(
            Generation::new(1).unwrap(),
            t(0),
            t(10_000_000_000),
            CommandPolicy::new(
                SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_millis(100),
            )
            .unwrap(),
        )
        .unwrap(),
    )
}
fn command(value: f32, seq: u64, now: i128) -> Command {
    Command {
        holder: 1,
        capability: 2,
        value,
        meta: CommandMeta {
            generation: Generation::new(1).unwrap(),
            sequence: seq,
            source_at: t(now),
            deadline: t(now + 1_000_000_000),
            timeline: 1,
        },
    }
}
fn gate() -> CommandGate {
    CommandGate::new(Limits::new(-1.0, 1.0, 0.5).unwrap(), lease(), 0.0).unwrap()
}
#[test]
fn limits_reject_invalid_envelopes_and_rates() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(Limits::new(bad, 1.0, 0.5).is_none());
        assert!(Limits::new(-1.0, bad, 0.5).is_none());
        assert!(Limits::new(-1.0, 1.0, bad).is_none());
    }
    assert!(Limits::new(1.0, -1.0, 0.5).is_none());
    assert!(Limits::new(-1.0, 1.0, -0.1).is_none());
    let l = Limits::new(1.0, 1.0, 0.0).unwrap();
    assert_eq!((l.min(), l.max(), l.max_step()), (1.0, 1.0, 0.0));
}
#[test]
fn invalid_safe_outputs_cannot_create_a_gate() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.1, 1.1] {
        assert!(matches!(
            CommandGate::new(Limits::new(-1.0, 1.0, 0.5).unwrap(), lease(), value),
            Err(neuradix_embedded_core::GateConfigError::InvalidSafeOutput)
        ));
    }
}
#[test]
fn first_command_is_range_limited_only_then_per_step_slew_applies() {
    let mut g = gate();
    assert_eq!(g.evaluate(Some(command(0.8, 0, 0)), t(0)).applied, 0.8);
    let d = g.evaluate(Some(command(-5.0, 1, 0)), t(0));
    assert!(d.range_clamped && d.slew_limited);
    assert!((d.applied - 0.3).abs() < 1e-6);
}
#[test]
fn equal_evaluation_times_keep_per_step_slew_semantics() {
    let mut g = gate();
    g.evaluate(Some(command(0.0, 0, 0)), t(0));
    assert_eq!(g.evaluate(Some(command(1.0, 1, 0)), t(0)).applied, 0.5);
    assert_eq!(g.evaluate(Some(command(1.0, 2, 0)), t(0)).applied, 1.0);
}
#[test]
fn non_finite_commands_use_configured_safe_output_without_watchdog_feed() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut g = CommandGate::new(Limits::new(-1.0, 1.0, 0.5).unwrap(), lease(), -0.25).unwrap();
        let d = g.evaluate(Some(command(bad, 0, 0)), t(0));
        assert_eq!(d.outcome, Outcome::SafeState(R::NonFiniteCommand));
        assert_eq!(d.applied, -0.25);
        assert_eq!(g.last_accepted_at(), None);
    }
}
#[test]
fn embedded_arithmetic_overflow_falls_back_without_committing_sequence() {
    let mut g = CommandGate::new(
        Limits::new(-f32::MAX, f32::MAX, f32::MAX).unwrap(),
        lease(),
        0.0,
    )
    .unwrap();
    assert_eq!(
        g.evaluate(Some(command(-f32::MAX, 0, 0)), t(0)).applied,
        -f32::MAX
    );
    let d = g.evaluate(Some(command(f32::MAX, 1, 1)), t(1));
    assert_eq!(d.outcome, Outcome::SafeState(R::InvalidOutput));
    assert_eq!(d.applied, 0.0);
    assert_eq!(g.last_accepted_at(), Some(t(0)));
    assert_eq!(
        g.evaluate(Some(command(0.5, 1, 2)), t(2)).outcome,
        Outcome::Accepted
    );
}
#[test]
fn propulsion_node_reports_health_and_expires_on_idle_ticks() {
    let mut node = PropulsionNode::new(NodeId::new("thruster"), gate());
    assert_eq!(node.health(), HealthState::Unknown);
    assert_eq!(node.tick(t(0), Some(command(0.8, 0, 0))), 0.8);
    assert_eq!(node.health(), HealthState::Healthy);
    assert_eq!(node.tick(t(100_000_000), None), 0.8);
    assert_eq!(node.tick(t(100_000_001), None), 0.0);
    assert_eq!(node.health(), HealthState::Degraded);
    assert_eq!(
        node.last_decision().unwrap().outcome,
        Outcome::SafeState(R::WatchdogExpired)
    );
    assert_eq!(
        node.tick(t(110_000_000), Some(command(1.0, 1, 110_000_000))),
        0.5
    );
}
#[test]
fn standalone_watchdog_starts_expired_and_has_inclusive_timeout() {
    let mut wd = Watchdog::new(Duration::from_millis(100));
    assert!(wd.is_expired(t(0)));
    wd.feed(t(0));
    assert!(!wd.is_expired(t(100_000_000)));
    assert!(wd.is_expired(t(100_000_001)));
}
