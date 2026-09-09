//! Bounded direct uncompressed MCAP output through the pinned maintained writer.
//!
//! Definitions precede use. Payloads are borrowed, written synchronously and not
//! retained. The sole summary group is Statistics: no seeking indexes or repeated
//! definitions are emitted. Full definitions remain in the data section. Data
//! and summary CRCs are enabled. Blocking sinks/callbacks need external supervision.
//!
//! Every returned operation error latches failure. Drop/abort suppress upstream
//! automatic finalization. A failed/aborted output is provisional, even if a sink
//! accepted bytes before an I/O error. Only successful finish authorizes publishing
//! the file; flush is not a power-loss durability guarantee. Keep outside control.
pub(crate) mod legacy;
mod limits;
use crate::{
    McapChannel, McapHeader, McapMessageRef, McapMetadata, McapSchema, RecordError, Result,
};
pub use limits::McapWriteLimits;
use limits::{add, check};
use mcap::write::NoSeek;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

/// Successful admission counters; partial bytes from failed sink calls are not included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McapWriteStats {
    /// Header/magic and completed data record bytes, excluding reserved finalization.
    pub accepted_bytes: u64,
    /// Completed records, excluding three reserved finalization records.
    pub records: u64,
    /// Completed messages.
    pub messages: u64,
    /// Unique schema IDs.
    pub schemas: usize,
    /// Unique channel IDs.
    pub channels: usize,
    /// Unique metadata names.
    pub metadata: usize,
    /// Conservative definition and finalization storage charge, excluding caller sink.
    pub state_bytes: u64,
}

struct Sink<W> {
    inner: W,
    bytes: u64,
    limit: u64,
}
impl<W: Write> Write for Sink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > self.limit)
        {
            return Err(io::Error::other("MCAP output limit"));
        }
        let written = self.inner.write(bytes)?;
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Validated uncompressed streaming writer. No payload queue or worker thread.
/// Caller-owned sinks such as Vec may retain output; use File for bounded RSS.
pub struct McapStreamWriter<W: Write> {
    inner: Option<mcap::Writer<NoSeek<Sink<W>>>>,
    limits: McapWriteLimits,
    stats: McapWriteStats,
    schemas: BTreeSet<u16>,
    channels: BTreeMap<u16, bool>,
    metadata: BTreeMap<String, McapMetadata>,
    active_channels: usize,
    failed: bool,
}
impl<W: Write> McapStreamWriter<W> {
    /// Validate policy/header and reserve finalization before writing initial bytes.
    pub fn new(inner: W, header: &McapHeader, limits: McapWriteLimits) -> Result<Self> {
        let body = limits::header(header, limits)?;
        check(body, limits.record_bytes as u64, "record bytes")?;
        check(46, limits.record_bytes as u64, "statistics record bytes")?;
        let accepted_bytes = add(17, body)?;
        check(
            add(accepted_bytes, 105)?,
            limits.output_bytes,
            "output bytes",
        )?;
        check(4, limits.records, "records")?;
        let state_bytes = limits::charge(body, 0)?;
        check(state_bytes, limits.state_bytes as u64, "state bytes")?;
        let writer = mcap::WriteOptions::new()
            .profile(header.profile.clone())
            .library(header.library.clone())
            .compression(None)
            .use_chunks(false)
            .disable_seeking(true)
            .emit_summary_records(false)
            .emit_statistics(true)
            .emit_summary_offsets(false)
            .emit_message_indexes(false)
            .calculate_data_section_crc(true)
            .calculate_summary_section_crc(true)
            .create(NoSeek::new(Sink {
                inner,
                bytes: 0,
                limit: limits.output_bytes,
            }))
            .map_err(vendor)?;
        Ok(Self {
            inner: Some(writer),
            limits,
            stats: McapWriteStats {
                accepted_bytes,
                records: 1,
                state_bytes,
                ..Default::default()
            },
            schemas: BTreeSet::new(),
            channels: BTreeMap::new(),
            metadata: BTreeMap::new(),
            active_channels: 0,
            failed: false,
        })
    }
    /// Admission counters. Failure does not undo bytes already accepted by the sink.
    pub fn stats(&self) -> &McapWriteStats {
        &self.stats
    }
    /// Whether an earlier operation failed; this state never recovers.
    pub fn is_failed(&self) -> bool {
        self.failed
    }
    /// Exact successful final file size for current accepted data.
    pub fn projected_final_bytes(&self) -> u64 {
        self.stats.accepted_bytes + 105 + self.active_channels as u64 * 10
    }
    fn run<T>(&mut self, op: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.failed {
            return Err(RecordError::McapWriterFailed);
        }
        let result = op(self);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub(crate) fn reject<T>(&mut self, error: RecordError) -> Result<T> {
        self.run(|_| Err(error))
    }
    fn reserve(&self, body: u64, charge: u64, active: usize) -> Result<()> {
        check(body, self.limits.record_bytes as u64, "record bytes")?;
        check(
            46 + active as u64 * 10,
            self.limits.record_bytes as u64,
            "statistics record bytes",
        )?;
        check(
            add(
                add(self.stats.accepted_bytes, add(9, body)?)?,
                105 + active as u64 * 10,
            )?,
            self.limits.output_bytes,
            "output bytes",
        )?;
        check(add(self.stats.records, 4)?, self.limits.records, "records")?;
        check(
            add(self.stats.state_bytes, charge)?,
            self.limits.state_bytes as u64,
            "state bytes",
        )
    }
    fn accepted(&mut self, body: u64, charge: u64) {
        // Checked against validated ceilings by reserve before any write.
        self.stats.accepted_bytes += 9 + body;
        self.stats.records += 1;
        self.stats.state_bytes += charge;
    }
    /// Define a complete schema; identical duplicate IDs normalize, conflicts fail.
    pub fn schema(&mut self, schema: &McapSchema) -> Result<()> {
        self.run(|this| {
            let body = limits::schema(schema, this.limits)?;
            check(body, this.limits.record_bytes as u64, "record bytes")?;
            let new = !this.schemas.contains(&schema.id);
            let charge = limits::charge(body, 0)?;
            if new {
                check(
                    this.schemas.len() as u64 + 1,
                    this.limits.schemas as u64,
                    "schemas",
                )?;
                this.reserve(body, charge, this.active_channels)?;
            }
            this.inner
                .as_mut()
                .unwrap()
                .add_schema_with_id(schema.id, &schema.name, &schema.encoding, &schema.data)
                .map_err(vendor)?;
            if new {
                this.schemas.insert(schema.id);
                this.stats.schemas += 1;
                this.accepted(body, charge);
            }
            Ok(())
        })
    }
    /// Define a complete channel; referenced nonzero schemas must exist already.
    pub fn channel(&mut self, channel: &McapChannel) -> Result<()> {
        self.run(|this| {
            let body = limits::channel(channel, this.limits)?;
            check(body, this.limits.record_bytes as u64, "record bytes")?;
            if channel.schema_id != 0 && !this.schemas.contains(&channel.schema_id) {
                return Err(RecordError::InvalidMcapWrite("missing schema"));
            }
            let new = !this.channels.contains_key(&channel.id);
            let charge = limits::charge(body, channel.metadata.len())?;
            if new {
                check(
                    this.channels.len() as u64 + 1,
                    this.limits.channels as u64,
                    "channels",
                )?;
                this.reserve(body, charge, this.active_channels)?;
            }
            this.inner
                .as_mut()
                .unwrap()
                .add_channel_with_id(
                    channel.id,
                    channel.schema_id,
                    &channel.topic,
                    &channel.message_encoding,
                    &channel.metadata,
                )
                .map_err(vendor)?;
            if new {
                this.channels.insert(channel.id, false);
                this.stats.channels += 1;
                this.accepted(body, charge);
            }
            Ok(())
        })
    }
    /// Write bounded named metadata. Identical duplicates normalize; conflicts fail.
    pub fn metadata(&mut self, metadata: &McapMetadata) -> Result<()> {
        self.run(|this| {
            let body = limits::metadata(metadata, this.limits)?;
            check(body, this.limits.record_bytes as u64, "record bytes")?;
            if let Some(old) = this.metadata.get(&metadata.name) {
                return if old == metadata {
                    Ok(())
                } else {
                    Err(RecordError::InvalidMcapWrite("conflicting metadata"))
                };
            }
            check(
                this.metadata.len() as u64 + 1,
                this.limits.metadata as u64,
                "metadata names",
            )?;
            let charge = limits::charge(body, metadata.entries.len())?;
            this.reserve(body, charge, this.active_channels)?;
            this.inner
                .as_mut()
                .unwrap()
                .write_metadata(&mcap::records::Metadata {
                    name: metadata.name.clone(),
                    metadata: metadata.entries.clone(),
                })
                .map_err(vendor)?;
            this.metadata
                .insert(metadata.name.clone(), metadata.clone());
            this.stats.metadata += 1;
            this.accepted(body, charge);
            Ok(())
        })
    }
    /// Write borrowed opaque bytes directly, preserving both raw timestamps/sequence.
    pub fn message(&mut self, message: McapMessageRef<'_>) -> Result<()> {
        self.run(|this| {
            let used = *this
                .channels
                .get(&message.channel_id)
                .ok_or(RecordError::InvalidMcapWrite("missing channel"))?;
            check(
                message.data.len() as u64,
                this.limits.message_bytes as u64,
                "message bytes",
            )?;
            check(
                add(this.stats.messages, 1)?,
                this.limits.messages,
                "messages",
            )?;
            let active = this.active_channels + usize::from(!used);
            check(
                active as u64,
                this.limits.entries as u64,
                "statistics map entries",
            )?;
            let body = add(22, message.data.len() as u64)?;
            this.reserve(body, 0, active)?;
            this.inner
                .as_mut()
                .unwrap()
                .write_to_known_channel(
                    &mcap::records::MessageHeader {
                        channel_id: message.channel_id,
                        sequence: message.sequence,
                        log_time: message.log_time,
                        publish_time: message.publish_time,
                    },
                    message.data,
                )
                .map_err(vendor)?;
            this.channels.insert(message.channel_id, true);
            this.active_channels = active;
            this.stats.messages += 1;
            this.accepted(body, 0);
            Ok(())
        })
    }
    /// Flush current output without finalizing. Flush errors latch failure.
    pub fn flush(&mut self) -> Result<()> {
        self.run(|this| this.inner.as_mut().unwrap().flush().map_err(vendor))
    }
    /// Complete the reserved summary/footer and flush. Failure is never retried.
    pub fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(RecordError::McapWriterFailed);
        }
        let result = self
            .inner
            .as_mut()
            .unwrap()
            .finish()
            .map(|_| ())
            .map_err(vendor);
        let sink = self.inner.take().unwrap().into_inner().into_inner();
        result?;
        debug_assert_eq!(sink.bytes, self.projected_final_bytes());
        Ok(sink.inner)
    }
    /// Return the sink without a footer. The caller must discard the partial file.
    pub fn abort(mut self) -> W {
        self.inner.take().unwrap().into_inner().into_inner().inner
    }
}
impl<W: Write> Drop for McapStreamWriter<W> {
    fn drop(&mut self) {
        if let Some(writer) = self.inner.take() {
            // into_inner explicitly suppresses the maintained writer's Drop finish.
            drop(writer.into_inner());
        }
    }
}
fn vendor(error: mcap::McapError) -> RecordError {
    RecordError::McapWrite(error.to_string())
}
