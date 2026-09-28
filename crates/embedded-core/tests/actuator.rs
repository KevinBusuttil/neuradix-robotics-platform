//! WP-A08 embedded actuator permission boundary with an instrumented driver.
// Explicit drops document that adapter Drop performs no driver I/O.
#![allow(clippy::drop_non_drop)]
use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
    CommandPolicy, DriverError, DriverPermission, ExecutionMode, Generation, Limits, Outcome,
    PermissionError, PermissionStatus, SafeReason as R, SessionConfig, SessionError,
    SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use std::cell::RefCell;

/// Fixed-capacity call log so the driver itself never allocates.
struct Trace {
    calls: [f32; 64],
    len: usize,
    /// 1-based call indices that fail.
    fail: [usize; 4],
    error: DriverError,
}
impl Trace {
    fn new() -> RefCell<Self> {
        RefCell::new(Self {
            calls: [0.0; 64],
            len: 0,
            fail: [0; 4],
            error: DriverError::UnknownOutcome,
        })
    }
}
fn calls(trace: &RefCell<Trace>) -> Vec<f32> {
    let t = trace.borrow();
    t.calls[..t.len].to_vec()
}
fn fail_calls(trace: &RefCell<Trace>, indices: &[usize], error: DriverError) {
    let mut t = trace.borrow_mut();
    for (slot, index) in t.fail.iter_mut().zip(indices) {
        *slot = *index;
    }
    t.error = error;
}

struct Driver<'a> {
    trace: &'a RefCell<Trace>,
    endpoint: u64,
    mode: ExecutionMode,
}
impl ActuatorDriver for Driver<'_> {
    fn endpoint(&self) -> u64 {
        self.endpoint
    }
    fn mode(&self) -> ExecutionMode {
        self.mode
    }
    fn write(&mut self, value: f32) -> Result<(), DriverError> {
        let mut t = self.trace.borrow_mut();
        let index = t.len;
        t.calls[index] = value;
        t.len += 1;
        if t.fail.contains(&t.len) {
            Err(t.error)
        } else {
            Ok(())
        }
    }
}

const HOLDER: u64 = 1;
const CAPABILITY: u64 = 2;
const ENDPOINT: u64 = 7;

fn at(mode: ExecutionMode, ms: i128) -> Timestamp {
    Timestamp::new(mode.clock_domain(), ms * 1_000_000)
}
fn t(ms: i128) -> Timestamp {
    at(ExecutionMode::Live, ms)
}
fn binding(mode: ExecutionMode) -> ActuatorBinding {
    ActuatorBinding::new(HOLDER, CAPABILITY, ENDPOINT, mode)
}
fn limits(rate: f32) -> Limits {
    Limits::with_slew_rate(-1.0, 1.0, rate).unwrap()
}
fn lease_in(mode: ExecutionMode, g: u128, expires_ms: i128) -> AuthorityLease {
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, mode.clock_domain()).unwrap(),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    AuthorityLease::new(
        HOLDER,
        CAPABILITY,
        SessionConfig::new(
            Generation::new(g).unwrap(),
            at(mode, 0),
            at(mode, expires_ms),
            policy,
        )
        .unwrap(),
    )
}
fn permission(g: u128) -> DriverPermission {
    DriverPermission::new(
        binding(ExecutionMode::Live),
        lease_in(ExecutionMode::Live, g, 1000),
    )
    .unwrap()
}
fn cmd_in(mode: ExecutionMode, value: f32, g: u128, seq: u64, source_ms: i128) -> Command {
    Command {
        holder: HOLDER,
        capability: CAPABILITY,
        value,
        meta: CommandMeta {
            generation: Generation::new(g).unwrap(),
            sequence: seq,
            source_at: at(mode, source_ms),
            deadline: at(mode, source_ms + 400),
            timeline: 1,
        },
    }
}
fn cmd(value: f32, g: u128, seq: u64, source_ms: i128) -> Command {
    cmd_in(ExecutionMode::Live, value, g, seq, source_ms)
}
fn adapter(trace: &RefCell<Trace>, rate: f32) -> ActuatorAdapter<Driver<'_>> {
    ActuatorAdapter::new(
        binding(ExecutionMode::Live),
        limits(rate),
        0.0,
        Driver {
            trace,
            endpoint: ENDPOINT,
            mode: ExecutionMode::Live,
        },
    )
    .unwrap()
}
fn live(
    a: &mut ActuatorAdapter<Driver<'_>>,
    ms: i128,
    input: Option<Command>,
) -> neuradix_embedded_core::DispatchReport {
    a.port(ExecutionMode::Live, t(ms)).tick(input)
}
fn reason(report: &neuradix_embedded_core::DispatchReport) -> Option<R> {
    match report.decision?.outcome {
        Outcome::SafeState(r) => Some(r),
        _ => None,
    }
}

#[test]
fn construction_validates_driver_and_performs_no_io() {
    let trace = Trace::new();
    let driver = |endpoint, mode| Driver {
        trace: &trace,
        endpoint,
        mode,
    };
    let live = binding(ExecutionMode::Live);
    assert_eq!(
        ActuatorAdapter::new(live, limits(1.0), 0.0, driver(8, ExecutionMode::Live)).err(),
        Some(PermissionError::BindingMismatch)
    );
    assert_eq!(
        ActuatorAdapter::new(
            live,
            limits(1.0),
            0.0,
            driver(ENDPOINT, ExecutionMode::Simulation)
        )
        .err(),
        Some(PermissionError::ModeMismatch)
    );
    for bad in [f32::NAN, f32::INFINITY, 1.5, -1.5] {
        assert_eq!(
            ActuatorAdapter::new(
                live,
                limits(1.0),
                bad,
                driver(ENDPOINT, ExecutionMode::Live)
            )
            .err(),
            Some(PermissionError::InvalidOutputConfig)
        );
    }
    let a = adapter(&trace, 1.0);
    assert_eq!(a.status(), PermissionStatus::Missing);
    assert_eq!(a.generation(), None);
    assert!(calls(&trace).is_empty(), "construction must not write");
    drop(a);
    assert!(calls(&trace).is_empty(), "drop must not write");
}

#[test]
fn missing_permission_selects_safe_and_never_writes_request() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1.0);
    // A fully valid-looking command cannot authorize itself without a grant.
    let report = live(&mut a, 0, Some(cmd(0.9, 1, 0, 0)));
    assert_eq!(report.permission, PermissionStatus::Missing);
    assert_eq!(report.decision, None);
    assert_eq!(report.output, 0.0);
    assert_eq!(report.write_result, Some(Ok(())));
    assert_eq!(calls(&trace), vec![0.0]);
    // Idle tick before permission still writes only the safe output.
    assert_eq!(live(&mut a, 10, None).output, 0.0);
    assert_eq!(calls(&trace), vec![0.0, 0.0]);
    assert_eq!(a.last_accepted_at(), None);
}

#[test]
fn grant_initializes_safe_and_first_command_slews_from_it() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 10.0);
    let init = a.grant(permission(1), t(0)).unwrap();
    assert_eq!(init.permission, PermissionStatus::Initialized);
    assert_eq!(init.output, 0.0);
    assert_eq!(init.generation, Generation::new(1));
    assert_eq!(init.driver_calls(), 1);
    // Zero elapsed time after installation: slew holds the safe reference.
    let zero = live(&mut a, 0, Some(cmd(0.9, 1, 0, 0)));
    assert_eq!(zero.output, 0.0);
    assert_eq!(zero.decision.unwrap().outcome, Outcome::Modified);
    // 10 units/s over 20 ms => 0.2.
    let report = live(&mut a, 20, Some(cmd(0.9, 1, 1, 20)));
    assert!((report.output - 0.2).abs() < 1e-6);
    assert!(report.decision.unwrap().slew_limited);
    assert_eq!(calls(&trace).len(), 3);
    assert!((calls(&trace)[2] - 0.2).abs() < 1e-6);
    assert_eq!(a.last_accepted_at(), Some(t(20)));
}

#[test]
fn unauthorized_and_mismatched_requests_are_rejected() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(live(&mut a, 1, Some(cmd(0.5, 1, 0, 1))).output, 0.5);
    let mut wrong_holder = cmd(0.9, 1, 1, 2);
    wrong_holder.holder = 99;
    let mut wrong_cap = cmd(0.9, 1, 2, 3);
    wrong_cap.capability = 99;
    for (ms, input) in [(2, wrong_holder), (3, wrong_cap)] {
        let report = live(&mut a, ms, Some(input));
        assert_eq!(reason(&report), Some(R::UnknownBinding));
        assert_eq!(report.output, 0.0);
    }
    // Rejected traffic never refreshed liveness or consumed the sequence.
    assert_eq!(a.last_accepted_at(), Some(t(1)));
    assert_eq!(calls(&trace), vec![0.0, 0.5, 0.0, 0.0]);

    // Grants for another holder/capability/endpoint/mode are refused with no I/O.
    let other_lease = AuthorityLease::new(
        HOLDER,
        99,
        SessionConfig::new(
            Generation::new(5).unwrap(),
            t(0),
            t(1000),
            CommandPolicy::new(
                SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
                Duration::from_millis(500),
                Duration::ZERO,
                Duration::from_millis(100),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(
        DriverPermission::new(binding(ExecutionMode::Live), other_lease).err(),
        Some(PermissionError::BindingMismatch)
    );
    let other_endpoint = ActuatorBinding::new(HOLDER, CAPABILITY, 8, ExecutionMode::Live);
    let p = DriverPermission::new(other_endpoint, lease_in(ExecutionMode::Live, 5, 1000)).unwrap();
    assert_eq!(
        a.grant(p, t(4)).err(),
        Some(PermissionError::BindingMismatch)
    );
    assert_eq!(a.generation(), Generation::new(1));
    assert_eq!(calls(&trace).len(), 4);
}

#[test]
fn stale_duplicate_out_of_order_and_generation_changes_reject() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(live(&mut a, 10, Some(cmd(0.4, 1, 5, 10))).output, 0.4);
    let cases = [
        (11, cmd(0.9, 1, 5, 11), R::Duplicate),
        (12, cmd(0.9, 1, 4, 12), R::OutOfOrder),
        (13, cmd(0.9, 2, 6, 13), R::GenerationMismatch),
    ];
    for (ms, input, expected) in cases {
        let report = live(&mut a, ms, Some(input));
        assert_eq!(reason(&report), Some(expected), "{expected:?}");
        assert_eq!(report.output, 0.0);
    }
    // Stale by source age (max 500 ms) with a still-open deadline.
    let mut stale = cmd(0.9, 1, 7, 0);
    stale.meta.deadline = t(2000);
    assert_eq!(
        reason(&live(&mut a, 600, Some(stale))),
        Some(R::StaleCommand)
    );
    // Recovery requires a new valid command, slewed from safe.
    let recovered = live(&mut a, 601, Some(cmd(0.9, 1, 8, 601)));
    assert!((recovered.output - 0.9).abs() < 1e-6);

    // Replacement with generation 2 re-initializes safe; old-generation commands
    // (a replayed stream) are rejected even with a fresh sequence.
    let init = a.grant(permission(2), t(602)).unwrap();
    assert_eq!(init.output, 0.0);
    assert_eq!(init.generation, Generation::new(2));
    assert_eq!(
        reason(&live(&mut a, 603, Some(cmd(0.9, 1, 9, 603)))),
        Some(R::GenerationMismatch)
    );
    let fresh = live(&mut a, 604, Some(cmd(0.3, 2, 0, 604)));
    assert!((fresh.output - 0.3).abs() < 1e-6);
}

#[test]
fn replacement_requires_strictly_newer_generation_including_after_revocation() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(3), t(0)).unwrap();
    let before = calls(&trace).len();
    for g in [1, 3] {
        assert_eq!(
            a.grant(permission(g), t(1)).err(),
            Some(PermissionError::Session(SessionError::ReusedGeneration))
        );
    }
    assert_eq!(calls(&trace).len(), before, "failed grants perform no I/O");
    assert_eq!(a.status(), PermissionStatus::Granted);

    let revoked = a.revoke(t(2));
    assert_eq!(revoked.permission, PermissionStatus::Revoked);
    assert_eq!(revoked.output, 0.0);
    assert_eq!(revoked.generation, Generation::new(3), "watermark retained");
    assert_eq!(
        live(&mut a, 3, Some(cmd(0.9, 3, 0, 3))).permission,
        PermissionStatus::Revoked
    );
    assert_eq!(
        a.grant(permission(3), t(4)).err(),
        Some(PermissionError::Session(SessionError::ReusedGeneration))
    );
    assert_eq!(
        a.renew(t(5000), t(4)).err(),
        Some(PermissionError::NotGranted)
    );
    a.grant(permission(4), t(5)).unwrap();
    assert!((live(&mut a, 6, Some(cmd(0.2, 4, 0, 6))).output - 0.2).abs() < 1e-6);
    // No requested value was ever written while revoked.
    assert!(!calls(&trace)[..before + 3].contains(&0.9));
}

#[test]
fn renewal_extends_lease_without_resetting_watchdog_or_sequence() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    assert_eq!(
        a.renew(t(2000), t(0)).err(),
        Some(PermissionError::NotGranted)
    );
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(live(&mut a, 950, Some(cmd(0.5, 1, 0, 950))).output, 0.5);
    a.renew(t(2000), t(960)).unwrap();
    // Holding is valid within the watchdog; renewal did not refresh it.
    assert_eq!(
        live(&mut a, 1040, None).decision.unwrap().outcome,
        Outcome::Accepted
    );
    let expired = live(&mut a, 1051, None);
    assert_eq!(reason(&expired), Some(R::WatchdogExpired));
    assert_eq!(expired.output, 0.0);
    // Sequence watermark survives renewal.
    assert_eq!(
        reason(&live(&mut a, 1060, Some(cmd(0.5, 1, 0, 1060)))),
        Some(R::Duplicate)
    );
    // Renewal cannot move backwards or use a control time before evaluation.
    assert_eq!(
        a.renew(t(3000), t(1000)).err(),
        Some(PermissionError::Session(
            SessionError::InvalidEvaluationTime
        ))
    );
    assert_eq!(
        a.renew(t(1500), t(1070)).err(),
        Some(PermissionError::Session(SessionError::InvalidLease))
    );
}

#[test]
fn idle_watchdog_and_lease_expiry_select_safe() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(reason(&live(&mut a, 5, None)), Some(R::NoCommand));
    assert_eq!(live(&mut a, 10, Some(cmd(0.7, 1, 0, 10))).output, 0.7);
    let held = live(&mut a, 110, None);
    assert_eq!(held.decision.unwrap().outcome, Outcome::Accepted);
    assert_eq!(held.output, 0.7);
    let tripped = live(&mut a, 111, None);
    assert_eq!(reason(&tripped), Some(R::WatchdogExpired));
    assert_eq!(tripped.output, 0.0);
    // Lease expiry is exclusive at 1000 ms.
    assert_eq!(live(&mut a, 990, Some(cmd(0.1, 1, 1, 990))).output, 0.1);
    let expired = live(&mut a, 1000, Some(cmd(0.2, 1, 2, 1000)));
    assert_eq!(reason(&expired), Some(R::LeaseExpired));
    assert_eq!(expired.output, 0.0);
    assert_eq!(reason(&live(&mut a, 1001, None)), Some(R::LeaseExpired));
    // Renewal after expiry is not reinstatement.
    assert_eq!(
        a.renew(t(5000), t(1002)).err(),
        Some(PermissionError::Session(SessionError::InactiveLease))
    );
}

#[test]
fn clock_faults_latch_across_grant_and_before_first_grant() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    // Runtime time observed before any grant is carried into the gate.
    live(&mut a, 50, None);
    assert_eq!(
        a.grant(permission(1), t(40)).err(),
        Some(PermissionError::Session(
            SessionError::InvalidEvaluationTime
        ))
    );
    a.grant(permission(1), t(50)).unwrap();
    assert_eq!(live(&mut a, 60, Some(cmd(0.5, 1, 0, 60))).output, 0.5);
    let regressed = live(&mut a, 55, Some(cmd(0.6, 1, 1, 55)));
    assert_eq!(reason(&regressed), Some(R::EvaluationTimeRegression));
    assert_eq!(regressed.output, 0.0);
    // Latched: later valid time still rejects, grants cannot clear it.
    assert_eq!(
        reason(&live(&mut a, 70, Some(cmd(0.6, 1, 2, 70)))),
        Some(R::EvaluationTimeRegression)
    );
    assert_eq!(
        a.grant(permission(2), t(80)).err(),
        Some(PermissionError::Session(
            SessionError::InvalidEvaluationTime
        ))
    );
    let revoked = a.revoke(t(90));
    assert_eq!(revoked.evaluation_fault, Some(R::EvaluationTimeRegression));
    assert_eq!(revoked.output, 0.0);

    // A domain change is a separate latched fault, including before any grant.
    let trace2 = Trace::new();
    let mut b = adapter(&trace2, 1000.0);
    b.port(ExecutionMode::Live, t(0)).tick(None);
    let report = b
        .port(
            ExecutionMode::Live,
            Timestamp::new(ClockDomain::Simulation, 1),
        )
        .tick(None);
    assert_eq!(report.evaluation_fault, Some(R::EvaluationClockMismatch));
    assert_eq!(report.output, 0.0);
    assert!(b.grant(permission(1), t(1)).is_err());
}

#[test]
fn simulation_replay_and_live_are_separated() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(live(&mut a, 1, Some(cmd(0.5, 1, 0, 1))).output, 0.5);
    // A replay or simulation port on a live endpoint cannot dispatch requested
    // values, even with otherwise valid live metadata; it safes instead.
    for mode in [ExecutionMode::Replay, ExecutionMode::Simulation] {
        let report = a.port(mode, t(2)).tick(Some(cmd(0.9, 1, 1, 2)));
        assert_eq!(report.permission, PermissionStatus::ModeMismatch);
        assert_eq!(report.decision, None);
        assert_eq!(report.output, 0.0);
    }
    assert!(!calls(&trace).contains(&0.9));
    // The sequence was not consumed by the denied requests.
    assert_eq!(live(&mut a, 3, Some(cmd(0.6, 1, 1, 3))).output, 0.6);

    // Grant/control time must be in the endpoint's domain.
    assert_eq!(
        a.grant(permission(2), at(ExecutionMode::Simulation, 4))
            .err(),
        Some(PermissionError::ModeMismatch)
    );
    // A simulation-timeline lease cannot become a live permission.
    assert_eq!(
        DriverPermission::new(
            binding(ExecutionMode::Live),
            lease_in(ExecutionMode::Simulation, 2, 1000)
        )
        .err(),
        Some(PermissionError::ModeMismatch)
    );
    // A simulation binding cannot be installed on the live adapter.
    let sim = DriverPermission::new(
        binding(ExecutionMode::Simulation),
        lease_in(ExecutionMode::Simulation, 2, 1000),
    )
    .unwrap();
    assert_eq!(
        a.grant(sim, t(4)).err(),
        Some(PermissionError::BindingMismatch)
    );

    // A simulation adapter accepts simulation commands and refuses live-timeline ones.
    let sim_trace = Trace::new();
    let mut s = ActuatorAdapter::new(
        binding(ExecutionMode::Simulation),
        limits(1000.0),
        0.0,
        Driver {
            trace: &sim_trace,
            endpoint: ENDPOINT,
            mode: ExecutionMode::Simulation,
        },
    )
    .unwrap();
    let sim_permission = DriverPermission::new(
        binding(ExecutionMode::Simulation),
        lease_in(ExecutionMode::Simulation, 1, 1000),
    )
    .unwrap();
    let sim_t = |ms| at(ExecutionMode::Simulation, ms);
    s.grant(sim_permission, sim_t(0)).unwrap();
    let ok = s
        .port(ExecutionMode::Simulation, sim_t(1))
        .tick(Some(cmd_in(ExecutionMode::Simulation, 0.4, 1, 0, 1)));
    assert!((ok.output - 0.4).abs() < 1e-6);
    let relabelled = s
        .port(ExecutionMode::Simulation, sim_t(2))
        .tick(Some(cmd(0.8, 1, 1, 2)));
    assert_eq!(reason(&relabelled), Some(R::UnsupportedClockRelationship));
    assert_eq!(
        s.port(ExecutionMode::Live, sim_t(3)).tick(None).permission,
        PermissionStatus::ModeMismatch
    );
}

#[test]
fn failed_request_write_falls_back_once_and_latches() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap(); // call 1
    fail_calls(&trace, &[2], DriverError::UnknownOutcome);
    let report = live(&mut a, 1, Some(cmd(0.8, 1, 0, 1)));
    assert_eq!(report.permission, PermissionStatus::Granted);
    assert_eq!(report.output, 0.8, "gate selection is still reported");
    assert_eq!(report.write_result, Some(Err(DriverError::UnknownOutcome)));
    assert_eq!(report.fallback_result, Some(Ok(())));
    assert_eq!(
        report.latched_driver_fault,
        Some(DriverError::UnknownOutcome)
    );
    assert_eq!(report.driver_calls(), 2);
    // Exactly one fallback to safe, and the requested output is never retried.
    assert_eq!(calls(&trace), vec![0.0, 0.8, 0.0]);

    // Later traffic only attempts safe output; permission changes are refused.
    let next = live(&mut a, 2, Some(cmd(0.8, 1, 1, 2)));
    assert_eq!(next.permission, PermissionStatus::DriverFault);
    assert_eq!(next.output, 0.0);
    assert_eq!(next.driver_calls(), 1);
    assert_eq!(
        a.grant(permission(2), t(3)).err(),
        Some(PermissionError::DriverFault)
    );
    assert_eq!(
        a.renew(t(2000), t(3)).err(),
        Some(PermissionError::DriverFault)
    );
    assert_eq!(calls(&trace), vec![0.0, 0.8, 0.0, 0.0]);
    assert_eq!(a.driver_fault(), Some(DriverError::UnknownOutcome));
}

#[test]
fn failed_initial_safe_write_and_failed_fallback_are_bounded() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    fail_calls(&trace, &[1, 2], DriverError::Unavailable);
    let init = a.grant(permission(1), t(0)).unwrap();
    assert_eq!(init.permission, PermissionStatus::Initialized);
    assert_eq!(init.write_result, Some(Err(DriverError::Unavailable)));
    assert_eq!(init.fallback_result, Some(Err(DriverError::Unavailable)));
    assert_eq!(init.driver_calls(), 2);
    assert_eq!(calls(&trace), vec![0.0, 0.0]);
    // First fault stays latched even after a subsequent safe write succeeds, and
    // a different later error does not overwrite it.
    fail_calls(&trace, &[3], DriverError::Rejected);
    let later = live(&mut a, 1, Some(cmd(0.5, 1, 0, 1)));
    assert_eq!(later.permission, PermissionStatus::DriverFault);
    assert_eq!(later.write_result, Some(Err(DriverError::Rejected)));
    assert_eq!(later.fallback_result, Some(Ok(())));
    assert_eq!(later.latched_driver_fault, Some(DriverError::Unavailable));
    assert_eq!(calls(&trace), vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn shutdown_is_terminal_and_suppresses_io() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 1000.0);
    a.grant(permission(1), t(0)).unwrap();
    assert_eq!(live(&mut a, 1, Some(cmd(0.5, 1, 0, 1))).output, 0.5);
    let first = a.shutdown(t(2));
    assert_eq!(first.permission, PermissionStatus::Shutdown);
    assert_eq!(first.output, 0.0);
    assert_eq!(first.write_result, Some(Ok(())));
    let n = calls(&trace).len();
    let again = a.shutdown(t(3));
    assert_eq!(again.driver_calls(), 0);
    let port = live(&mut a, 4, Some(cmd(0.9, 1, 1, 4)));
    assert_eq!(port.permission, PermissionStatus::Shutdown);
    assert_eq!(port.driver_calls(), 0);
    assert_eq!(a.revoke(t(5)).driver_calls(), 0);
    assert_eq!(
        a.grant(permission(2), t(6)).err(),
        Some(PermissionError::Shutdown)
    );
    assert_eq!(
        a.renew(t(2000), t(6)).err(),
        Some(PermissionError::Shutdown)
    );
    drop(a);
    assert_eq!(calls(&trace).len(), n, "no I/O after shutdown or on drop");

    // Shutdown with a failing driver: exactly one fallback, then silence.
    let trace = Trace::new();
    let mut b = adapter(&trace, 1000.0);
    fail_calls(&trace, &[1], DriverError::UnknownOutcome);
    let report = b.shutdown(t(0));
    assert_eq!(report.driver_calls(), 2);
    assert_eq!(
        report.latched_driver_fault,
        Some(DriverError::UnknownOutcome)
    );
    assert_eq!(b.shutdown(t(1)).driver_calls(), 0);
    assert_eq!(calls(&trace), vec![0.0, 0.0]);
}

#[test]
fn every_operation_is_bounded_to_two_driver_calls_and_fixed_size() {
    let trace = Trace::new();
    let mut a = adapter(&trace, 5.0);
    fail_calls(&trace, &[3, 4, 9, 10], DriverError::UnknownOutcome);
    let mut total = 0;
    let mut reports = 0;
    let mut seq = 0;
    for ms in 0..40 {
        let report = match ms % 5 {
            0 if ms == 0 => a.grant(permission(1), t(ms)).unwrap(),
            1 => {
                seq += 1;
                live(&mut a, ms, Some(cmd(0.9, 1, seq, ms)))
            }
            4 => a.revoke(t(ms)),
            _ => live(&mut a, ms, None),
        };
        assert!(report.driver_calls() <= 2);
        total += report.driver_calls();
        reports += 1;
    }
    assert_eq!(total, calls(&trace).len());
    assert!(total <= 2 * reports);
    // Statically sized, heap-free types (host x86_64: 288 and 576 bytes).
    assert!(core::mem::size_of::<neuradix_embedded_core::DispatchReport>() <= 320);
    assert!(core::mem::size_of::<ActuatorAdapter<Driver<'_>>>() <= 640);
}
