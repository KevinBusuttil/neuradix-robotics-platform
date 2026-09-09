//! Bounded restart supervision for a Python worker.

use neuradix_runtime::HealthState;

use crate::error::WorkerError;
use crate::worker::{PythonWorker, WorkerConfig};
use crate::WorkerFailure;

/// Supervises a single Python worker with a bounded restart budget, so a
/// crashing (flapping) worker cannot restart forever.
pub struct WorkerSupervisor {
    config: WorkerConfig,
    worker: Option<PythonWorker>,
    restarts_used: u32,
    max_restarts: u32,
    last_failure: Option<WorkerFailure>,
    stopped: bool,
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
            last_failure: None,
            stopped: false,
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
            Some(worker) => {
                let health = worker.health();
                if let Some(reason) = worker.last_failure() {
                    self.last_failure = Some(reason);
                }
                health
            }
            None => HealthState::Unavailable,
        }
    }

    /// Most recent terminal worker or replacement-launch failure. Retained after
    /// replacement and shutdown for audit; this is not the new worker's health.
    pub fn last_failure(&self) -> Option<WorkerFailure> {
        self.worker.as_ref().and_then(PythonWorker::last_failure).or(self.last_failure)
    }

    /// Periodic supervision outside conventional control. A call performs at
    /// most one heartbeat OR replacement launch. A newly detected failure is
    /// returned before any recovery; a later call spends one restart attempt.
    /// Call at or before heartbeat_due even with no ordinary requests. A ready
    /// replacement is Unknown until it produces a matching timely reply.
    pub fn poll(&mut self) -> Result<HealthState, WorkerError> {
        if self.stopped {
            return Err(WorkerError::Unavailable);
        }
        if let Some(worker) = self.worker.as_mut().filter(|worker| worker.available()) {
            if let Err(error) = worker.check_heartbeat() {
                self.last_failure = worker.last_failure();
                return Err(error);
            }
            return Ok(worker.health());
        }
        self.ensure_alive()?;
        Ok(self.health())
    }

    /// Ensure a usable worker exists, restarting a previously failed one if the
    /// budget permits. A failure first observed here is returned before recovery;
    /// retry on a later supervision call. This does not issue idle heartbeats:
    /// use poll for periodic responsiveness checks.
    ///
    /// Returns `Ok(())` if a live worker is available afterwards, or
    /// [`WorkerError::RestartBudgetExhausted`] if the worker is dead and no
    /// restarts remain.
    pub fn ensure_alive(&mut self) -> Result<(), WorkerError> {
        if self.stopped {
            return Err(WorkerError::Unavailable);
        }
        if let Some(worker) = self.worker.as_mut().filter(|worker| worker.available()) {
            let result = worker.check_session().map(|_| ());
            if result.is_err() {
                self.last_failure = worker.last_failure();
            }
            return result;
        }

        // Reap the dead worker (if any).
        if let Some(mut dead) = self.worker.take() {
            self.last_failure = dead.last_failure().or(self.last_failure);
            dead.shutdown();
        }

        if self.restarts_used >= self.max_restarts {
            return Err(WorkerError::RestartBudgetExhausted {
                used: self.restarts_used,
                max: self.max_restarts,
            });
        }

        // Charge failed launch/handshake attempts to the same restart budget.
        self.restarts_used += 1;
        let worker = PythonWorker::launch(&self.config).inspect_err(|_| {
            self.last_failure = Some(WorkerFailure::Launch);
        })?;
        self.worker = Some(worker);
        Ok(())
    }

    /// Shut the supervised worker down.
    pub fn shutdown(&mut self) -> Option<crate::CleanupReport> {
        self.stopped = true;
        self.worker.take().map(|mut worker| {
            let report = worker.shutdown();
            self.last_failure = worker.last_failure().or(self.last_failure);
            report
        })
    }
}
