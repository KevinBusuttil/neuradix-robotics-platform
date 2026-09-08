//! Every adversarial scenario runs in a subprocess with a 10-second external
//! timeout. CI also bounds the entire test command. No optional Python skips.
#![cfg(all(target_os = "linux", not(target_env = "uclibc")))]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use neuradix_python::{CleanupState, IoLimits, PythonWorker, Timeouts, WorkerConfig, WorkerError, WorkerSupervisor};
use neuradix_runtime::HealthState;
use serde_json::{Value, json};

const TOTAL: Duration = Duration::from_millis(300);
const RESERVE: Duration = Duration::from_millis(50);
const TOLERANCE: Duration = Duration::from_millis(400);

fn config(mode: &str) -> WorkerConfig {
    WorkerConfig::new("python3", format!("{}/tests/workers/adversary.py", env!("CARGO_MANIFEST_DIR")))
        .with_arg(mode)
        .with_limits(IoLimits::new(256, 1_048_576, 1024, 4).unwrap())
        .with_timeouts(Timeouts::new(Duration::from_secs(1), TOTAL, TOTAL, RESERVE).unwrap())
}
fn cause(error: &WorkerError) -> &WorkerError {
    match error { WorkerError::Cleanup { cause: inner, .. } => cause(inner), other => other }
}
fn assert_bounded(start: Instant, total: Duration) {
    assert!(start.elapsed() <= total + TOLERANCE, "elapsed {:?}, budget {total:?}", start.elapsed());
}
fn assert_storage(worker: &PythonWorker, config: &WorkerConfig) {
    let stats = worker.io_stats();
    assert!(stats.queued_bytes <= config.limits().queued_bytes());
    assert!(stats.queued_messages <= config.limits().queued_messages());
    assert!(stats.outgoing_bytes <= config.limits().outgoing_bytes());
}
fn running(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false; };
    !stat.rsplit_once(") ").is_some_and(|(_, tail)| tail.starts_with('Z'))
}
fn await_dead(pid: u32) {
    let end = Instant::now() + Duration::from_secs(1);
    while running(pid) && Instant::now() < end { std::thread::sleep(Duration::from_millis(2)); }
    assert!(!running(pid), "process {pid} survived cleanup");
}
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("neuradix-a05-{}-{name}", std::process::id()))
}
fn external(case: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "scenario_child", "--nocapture"])
        .env("NEURADIX_BOUND_CASE", case)
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit())
        .spawn().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() { assert!(status.success(), "scenario {case} failed"); return; }
        if Instant::now() >= end {
            let _ = child.kill();
            panic!("external timeout in {case}; fixture workers self-expire");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

macro_rules! scenario {
    ($($name:ident),+ $(,)?) => { $(#[test] fn $name() { external(stringify!($name)); })+ };
}
scenario!(valid_progression, exact_outgoing_limit, exact_incoming_limit,
    blocked_stdin, flood, slow_noise, oversized, invalid_json, queue_bytes,
    close_stdout, handshake_failures, bounded_shutdown_and_drop, descendant_cleanup,
    exited_leader_cleanup, failed_restart_budget, process_admission_bound,
    local_control_continues, remote_error_recovery, write_and_response_share_deadline);

#[test]
fn scenario_child() {
    let Ok(case) = std::env::var("NEURADIX_BOUND_CASE") else { return; };
    assert!(Command::new("python3").arg("--version").status().unwrap().success());
    match case.as_str() {
        "valid_progression" => {
            let cfg = config("echo"); let mut w = PythonWorker::launch(&cfg).unwrap();
            for seq in 1..=10 { assert_eq!(w.send(&json!("hello ☃")).unwrap()["seq"], seq); }
            assert_storage(&w, &cfg);
            assert_eq!(w.shutdown().state, CleanupState::Reaped);
        },
        "exact_outgoing_limit" => {
            let cfg = config("echo").with_limits(IoLimits::new(256, 256, 1024, 4).unwrap());
            let mut w = PythonWorker::launch(&cfg).unwrap();
            let overhead = b"{\"kind\":\"request\",\"seq\":1,\"payload\":\"\"}\n".len();
            let exact = json!("x".repeat(256 - overhead));
            assert_eq!(w.send(&exact).unwrap()["bytes"], 256);
            assert_eq!(w.io_stats().outgoing_bytes, 256);
            let error = w.send(&json!("x".repeat(257 - overhead))).unwrap_err();
            assert!(matches!(error, WorkerError::OutgoingTooLarge { limit: 256 }));
            assert_eq!(w.send(&Value::Null).unwrap()["seq"], 2); // local failure consumed no sequence
            assert_storage(&w, &cfg);
        },
        "exact_incoming_limit" => {
            let cfg = config("sized_response").with_limits(IoLimits::new(128, 256, 512, 4).unwrap());
            let mut w = PythonWorker::launch(&cfg).unwrap();
            assert!(w.send(&json!({"bytes":128})).unwrap().is_string());
            assert!(matches!(cause(&w.send(&json!({"bytes":129})).unwrap_err()), WorkerError::IncomingTooLarge { limit: 128 }));
            assert_storage(&w, &cfg);
        },
        "blocked_stdin" | "flood" | "slow_noise" | "oversized" | "invalid_json" | "queue_bytes" | "close_stdout" => {
            let mut cfg = config(&case);
            if case == "queue_bytes" { cfg = cfg.with_limits(IoLimits::new(160, 256, 192, 4).unwrap()); }
            let mut w = PythonWorker::launch(&cfg).unwrap(); let pid = w.process_id();
            let payload = if case == "blocked_stdin" { json!("x".repeat(900_000)) } else { Value::Null };
            let start = Instant::now(); let error = w.send(&payload).unwrap_err();
            assert_bounded(start, TOTAL);
            match case.as_str() {
                "blocked_stdin" | "slow_noise" => assert!(matches!(cause(&error), WorkerError::Timeout), "{error:?}"),
                "flood" => assert!(matches!(cause(&error), WorkerError::QueueMessagesExceeded { .. }), "{error:?}"),
                "queue_bytes" => assert!(matches!(cause(&error), WorkerError::QueueBytesExceeded { .. }), "{error:?}"),
                "oversized" => assert!(matches!(cause(&error), WorkerError::IncomingTooLarge { .. })),
                "invalid_json" => assert!(matches!(cause(&error), WorkerError::Protocol(_))),
                "close_stdout" => assert!(matches!(cause(&error), WorkerError::StdoutClosed)),
                _ => unreachable!(),
            }
            assert_eq!(w.health(), HealthState::Unavailable);
            assert!(matches!(w.send(&Value::Null), Err(WorkerError::Unavailable)));
            let report = w.cleanup_report().unwrap();
            let repeat = Instant::now(); assert_eq!(w.shutdown(), report); assert_bounded(repeat, Duration::ZERO);
            assert_storage(&w, &cfg); await_dead(pid);
        },
        "handshake_failures" => {
            for mode in ["no_handshake", "bad_handshake", "huge_handshake"] {
                let cfg = config(mode).with_timeouts(Timeouts::new(TOTAL, TOTAL, TOTAL, RESERVE).unwrap());
                let start = Instant::now(); let error = PythonWorker::launch(&cfg).err().expect("handshake should fail");
                assert_bounded(start, TOTAL);
                match mode {
                    "no_handshake" => assert!(matches!(cause(&error), WorkerError::HandshakeTimeout)),
                    "bad_handshake" => assert!(matches!(cause(&error), WorkerError::Protocol(_))),
                    _ => assert!(matches!(cause(&error), WorkerError::IncomingTooLarge { .. })),
                }
            }
        },
        "bounded_shutdown_and_drop" => {
            let mut w = PythonWorker::launch(&config("blocked_stdin")).unwrap();
            let pid = w.process_id(); let start = Instant::now();
            let report = w.shutdown(); assert_bounded(start, TOTAL);
            assert!(matches!(report.state, CleanupState::Reaped | CleanupState::Deferred)); await_dead(pid);
            let w = PythonWorker::launch(&config("blocked_stdin")).unwrap();
            let pid = w.process_id(); let start = Instant::now(); drop(w); assert_bounded(start, RESERVE); await_dead(pid);
        },
        "descendant_cleanup" => {
            let mut w = PythonWorker::launch(&config("descendant")).unwrap();
            let pid = w.send(&Value::Null).unwrap()["pid"].as_u64().unwrap() as u32;
            let start = Instant::now(); let error = w.send(&Value::Null).unwrap_err();
            assert!(matches!(cause(&error), WorkerError::Timeout)); assert_bounded(start, TOTAL);
            await_dead(pid); await_dead(w.process_id());
        },
        "exited_leader_cleanup" => {
            let path = temp("descendant");
            let mut w = PythonWorker::launch(&config("exit_with_descendant").with_arg(path.to_string_lossy())).unwrap();
            let start = Instant::now(); let _ = w.send(&Value::Null).unwrap_err(); assert_bounded(start, TOTAL);
            let pid: u32 = std::fs::read_to_string(&path).unwrap().parse().unwrap();
            await_dead(pid); await_dead(w.process_id()); std::fs::remove_file(path).unwrap();
        },
        "failed_restart_budget" => {
            let path = temp("restarts");
            let cfg = config("retry_handshake").with_arg(path.to_string_lossy()).with_timeouts(Timeouts::new(TOTAL, TOTAL, TOTAL, RESERVE).unwrap());
            let mut supervisor = WorkerSupervisor::start(cfg, 2).unwrap();
            supervisor.worker().unwrap().send(&Value::Null).unwrap_err();
            for used in 1..=2 {
                let start = Instant::now(); let error = supervisor.ensure_alive().unwrap_err();
                assert!(matches!(cause(&error), WorkerError::HandshakeTimeout)); assert_bounded(start, TOTAL);
                assert_eq!(supervisor.restarts_used(), used);
            }
            assert!(matches!(supervisor.ensure_alive(), Err(WorkerError::RestartBudgetExhausted { used: 2, max: 2 })));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "3"); std::fs::remove_file(path).unwrap();
        },
        "process_admission_bound" => {
            let cfg = config("echo"); let mut workers = Vec::new();
            for _ in 0..32 { workers.push(PythonWorker::launch(&cfg).unwrap()); }
            assert!(matches!(PythonWorker::launch(&cfg), Err(WorkerError::ProcessCapacity)));
            drop(workers.pop());
            workers.push(PythonWorker::launch(&cfg).unwrap());
            for w in &mut workers { w.shutdown(); }
        },
        "remote_error_recovery" => {
            let mut w = PythonWorker::launch(&config("remote_error")).unwrap();
            assert!(matches!(w.send(&json!(true)), Err(WorkerError::Remote(_))));
            assert_eq!(w.send(&json!(false)).unwrap(), 2);
            assert_eq!(w.health(), HealthState::Healthy);
        },
        "write_and_response_share_deadline" => {
            let total = Duration::from_secs(1);
            let cfg = config("write_then_hang").with_timeouts(Timeouts::new(total, total, TOTAL, Duration::from_millis(100)).unwrap());
            let mut w = PythonWorker::launch(&cfg).unwrap();
            let start = Instant::now();
            let error = w.send(&json!("x".repeat(900_000))).unwrap_err();
            assert!(matches!(cause(&error), WorkerError::Timeout));
            // 550ms is consumed before the worker drains stdin. Resetting the
            // receive deadline after write would exceed this 1.25s ceiling.
            assert!(start.elapsed() <= total + Duration::from_millis(250));
        },
        "local_control_continues" => local_control(),
        other => panic!("unknown scenario {other}"),
    }
}

fn local_control() {
    use neuradix_safety::{AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Constraint, Generation, Identity, LeaseTable, SafetyGate, SessionConfig, SharedTimeline};
    use neuradix_time::{ClockDomain, Duration as RobotDuration, Timestamp};
    let t = |n| Timestamp::new(ClockDomain::Monotonic, n);
    let generation = Generation::new(1).unwrap();
    let policy = CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(), RobotDuration::from_secs(1), RobotDuration::ZERO, RobotDuration::from_millis(100)).unwrap();
    let session = SessionConfig::new(generation, t(0), t(10_000_000_000), policy).unwrap();
    let mut table = LeaseTable::new();
    table.grant(AuthorityLease::new(Identity::new("local"), Capability::new("thrust"), session, None)).unwrap();
    let mut gate = SafetyGate::new(table, vec![Constraint::range("range", -1.0, 1.0).unwrap()], 0.0).unwrap();
    let mut worker = PythonWorker::launch(&config("hang")).unwrap();
    let task = std::thread::spawn(move || { let error = worker.send(&Value::Null).unwrap_err(); assert!(matches!(cause(&error), WorkerError::Timeout)); worker.health() });
    let mut ticks = 0u64;
    while !task.is_finished() {
        let now = t(ticks as i128 * 5_000_000);
        let meta = CommandMeta { generation, sequence: ticks, source_at: now, deadline: t(now.as_nanos() + 100_000_000), timeline: 1 };
        let decision = gate.evaluate(Some(CommandRequest::new(Identity::new("local"), Capability::new("thrust"), 0.5, meta)), now);
        assert_eq!(decision.applied, 0.5); ticks += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ticks >= 5, "local controller did not progress during worker failure");
    assert_eq!(task.join().unwrap(), HealthState::Unavailable);
    gate.leases_mut().revoke(&Identity::new("local"), &Capability::new("thrust"));
    assert_eq!(gate.evaluate(None, t(ticks as i128 * 5_000_000)).applied, 0.0);
}
