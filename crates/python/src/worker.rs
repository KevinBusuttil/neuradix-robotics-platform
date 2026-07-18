//! A managed, isolated Python worker process.
//!
//! The worker runs as a separate OS process and communicates over
//! newline-delimited JSON on stdin/stdout (stderr is inherited, so Python
//! tracebacks are visible). A background reader thread turns the blocking pipe
//! into a channel, which lets requests time out and lets a worker crash be
//! observed as a recoverable [`WorkerError::WorkerExited`] rather than blocking
//! or crashing the supervisor.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use neuradix_runtime::HealthState;
use serde_json::{Value, json};

use crate::error::WorkerError;

/// Configuration for launching a Python worker.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// The Python interpreter (e.g. `python3`).
    pub interpreter: PathBuf,
    /// The component script to run.
    pub script: PathBuf,
    /// Extra command-line arguments for the script.
    pub args: Vec<String>,
    /// Directories added to `PYTHONPATH` (e.g. where `neuradix_worker.py` lives).
    pub python_path: Vec<PathBuf>,
    /// Structured configuration passed to the worker as JSON.
    pub config: Value,
    /// Handshake timeout.
    pub handshake_timeout: Duration,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Whether the component declares that input samples may be skipped (§19.4).
    pub skip_inputs: bool,
}

impl WorkerConfig {
    /// A configuration with sensible default timeouts.
    pub fn new(interpreter: impl Into<PathBuf>, script: impl Into<PathBuf>) -> Self {
        Self {
            interpreter: interpreter.into(),
            script: script.into(),
            args: Vec::new(),
            python_path: Vec::new(),
            config: Value::Null,
            handshake_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(10),
            skip_inputs: true,
        }
    }

    /// Set the structured configuration passed to the worker.
    pub fn with_config(mut self, config: Value) -> Self {
        self.config = config;
        self
    }

    /// Add a `PYTHONPATH` directory.
    pub fn with_python_path(mut self, dir: impl Into<PathBuf>) -> Self {
        self.python_path.push(dir.into());
        self
    }

    /// Set the per-request timeout.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

/// Information reported by the worker at startup.
#[derive(Debug, Clone)]
pub struct ReadyInfo {
    /// The worker's declared name.
    pub name: String,
    /// The worker's declared input-skip policy.
    pub skip_policy: String,
}

enum ReaderEvent {
    Line(String),
    Closed,
}

/// A running, supervised Python worker.
pub struct PythonWorker {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<ReaderEvent>,
    seq: u64,
    request_timeout: Duration,
    ready: ReadyInfo,
}

impl PythonWorker {
    /// Launch a worker and complete its startup handshake.
    pub fn launch(config: &WorkerConfig) -> Result<Self, WorkerError> {
        let mut command = Command::new(&config.interpreter);
        command
            .arg(&config.script)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        if !config.python_path.is_empty() {
            let joined = config
                .python_path
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(path_separator());
            command.env("PYTHONPATH", joined);
        }
        command.env("NEURADIX_WORKER_CONFIG", config.config.to_string());
        command.env(
            "NEURADIX_WORKER_SKIP_INPUTS",
            config.skip_inputs.to_string(),
        );

        let mut child = command.spawn().map_err(WorkerError::Launch)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::Protocol("failed to capture worker stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::Protocol("failed to capture worker stdout".to_owned()))?;

        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(ReaderEvent::Closed);
                        break;
                    }
                    Ok(_) => {
                        if tx.send(ReaderEvent::Line(line.clone())).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(ReaderEvent::Closed);
                        break;
                    }
                }
            }
        });

        let mut worker = Self {
            child,
            stdin: Some(stdin),
            rx,
            seq: 0,
            request_timeout: config.request_timeout,
            ready: ReadyInfo {
                name: String::new(),
                skip_policy: String::new(),
            },
        };
        worker.handshake(config.handshake_timeout)?;
        Ok(worker)
    }

    fn handshake(&mut self, timeout: Duration) -> Result<(), WorkerError> {
        match self.rx.recv_timeout(timeout) {
            Ok(ReaderEvent::Line(line)) => {
                let value: Value = serde_json::from_str(line.trim())
                    .map_err(|e| WorkerError::Protocol(format!("bad handshake: {e}")))?;
                if value.get("kind").and_then(Value::as_str) != Some("ready") {
                    return Err(WorkerError::Protocol(format!(
                        "expected `ready`, got: {}",
                        line.trim()
                    )));
                }
                self.ready = ReadyInfo {
                    name: value
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    skip_policy: value
                        .get("skipPolicy")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                };
                Ok(())
            }
            Ok(ReaderEvent::Closed) => Err(self.exit_error()),
            Err(RecvTimeoutError::Timeout) => Err(WorkerError::HandshakeTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(self.exit_error()),
        }
    }

    /// The worker's startup information.
    pub fn ready_info(&self) -> &ReadyInfo {
        &self.ready
    }

    /// Send a request payload and await the worker's response payload.
    pub fn send(&mut self, payload: Value) -> Result<Value, WorkerError> {
        self.seq += 1;
        let seq = self.seq;
        let request = json!({ "kind": "request", "seq": seq, "payload": payload });
        self.write_line(&request)?;

        let deadline = Instant::now() + self.request_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(remaining) {
                Ok(ReaderEvent::Line(line)) => {
                    let value: Value = serde_json::from_str(line.trim())
                        .map_err(|e| WorkerError::Protocol(format!("bad response: {e}")))?;
                    let matches_seq = value.get("seq").and_then(Value::as_i64) == Some(seq as i64);
                    match value.get("kind").and_then(Value::as_str) {
                        Some("response") if matches_seq => {
                            return Ok(value.get("payload").cloned().unwrap_or(Value::Null));
                        }
                        Some("error") if matches_seq => {
                            let message = value
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown error");
                            return Err(WorkerError::Remote(message.to_owned()));
                        }
                        // Logs or stale/mismatched lines are skipped.
                        _ => continue,
                    }
                }
                Ok(ReaderEvent::Closed) => return Err(self.exit_error()),
                Err(RecvTimeoutError::Timeout) => return Err(WorkerError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(self.exit_error()),
            }
        }
    }

    /// Whether the worker process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// The worker's current health: running is [`HealthState::Healthy`]; an
    /// exited process is [`HealthState::Unavailable`].
    pub fn health(&mut self) -> HealthState {
        match self.child.try_wait() {
            Ok(None) => HealthState::Healthy,
            Ok(Some(_)) => HealthState::Unavailable,
            Err(_) => HealthState::Unknown,
        }
    }

    /// Shut the worker down cleanly, killing it if it does not exit promptly.
    pub fn shutdown(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"{\"kind\":\"shutdown\"}\n");
            let _ = stdin.flush();
            // Dropping stdin sends EOF so the worker's read loop ends.
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn write_line(&mut self, value: &Value) -> Result<(), WorkerError> {
        let mut line = value.to_string();
        line.push('\n');
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| WorkerError::Protocol("worker stdin is closed".to_owned()))?;
        match stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
        {
            Ok(()) => Ok(()),
            Err(e) => {
                // A broken pipe means the worker died; report that, not raw I/O.
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    Err(self.exit_error())
                } else {
                    Err(WorkerError::Io(e))
                }
            }
        }
    }

    fn exit_error(&mut self) -> WorkerError {
        let status = match self.child.wait() {
            Ok(status) => status.to_string(),
            Err(e) => format!("unknown (wait failed: {e})"),
        };
        WorkerError::WorkerExited { status }
    }
}

impl Drop for PythonWorker {
    fn drop(&mut self) {
        // Never leak a child process.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn path_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}
