//! Typed Python-worker errors.

/// Errors from launching, supervising or exchanging with a Python worker.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// Invalid immutable I/O or timeout configuration.
    #[error("invalid worker configuration: {0}")]
    InvalidConfig(&'static str),

    /// This increment implements bounded stdio only on Linux GNU/musl.
    #[error("bounded Python worker I/O is unsupported on this platform")]
    UnsupportedPlatform,

    /// The 32 live/deferred process slots are exhausted or admission is busy.
    #[error("worker process admission is full or busy")]
    ProcessCapacity,

    /// Another reaper consumed a child owned by this library.
    #[error("worker child ownership was lost to an external reaper")]
    ProcessOwnershipLost,

    /// The stdout stream closed while the direct child was still alive.
    #[error("worker closed stdout while remaining alive")]
    StdoutClosed,

    /// A terminally failed or shut-down worker cannot accept more requests.
    #[error("worker is unavailable; restart it within the supervision budget")]
    Unavailable,

    /// The synchronous sequence space cannot wrap.
    #[error("worker request sequence exhausted")]
    SequenceExhausted,

    /// Incoming line exceeded the validated byte limit.
    #[error("incoming worker line exceeds {limit} bytes")]
    IncomingTooLarge {
        /// Inclusive newline-counted limit.
        limit: usize,
    },

    /// Outgoing line exceeded the validated byte limit before any write.
    #[error("outgoing worker line exceeds {limit} bytes")]
    OutgoingTooLarge {
        /// Inclusive newline-counted limit.
        limit: usize,
    },

    /// Complete and partial incoming lines exceeded the storage budget.
    #[error("worker receive storage exceeds {limit} bytes")]
    QueueBytesExceeded {
        /// Maximum queued bytes.
        limit: usize,
    },

    /// An incoming burst exceeded the complete-message queue budget.
    #[error("worker receive queue exceeds {limit} messages")]
    QueueMessagesExceeded {
        /// Maximum complete queued lines.
        limit: usize,
    },

    /// Cleanup could not establish normal direct-child reaping. The original
    /// operation failure and bounded cleanup outcome are retained together.
    #[error("{cause}; cleanup: {report:?}")]
    Cleanup {
        /// Original failure.
        #[source]
        cause: Box<WorkerError>,
        /// Cleanup result, including deferred-reaper admission if needed.
        report: crate::CleanupReport,
    },
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
