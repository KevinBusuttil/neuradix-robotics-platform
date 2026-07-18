//! # neuradix-python
//!
//! Managed, **isolated** Python worker processes.
//!
//! A Python component runs as a separate OS process supervised from Rust over a
//! newline-delimited JSON protocol. This upholds the platform's Python rules
//! (§19.4): Python runs outside the deterministic control path, and a Python
//! **crash is isolated** — it surfaces as a recoverable
//! [`WorkerError::WorkerExited`], never as a crash of the runtime (v1.0
//! acceptance criterion §41.6).
//!
//! The [`WorkerSupervisor`] adds a bounded restart budget so a flapping worker
//! cannot restart forever. A worker's [`neuradix_runtime::HealthState`] can be
//! fed to `neuradix_safety::FdirMonitor` so a Python crash drives the system to
//! a safe mode.
//!
//! The companion Python library `python/neuradix_worker.py` provides a native
//! `run(handler)` loop so a component author writes ordinary Python.
//!
//! ## Not yet implemented
//!
//! In-process PyO3/Maturin bindings with NumPy zero-copy views (§19.1–§19.2) are
//! a planned enhancement; this increment delivers the process-isolation
//! supervisor and the crash-safety guarantee first.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod supervisor;
pub mod worker;

pub use error::WorkerError;
pub use supervisor::WorkerSupervisor;
pub use worker::{PythonWorker, ReadyInfo, WorkerConfig};
