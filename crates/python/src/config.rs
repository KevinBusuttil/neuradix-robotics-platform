//! Validated worker I/O and deadline configuration.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::WorkerError;

/// Immutable wire/storage limits. Line sizes include the terminating newline.
/// One request is outstanding at a time; there is no outbound request queue.
///
/// ```compile_fail
/// use neuradix_python::IoLimits;
/// let mut limits = IoLimits::default();
/// limits.incoming_bytes = usize::MAX;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IoLimits {
    incoming_bytes: usize,
    outgoing_bytes: usize,
    queued_bytes: usize,
    queued_messages: usize,
}
impl IoLimits {
    /// Validate line limits (64 bytes..=1 MiB), receive storage (one incoming
    /// line..=4 MiB) and complete queued messages (1..=128).
    pub fn new(
        incoming_bytes: usize,
        outgoing_bytes: usize,
        queued_bytes: usize,
        queued_messages: usize,
    ) -> Result<Self, WorkerError> {
        if !(64..=1_048_576).contains(&incoming_bytes)
            || !(64..=1_048_576).contains(&outgoing_bytes)
            || !(incoming_bytes..=4_194_304).contains(&queued_bytes)
            || !(1..=128).contains(&queued_messages)
        {
            return Err(WorkerError::InvalidConfig("invalid line or queue limits"));
        }
        Ok(Self {
            incoming_bytes,
            outgoing_bytes,
            queued_bytes,
            queued_messages,
        })
    }
    /// Maximum incoming JSON line bytes, including newline.
    pub fn incoming_bytes(self) -> usize {
        self.incoming_bytes
    }
    /// Maximum outgoing JSON line bytes, including newline.
    pub fn outgoing_bytes(self) -> usize {
        self.outgoing_bytes
    }
    /// Maximum receive storage, including complete lines and the partial line.
    pub fn queued_bytes(self) -> usize {
        self.queued_bytes
    }
    /// Maximum complete lines waiting in receive storage.
    pub fn queued_messages(self) -> usize {
        self.queued_messages
    }
    /// Exactly one synchronous request; concurrent access requires exclusive ownership.
    pub fn outstanding_requests(self) -> usize {
        1
    }
}
impl Default for IoLimits {
    fn default() -> Self {
        Self::new(65_536, 65_536, 262_144, 16).expect("valid defaults")
    }
}

/// Total operation timeouts and cleanup reserve, all immutable and validated.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    handshake: Duration,
    request: Duration,
    shutdown: Duration,
    cleanup: Duration,
}
impl Timeouts {
    /// Each total must be greater than the nonzero cleanup reserve and at most
    /// 60 seconds. Cleanup is at most one second. No timeout restarts mid-operation.
    pub fn new(
        handshake: Duration,
        request: Duration,
        shutdown: Duration,
        cleanup: Duration,
    ) -> Result<Self, WorkerError> {
        if cleanup.is_zero()
            || cleanup > Duration::from_secs(1)
            || [handshake, request, shutdown]
                .into_iter()
                .any(|t| t <= cleanup || t > Duration::from_secs(60))
        {
            return Err(WorkerError::InvalidConfig(
                "invalid total timeout or cleanup reserve",
            ));
        }
        Ok(Self {
            handshake,
            request,
            shutdown,
            cleanup,
        })
    }
    /// Total launch/handshake operation budget, including cleanup.
    pub fn handshake(self) -> Duration {
        self.handshake
    }
    /// Total request operation budget, including serialization and cleanup.
    pub fn request(self) -> Duration {
        self.request
    }
    /// Total explicit graceful-shutdown budget, including cleanup.
    pub fn shutdown(self) -> Duration {
        self.shutdown
    }
    /// Reserved cleanup budget; also the entire Drop budget.
    pub fn cleanup(self) -> Duration {
        self.cleanup
    }
}
impl Default for Timeouts {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_millis(500),
            Duration::from_millis(50),
        )
        .expect("valid defaults")
    }
}

/// Launch configuration. Operational invariants can only be changed through
/// validated value types. Launch arguments and the executable are trusted input.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub(crate) interpreter: PathBuf,
    pub(crate) script: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) python_path: Vec<PathBuf>,
    pub(crate) config: Value,
    pub(crate) skip_inputs: bool,
    pub(crate) limits: IoLimits,
    pub(crate) timeouts: Timeouts,
}
impl WorkerConfig {
    /// Configure an interpreter and script with bounded defaults.
    pub fn new(interpreter: impl Into<PathBuf>, script: impl Into<PathBuf>) -> Self {
        Self {
            interpreter: interpreter.into(),
            script: script.into(),
            args: Vec::new(),
            python_path: Vec::new(),
            config: Value::Null,
            skip_inputs: true,
            limits: IoLimits::default(),
            timeouts: Timeouts::default(),
        }
    }
    /// Set trusted structured startup configuration; launch bounds its encoding.
    pub fn with_config(mut self, config: Value) -> Self {
        self.config = config;
        self
    }
    /// Add a Python import directory.
    pub fn with_python_path(mut self, dir: impl Into<PathBuf>) -> Self {
        self.python_path.push(dir.into());
        self
    }
    /// Add a trusted script argument.
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }
    /// Set the declared input-skip policy.
    pub fn with_skip_inputs(mut self, skip: bool) -> Self {
        self.skip_inputs = skip;
        self
    }
    /// Install validated immutable I/O limits.
    pub fn with_limits(mut self, limits: IoLimits) -> Self {
        self.limits = limits;
        self
    }
    /// Install validated immutable total timeouts.
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
    /// Validate a new total request timeout. This builder now returns a Result.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, WorkerError> {
        self.timeouts = Timeouts::new(
            self.timeouts.handshake,
            timeout,
            self.timeouts.shutdown,
            self.timeouts.cleanup,
        )?;
        Ok(self)
    }
    /// Configured immutable I/O limits.
    pub fn limits(&self) -> IoLimits {
        self.limits
    }
    /// Configured immutable operation budgets.
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_storage_and_deadline_configurations_are_rejected() {
        for (incoming, outgoing, bytes, messages) in [
            (0, 64, 64, 1),
            (64, usize::MAX, 64, 1),
            (128, 64, 127, 1),
            (64, 64, usize::MAX, 1),
            (64, 64, 64, 0),
            (64, 64, 64, 129),
        ] {
            assert!(IoLimits::new(incoming, outgoing, bytes, messages).is_err());
        }
        let total = Duration::from_millis(100);
        for reserve in [Duration::ZERO, total, Duration::from_secs(2)] {
            assert!(Timeouts::new(total, total, total, reserve).is_err());
        }
        assert!(
            WorkerConfig::new("python3", "worker.py")
                .with_request_timeout(Duration::ZERO)
                .is_err()
        );
        assert_eq!(IoLimits::default().outstanding_requests(), 1);
    }
}
