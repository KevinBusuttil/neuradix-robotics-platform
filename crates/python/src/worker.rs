//! Synchronous bounded Python-worker stdio, outside the local control executor.

pub use crate::config::WorkerConfig;
use crate::heartbeat::Heartbeat;
use crate::process::{Process, pause_until};
use crate::protocol::{Frames, encode};
use crate::{CleanupReport, CleanupState, Timeouts, WorkerError, WorkerFailure};
use neuradix_runtime::HealthState;
use serde_json::{Value, json};
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Deadline {
    io: Instant,
    end: Instant,
    reserve: Duration,
}

fn handshake_error(error: WorkerError) -> WorkerError {
    match error {
        WorkerError::Timeout => WorkerError::HandshakeTimeout,
        other => other,
    }
}
impl Deadline {
    fn new(total: Duration, reserve: Duration) -> Result<Self, WorkerError> {
        let start = Instant::now();
        let end = start
            .checked_add(total)
            .ok_or(WorkerError::InvalidConfig("deadline overflow"))?;
        let io = end
            .checked_sub(reserve)
            .ok_or(WorkerError::InvalidConfig("cleanup reserve overflow"))?;
        Ok(Self { io, end, reserve })
    }
    fn check(&self) -> Result<(), WorkerError> {
        self.check_at(Instant::now())
    }
    fn check_at(&self, now: Instant) -> Result<(), WorkerError> {
        if now >= self.io {
            Err(WorkerError::Timeout)
        } else {
            Ok(())
        }
    }
    fn heartbeat(io: Instant, reserve: Duration) -> Result<Self, WorkerError> {
        let end = io
            .checked_add(reserve)
            .ok_or(WorkerError::SupervisionClock)?;
        Ok(Self { io, end, reserve })
    }
    fn cap_io(mut self, expiry: Instant) -> Result<Self, WorkerError> {
        self.io = self.io.min(expiry);
        self.end = self.end.min(
            self.io
                .checked_add(self.reserve)
                .ok_or(WorkerError::SupervisionClock)?,
        );
        Ok(self)
    }
    fn cleanup_end(&self) -> Instant {
        Instant::now()
            .checked_add(self.reserve)
            .unwrap_or(self.end)
            .min(self.end)
    }
}

/// Information reported by the worker at startup, bounded by the incoming line.
#[derive(Debug, Clone)]
pub struct ReadyInfo {
    /// Worker-declared component name (not authenticated identity).
    pub name: String,
    /// Worker-declared input-skip policy.
    pub skip_policy: String,
}
/// Protocol storage high-water marks. Kernel buffers and the caller's existing
/// Value are separate from these user-space budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoStats {
    /// Maximum complete/partial receive bytes stored together.
    pub queued_bytes: usize,
    /// Maximum complete lines queued together.
    pub queued_messages: usize,
    /// Maximum encoded outbound line length.
    pub outgoing_bytes: usize,
}

/// One managed worker; exactly one request may be outstanding through `&mut self`.
/// No reader/writer thread or unbounded channel is created per worker.
/// Total deadlines include serialization, nonblocking write, response processing
/// and reserved cleanup. Fatal I/O/protocol failures terminate the session.
/// Application Remote errors and local pre-write serialization rejections leave
/// the session usable. Linux GNU/musl only; OS scheduling and syscall latency
/// are not hard-real-time guarantees. Keep calls outside local control.
pub struct PythonWorker {
    process: Process,
    frames: Frames,
    sequence: u64,
    outgoing_limit: usize,
    outgoing_high: usize,
    timeouts: Timeouts,
    ready: ReadyInfo,
    unavailable: bool,
    cleanup: Option<CleanupReport>,
    heartbeat: Heartbeat,
}
impl PythonWorker {
    /// Launch and handshake within one total budget. This library must exclusively
    /// own child reaping (no external waitpid(-1) or SIGCHLD auto-reaping).
    pub fn launch(config: &WorkerConfig) -> Result<Self, WorkerError> {
        if !cfg!(all(target_os = "linux", not(target_env = "uclibc"))) {
            return Err(WorkerError::UnsupportedPlatform);
        }
        let deadline = Deadline::new(config.timeouts.handshake(), config.timeouts.cleanup())?;
        let encoded_config = encode(
            &config.config,
            None,
            config.limits.outgoing_bytes(),
            deadline.io,
        )
        .map_err(handshake_error)?;
        let mut command = Command::new(&config.interpreter);
        command
            .arg(&config.script)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if !config.python_path.is_empty() {
            let joined = std::env::join_paths(&config.python_path)
                .map_err(|_| WorkerError::InvalidConfig("invalid Python import path"))?;
            command.env("PYTHONPATH", joined);
        }
        command.env(
            "NEURADIX_WORKER_CONFIG",
            std::str::from_utf8(&encoded_config).expect("JSON UTF-8"),
        );
        command.env(
            "NEURADIX_WORKER_SKIP_INPUTS",
            config.skip_inputs.to_string(),
        );
        command.env(
            "NEURADIX_WORKER_MAX_INPUT_BYTES",
            config.limits.outgoing_bytes().to_string(),
        );
        command.env(
            "NEURADIX_WORKER_MAX_OUTPUT_BYTES",
            config.limits.incoming_bytes().to_string(),
        );
        deadline.check().map_err(handshake_error)?;
        let heartbeat = Heartbeat::new(config.heartbeat, Instant::now())?;
        let process = Process::spawn(&mut command, deadline.cleanup_end())?;
        let mut worker = Self {
            process,
            frames: Frames::new(config.limits),
            sequence: 0,
            outgoing_limit: config.limits.outgoing_bytes(),
            outgoing_high: 0,
            timeouts: config.timeouts,
            ready: ReadyInfo {
                name: String::new(),
                skip_policy: String::new(),
            },
            unavailable: false,
            cleanup: None,
            heartbeat,
        };
        let handshake = worker.receive(&deadline).and_then(|value| {
            if value.get("kind").and_then(Value::as_str) != Some("ready") {
                return Err(WorkerError::Protocol("expected ready handshake".into()));
            }
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| WorkerError::Protocol("ready.name must be a string".into()))?;
            let skip_policy = value
                .get("skipPolicy")
                .and_then(Value::as_str)
                .ok_or_else(|| WorkerError::Protocol("ready.skipPolicy must be a string".into()))?;
            worker.ready = ReadyInfo {
                name: name.to_owned(),
                skip_policy: skip_policy.to_owned(),
            };
            deadline.check()?;
            // A ready declaration starts the initial scheduling window but is
            // not responsiveness confirmation. No worker time is accepted.
            worker.heartbeat = Heartbeat::new(config.heartbeat, Instant::now())?;
            Ok(())
        });
        if let Err(error) = handshake {
            return Err(worker.fail(handshake_error(error), &deadline));
        }
        Ok(worker)
    }
    /// Bounded startup metadata; worker declarations grant no authority.
    pub fn ready_info(&self) -> &ReadyInfo {
        &self.ready
    }
    /// Direct child PID, for diagnosis (do not reap it externally).
    pub fn process_id(&self) -> u32 {
        self.process.id()
    }
    /// Most recent bounded cleanup result, including deferred reaping.
    pub fn cleanup_report(&self) -> Option<CleanupReport> {
        self.cleanup
    }
    /// Actual receive/outbound high-water marks, retained after cleanup.
    pub fn io_stats(&self) -> IoStats {
        IoStats {
            queued_bytes: self.frames.high_bytes,
            queued_messages: self.frames.high_messages,
            outgoing_bytes: self.outgoing_high,
        }
    }
    /// First terminal failure, retained through shutdown and late traffic.
    pub fn last_failure(&self) -> Option<WorkerFailure> {
        self.heartbeat.failure()
    }
    /// Next monotonic probe due time. Call check_heartbeat even during idle
    /// periods; ordinary replies may move this time forward.
    pub fn heartbeat_due(&self) -> Instant {
        self.heartbeat.due()
    }
    /// Absolute responsiveness expiry; equality is expired. A delayed poll
    /// receives only the remaining response budget, never a new full timeout.
    pub fn heartbeat_expires(&self) -> Instant {
        self.heartbeat.expires()
    }
    pub(crate) fn available(&self) -> bool {
        !self.unavailable
    }
    pub(crate) fn check_session(&mut self) -> Result<HealthState, WorkerError> {
        if self.unavailable {
            return Err(WorkerError::Unavailable);
        }
        let result = self.heartbeat.observe(Instant::now()).and_then(|state| {
            if self.process.has_exited()? {
                Err(WorkerError::WorkerExited {
                    status: "observed exit".into(),
                })
            } else {
                Ok(state)
            }
        });
        result.map_err(|error| self.retire(error))
    }

    /// Perform at most one due ping/pong operation, returning true if confirmed.
    /// Before due time this does no pipe I/O. Poll outside local control, at due
    /// time or earlier; lateness shortens the anchored response window. No
    /// background task exists. A failure retires this session until replacement.
    pub fn check_heartbeat(&mut self) -> Result<bool, WorkerError> {
        self.check_session()?;
        if Instant::now() < self.heartbeat.due() {
            return Ok(false);
        }
        let deadline = Deadline::heartbeat(self.heartbeat.expires(), self.timeouts.cleanup())
            .map_err(|error| self.retire(error))?;
        self.exchange(None, deadline).map(|_| true)
    }

    /// Send a borrowed payload without cloning its tree. Encoding is bounded
    /// before any write. Sequences start at 1, never wrap and are consumed only
    /// when a write is attempted. Unrelated lines never extend the deadline.
    /// The request's total budget is capped by current responsiveness expiry.
    /// A matching timely response or Remote error confirms responsiveness.
    pub fn send(&mut self, payload: &Value) -> Result<Value, WorkerError> {
        self.check_session()?;
        let deadline = Deadline::new(self.timeouts.request(), self.timeouts.cleanup())?
            .cap_io(self.heartbeat.expires())
            .map_err(|error| self.retire(error))?;
        self.exchange(Some(payload), deadline)
    }
    fn exchange(
        &mut self,
        payload: Option<&Value>,
        deadline: Deadline,
    ) -> Result<Value, WorkerError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(WorkerError::SequenceExhausted)
            .map_err(|error| self.fail(error, &deadline))?;
        let encoded = match payload {
            Some(payload) => encode(payload, Some(sequence), self.outgoing_limit, deadline.io),
            None => encode(
                &json!({"kind": "ping", "seq": sequence}),
                None,
                self.outgoing_limit,
                deadline.io,
            ),
        };
        let line = match encoded {
            Ok(line) => line,
            Err(WorkerError::Timeout) => {
                let error = if payload.is_none() {
                    WorkerError::HeartbeatTimeout
                } else {
                    WorkerError::Timeout
                };
                return Err(self.fail(error, &deadline));
            }
            Err(error) => return Err(error),
        };
        self.outgoing_high = self.outgoing_high.max(line.len());
        let result = (|| {
            // Replies already present before issue are not responsiveness
            // evidence. Drain bounded stale traffic, including partial lines,
            // under this same absolute deadline before attempting a write.
            self.drain_before_issue(&deadline)?;
            self.sequence = sequence;
            self.write(&line, &deadline)?;
            loop {
                let mut value = self.receive(&deadline)?;
                if value.get("seq").and_then(Value::as_u64) != Some(sequence) {
                    continue;
                }
                match (payload.is_some(), value.get("kind").and_then(Value::as_str)) {
                    (false, Some("pong")) => {
                        deadline.check()?;
                        return Ok(Value::Null);
                    }
                    (true, Some("response")) => {
                        deadline.check()?;
                        return Ok(value
                            .get_mut("payload")
                            .map(Value::take)
                            .unwrap_or(Value::Null));
                    }
                    (true, Some("error")) => {
                        let message = value
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unspecified worker error")
                            .to_owned();
                        deadline.check()?;
                        return Err(WorkerError::Remote(message));
                    }
                    _ => {}
                }
            }
        })();
        // Both application responses and matching Remote errors demonstrate
        // loop/handler responsiveness, not application correctness. Pongs are
        // accepted only for a ping using this session's next sequence.
        let completed_at = Instant::now();
        if (result.is_ok() || matches!(&result, Err(WorkerError::Remote(_))))
            && let Err(error) = deadline
                .check_at(completed_at)
                .and_then(|()| self.heartbeat.confirm(completed_at))
        {
            let error = if payload.is_none() && matches!(error, WorkerError::Timeout) {
                WorkerError::HeartbeatTimeout
            } else {
                error
            };
            return Err(self.fail(error, &deadline));
        }
        match result {
            Err(error @ WorkerError::Remote(_)) => Err(error),
            Err(error) => {
                let error = if payload.is_none() && matches!(error, WorkerError::Timeout) {
                    WorkerError::HeartbeatTimeout
                } else {
                    error
                };
                Err(self.fail(error, &deadline))
            }
            ok => ok,
        }
    }
    fn drain_before_issue(&mut self, deadline: &Deadline) -> Result<(), WorkerError> {
        loop {
            deadline.check()?;
            if let Some(line) = self.frames.pop() {
                let value: Value = serde_json::from_slice(&line)
                    .map_err(|e| WorkerError::Protocol(format!("invalid worker JSON: {e}")))?;
                if !value.is_object() {
                    return Err(WorkerError::Protocol(
                        "protocol line must be an object".into(),
                    ));
                }
                let kind = value.get("kind").and_then(Value::as_str);
                let future = value
                    .get("seq")
                    .and_then(Value::as_u64)
                    .is_some_and(|seq| seq > self.sequence);
                if matches!(kind, Some("response" | "error" | "pong")) && future {
                    return Err(WorkerError::Protocol("reply preceded its request".into()));
                }
            } else if !self.pump()? {
                if self.frames.is_empty() {
                    return deadline.check();
                }
                // Do not carry a pre-issued partial frame into a new operation.
                pause_until(deadline.io);
            }
        }
    }
    fn write(&mut self, bytes: &[u8], deadline: &Deadline) -> Result<(), WorkerError> {
        let mut written = 0;
        while written < bytes.len() {
            deadline.check()?;
            if self.process.has_exited()? {
                return Err(WorkerError::WorkerExited {
                    status: "observed exit".into(),
                });
            }
            let before = written;
            match self
                .process
                .write(&bytes[written..bytes.len().min(written + 4096)])
            {
                Ok(0) => return Err(WorkerError::Io(io::ErrorKind::WriteZero.into())),
                Ok(n) => written += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(WorkerError::Io(e)),
            }
            let read = self.pump()?;
            if written == before && !read {
                pause_until(deadline.io);
            }
        }
        deadline.check()
    }
    fn pump(&mut self) -> Result<bool, WorkerError> {
        let mut buffer = [0u8; 1024];
        match self.process.read(&mut buffer) {
            Ok(0) => {
                if self.process.has_exited()? {
                    Err(WorkerError::WorkerExited {
                        status: "stdout EOF after exit".into(),
                    })
                } else {
                    Err(WorkerError::StdoutClosed)
                }
            }
            Ok(n) => {
                self.frames.push(&buffer[..n])?;
                Ok(true)
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(WorkerError::Io(e)),
        }
    }
    fn receive(&mut self, deadline: &Deadline) -> Result<Value, WorkerError> {
        loop {
            deadline.check()?;
            if let Some(line) = self.frames.pop() {
                let value: Value = serde_json::from_slice(&line)
                    .map_err(|e| WorkerError::Protocol(format!("invalid worker JSON: {e}")))?;
                deadline.check()?;
                if !value.is_object() {
                    return Err(WorkerError::Protocol(
                        "protocol line must be an object".into(),
                    ));
                }
                return Ok(value);
            }
            if self.process.has_exited()? {
                return Err(WorkerError::WorkerExited {
                    status: "observed exit".into(),
                });
            }
            if !self.pump()? {
                pause_until(deadline.io);
            }
        }
    }
    fn finish(&mut self, end: Instant) -> CleanupReport {
        self.unavailable = true;
        self.heartbeat.fail(WorkerFailure::Shutdown);
        self.frames.clear();
        let report = self.process.finish(end);
        self.cleanup = Some(report);
        report
    }
    fn fail(&mut self, error: WorkerError, deadline: &Deadline) -> WorkerError {
        self.heartbeat.fail(WorkerFailure::from_error(&error));
        let report = self.finish(deadline.cleanup_end());
        Self::with_cleanup(error, report)
    }
    fn retire(&mut self, error: WorkerError) -> WorkerError {
        self.heartbeat.fail(WorkerFailure::from_error(&error));
        let now = Instant::now();
        let report = self.finish(now.checked_add(self.timeouts.cleanup()).unwrap_or(now));
        Self::with_cleanup(error, report)
    }
    fn with_cleanup(error: WorkerError, report: CleanupReport) -> WorkerError {
        if report.state == CleanupState::Reaped && report.signal_error.is_none() {
            error
        } else {
            WorkerError::Cleanup {
                cause: Box::new(error),
                report,
            }
        }
    }
    /// Whether the session remains usable and unexpired. This never probes an
    /// idle process; periodic check_heartbeat is required to demonstrate health.
    pub fn is_running(&mut self) -> bool {
        self.check_session().is_ok()
    }
    /// Unknown until a matching timely reply, Healthy before the next due time,
    /// Degraded when a confirmed session owes a probe, Unavailable after expiry
    /// or failure. Observation can retire an expired session using cleanup reserve;
    /// it does not itself issue a ping. Keep all supervision outside local control.
    pub fn health(&mut self) -> HealthState {
        self.check_session().unwrap_or(HealthState::Unavailable)
    }
    /// Best-effort graceful shutdown, group termination and bounded reaping,
    /// within the configured total. Repeated calls reuse the cleanup report.
    pub fn shutdown(&mut self) -> CleanupReport {
        if let Some(report) = self.cleanup {
            return report;
        }
        let deadline = Deadline::new(self.timeouts.shutdown(), self.timeouts.cleanup())
            .expect("validated timeouts");
        let _ = self.write(b"{\"kind\":\"shutdown\"}\n", &deadline);
        self.process.close_stdin();
        while deadline.check().is_ok() {
            if !matches!(self.process.has_exited(), Ok(false)) {
                break;
            }
            pause_until(deadline.io);
        }
        self.finish(deadline.cleanup_end())
    }
}
impl Drop for PythonWorker {
    fn drop(&mut self) {
        if self.cleanup.is_none() {
            let end = Instant::now()
                .checked_add(self.timeouts.cleanup())
                .unwrap_or_else(Instant::now);
            self.finish(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_deadlines_cap_requests_and_reserve_cleanup() {
        let start = Instant::now();
        let reserve = Duration::from_millis(50);
        let expiry = start + Duration::from_millis(200);
        let deadline = Deadline::heartbeat(expiry, reserve).unwrap();
        assert!(deadline.check_at(expiry - Duration::from_nanos(1)).is_ok());
        assert!(matches!(
            deadline.check_at(expiry),
            Err(WorkerError::Timeout)
        ));
        assert_eq!(deadline.end, expiry + reserve);
        let capped = Deadline::new(Duration::from_secs(10), reserve)
            .unwrap()
            .cap_io(expiry)
            .unwrap();
        assert_eq!(capped.io, expiry);
        assert_eq!(capped.end, expiry + reserve);
        // An operation's earlier total deadline remains the limiting one.
        let short = Deadline::new(Duration::from_millis(100), reserve).unwrap();
        let end = short.end;
        assert_eq!(short.cap_io(expiry).unwrap().end, end);
    }
}
