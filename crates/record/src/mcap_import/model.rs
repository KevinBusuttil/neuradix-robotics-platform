//! Container-faithful semantic types; payloads and schema encodings remain opaque.
use std::collections::BTreeMap;

/// Original producer profile/library declarations; neither is authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapHeader {
    /// Original profile declaration.
    pub profile: String,
    /// Original producing library declaration.
    pub library: String,
}
/// A full schema definition, without interpreting its schema language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapSchema {
    /// File-local nonzero schema ID.
    pub id: u16,
    /// Original name.
    pub name: String,
    /// Original encoding identifier.
    pub encoding: String,
    /// Original schema bytes.
    pub data: Vec<u8>,
}
/// A channel with its full encoding and producer metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapChannel {
    /// File-local channel ID.
    pub id: u16,
    /// File-local schema ID; zero means no schema.
    pub schema_id: u16,
    /// Original topic.
    pub topic: String,
    /// Original payload encoding; data is retained without decoding it.
    pub message_encoding: String,
    /// Original metadata. No default clock domain or epoch is inferred.
    pub metadata: BTreeMap<String, String>,
}
/// Named user metadata. Conflicting reuse of a name rejects this import profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapMetadata {
    /// Original name.
    pub name: String,
    /// Original key/value map.
    pub entries: BTreeMap<String, String>,
}
/// Borrowed message valid only during the visitor call.
#[derive(Debug, Clone, Copy)]
pub struct McapMessageRef<'a> {
    /// File-local channel ID.
    pub channel_id: u16,
    /// Original optional sequence counter; zero/repetitions are valid MCAP.
    pub sequence: u32,
    /// Original logging nanoseconds, with producer-defined epoch.
    pub log_time: u64,
    /// Original publishing nanoseconds, independently retained.
    pub publish_time: u64,
    /// Original opaque payload, borrowed from one bounded record/chunk.
    pub data: &'a [u8],
}
/// A retained message; no timestamp conversion or epoch assumption is performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapMessage {
    /// File-local channel ID.
    pub channel_id: u16,
    /// Original optional sequence counter.
    pub sequence: u32,
    /// Original logging nanoseconds.
    pub log_time: u64,
    /// Original publishing nanoseconds.
    pub publish_time: u64,
    /// Original opaque payload.
    pub data: Vec<u8>,
}
/// An index/statistics record retained for audit, never used to skip file data.
/// Its bytes are preserved; index target correctness is not certified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapAuxiliary {
    /// Original opcode.
    pub opcode: u8,
    /// Logical record ordinal, including chunk envelopes and inner records.
    pub ordinal: u64,
    /// Original record body.
    pub data: Vec<u8>,
}
/// A semantic streaming event. Identical repeated definitions are normalized.
#[derive(Debug)]
pub enum McapEventKind<'a> {
    /// Producer declarations.
    Header(&'a McapHeader),
    /// New complete schema definition.
    Schema(&'a McapSchema),
    /// New complete channel definition.
    Channel(&'a McapChannel),
    /// New named metadata definition.
    Metadata(&'a McapMetadata),
    /// Message in file order, without sorting or sequence filtering.
    Message(McapMessageRef<'a>),
    /// Structurally parsed index/statistics bytes; offsets are not trusted.
    Auxiliary {
        /// Record opcode.
        opcode: u8,
        /// Original body.
        data: &'a [u8],
    },
}
/// Visitor event, provisional until the complete import returns success.
#[derive(Debug)]
pub struct McapEvent<'a> {
    /// Semantic value, borrowed only for this callback.
    pub kind: McapEventKind<'a>,
    /// Logical ordinal; duplicate definitions can leave gaps between events.
    pub ordinal: u64,
    /// Conservative retained-storage charge used by the materializer.
    pub accounted_bytes: u64,
}
/// Executed import counters. These are byte/count budgets, not process RSS.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct McapImportStats {
    /// Actual file bytes consumed, excluding the single EOF probe.
    pub input_bytes: u64,
    /// Physical records plus decoded inner records.
    pub records: u64,
    /// Accepted messages.
    pub messages: u64,
    /// Decoded chunks.
    pub chunks: u64,
    /// Cumulative declared and verified decoded chunk bytes.
    pub decoded_bytes: u64,
    /// Conservative bytes charged for retained definitions.
    pub state_bytes: u64,
    /// Largest supplied parser read buffer.
    pub read_buffer_bytes: usize,
    /// Largest decoded chunk.
    pub largest_chunk_bytes: usize,
}
