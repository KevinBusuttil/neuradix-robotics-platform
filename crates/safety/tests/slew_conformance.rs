//! A04.3 conformance through both actual gates, with independent per-tick bounds.
use neuradix_embedded_core as mcu;
use neuradix_safety::{AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest,
    Constraint, Generation, Identity, LeaseTable, Outcome, RejectReason as R, SafetyGate,
    SessionConfig, SharedTimeline};
use neuradix_time::{ClockDomain, Duration, Timestamp};

fn t(nanos: i128) -> Timestamp { Timestamp::new(ClockDomain::Monotonic, nanos) }
fn ms(value: i128) -> Timestamp { t(value * 1_000_000) }
fn config(generation: u128) -> SessionConfig {
    SessionConfig::new(Generation::new(generation).unwrap(), t(i128::MIN), t(i128::MAX),
        CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
            Duration::from_secs(10), Duration::ZERO, Duration::from_secs(10)).unwrap()).unwrap()
}

struct Rig {
    host: SafetyGate,
    embedded: mcu::CommandGate,
    rate: f64,
    min: f64,
    max: f64,
    safe: f64,
    generation: Generation,
    sequence: u64,
    last: Option<(Timestamp, f64, f64)>,
    steps: usize,
    scale: f64,
}
impl Rig {
    fn new(rate: f32, min: f32, max: f32, safe: f32, slew_first: bool) -> Self {
        let mut table = LeaseTable::new();
        table.grant(AuthorityLease::new(Identity::new("controller"), Capability::new("thrust"), config(1), None)).unwrap();
        let mut constraints = vec![Constraint::range("range", min as f64, max as f64).unwrap(),
            Constraint::slew_rate("slew", rate as f64).unwrap()];
        if slew_first { constraints.reverse(); }
        Self {
            host: SafetyGate::new(table, constraints, safe as f64).unwrap(),
            embedded: mcu::CommandGate::new(mcu::Limits::with_slew_rate(min, max, rate).unwrap(),
                mcu::AuthorityLease::new(1, 2, config(1)), safe).unwrap(),
            rate: rate as f64, min: min as f64, max: max as f64, safe: safe as f64,
            generation: Generation::new(1).unwrap(), sequence: 0, last: None, steps: 0, scale: 1.0,
        }
    }
    fn eval(&mut self, input: Option<f32>, now: Timestamp, expected: Option<R>) -> (f64, f64) {
        self.eval_with_source(input, now, now, expected)
    }
    fn eval_with_source(&mut self, input: Option<f32>, now: Timestamp, source: Timestamp, expected: Option<R>) -> (f64, f64) {
        let meta = CommandMeta { generation: self.generation, sequence: self.sequence,
            source_at: source, deadline: t(source.as_nanos().saturating_add(10_000_000_000)), timeline: 1 };
        let host_input = input.map(|value| CommandRequest::new(Identity::new("controller"),
            Capability::new("thrust"), value as f64, meta));
        let mcu_input = input.map(|value| mcu::Command { holder: 1, capability: 2, value, meta });
        let h = self.host.evaluate(host_input, now);
        let e = self.embedded.evaluate(mcu_input, now);
        assert_eq!(match h.outcome { Outcome::Rejected(r) => Some(r), _ => None }, expected);
        assert_eq!(match e.outcome { mcu::Outcome::SafeState(r) => Some(r), _ => None }, expected);
        assert_eq!(h.at, now); assert_eq!(e.at, now);
        let values = (h.applied, e.applied as f64);
        self.scale = self.scale.max(values.0.abs()).max(values.1.abs());
        self.steps += 1;
        // Each inward binary32 rounding loses less than one ULP. Use an explicit
        // accumulated bound over the largest visited magnitude (minimum scale 1).
        let tolerance = 2.0 * self.steps as f64 * f32::EPSILON as f64 * self.scale;
        assert!((values.0 - values.1).abs() <= tolerance, "host/MCU {:?} at {now}, tolerance {tolerance}", values);
        for value in [values.0, values.1] {
            assert!(value.is_finite() && value >= self.min && value <= self.max);
        }
        if expected.is_some() {
            assert_eq!(values, (self.safe, self.safe));
        } else if let Some((prev_at, hprev, eprev)) = self.last {
            let dt = now.duration_since(prev_at).unwrap().as_nanos() as f64 / 1_000_000_000.0;
            let budget = self.rate * dt;
            for (value, prev) in [(values.0, hprev), (values.1, eprev)] {
                if input.is_none() || budget == 0.0 {
                    assert_eq!(value, prev, "idle/equal-time/zero-rate must hold exactly");
                } else {
                    let rounding = 8.0 * f64::EPSILON * self.scale.max(budget);
                    assert!((value - prev).abs() <= budget + rounding,
                        "rate exceeded: {prev} -> {value}, budget {budget}");
                }
            }
        }
        if input.is_some() && expected.is_none() { self.sequence += 1; }
        self.last = Some((now, values.0, values.1));
        values
    }
    fn replace(&mut self, generation: u128) {
        self.host.leases_mut().grant(AuthorityLease::new(Identity::new("controller"),
            Capability::new("thrust"), config(generation), None)).unwrap();
        self.embedded.replace_lease(mcu::AuthorityLease::new(1, 2, config(generation))).unwrap();
        self.generation = Generation::new(generation).unwrap(); self.sequence = 0;
    }
    fn accepted_at(&self) -> Option<Timestamp> {
        let at = self.host.leases().last_accepted_at(&Identity::new("controller"), &Capability::new("thrust"));
        assert_eq!(at, self.embedded.last_accepted_at()); at
    }
}

#[test]
fn constant_target_has_equal_progress_over_different_tick_partitions() {
    for periods in [vec![1000], vec![500, 500], vec![20; 50], vec![17, 23, 110, 350, 200, 300]] {
        let mut r = Rig::new(2.0, -10.0, 10.0, 0.0, false);
        r.eval(Some(0.0), ms(0), None);
        let mut elapsed = 0;
        for dt in periods { elapsed += dt; r.eval(Some(10.0), ms(elapsed), None); }
        let (_, host, embedded) = r.last.unwrap();
        assert!((host - 2.0).abs() < 1e-12);
        assert!((embedded - 2.0).abs() < 1e-5);
    }
}

#[test]
fn jitter_zero_periods_and_direction_reversals_obey_rate() {
    for rate in [0.25, 1.0, 25.0] {
        let mut r = Rig::new(rate, -10.0, 10.0, 0.0, false);
        r.eval(Some(0.0), ms(0), None);
        let mut elapsed = 0;
        for (dt, target) in [(17, 10.0), (5, 10.0), (63, -10.0), (0, 10.0), (125, -10.0), (9, 2.0), (200, 0.0)] {
            elapsed += dt; r.eval(Some(target), ms(elapsed), None);
        }
    }
}

#[test]
fn idle_ticks_hold_and_do_not_bank_a_slew_allowance() {
    let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.0), ms(0), None);
    let before = r.eval(Some(1.0), ms(100), None);
    assert_eq!(r.eval(None, ms(150), None), before);
    let (host, embedded) = r.eval(Some(1.0), ms(200), None);
    assert!((host - 0.15).abs() < 1e-12);
    assert!((embedded - 0.15).abs() < 1e-6);
    assert_eq!(r.accepted_at(), Some(ms(200)));
}

#[test]
fn first_evaluation_initializes_but_an_idle_first_tick_establishes_safe_reference() {
    let mut r = Rig::new(0.0, -1.0, 1.0, -0.25, false);
    assert_eq!(r.eval(Some(5.0), ms(0), None), (1.0, 1.0)); // first tick: hard range only
    assert_eq!(r.eval(Some(-1.0), ms(1000), None), (1.0, 1.0)); // zero rate holds
    let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
    r.eval(None, ms(0), Some(R::NoCommand));
    assert_eq!(r.eval(Some(1.0), ms(0), None), (0.0, 0.0));
    let (host, embedded) = r.eval(Some(1.0), ms(100), None);
    assert!((host - 0.1).abs() < 1e-12); assert!((embedded - 0.1).abs() < 1e-6);
}

#[test]
fn repeated_same_timestamp_commands_cannot_move_the_actuator() {
    let mut r = Rig::new(f32::MAX, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.0), ms(0), None);
    for _ in 0..100 { assert_eq!(r.eval(Some(1.0), ms(0), None), (0.0, 0.0)); }
}

#[test]
fn rejection_safes_immediately_and_recovery_uses_the_last_safe_tick() {
    let mut r = Rig::new(1.0, -1.0, 1.0, -0.25, false);
    r.eval(Some(0.75), ms(0), None);
    r.sequence = 0;
    r.eval(Some(1.0), ms(100), Some(R::Duplicate)); // immediate 1.0 drop bypasses slew
    assert_eq!(r.accepted_at(), Some(ms(0)));
    r.sequence = 1;
    r.eval(Some(1.0), ms(100), None); // same time: still safe
    r.eval(Some(f32::NAN), ms(150), Some(R::NonFiniteCommand));
    assert_eq!(r.accepted_at(), Some(ms(100)));
    let (host, embedded) = r.eval(Some(1.0), ms(200), None);
    assert!((host + 0.2).abs() < 1e-12); assert!((embedded + 0.2).abs() < 1e-6);
}

#[test]
fn renewal_replacement_and_revocation_do_not_reset_slew_reference() {
    let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.0), ms(0), None);
    r.eval(Some(1.0), ms(100), None);
    // Use a shorter lease so it can be renewed; replacing this initial trusted
    // session preserves the gate's last applied output and evaluation clock.
    let short = SessionConfig::new(Generation::new(2).unwrap(), ms(0), ms(1000), config(2).policy()).unwrap();
    r.host.leases_mut().grant(AuthorityLease::new(Identity::new("controller"), Capability::new("thrust"), short, None)).unwrap();
    r.embedded.replace_lease(mcu::AuthorityLease::new(1, 2, short)).unwrap();
    r.generation = Generation::new(2).unwrap(); r.sequence = 0;
    r.host.renew_lease(&Identity::new("controller"), &Capability::new("thrust"), ms(2000), ms(100)).unwrap();
    r.embedded.renew_lease(ms(2000), ms(100)).unwrap();
    let held = r.last.unwrap();
    assert_eq!(r.eval(Some(1.0), ms(100), None), (held.1, held.2));
    let (host, embedded) = r.eval(Some(1.0), ms(200), None);
    assert!((host - 0.2).abs() < 1e-12); assert!((embedded - 0.2).abs() < 1e-6);
    r.host.leases_mut().revoke(&Identity::new("controller"), &Capability::new("thrust"));
    r.embedded.revoke_lease();
    r.eval(None, ms(210), Some(R::LeaseRevoked));
    r.replace(3);
    let (host, embedded) = r.eval(Some(1.0), ms(220), None);
    assert!((host - 0.01).abs() < 1e-12); assert!((embedded - 0.01).abs() < 1e-6);
}

#[test]
fn range_order_and_direction_changes_preserve_hard_and_rate_bounds() {
    for reverse in [false, true] {
        let mut r = Rig::new(1.0, -0.5, 0.5, 0.0, reverse);
        r.eval(Some(5.0), ms(0), None);
        for (at, value) in [(250, -5.0), (500, -5.0), (750, 5.0), (1000, -5.0)] {
            r.eval(Some(value), ms(at), None);
        }
    }
}

#[test]
fn binary32_quantization_can_hold_without_accumulating_fractional_budget() {
    let tiny = f32::from_bits(1);
    let mut r = Rig::new(tiny, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.0), t(0), None);
    for at in 1..=10 {
        let (host, embedded) = r.eval(Some(tiny), t(at), None);
        assert!(host > 0.0); assert_eq!(embedded, 0.0);
    }
    let mut coarse = Rig::new(tiny, -1.0, 1.0, 0.0, false);
    coarse.eval(Some(0.0), t(0), None);
    assert_eq!(coarse.eval(Some(tiny), ms(1000), None), (tiny as f64, tiny as f64));
}

#[test]
fn extreme_finite_binary32_values_remain_finite_on_both_paths() {
    let mut r = Rig::new(f32::MAX, -f32::MAX, f32::MAX, 0.0, false);
    r.eval(Some(-f32::MAX), ms(0), None);
    assert_eq!(r.eval(Some(f32::MAX), ms(2000), None), (f32::MAX as f64, f32::MAX as f64));
    assert_eq!(r.eval(Some(0.0), ms(3000), None), (0.0, 0.0));
}

#[test]
fn clock_faults_still_latch_after_slew_and_session_replacement() {
    for (bad, reason) in [(ms(99), R::EvaluationTimeRegression),
        (Timestamp::new(ClockDomain::Utc, 100_000_000), R::EvaluationClockMismatch)] {
        let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
        r.eval(Some(0.5), ms(100), None);
        r.eval(None, bad, Some(reason)); r.replace(2);
        r.eval(Some(1.0), ms(101), Some(reason));
    }
    let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.5), t(i128::MIN), None);
    r.eval(None, t(i128::MAX - 1), Some(R::EvaluationTimeOverflow));
    r.replace(2);
    r.eval(None, t(i128::MAX - 1), Some(R::EvaluationTimeOverflow));
}

#[test]
fn source_time_never_supplies_slew_elapsed_time() {
    let mut r = Rig::new(1.0, -1.0, 1.0, 0.0, false);
    r.eval(Some(0.0), ms(0), None);
    let (host, embedded) = r.eval_with_source(Some(1.0), ms(100), ms(0), None);
    assert!((host - 0.1).abs() < 1e-12); assert!((embedded - 0.1).abs() < 1e-6);
    let (host, embedded) = r.eval_with_source(Some(1.0), ms(200), ms(200), None);
    assert!((host - 0.2).abs() < 1e-12); assert!((embedded - 0.2).abs() < 1e-6);
}
