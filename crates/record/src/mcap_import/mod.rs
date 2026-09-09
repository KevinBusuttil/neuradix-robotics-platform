//! Bounded, synchronous semantic MCAP import using pinned maintained parsing.
//!
//! Supports uncompressed data/chunks and one LZ4 frame per compressed chunk.
//! Schema languages and message encodings stay opaque. Attachments, unknown
//! opcodes, nested chunks and extension fields reject explicitly. Known indexes
//! are retained opaquely and never used for traversal. All messages are visited.
//!
//! Callbacks provide backpressure: no next record is processed until return.
//! Borrowed payloads cannot escape without an explicit caller-owned copy. Events
//! remain provisional until EOF, structure and available CRC checks all succeed.
//! A failed import can have delivered a prefix; never use callbacks as actuator
//! authority. Caller I/O/callback latency is not bounded by this synchronous API.
//! No background work is spawned; keep import outside local control.
mod chunk;
mod limits;
mod model;
mod preflight;
mod projection;

use crate::{RecordError, Result};
pub use limits::McapImportLimits;
use limits::{add, check};
use mcap::records::Record;
use mcap::sans_io::linear_reader::{LinearReadEvent, LinearReader, LinearReaderOptions};
pub use model::*;
use std::collections::BTreeMap;
use std::io::Read;

fn malformed(message: &str) -> RecordError {
    RecordError::Mcap(message.to_owned())
}
fn vendor(error: mcap::McapError) -> RecordError {
    RecordError::Mcap(error.to_string())
}

/// Successful streaming result with bounded complete definitions and counters.
#[derive(Debug, Clone)]
pub struct McapImportSummary {
    header: McapHeader,
    schemas: BTreeMap<u16, McapSchema>,
    channels: BTreeMap<u16, McapChannel>,
    metadata: BTreeMap<String, McapMetadata>,
    stats: McapImportStats,
}
impl McapImportSummary {
    /// Producer profile/library, preserved without authentication claims.
    pub fn header(&self) -> &McapHeader {
        &self.header
    }
    /// Full, immutable schema definitions.
    pub fn schemas(&self) -> &BTreeMap<u16, McapSchema> {
        &self.schemas
    }
    /// Full, immutable channel definitions.
    pub fn channels(&self) -> &BTreeMap<u16, McapChannel> {
        &self.channels
    }
    /// Complete named metadata definitions.
    pub fn metadata(&self) -> &BTreeMap<String, McapMetadata> {
        &self.metadata
    }
    /// Executed budget counters.
    pub fn stats(&self) -> &McapImportStats {
        &self.stats
    }
}

/// A bounded materialization preserving both timestamps and opaque encodings.
/// This intentionally does not implement the legacy one-timestamp Recording
/// trait. Use explicit checked conversion only for representable Neuradix data.
#[derive(Debug, Clone)]
pub struct McapArchive {
    summary: McapImportSummary,
    messages: Vec<McapMessage>,
    auxiliary: Vec<McapAuxiliary>,
    retained_bytes: u64,
}
impl McapArchive {
    /// Materialize within the validated budget. Failure returns no partial archive.
    pub fn from_reader(reader: impl Read, limits: McapImportLimits) -> Result<Self> {
        let mut messages = Vec::new();
        let mut auxiliary = Vec::new();
        let mut retained = 0;
        let summary = import_mcap(reader, limits, |event| {
            add(
                &mut retained,
                event.accounted_bytes,
                limits.retained_bytes as u64,
                "retained bytes",
            )?;
            match event.kind {
                McapEventKind::Message(m) => messages.push(McapMessage {
                    channel_id: m.channel_id,
                    sequence: m.sequence,
                    log_time: m.log_time,
                    publish_time: m.publish_time,
                    data: m.data.to_vec(),
                }),
                McapEventKind::Auxiliary { opcode, data } => auxiliary.push(McapAuxiliary {
                    opcode,
                    ordinal: event.ordinal,
                    data: data.to_vec(),
                }),
                _ => {}
            }
            Ok(())
        })?;
        Ok(Self {
            summary,
            messages,
            auxiliary,
            retained_bytes: retained,
        })
    }
    /// Header, definitions, metadata and successful import counters.
    pub fn summary(&self) -> &McapImportSummary {
        &self.summary
    }
    /// All messages in file order, including original publishing/logging times.
    pub fn messages(&self) -> &[McapMessage] {
        &self.messages
    }
    /// Preserved index/statistics bodies; no index target correctness is claimed.
    pub fn auxiliary(&self) -> &[McapAuxiliary] {
        &self.auxiliary
    }
    /// Accounted retained bytes, including object/node allowances.
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
}

struct State {
    limits: McapImportLimits,
    summary: McapImportSummary,
    started: bool,
    data_end: bool,
    finished: bool,
    statistics: bool,
    metadata_records: u64,
    per_channel: BTreeMap<u16, u64>,
    min_time: Option<u64>,
    max_time: Option<u64>,
    outer_offset: u64,
    summary_start: u64,
    offset_start: u64,
    summary_groups: BTreeMap<u8, (u64, u64)>,
    offset_groups: std::collections::BTreeSet<u8>,
}
impl State {
    fn charge(&mut self, n: u64) -> Result<()> {
        add(
            &mut self.summary.stats.state_bytes,
            n,
            self.limits.state_bytes as u64,
            "state bytes",
        )
    }
    fn record(
        &mut self,
        op: u8,
        data: &[u8],
        inside: bool,
        visit: &mut impl FnMut(McapEvent<'_>) -> Result<()>,
    ) -> Result<()> {
        add(
            &mut self.summary.stats.records,
            1,
            self.limits.records,
            "records",
        )?;
        let charge = preflight::validate(op, data, self.limits)?;
        if !self.started && op != 1 {
            return Err(malformed("header must be first"));
        }
        if inside && !matches!(op, 3..=5) {
            return Err(RecordError::UnsupportedMcap(
                "non-definition/message inside chunk",
            ));
        }
        if self.finished {
            return Err(malformed("records after footer"));
        }
        if self.data_end && !matches!(op, 2..=4 | 8 | 0x0b | 0x0d | 0x0e) {
            return Err(malformed("data record after DataEnd"));
        }
        if !self.data_end && matches!(op, 8 | 0x0b | 0x0d | 0x0e) {
            return Err(malformed("summary record before DataEnd"));
        }
        let record = mcap::parse_record(op, data).map_err(vendor)?;
        let start = self.outer_offset;
        if !inside {
            self.outer_offset = start
                .checked_add(9 + data.len() as u64)
                .ok_or_else(|| malformed("record offset overflow"))?;
            if self.data_end && op != 2 {
                if op == 0x0e {
                    if self.summary_start == 0 {
                        return Err(malformed("offsets without summary"));
                    }
                    if self.offset_start == 0 {
                        self.offset_start = start;
                    }
                } else {
                    if self.offset_start != 0 {
                        return Err(malformed("summary after offset section"));
                    }
                    if self.summary_start == 0 {
                        self.summary_start = start;
                    }
                    let group = self.summary_groups.entry(op).or_insert((start, start));
                    if group.1 != start {
                        return Err(malformed("noncontiguous summary group"));
                    }
                    group.1 = self.outer_offset;
                }
            }
        }
        let ordinal = self.summary.stats.records;
        match record {
            Record::Header(h) => {
                if self.started {
                    return Err(malformed("repeated header"));
                }
                self.charge(charge)?;
                self.started = true;
                self.summary.header = McapHeader {
                    profile: h.profile,
                    library: h.library,
                };
                visit(McapEvent {
                    kind: McapEventKind::Header(&self.summary.header),
                    ordinal,
                    accounted_bytes: charge,
                })?;
            }
            Record::Schema { header: h, data } => {
                if h.id == 0 {
                    return Err(malformed("schema ID zero"));
                }
                if h.encoding.is_empty() && !data.is_empty() {
                    return Err(malformed("empty schema encoding with data"));
                }
                let schema = McapSchema {
                    id: h.id,
                    name: h.name,
                    encoding: h.encoding,
                    data: data.into_owned(),
                };
                if let Some(old) = self.summary.schemas.get(&schema.id) {
                    if old != &schema {
                        return Err(malformed("conflicting schema definition"));
                    }
                } else {
                    if self.data_end {
                        return Err(malformed("new schema in summary"));
                    }
                    check(
                        self.summary.schemas.len() as u64 + 1,
                        self.limits.schemas as u64,
                        "schemas",
                    )?;
                    self.charge(charge)?;
                    visit(McapEvent {
                        kind: McapEventKind::Schema(&schema),
                        ordinal,
                        accounted_bytes: charge,
                    })?;
                    self.summary.schemas.insert(schema.id, schema);
                }
            }
            Record::Channel(c) => {
                if c.schema_id != 0 && !self.summary.schemas.contains_key(&c.schema_id) {
                    return Err(malformed("channel references missing schema"));
                }
                let channel = McapChannel {
                    id: c.id,
                    schema_id: c.schema_id,
                    topic: c.topic,
                    message_encoding: c.message_encoding,
                    metadata: c.metadata,
                };
                if let Some(old) = self.summary.channels.get(&channel.id) {
                    if old != &channel {
                        return Err(malformed("conflicting channel definition"));
                    }
                } else {
                    if self.data_end {
                        return Err(malformed("new channel in summary"));
                    }
                    check(
                        self.summary.channels.len() as u64 + 1,
                        self.limits.channels as u64,
                        "channels",
                    )?;
                    self.charge(charge)?;
                    visit(McapEvent {
                        kind: McapEventKind::Channel(&channel),
                        ordinal,
                        accounted_bytes: charge,
                    })?;
                    self.per_channel.insert(channel.id, 0);
                    self.summary.channels.insert(channel.id, channel);
                }
            }
            Record::Message { header: h, data } => {
                let count = self
                    .per_channel
                    .get_mut(&h.channel_id)
                    .ok_or_else(|| malformed("message references missing channel"))?;
                add(
                    &mut self.summary.stats.messages,
                    1,
                    self.limits.messages,
                    "messages",
                )?;
                *count += 1;
                self.min_time = Some(self.min_time.map_or(h.log_time, |t| t.min(h.log_time)));
                self.max_time = Some(self.max_time.map_or(h.log_time, |t| t.max(h.log_time)));
                visit(McapEvent {
                    kind: McapEventKind::Message(McapMessageRef {
                        channel_id: h.channel_id,
                        sequence: h.sequence,
                        log_time: h.log_time,
                        publish_time: h.publish_time,
                        data: &data,
                    }),
                    ordinal,
                    accounted_bytes: charge,
                })?;
            }
            Record::Metadata(m) => {
                self.metadata_records += 1;
                let metadata = McapMetadata {
                    name: m.name,
                    entries: m.metadata,
                };
                if let Some(old) = self.summary.metadata.get(&metadata.name) {
                    if old != &metadata {
                        return Err(malformed("conflicting named metadata"));
                    }
                } else {
                    self.charge(charge)?;
                    visit(McapEvent {
                        kind: McapEventKind::Metadata(&metadata),
                        ordinal,
                        accounted_bytes: charge,
                    })?;
                    self.summary
                        .metadata
                        .insert(metadata.name.clone(), metadata);
                }
            }
            Record::Chunk { header: h, data } => {
                check(
                    h.uncompressed_size,
                    self.limits.chunk_bytes as u64,
                    "decoded chunk bytes",
                )?;
                add(
                    &mut self.summary.stats.decoded_bytes,
                    h.uncompressed_size,
                    self.limits.decoded_bytes,
                    "decoded bytes",
                )?;
                self.summary.stats.chunks += 1;
                let decoded;
                let bytes = match h.compression.as_str() {
                    "" => {
                        if data.len() as u64 != h.uncompressed_size {
                            return Err(malformed("uncompressed chunk length mismatch"));
                        }
                        &*data
                    }
                    "lz4" => {
                        decoded = chunk::decode(&data, h.uncompressed_size as usize)?;
                        &decoded
                    }
                    _ => {
                        return Err(RecordError::UnsupportedMcap(
                            "compression: only uncompressed and LZ4 are enabled",
                        ));
                    }
                };
                self.summary.stats.largest_chunk_bytes =
                    self.summary.stats.largest_chunk_bytes.max(bytes.len());
                if h.uncompressed_crc != 0 && crc32fast::hash(bytes) != h.uncompressed_crc {
                    return Err(malformed("chunk CRC mismatch"));
                }
                // Validate chunk extent/times before delivering its events.
                let mut cursor = preflight::Cursor::new(bytes);
                let mut min = None::<u64>;
                let mut max = None::<u64>;
                while cursor.remaining() > 0 {
                    let op = cursor.take(1)?[0];
                    let n = cursor.u64()?;
                    check(n, self.limits.record_bytes as u64, "record bytes")?;
                    let body = cursor.take(n as usize)?;
                    if !matches!(op, 3..=5) {
                        return Err(RecordError::UnsupportedMcap(
                            "non-definition/message inside chunk",
                        ));
                    }
                    preflight::validate(op, body, self.limits)?;
                    if op == 5 {
                        let time = u64::from_le_bytes(body[6..14].try_into().unwrap());
                        min = Some(min.map_or(time, |v| v.min(time)));
                        max = Some(max.map_or(time, |v| v.max(time)));
                    }
                }
                if min.unwrap_or(0) != h.message_start_time
                    || max.unwrap_or(0) != h.message_end_time
                {
                    return Err(malformed("chunk message-time bounds mismatch"));
                }
                let mut cursor = preflight::Cursor::new(bytes);
                while cursor.remaining() > 0 {
                    let op = cursor.take(1)?[0];
                    let n = cursor.u64()?;
                    self.record(op, cursor.take(n as usize)?, true, visit)?;
                }
            }
            Record::DataEnd(_) => {
                if self.data_end {
                    return Err(malformed("repeated DataEnd"));
                }
                self.data_end = true;
            }
            Record::Footer(footer) => {
                if !self.data_end {
                    return Err(malformed("missing DataEnd"));
                }
                if footer.summary_start != self.summary_start
                    || footer.summary_offset_start != self.offset_start
                {
                    return Err(malformed("footer section offsets disagree with file"));
                }
                self.finished = true;
            }
            Record::Statistics(s) => {
                if self.statistics
                    || s.message_count != self.summary.stats.messages
                    || s.schema_count as usize != self.summary.schemas.len()
                    || s.channel_count as usize != self.summary.channels.len()
                    || s.attachment_count != 0
                    || s.metadata_count as u64 != self.metadata_records
                    || s.chunk_count as u64 != self.summary.stats.chunks
                    || s.message_start_time != self.min_time.unwrap_or(0)
                    || s.message_end_time != self.max_time.unwrap_or(0)
                {
                    return Err(malformed("statistics disagree with observed data"));
                }
                for (id, count) in &s.channel_message_counts {
                    if self.per_channel.get(id) != Some(count) {
                        return Err(malformed("channel statistics mismatch"));
                    }
                }
                self.statistics = true;
                visit(McapEvent {
                    kind: McapEventKind::Auxiliary { opcode: op, data },
                    ordinal,
                    accounted_bytes: charge,
                })?;
            }
            Record::SummaryOffset(offset) => {
                let end = offset
                    .group_start
                    .checked_add(offset.group_length)
                    .ok_or_else(|| malformed("summary group offset overflow"))?;
                let empty = offset.group_length == 0
                    && matches!(offset.group_opcode, 3 | 4 | 8 | 0x0a | 0x0b | 0x0d)
                    && !self.summary_groups.contains_key(&offset.group_opcode)
                    && (offset.group_start == self.offset_start
                        || self
                            .summary_groups
                            .values()
                            .any(|group| group.0 == offset.group_start));
                let matches_group = self.summary_groups.get(&offset.group_opcode)
                    == Some(&(offset.group_start, end));
                if (!empty && !matches_group) || !self.offset_groups.insert(offset.group_opcode) {
                    return Err(malformed("summary offset disagrees with group"));
                }
                visit(McapEvent {
                    kind: McapEventKind::Auxiliary { opcode: op, data },
                    ordinal,
                    accounted_bytes: charge,
                })?;
            }
            Record::MessageIndex(_) | Record::ChunkIndex(_) | Record::MetadataIndex(_) => {
                visit(McapEvent {
                    kind: McapEventKind::Auxiliary { opcode: op, data },
                    ordinal,
                    accounted_bytes: charge,
                })?;
            }
            _ => return Err(RecordError::UnsupportedMcapRecord(op)),
        }
        Ok(())
    }
}

/// Stream a complete MCAP through a synchronous visitor under validated budgets.
/// Reads at most the input budget plus one EOF-probe byte. Callback-owned copies
/// are the caller's responsibility; McapArchive enforces materialization bounds.
pub fn import_mcap(
    mut input: impl Read,
    limits: McapImportLimits,
    mut visit: impl FnMut(McapEvent<'_>) -> Result<()>,
) -> Result<McapImportSummary> {
    let mut parser = LinearReader::new_with_options(
        LinearReaderOptions::default()
            .with_emit_chunks(true)
            .with_record_length_limit(limits.record_bytes)
            .with_validate_data_section_crc(true)
            .with_validate_summary_section_crc(true)
            .with_check_finishes_after_end_magic(true),
    );
    let mut state = State {
        limits,
        summary: McapImportSummary {
            header: McapHeader {
                profile: String::new(),
                library: String::new(),
            },
            schemas: BTreeMap::new(),
            channels: BTreeMap::new(),
            metadata: BTreeMap::new(),
            stats: McapImportStats::default(),
        },
        started: false,
        data_end: false,
        finished: false,
        statistics: false,
        metadata_records: 0,
        per_channel: BTreeMap::new(),
        min_time: None,
        max_time: None,
        outer_offset: 8,
        summary_start: 0,
        offset_start: 0,
        summary_groups: BTreeMap::new(),
        offset_groups: std::collections::BTreeSet::new(),
    };
    while let Some(event) = parser.next_event() {
        match event.map_err(vendor)? {
            LinearReadEvent::ReadRequest(need) => {
                if need > limits.record_bytes + 64 {
                    return Err(RecordError::ImportLimit {
                        kind: "parser read request",
                        limit: limits.record_bytes as u64,
                    });
                }
                let left = limits.input_bytes - state.summary.stats.input_bytes;
                if left == 0 {
                    let n = input.read(&mut [0u8; 1])?;
                    if n != 0 {
                        return Err(RecordError::ImportLimit {
                            kind: "input bytes",
                            limit: limits.input_bytes,
                        });
                    }
                    parser.notify_read(0);
                } else {
                    let size = need
                        .clamp(1, 64 << 10)
                        .min(left.min(usize::MAX as u64) as usize);
                    state.summary.stats.read_buffer_bytes =
                        state.summary.stats.read_buffer_bytes.max(size);
                    let n = input.read(parser.insert(size))?;
                    state.summary.stats.input_bytes += n as u64;
                    parser.notify_read(n);
                }
            }
            LinearReadEvent::Record { opcode, data } => {
                state.record(opcode, data, false, &mut visit)?
            }
        }
    }
    if !state.started || !state.finished {
        return Err(malformed("incomplete MCAP"));
    }
    Ok(state.summary)
}
