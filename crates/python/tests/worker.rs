//! Integration tests for isolated Python worker supervision.
//!
//! These spawn a real `python3` process. If no interpreter is available the
//! tests skip cleanly so the workspace suite still passes.

use std::process::Command;
use std::time::Duration;

use neuradix_python::{PythonWorker, WorkerConfig, WorkerError, WorkerSupervisor};
use neuradix_runtime::HealthState;
use serde_json::json;

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn base_config() -> WorkerConfig {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let script = format!("{manifest}/tests/workers/testkit_worker.py");
    let python_dir = format!("{manifest}/../../python");
    WorkerConfig::new("python3", script)
        .with_python_path(python_dir)
        .with_config(json!({ "threshold": 12.0 }))
}

#[test]
fn round_trips_a_request_and_passes_config() {
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let mut worker = PythonWorker::launch(&base_config()).expect("launch");
    assert_eq!(worker.ready_info().name, "testkit-worker");
    assert_eq!(worker.health(), HealthState::Healthy);

    let response = worker.send(json!({ "depth": 3.0 })).expect("send");
    assert_eq!(response["echo"]["depth"], 3.0);
    assert_eq!(response["config"]["threshold"], 12.0);

    worker.shutdown();
}

#[test]
fn python_crash_is_isolated_and_recoverable() {
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let mut worker = PythonWorker::launch(&base_config()).expect("launch");

    // A hard crash inside the worker surfaces as a recoverable error, NOT a
    // crash of this (the supervising) process.
    let err = worker
        .send(json!({ "crash": true }))
        .expect_err("worker should die");
    assert!(
        matches!(err, WorkerError::WorkerExited { .. }),
        "got {err:?}"
    );
    assert_eq!(worker.health(), HealthState::Unavailable);

    // We are still running (this line executes) — isolation holds. A fresh
    // worker launches and works normally.
    let mut replacement = PythonWorker::launch(&base_config()).expect("relaunch");
    let response = replacement.send(json!({ "depth": 1.0 })).expect("send");
    assert_eq!(response["echo"]["depth"], 1.0);
    replacement.shutdown();
}

#[test]
fn supervisor_restarts_within_budget_then_gives_up() {
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let mut supervisor = WorkerSupervisor::start(base_config(), 1).expect("start");

    // Crash #1 -> a restart is available.
    let _ = supervisor.worker().unwrap().send(json!({ "crash": true }));
    supervisor
        .ensure_alive()
        .expect("first restart within budget");
    assert_eq!(supervisor.restarts_used(), 1);
    let response = supervisor
        .worker()
        .unwrap()
        .send(json!({ "depth": 2.0 }))
        .expect("send");
    assert_eq!(response["echo"]["depth"], 2.0);

    // Crash #2 -> budget exhausted.
    let _ = supervisor.worker().unwrap().send(json!({ "crash": true }));
    let err = supervisor.ensure_alive().expect_err("budget exhausted");
    assert!(
        matches!(err, WorkerError::RestartBudgetExhausted { used: 1, max: 1 }),
        "got {err:?}"
    );

    supervisor.shutdown();
}

#[test]
fn a_slow_request_times_out_without_killing_the_supervisor() {
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let config = base_config().with_request_timeout(Duration::from_millis(200));
    let mut worker = PythonWorker::launch(&config).expect("launch");

    let err = worker
        .send(json!({ "sleep": 1.0 }))
        .expect_err("should time out");
    assert!(matches!(err, WorkerError::Timeout), "got {err:?}");

    worker.shutdown(); // still controllable after a timeout
}
