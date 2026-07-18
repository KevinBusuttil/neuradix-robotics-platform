//! Typed Python-worker errors.

/// Errors from launching, supervising or exchanging with a Python worker.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// The worker process could not be launched.
    #[error("failed to launch python worker: {0}")]
    Launch(#[source] std::io::Error),

    /// The worker did not complete its startup handshake in time.
    #[error("python worker did not hand shake within the timeout")]
    HandshakeTimeout,

    /// The worker process exited unexpectedly. Crucially, this is a normal,
    /// recoverable error on the Rust side — a Python crash does not crash the
    /// supervisor.
    #[error("python worker exited unexpectedly ({status})")]
    WorkerExited {
        /// A description of the exit status.
        status: String,
    },

    /// A request exceeded its timeout while the worker was still running.
    #[error("python worker request timed out")]
    Timeout,

    /// The worker sent a message that did not match the protocol.
    #[error("python worker protocol error: {0}")]
    Protocol(String),

    /// The worker reported an application error while handling a request.
    #[error("python worker error: {0}")]
    Remote(String),

    /// The restart budget was exhausted; the worker cannot be recovered.
    #[error("python worker restart budget exhausted ({used}/{max})")]
    RestartBudgetExhausted {
        /// Restarts used.
        used: u32,
        /// Maximum restarts.
        max: u32,
    },

    /// An I/O error occurred communicating with the worker.
    #[error("python worker i/o error: {0}")]
    Io(#[from] std::io::Error),
}
