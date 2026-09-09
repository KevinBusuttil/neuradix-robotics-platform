//! Real worker scenarios run in externally timed subprocesses; CI also bounds
//! the whole test command. Python is required, never silently skipped here.
#![cfg(all(target_os = "linux", not(target_env = "uclibc")))]

use neuradix_python::{
    HeartbeatPolicy, IoLimits, PythonWorker, Timeouts, WorkerConfig, WorkerError, WorkerFailure,
    WorkerSupervisor,
};
use neuradix_runtime::HealthState;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_millis(60);
const RESPONSE: Duration = Duration::from_millis(250);
const RESERVE: Duration = Duration::from_millis(50);
const TOLERANCE: Duration = Duration::from_millis(200);

fn config(mode: &str) -> WorkerConfig {
    WorkerConfig::new(
        "python3",
        format!("{}/tests/workers/heartbeat.py", env!("CARGO_MANIFEST_DIR")),
    )
    .with_arg(mode)
    .with_limits(IoLimits::new(256, 256, 1024, 4).unwrap())
    .with_timeouts(
        Timeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(300),
            RESERVE,
        )
        .unwrap(),
    )
    .with_heartbeat(HeartbeatPolicy::new(INTERVAL, RESPONSE).unwrap())
}
fn cause(error: &WorkerError) -> &WorkerError {
    match error {
        WorkerError::Cleanup { cause: inner, .. } => cause(inner),
        other => other,
    }
}
fn until(time: Instant) {
    std::thread::sleep(time.saturating_duration_since(Instant::now()));
}
fn bounded(start: Instant, budget: Duration) {
    assert!(
        start.elapsed() <= budget + TOLERANCE,
        "elapsed {:?}, budget {budget:?}",
        start.elapsed()
    );
}
fn storage(worker: &PythonWorker, cfg: &WorkerConfig) {
    let stats = worker.io_stats();
    assert!(stats.queued_bytes <= cfg.limits().queued_bytes());
    assert!(stats.queued_messages <= cfg.limits().queued_messages());
    assert!(stats.outgoing_bytes <= cfg.limits().outgoing_bytes());
    assert_eq!(cfg.limits().outstanding_requests(), 1);
}
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("neuradix-heartbeat-{}-{name}", std::process::id()))
}
fn external(case: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "heartbeat_child", "--nocapture"])
        .env("NEURADIX_HEARTBEAT_CASE", case)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "scenario {case} failed");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("external timeout in {case}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
macro_rules! scenarios {
    ($($case:ident),+ $(,)?) => { $(#[test] fn $case() { external(stringify!($case)); })+ };
}
scenarios!(
    healthy_idle,
    sdk_idle,
    ordinary_requests,
    hung_handler,
    stopped_worker,
    scheduling_gap,
    late_poll,
    unrelated_traffic,
    duplicate_replies,
    late_reply,
    flood_bounds,
    replacement_state,
    failed_replacement_handshakes,
    failed_launches,
    shutdown_and_drop
);

#[test]
fn heartbeat_child() {
    let Ok(case) = std::env::var("NEURADIX_HEARTBEAT_CASE") else {
        return;
    };
    assert!(
        Command::new("python3")
            .arg("--version")
            .status()
            .unwrap()
            .success()
    );
    match case.as_str() {
        "healthy_idle" => {
            let cfg = config("healthy").with_limits(IoLimits::new(64, 64, 128, 2).unwrap());
            let mut worker = PythonWorker::launch(&cfg).unwrap();
            assert_eq!(worker.health(), HealthState::Unknown);
            assert!(!worker.check_heartbeat().unwrap());
            for seq in [2, 4, 6] {
                until(worker.heartbeat_due());
                assert!(worker.check_heartbeat().unwrap());
                assert_eq!(worker.health(), HealthState::Healthy);
                assert_eq!(worker.send(&Value::Null).unwrap(), seq);
                assert!(!worker.check_heartbeat().unwrap());
            }
            storage(&worker, &cfg);
        }
        "sdk_idle" => {
            let cfg = WorkerConfig::new(
                "python3",
                format!(
                    "{}/tests/workers/testkit_worker.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
            )
            .with_python_path(format!("{}/../../python", env!("CARGO_MANIFEST_DIR")))
            .with_heartbeat(HeartbeatPolicy::new(INTERVAL, RESPONSE).unwrap());
            let mut worker = PythonWorker::launch(&cfg).unwrap();
            until(worker.heartbeat_due());
            assert!(worker.check_heartbeat().unwrap());
            assert_eq!(worker.health(), HealthState::Healthy);
            assert_eq!(
                worker.send(&json!({"depth": 3})).unwrap()["echo"]["depth"],
                3
            );
        }
        "ordinary_requests" => {
            let mut worker = PythonWorker::launch(&config("healthy")).unwrap();
            let initial = worker.heartbeat_due();
            assert_eq!(worker.send(&Value::Null).unwrap(), 1);
            assert!(worker.heartbeat_due() >= initial);
            let before = worker.heartbeat_due();
            until(before);
            assert_eq!(worker.health(), HealthState::Degraded);
            // A request in the remaining health window substitutes for a ping.
            assert!(matches!(
                worker.send(&json!("remote")),
                Err(WorkerError::Remote(_))
            ));
            assert_eq!(worker.health(), HealthState::Healthy);
            let due = worker.heartbeat_due();
            assert!(due > before);
            assert!(!worker.check_heartbeat().unwrap());
            assert!(matches!(
                worker.send(&json!("x".repeat(300))),
                Err(WorkerError::OutgoingTooLarge { .. })
            ));
            assert_eq!(worker.heartbeat_due(), due); // local rejection is not proof
            assert_eq!(worker.send(&Value::Null).unwrap(), 3); // no ping/local-rejection sequence
        }
        "hung_handler" => {
            let mut worker = PythonWorker::launch(&config("healthy")).unwrap();
            let start = Instant::now();
            // The 1s request budget is capped by the initial 310ms health window.
            let error = worker.send(&json!("hang")).unwrap_err();
            assert!(matches!(cause(&error), WorkerError::Timeout));
            bounded(start, INTERVAL + RESPONSE + RESERVE);
            assert_eq!(worker.last_failure(), Some(WorkerFailure::RequestTimeout));
            assert!(matches!(
                worker.check_heartbeat(),
                Err(WorkerError::Unavailable)
            ));
        }
        "stopped_worker" => {
            let mut worker = PythonWorker::launch(&config("healthy")).unwrap();
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(worker.process_id() as i32),
                nix::sys::signal::Signal::SIGSTOP,
            )
            .unwrap();
            until(worker.heartbeat_due());
            let start = Instant::now();
            let error = worker.check_heartbeat().unwrap_err();
            assert!(matches!(cause(&error), WorkerError::HeartbeatTimeout));
            bounded(start, RESPONSE + RESERVE);
            assert_eq!(worker.health(), HealthState::Unavailable);
            assert!(worker.cleanup_report().is_some());
        }
        "scheduling_gap" => {
            let mut worker = PythonWorker::launch(&config("healthy")).unwrap();
            worker.send(&Value::Null).unwrap();
            until(worker.heartbeat_expires() + Duration::from_millis(10));
            let start = Instant::now();
            assert!(matches!(
                cause(&worker.check_heartbeat().unwrap_err()),
                WorkerError::HeartbeatExpired
            ));
            bounded(start, RESERVE);
            assert_eq!(worker.last_failure(), Some(WorkerFailure::HeartbeatExpired));
            assert!(matches!(
                worker.send(&Value::Null),
                Err(WorkerError::Unavailable)
            ));
            assert_eq!(worker.health(), HealthState::Unavailable);
        }
        "late_poll" => {
            let cfg = config("hang").with_heartbeat(
                HeartbeatPolicy::new(INTERVAL, Duration::from_millis(700)).unwrap(),
            );
            let mut worker = PythonWorker::launch(&cfg).unwrap();
            until(worker.heartbeat_due() + Duration::from_millis(500));
            let start = Instant::now();
            assert!(matches!(
                cause(&worker.check_heartbeat().unwrap_err()),
                WorkerError::HeartbeatTimeout
            ));
            bounded(start, Duration::from_millis(200) + RESERVE); // restarting 700ms would fail
        }
        "unrelated_traffic" | "duplicate_replies" | "late_reply" | "flood_bounds" => {
            let mode = match case.as_str() {
                "unrelated_traffic" => "unrelated",
                "duplicate_replies" => "duplicate",
                "late_reply" => "late",
                _ => "flood",
            };
            let cfg = config(mode);
            let mut worker = PythonWorker::launch(&cfg).unwrap();
            if mode == "duplicate" {
                until(worker.heartbeat_due());
                assert!(worker.check_heartbeat().unwrap());
            }
            until(worker.heartbeat_due());
            let expiry = worker.heartbeat_expires();
            let start = Instant::now();
            let error = worker.check_heartbeat().unwrap_err();
            if mode == "flood" {
                assert!(matches!(
                    cause(&error),
                    WorkerError::QueueMessagesExceeded { .. }
                        | WorkerError::QueueBytesExceeded { .. }
                ));
            } else {
                assert!(matches!(cause(&error), WorkerError::HeartbeatTimeout));
            }
            bounded(start, RESPONSE + RESERVE);
            assert_eq!(worker.heartbeat_expires(), expiry);
            assert_eq!(worker.health(), HealthState::Unavailable);
            let report = worker.cleanup_report();
            until(start + Duration::from_millis(500));
            assert_eq!(worker.health(), HealthState::Unavailable);
            assert!(matches!(
                worker.check_heartbeat(),
                Err(WorkerError::Unavailable)
            ));
            assert_eq!(worker.cleanup_report(), report);
            storage(&worker, &cfg);
        }
        "replacement_state" | "failed_replacement_handshakes" => {
            let path = temp(&case);
            let mode = if case == "replacement_state" {
                "replace"
            } else {
                "failed_handshake"
            };
            let mut supervisor =
                WorkerSupervisor::start(config(mode).with_arg(path.to_string_lossy()), 2).unwrap();
            until(supervisor.worker().unwrap().heartbeat_due());
            assert!(matches!(
                cause(&supervisor.poll().unwrap_err()),
                WorkerError::HeartbeatTimeout
            ));
            assert_eq!(supervisor.restarts_used(), 0); // detection never hides a replacement
            assert_eq!(supervisor.health(), HealthState::Unavailable);
            if mode == "replace" {
                assert_eq!(supervisor.poll().unwrap(), HealthState::Unknown);
                assert_eq!(supervisor.restarts_used(), 1);
                assert_eq!(
                    supervisor.last_failure(),
                    Some(WorkerFailure::HeartbeatTimeout)
                );
                assert_eq!(supervisor.worker().unwrap().send(&Value::Null).unwrap(), 1);
                assert_eq!(supervisor.health(), HealthState::Healthy);
            } else {
                for used in 1..=2 {
                    let start = Instant::now();
                    assert!(matches!(
                        cause(&supervisor.poll().unwrap_err()),
                        WorkerError::HandshakeTimeout
                    ));
                    bounded(start, Duration::from_secs(1));
                    assert_eq!(supervisor.restarts_used(), used);
                    assert_eq!(supervisor.last_failure(), Some(WorkerFailure::Launch));
                }
                assert!(matches!(
                    supervisor.poll(),
                    Err(WorkerError::RestartBudgetExhausted { used: 2, max: 2 })
                ));
                assert_eq!(std::fs::read_to_string(&path).unwrap(), "3");
            }
            supervisor.shutdown();
            std::fs::remove_file(path).unwrap();
        }
        "failed_launches" => {
            use std::os::unix::fs::PermissionsExt;
            let path = temp("interpreter");
            std::fs::write(&path, "#!/bin/sh\nexec python3 \"$@\"\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let cfg = WorkerConfig::new(
                &path,
                format!("{}/tests/workers/heartbeat.py", env!("CARGO_MANIFEST_DIR")),
            )
            .with_arg("hang")
            .with_heartbeat(HeartbeatPolicy::new(INTERVAL, RESPONSE).unwrap());
            let mut supervisor = WorkerSupervisor::start(cfg, 2).unwrap();
            std::fs::remove_file(path).unwrap();
            until(supervisor.worker().unwrap().heartbeat_due());
            supervisor.poll().unwrap_err();
            for used in 1..=2 {
                assert!(matches!(supervisor.poll(), Err(WorkerError::Launch(_))));
                assert_eq!(supervisor.restarts_used(), used);
            }
            assert!(matches!(
                supervisor.poll(),
                Err(WorkerError::RestartBudgetExhausted { used: 2, max: 2 })
            ));
        }
        "shutdown_and_drop" => {
            let mut supervisor = WorkerSupervisor::start(config("hang"), 2).unwrap();
            until(supervisor.worker().unwrap().heartbeat_due());
            supervisor.poll().unwrap_err();
            let report = supervisor.worker().unwrap().cleanup_report().unwrap();
            let start = Instant::now();
            assert_eq!(supervisor.shutdown(), Some(report));
            assert_eq!(
                supervisor.last_failure(),
                Some(WorkerFailure::HeartbeatTimeout)
            );
            assert!(matches!(supervisor.poll(), Err(WorkerError::Unavailable)));
            assert!(matches!(
                supervisor.ensure_alive(),
                Err(WorkerError::Unavailable)
            ));
            assert_eq!(supervisor.restarts_used(), 0);
            drop(supervisor);
            bounded(start, RESERVE);
            let mut worker = PythonWorker::launch(&config("hang")).unwrap();
            until(worker.heartbeat_due());
            worker.check_heartbeat().unwrap_err();
            let start = Instant::now();
            drop(worker);
            bounded(start, RESERVE);
        }
        other => panic!("unknown scenario {other}"),
    }
}
