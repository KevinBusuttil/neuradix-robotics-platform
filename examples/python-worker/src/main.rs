//! Isolated Python worker example.
//!
//! Runs an ordinary Python component (`detector.py`) as a supervised, isolated
//! process, then demonstrates the platform's Python crash-safety guarantee
//! (§41.6): a Python crash is detected, drives an FDIR monitor to a safe mode,
//! and the Rust runtime survives and restarts the worker within its budget.
//!
//! Unlike `minimal-depth-stream`, this example is intentionally NOT deterministic
//! — it drives a real OS process in real time (Python runs outside the
//! deterministic executor, §19.4).
#![forbid(unsafe_code)]

use std::error::Error;
use std::path::PathBuf;

use neuradix_python::{WorkerConfig, WorkerSupervisor};
use neuradix_runtime::HealthState;
use neuradix_safety::{FdirMonitor, FdirPolicy};
use neuradix_time::{ClockDomain, Timestamp};
use serde_json::json;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let config = WorkerConfig::new("python3", PathBuf::from(format!("{manifest}/detector.py")))
        .with_python_path(PathBuf::from(format!("{manifest}/../../python")))
        .with_config(json!({ "threshold": 12.0 }));

    println!("Neuradix — isolated Python worker example");

    // Supervise the worker with a one-restart budget.
    let mut supervisor = WorkerSupervisor::start(config, 1)?;
    let (name, skip_policy) = {
        let info = supervisor.worker().unwrap().ready_info();
        (info.name.clone(), info.skip_policy.clone())
    };
    println!("  worker   : {name} (skip policy: {skip_policy})");

    // 1. Normal operation: the Python component classifies depth samples.
    println!("\ndetection");
    for depth in [10.0_f64, 13.0, 11.5] {
        let out = supervisor
            .worker()
            .unwrap()
            .send(json!({ "depth": depth }))?;
        println!(
            "  depth={:.1}m -> belowThreshold={}",
            depth, out["belowThreshold"]
        );
    }
    println!("  health   : {}", supervisor.health());

    // 2. Crash isolation + FDIR safing.
    println!("\ncrash isolation");
    let mut fdir = FdirMonitor::new(FdirPolicy::new(1, 2, 3));
    let mut tick = 0i128;
    let mut next_ts = || {
        tick += 1;
        Timestamp::new(ClockDomain::Monotonic, tick * 1_000_000)
    };

    // Trigger a hard crash inside the Python process.
    let crash = supervisor.worker().unwrap().send(json!({ "crash": true }));
    println!("  sent crash request -> {}", describe(&crash));
    let health = supervisor.health();
    println!("  worker health: {health}");
    if health != HealthState::Healthy
        && let Some(t) = fdir.observe(health, next_ts())
    {
        println!("  FDIR: {} -> {} ({})", t.from, t.to, t.reason);
    }
    println!("  runtime is still alive (this line printed) — crash isolated");

    // 3. Recovery within the restart budget.
    println!("\nrecovery");
    supervisor.ensure_alive()?;
    println!(
        "  restarted worker (restarts used: {})",
        supervisor.restarts_used()
    );
    let out = supervisor.worker().unwrap().send(json!({ "depth": 9.0 }))?;
    println!("  depth=9.0m -> belowThreshold={}", out["belowThreshold"]);

    supervisor.shutdown();
    println!("\ndone.");
    Ok(())
}

fn describe(result: &Result<serde_json::Value, neuradix_python::WorkerError>) -> String {
    match result {
        Ok(value) => format!("ok: {value}"),
        Err(e) => format!("{e}"),
    }
}
