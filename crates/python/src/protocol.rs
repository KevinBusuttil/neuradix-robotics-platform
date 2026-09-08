//! Bounded framing and serialization; no blocking operations or reader threads.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::time::Instant;

use serde_json::Value;

use crate::{IoLimits, WorkerError};

pub(crate) struct Frames {
    bytes: VecDeque<u8>,
    lengths: VecDeque<usize>,
    partial: usize,
    limits: IoLimits,
    pub high_bytes: usize,
    pub high_messages: usize,
}
impl Frames {
    pub fn new(limits: IoLimits) -> Self {
        Self { bytes: VecDeque::with_capacity(limits.queued_bytes()), lengths: VecDeque::with_capacity(limits.queued_messages()), partial: 0, limits, high_bytes: 0, high_messages: 0 }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), WorkerError> {
        for &byte in bytes {
            if self.partial + 1 > self.limits.incoming_bytes()
                || (self.partial + 1 == self.limits.incoming_bytes() && byte != b'\n')
            {
                return Err(WorkerError::IncomingTooLarge { limit: self.limits.incoming_bytes() });
            }
            if self.bytes.len() == self.limits.queued_bytes() {
                return Err(WorkerError::QueueBytesExceeded { limit: self.limits.queued_bytes() });
            }
            if byte == b'\n' && self.lengths.len() == self.limits.queued_messages() {
                return Err(WorkerError::QueueMessagesExceeded { limit: self.limits.queued_messages() });
            }
            self.bytes.push_back(byte);
            self.partial += 1;
            if byte == b'\n' {
                self.lengths.push_back(self.partial);
                self.partial = 0;
            }
            self.high_bytes = self.high_bytes.max(self.bytes.len());
            self.high_messages = self.high_messages.max(self.lengths.len());
        }
        Ok(())
    }
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let length = self.lengths.pop_front()?;
        Some(self.bytes.drain(..length).collect())
    }
    pub fn clear(&mut self) {
        self.bytes.clear(); self.lengths.clear(); self.partial = 0;
    }
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
    deadline: Instant,
    failure: Option<WorkerError>,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if Instant::now() >= self.deadline {
            self.failure = Some(WorkerError::Timeout);
        } else if bytes.len() > self.limit - self.bytes.len() {
            self.failure = Some(WorkerError::OutgoingTooLarge { limit: self.limit });
        }
        if self.failure.is_some() { return Err(io::Error::other("bounded serialization stopped")); }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

// Bound recursion and visits before serde's recursive Value serializer. The
// caller already owns the Value; this crate never clones the payload tree.
fn check_value(value: &Value, depth: usize, remaining: &mut usize, deadline: Instant) -> Result<(), WorkerError> {
    if Instant::now() >= deadline { return Err(WorkerError::Timeout); }
    if depth > 32 { return Err(WorkerError::Protocol("outgoing JSON exceeds depth 32".into())); }
    if *remaining == 0 { return Err(WorkerError::Protocol("outgoing JSON exceeds node budget".into())); }
    *remaining -= 1;
    match value {
        Value::Array(values) => for v in values { check_value(v, depth + 1, remaining, deadline)?; },
        Value::Object(values) => for v in values.values() { check_value(v, depth + 1, remaining, deadline)?; },
        _ => {}
    }
    Ok(())
}

pub(crate) fn encode(value: &Value, sequence: Option<u64>, limit: usize, deadline: Instant) -> Result<Vec<u8>, WorkerError> {
    let mut nodes = limit;
    check_value(value, 0, &mut nodes, deadline)?;
    let mut writer = LimitedWriter { bytes: Vec::with_capacity(limit), limit, deadline, failure: None };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if let Some(seq) = sequence { write!(writer, "{{\"kind\":\"request\",\"seq\":{seq},\"payload\":")?; }
        serde_json::to_writer(&mut writer, value)?;
        if sequence.is_some() { writer.write_all(b"}")?; }
        writer.write_all(b"\n")?;
        Ok(())
    })();
    if let Some(error) = writer.failure { return Err(error); }
    result.map_err(|e| WorkerError::Protocol(e.to_string()))?;
    if Instant::now() >= deadline { return Err(WorkerError::Timeout); }
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use serde_json::json;
    #[test]
    fn line_boundary_counts_newline_and_rejects_unterminated_growth() {
        let mut frames = Frames::new(IoLimits::new(64, 64, 128, 2).unwrap());
        frames.push(&[b'x'; 63]).unwrap();
        frames.push(b"\n").unwrap();
        assert_eq!(frames.pop().unwrap().len(), 64);
        frames.push(&[b'x'; 63]).unwrap();
        assert!(matches!(frames.push(b"x"), Err(WorkerError::IncomingTooLarge { limit: 64 })));
        assert!(frames.high_bytes <= 128);
    }
    #[test]
    fn complete_and_partial_lines_share_the_exact_byte_budget() {
        let limits = IoLimits::new(64, 64, 100, 4).unwrap();
        let mut frames = Frames::new(limits);
        frames.push(&[b'x'; 49]).unwrap(); frames.push(b"\n").unwrap();
        frames.push(&[b'y'; 50]).unwrap();
        assert_eq!(frames.high_bytes, 100);
        assert!(matches!(frames.push(b"\n"), Err(WorkerError::QueueBytesExceeded { limit: 100 })));
        assert_eq!(frames.pop().unwrap().len(), 50);
        frames.push(b"\n").unwrap(); assert_eq!(frames.pop().unwrap().len(), 51);
    }
    #[test]
    fn message_queue_accepts_exact_count_then_rejects_burst() {
        let mut frames = Frames::new(IoLimits::new(64, 64, 128, 2).unwrap());
        frames.push(b"{}\n{}\n").unwrap(); assert_eq!(frames.high_messages, 2);
        assert!(matches!(frames.push(b"{}\n"), Err(WorkerError::QueueMessagesExceeded { limit: 2 })));
        assert_eq!(frames.high_messages, 2);
    }
    #[test]
    fn encoding_bounds_escaped_utf8_and_depth_before_writing() {
        let end = Instant::now() + Duration::from_secs(1);
        let value = json!("☃\n\"".repeat(10));
        let line = encode(&value, Some(1), 256, end).unwrap();
        assert_eq!(encode(&value, Some(1), line.len(), end).unwrap(), line);
        assert!(matches!(encode(&value, Some(1), line.len() - 1, end), Err(WorkerError::OutgoingTooLarge { .. })));
        let mut deep = Value::Null;
        for _ in 0..33 { deep = json!([deep]); }
        assert!(matches!(encode(&deep, Some(1), 256, end), Err(WorkerError::Protocol(_))));
    }
    #[test]
    fn expired_serialization_deadline_is_never_extended() {
        assert!(matches!(encode(&Value::Null, Some(u64::MAX), 256, Instant::now()), Err(WorkerError::Timeout)));
    }
}
