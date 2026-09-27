//! Every scenario runs in a separately timed process to prevent CI hangs.
use neuradix_safety::{
    AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Generation, Identity,
    Outcome, RejectReason, SessionConfig, SessionError, SharedTimeline, actuator::*,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use std::{
    cell::RefCell,
    process::{Command, Stdio},
    rc::Rc,
    time::{Duration as WallDuration, Instant},
};
#[derive(Default)]
struct Trace {
    calls: Vec<f64>,
    failures: Vec<usize>,
}
struct Driver {
    trace: Rc<RefCell<Trace>>,
    endpoint: String,
    mode: ExecutionMode,
}
impl ActuatorDriver for Driver {
    fn endpoint(&self) -> &str {
        &self.endpoint
    }
    fn mode(&self) -> ExecutionMode {
        self.mode
    }
    fn write(&mut self, value: f64) -> Result<(), DriverError> {
        let mut t = self.trace.borrow_mut();
        t.calls.push(value);
        if t.failures.contains(&t.calls.len()) {
            Err(DriverError::UnknownOutcome)
        } else {
            Ok(())
        }
    }
}
fn t(ms: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000)
}
fn binding() -> ActuatorBinding {
    ActuatorBinding::new("controller", "thrust", "driver/one", ExecutionMode::Live).unwrap()
}
fn config(rate: f64) -> ActuatorConfig {
    ActuatorConfig::new(binding(), -1.0, 1.0, rate, 0.0).unwrap()
}
fn lease(g: u128) -> AuthorityLease {
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    AuthorityLease::new(
        Identity::new("controller"),
        Capability::new("thrust"),
        SessionConfig::new(Generation::new(g).unwrap(), t(0), t(1000), policy).unwrap(),
        None,
    )
}
fn permission(g: u128) -> DriverPermission {
    DriverPermission::new(binding(), lease(g)).unwrap()
}
fn setup(rate: f64) -> (ActuatorAdapter<Driver>, Rc<RefCell<Trace>>) {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let driver = Driver {
        trace: trace.clone(),
        endpoint: "driver/one".into(),
        mode: ExecutionMode::Live,
    };
    (ActuatorAdapter::new(config(rate), driver).unwrap(), trace)
}
fn req(g: u128, seq: u64, ms: i128, v: f64) -> CommandRequest {
    CommandRequest::new(
        Identity::new("controller"),
        Capability::new("thrust"),
        v,
        CommandMeta {
            generation: Generation::new(g).unwrap(),
            sequence: seq,
            source_at: t(ms),
            deadline: t(ms + 500),
            timeline: 1,
        },
    )
}
fn tick(a: &mut ActuatorAdapter<Driver>, ms: i128, r: Option<CommandRequest>) -> DispatchReport {
    a.port(ExecutionMode::Live, t(ms)).tick(r)
}
fn run(name: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "actuator_child", "--nocapture"])
        .env("NEURADIX_ACTUATOR_CASE", name)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + WallDuration::from_secs(10);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            assert!(s.success(), "{name}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("deadline: {name}");
        }
        std::thread::sleep(WallDuration::from_millis(5));
    }
}
macro_rules! scenario {
    ($name:ident) => {
        #[test]
        fn $name() {
            run(stringify!($name));
        }
    };
}
scenario!(permission_and_actual_driver);
scenario!(mode_and_untrusted_input);
scenario!(configuration_and_capacity);
scenario!(replacement_revocation_and_renewal);
scenario!(clock_faults_latch);
scenario!(expiry_and_rejected_liveness);
scenario!(failure_and_shutdown);
scenario!(numeric_limits_and_recovery);
scenario!(independent_local_control);
#[test]
fn actuator_child() {
    let Ok(name) = std::env::var("NEURADIX_ACTUATOR_CASE") else {
        return;
    };
    match name.as_str() {
        "permission_and_actual_driver" => {
            let (mut a, trace) = setup(10.0);
            let mut r = req(1, 0, 0, 0.8);
            r.holder = Identity::new("safety");
            assert_eq!(
                tick(&mut a, 0, Some(r)).permission,
                PermissionStatus::Missing
            );
            assert_eq!(trace.borrow().calls, [0.0]);
            a.grant(permission(1), t(0)).unwrap();
            let r = tick(&mut a, 100, Some(req(1, 0, 100, 0.8)));
            assert_eq!(r.decision.unwrap().outcome, Outcome::Accepted);
            assert_eq!(r.write_result, Some(Ok(())));
            assert_eq!(trace.borrow().calls.last(), Some(&0.8));
            let mut r = req(1, 1, 110, 0.9);
            r.holder = Identity::new("safety");
            assert_eq!(
                tick(&mut a, 110, Some(r)).decision.unwrap().outcome,
                Outcome::Rejected(RejectReason::UnknownBinding)
            );
            let mut r = req(1, 1, 120, 0.9);
            r.capability = Capability::new("other");
            assert_eq!(tick(&mut a, 120, Some(r)).output, 0.0);
            assert_eq!(a.last_accepted_at(), Some(t(100)));
            assert!(
                trace
                    .borrow()
                    .calls
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1.0)
            );
        }
        "mode_and_untrusted_input" => {
            let (mut a, trace) = setup(10.0);
            a.grant(permission(1), t(0)).unwrap();
            for mode in [ExecutionMode::Simulation, ExecutionMode::Replay] {
                let r = a.port(mode, t(10)).tick(Some(req(1, 0, 10, 1.0)));
                assert_eq!(r.permission, PermissionStatus::ModeMismatch);
                assert!(r.decision.is_none());
            }
            assert!(a.last_accepted_at().is_none());
            assert!(trace.borrow().calls.iter().all(|v| *v == 0.0));
            let mut r = req(1, 0, 10, 1.0);
            r.holder = Identity::new("x".repeat(129));
            assert_eq!(
                tick(&mut a, 10, Some(r)).permission,
                PermissionStatus::OversizedInput
            );
            let mut r = req(1, 0, 20, 1.0);
            r.meta.source_at = Timestamp::new(ClockDomain::Replay, 0);
            assert_eq!(
                tick(&mut a, 20, Some(r)).decision.unwrap().outcome,
                Outcome::Rejected(RejectReason::UnsupportedClockRelationship)
            );
            // The same sequence is still usable: rejected traffic did not consume it.
            assert!(
                !tick(&mut a, 30, Some(req(1, 0, 30, 0.1)))
                    .decision
                    .unwrap()
                    .is_rejected()
            );
        }
        "configuration_and_capacity" => {
            assert!(ActuatorBinding::new(&"x".repeat(128), "a", "b", ExecutionMode::Live).is_ok());
            for s in [
                "".to_owned(),
                " ".to_owned(),
                "a b".to_owned(),
                "é".to_owned(),
                "x".repeat(129),
            ] {
                assert!(ActuatorBinding::new(&s, "a", "b", ExecutionMode::Live).is_err());
                assert!(ActuatorBinding::new("a", &s, "b", ExecutionMode::Live).is_err());
                assert!(ActuatorBinding::new("a", "b", &s, ExecutionMode::Live).is_err());
            }
            for (lo, hi, rate, safe) in [
                (f64::NAN, 1.0, 1.0, 0.0),
                (-1.0, f64::INFINITY, 1.0, 0.0),
                (1.0, -1.0, 1.0, 0.0),
                (-1.0, 1.0, -1.0, 0.0),
                (-1.0, 1.0, f64::INFINITY, 0.0),
                (-1.0, 1.0, 1.0, f64::NAN),
                (-1.0, 1.0, 1.0, 2.0),
            ] {
                assert!(ActuatorConfig::new(binding(), lo, hi, rate, safe).is_err());
            }
            assert!(ActuatorConfig::new(binding(), 0.0, 0.0, 0.0, 0.0).is_ok());
            let trace = Rc::new(RefCell::new(Trace::default()));
            assert!(matches!(
                ActuatorAdapter::new(
                    config(1.0),
                    Driver {
                        trace: trace.clone(),
                        endpoint: "wrong".into(),
                        mode: ExecutionMode::Live
                    }
                ),
                Err(PermissionError::BindingMismatch)
            ));
            assert!(matches!(
                ActuatorAdapter::new(
                    config(1.0),
                    Driver {
                        trace: trace.clone(),
                        endpoint: "driver/one".into(),
                        mode: ExecutionMode::Replay
                    }
                ),
                Err(PermissionError::ModeMismatch)
            ));
            assert!(trace.borrow().calls.is_empty());
            let wrong =
                ActuatorBinding::new("wrong", "thrust", "driver/one", ExecutionMode::Live).unwrap();
            assert!(matches!(
                DriverPermission::new(wrong, lease(1)),
                Err(PermissionError::BindingMismatch)
            ));
            let (mut a, trace) = setup(10.0);
            a.grant(permission(1), t(0)).unwrap();
            let wrong =
                ActuatorBinding::new("controller", "thrust", "driver/two", ExecutionMode::Live)
                    .unwrap();
            let grant = DriverPermission::new(wrong, lease(2)).unwrap();
            assert!(matches!(
                a.grant(grant, t(1)),
                Err(PermissionError::BindingMismatch)
            ));
            for _ in 0..1000 {
                assert!(matches!(
                    a.grant(permission(1), t(1)),
                    Err(PermissionError::Session(SessionError::ReusedGeneration))
                ));
            }
            assert_eq!(trace.borrow().calls.len(), 1); // No allocation of new binding slots or I/O on rejected grants.
            assert_eq!(a.binding_count(), 1);
            let wrong_mode = ActuatorBinding::new("controller", "thrust", "driver/one", ExecutionMode::Replay).unwrap();
            assert!(matches!(DriverPermission::new(wrong_mode, lease(2)), Err(PermissionError::ModeMismatch)));

        }
        "replacement_revocation_and_renewal" => {
            let (mut a, trace) = setup(1.0);
            a.grant(permission(1), t(0)).unwrap();
            assert_eq!(tick(&mut a, 100, Some(req(1, 0, 100, 1.0))).output, 0.1);
            a.renew(t(2000), t(100)).unwrap();
            assert_eq!(tick(&mut a, 100, Some(req(1, 1, 100, 1.0))).output, 0.1);
            let r = a.revoke(t(110));
            assert_eq!(r.output, 0.0);
            assert_eq!(trace.borrow().calls.last(), Some(&0.0));
            assert_eq!(a.renew(t(3000), t(120)), Err(PermissionError::NotGranted));
            assert!(a.grant(permission(1), t(120)).is_err());
            a.grant(permission(2), t(120)).unwrap();
            assert_eq!(
                tick(&mut a, 130, Some(req(1, 2, 130, 1.0)))
                    .decision
                    .unwrap()
                    .outcome,
                Outcome::Rejected(RejectReason::GenerationMismatch)
            );
            assert!((tick(&mut a, 140, Some(req(2, 0, 140, 1.0))).output - 0.01).abs() < 1e-15);
            // Restart requires a trusted non-reused generation; old delayed traffic fails.
            let (mut b, _) = setup(10.0);
            b.grant(permission(3), t(150)).unwrap();
            assert_eq!(
                tick(&mut b, 160, Some(req(2, 10, 160, 1.0)))
                    .decision
                    .unwrap()
                    .outcome,
                Outcome::Rejected(RejectReason::GenerationMismatch)
            );
        }
        "clock_faults_latch" => {
            for (first, bad, expected) in [
                (t(100), t(99), RejectReason::EvaluationTimeRegression),
                (
                    t(100),
                    Timestamp::new(ClockDomain::Replay, 100),
                    RejectReason::EvaluationClockMismatch,
                ),
                (
                    Timestamp::new(ClockDomain::Monotonic, i128::MIN),
                    Timestamp::new(ClockDomain::Monotonic, i128::MAX),
                    RejectReason::EvaluationTimeOverflow,
                ),
            ] {
                let (mut a, trace) = setup(10.0);
                a.grant(permission(1), first).unwrap();
                let r = a.port(ExecutionMode::Live, bad).tick(None);
                assert_eq!(r.decision.unwrap().outcome, Outcome::Rejected(expected));
                assert!(matches!(
                    a.grant(permission(2), t(200)),
                    Err(PermissionError::Session(
                        SessionError::InvalidEvaluationTime
                    ))
                ));
                let r = a.revoke(t(200));
                assert_eq!(r.evaluation_fault, Some(expected));
                assert!(trace.borrow().calls.iter().all(|v| *v == 0.0));
            }
        }
        "expiry_and_rejected_liveness" => {
            // Old source time cannot authorize a command at the captured runtime expiry.
            let (mut expired, _) = setup(10.0);
            expired.grant(permission(1), t(0)).unwrap();
            let report = expired.port(ExecutionMode::Live, t(1000)).tick(Some(req(1, 0, 500, 1.0)));
            assert_eq!(report.decision.unwrap().outcome, Outcome::Rejected(RejectReason::LeaseExpired));

            let (mut a, _) = setup(10.0);
            a.grant(permission(1), t(0)).unwrap();
            tick(&mut a, 100, Some(req(1, 0, 100, 1.0)));
            let r = tick(&mut a, 200, None);
            assert!(!r.decision.unwrap().is_rejected());
            assert_eq!(
                tick(&mut a, 201, None).decision.unwrap().outcome,
                Outcome::Rejected(RejectReason::WatchdogExpired)
            );
            tick(&mut a, 210, Some(req(1, 1, 210, 1.0)));
            for n in 211..220 {
                assert!(
                    tick(&mut a, n, Some(req(1, 1, 210, 1.0)))
                        .decision
                        .unwrap()
                        .is_rejected()
                );
            }
            assert_eq!(a.last_accepted_at(), Some(t(210)));
            assert_eq!(
                tick(&mut a, 311, None).decision.unwrap().outcome,
                Outcome::Rejected(RejectReason::WatchdogExpired)
            );
            let mut r = req(1, 2, 400, 1.0);
            r.meta.deadline = t(410);
            tick(&mut a, 400, Some(r));
            assert_eq!(
                tick(&mut a, 410, None).decision.unwrap().outcome,
                Outcome::Rejected(RejectReason::DeadlineExpired)
            );
            assert_eq!(
                tick(&mut a, 1000, Some(req(1, 3, 1000, 1.0)))
                    .decision
                    .unwrap()
                    .outcome,
                Outcome::Rejected(RejectReason::LeaseExpired)
            );
        }
        "failure_and_shutdown" => {
            let (mut a, trace) = setup(10.0);
            a.grant(permission(1), t(0)).unwrap();
            trace.borrow_mut().failures = vec![2, 3];
            let r = tick(&mut a, 100, Some(req(1, 0, 100, 1.0)));
            assert_eq!(r.write_result, Some(Err(DriverError::UnknownOutcome)));
            assert_eq!(r.fallback_result, Some(Err(DriverError::UnknownOutcome)));
            assert_eq!(trace.borrow().calls, [0.0, 1.0, 0.0]);
            assert_eq!(a.driver_fault(), Some(DriverError::UnknownOutcome));
            assert!(matches!(
                a.grant(permission(2), t(100)),
                Err(PermissionError::DriverFault)
            ));
            let r = tick(&mut a, 110, Some(req(1, 1, 110, 1.0)));
            assert_eq!(r.permission, PermissionStatus::DriverFault);
            assert_eq!(r.output, 0.0);
            assert_eq!(a.last_accepted_at(), Some(t(100)));
            a.shutdown(t(120));
            let n = trace.borrow().calls.len();
            assert_eq!(a.shutdown(t(120)).write_result, None);
            assert_eq!(
                tick(&mut a, 130, Some(req(1, 2, 130, 1.0))).permission,
                PermissionStatus::Shutdown
            );
            assert!(matches!(
                a.grant(permission(2), t(140)),
                Err(PermissionError::Shutdown)
            ));
            drop(a);
            assert_eq!(trace.borrow().calls.len(), n);
            let (mut a, trace) = setup(10.0);
            trace.borrow_mut().failures = vec![1];
            let r = a.grant(permission(1), t(0)).unwrap();
            assert_eq!(r.fallback_result, Some(Ok(())));
            assert!(a.driver_fault().is_some());
        }
        "numeric_limits_and_recovery" => {
            let (mut a, trace) = setup(1.0);
            a.grant(permission(1), t(0)).unwrap();
            assert_eq!(tick(&mut a, 0, Some(req(1, 0, 0, 1.0))).output, 0.0);
            assert_eq!(
                tick(&mut a, 100, Some(req(1, 1, 100, f64::MAX))).output,
                0.1
            );
            for (i, v) in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    tick(&mut a, 101 + i as i128, Some(req(1, 2, 101 + i as i128, v))).output,
                    0.0
                );
            }
            assert!((tick(&mut a, 203, Some(req(1, 2, 203, -1.0))).output + 0.1).abs() < 1e-15);
            assert!(
                trace
                    .borrow()
                    .calls
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1.0)
            );
            let (mut a, _) = setup(0.0);
            a.grant(permission(1), t(0)).unwrap();
            assert_eq!(tick(&mut a, 100, Some(req(1, 0, 100, 1.0))).output, 0.0);
            let (mut a, _) = setup(f64::MAX);
            a.grant(permission(1), t(0)).unwrap();
            assert!(
                tick(&mut a, 100, Some(req(1, 0, 100, 1.0)))
                    .output
                    .is_finite()
            );
        }
        "independent_local_control" => {
            let (mut failed, trace) = setup(10.0);
            trace.borrow_mut().failures = vec![1, 2];
            failed.grant(permission(1), t(0)).unwrap();
            let (mut local, trace) = setup(10.0);
            local.grant(permission(1), t(0)).unwrap();
            assert_eq!(
                tick(&mut failed, 100, Some(req(1, 0, 100, 1.0))).permission,
                PermissionStatus::DriverFault
            );
            assert_eq!(tick(&mut local, 100, Some(req(1, 0, 100, 0.5))).output, 0.5);
            assert_eq!(tick(&mut local, 201, None).output, 0.0);
            assert_eq!(trace.borrow().calls, [0.0, 0.5, 0.0]);
        }
        _ => panic!("unknown scenario"),
    }
}
