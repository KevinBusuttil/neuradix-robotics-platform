//! Unsupported platforms must fail before launching a worker.
#![cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]

use neuradix_python::{PythonWorker, WorkerConfig, WorkerError};

#[test]
fn unsupported_backend_rejects_before_spawn() {
    let config = WorkerConfig::new("does-not-exist", "does-not-exist.py");
    assert!(matches!(
        PythonWorker::launch(&config),
        Err(WorkerError::UnsupportedPlatform)
    ));
}
