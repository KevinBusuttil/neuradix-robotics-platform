//! Bounded restart supervision for a Python worker.

use neuradix_runtime::HealthState;

use crate::error::WorkerError;
use crate::worker::{PythonWorker, WorkerConfig};

/// Supervises a single Python worker with a bounded restart budget, so a
/// crashing (flapping) worker cannot restart forever.
pub struct WorkerSupervisor {
    config: WorkerConfig,
    worker: Option<PythonWorker>,
    restarts_used: u32,
    max_restarts: u32,
}

impl WorkerSupervisor {
    /// Launch a worker under supervision, allowing up to `max_restarts` restarts.
    pub fn start(config: WorkerConfig, max_restarts: u32) -> Result<Self, WorkerError> {
        let worker = PythonWorker::launch(&config)?;
        Ok(Self {
            config,
            worker: Some(worker),
            restarts_used: 0,
            max_restarts,
        })
    }

    /// The number of restarts used so far.
    pub fn restarts_used(&self) -> u32 {
        self.restarts_used
    }

    /// The current worker, if one is running.
    pub fn worker(&mut self) -> Option<&mut PythonWorker> {
        self.worker.as_mut()
    }

    /// The supervised worker's health, or [`HealthState::Unavailable`] if none is
    /// currently running.
    pub fn health(&mut self) -> HealthState {
        match self.worker.as_mut() {
            Some(worker) => worker.health(),
            None => HealthState::Unavailable,
        }
    }

    /// Ensure a live worker exists, restarting a dead one if the budget permits.
    ///
    /// Returns `Ok(())` if a live worker is available afterwards, or
    /// [`WorkerError::RestartBudgetExhausted`] if the worker is dead and no
    /// restarts remain.
    pub fn ensure_alive(&mut self) -> Result<(), WorkerError> {
        let alive = self.worker.as_mut().is_some_and(PythonWorker::is_running);
        if alive {
            return Ok(());
        }

        // Reap the dead worker (if any).
        if let Some(mut dead) = self.worker.take() {
            dead.shutdown();
        }

        if self.restarts_used >= self.max_restarts {
            return Err(WorkerError::RestartBudgetExhausted {
                used: self.restarts_used,
                max: self.max_restarts,
            });
        }

        let worker = PythonWorker::launch(&self.config)?;
        self.restarts_used += 1;
        self.worker = Some(worker);
        Ok(())
    }

    /// Shut the supervised worker down.
    pub fn shutdown(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.shutdown();
        }
    }
}
