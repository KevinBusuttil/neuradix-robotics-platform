//! Synchronous bounded Python-worker stdio, outside the local control executor.

pub use crate::config::WorkerConfig;
use crate::process::{Process, pause_until};
use crate::protocol::{Frames, encode};
use crate::{CleanupReport, CleanupState, Timeouts, WorkerError};
use neuradix_runtime::HealthState;
use serde_json::Value;
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Deadline {
    io: Instant,
    end: Instant,
    reserve: Duration,
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
        if Instant::now() >= self.io {
            Err(WorkerError::Timeout)
        } else {
            Ok(())
        }
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
        )?;
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
        deadline.check()?;
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
            deadline.check()
        });
        if let Err(error) = handshake {
            let error = if matches!(error, WorkerError::Timeout) {
                WorkerError::HandshakeTimeout
            } else {
                error
            };
            return Err(worker.fail(error, &deadline));
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

    /// Send a borrowed payload without cloning its tree. Encoding is bounded
    /// before any write. Sequences start at 1, never wrap and are consumed only
    /// when a write is attempted. Unrelated lines never extend the deadline.
    pub fn send(&mut self, payload: &Value) -> Result<Value, WorkerError> {
        if self.unavailable {
            return Err(WorkerError::Unavailable);
        }
        let deadline = Deadline::new(self.timeouts.request(), self.timeouts.cleanup())?;
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(WorkerError::SequenceExhausted)?;
        let line = match encode(payload, Some(sequence), self.outgoing_limit, deadline.io) {
            Ok(line) => line,
            Err(WorkerError::Timeout) => return Err(self.fail(WorkerError::Timeout, &deadline)),
            Err(error) => return Err(error),
        };
        self.outgoing_high = self.outgoing_high.max(line.len());
        self.sequence = sequence;
        let result = (|| {
            self.write(&line, &deadline)?;
            loop {
                let mut value = self.receive(&deadline)?;
                if value.get("seq").and_then(Value::as_u64) != Some(sequence) {
                    continue;
                }
                match value.get("kind").and_then(Value::as_str) {
                    Some("response") => {
                        deadline.check()?;
                        return Ok(value
                            .get_mut("payload")
                            .map(Value::take)
                            .unwrap_or(Value::Null));
                    }
                    Some("error") => {
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
        match result {
            Err(error @ WorkerError::Remote(_)) => Err(error),
            Err(error) => Err(self.fail(error, &deadline)),
            ok => ok,
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
        self.frames.clear();
        let report = self.process.finish(end);
        self.cleanup = Some(report);
        report
    }
    fn fail(&mut self, error: WorkerError, deadline: &Deadline) -> WorkerError {
        let report = self.finish(deadline.cleanup_end());
        if report.state == CleanupState::Reaped && report.signal_error.is_none() {
            error
        } else {
            WorkerError::Cleanup {
                cause: Box::new(error),
                report,
            }
        }
    }
    /// Available running process status, not a heartbeat or handler liveness proof.
    pub fn is_running(&mut self) -> bool {
        if self.unavailable {
            return false;
        }
        matches!(self.process.has_exited(), Ok(false))
    }
    /// Failed sessions remain Unavailable until replaced, even if reaping is deferred.
    pub fn health(&mut self) -> HealthState {
        if self.unavailable {
            return HealthState::Unavailable;
        }
        match self.process.has_exited() {
            Ok(false) => HealthState::Healthy,
            Ok(true) => HealthState::Unavailable,
            Err(_) => HealthState::Unknown,
        }
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
