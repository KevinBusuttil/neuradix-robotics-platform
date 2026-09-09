//! An MCAP recording backend.
//!
//! [MCAP](https://mcap.dev) is a widely supported container for timestamped
//! pub/sub messages (Foxglove, ROS 2 tooling, …). This module implements an
//! uncompressed, spec-compliant MCAP **writer** and a matching **reader** behind
//! the same recording surface as the native container: [`McapWriter`] mirrors
//! `NativeRecordWriter`'s method set, and [`McapRecording`] implements
//! [`crate::recording::Recording`], so digest, inspection and replay work
//! unchanged across containers.
//!
//! ## What is written
//!
//! ```text
//! magic (8) | Header | Metadata(manifest) | {Schema,Channel}* | Message* | DataEnd
//!           | {Schema,Channel}* Statistics (summary) | Footer | magic (8)
//! ```
//!
//! Message payloads are opaque bytes (`message_encoding = "neuradix"`); each
//! channel records its clock domain and the payload's content-addressed schema
//! identity in channel metadata, and the full Neuradix manifest is embedded
//! losslessly as an MCAP metadata record so a recording round-trips exactly.
//! CRC fields are written as `0` ("not computed"), which the MCAP spec permits.

use std::collections::BTreeMap;
use std::io::Write;

use neuradix_time::{ClockDomain, Timestamp};

use crate::error::{RecordError, Result};
use crate::model::{Channel, RawRecord, RecordingManifest};

/// The 8-byte MCAP magic at the start and end of every file: `\x89 MCAP0 \r \n`.
pub const MCAP_MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', b'0', 0x0D, 0x0A];

/// Record opcodes used by this backend (see the MCAP specification).
mod op {
    pub const HEADER: u8 = 0x01;
    pub const FOOTER: u8 = 0x02;
    pub const SCHEMA: u8 = 0x03;
    pub const CHANNEL: u8 = 0x04;
    pub const MESSAGE: u8 = 0x05;
    pub const STATISTICS: u8 = 0x0B;
    pub const METADATA: u8 = 0x0C;
    pub const DATA_END: u8 = 0x0F;
}

const PROFILE: &str = "neuradix";
const LIBRARY: &str = concat!("neuradix-record/", env!("CARGO_PKG_VERSION"));
const MESSAGE_ENCODING: &str = "neuradix";
const SCHEMA_ENCODING: &str = "neuradix/schema-id";
const MANIFEST_METADATA: &str = "neuradix.manifest";
const MANIFEST_KEY: &str = "json";
const META_CLOCK_DOMAIN: &str = "clockDomain";
const META_SCHEMA_IDENTITY: &str = "schemaIdentity";

// ---------------------------------------------------------------------------
// Writer.
// ---------------------------------------------------------------------------

/// Streaming writer for the MCAP container, mirroring `NativeRecordWriter`.
///
/// Records are buffered and the complete MCAP file is emitted on [`finish`]
/// (MCAP's summary/statistics require knowing the whole recording).
///
/// [`finish`]: McapWriter::finish
pub struct McapWriter<W: Write> {
    inner: W,
    manifest: RecordingManifest,
    channel_domains: BTreeMap<u16, ClockDomain>,
    channel_order: Vec<u16>,
    records: Vec<BufferedRecord>,
}

struct BufferedRecord {
    channel_id: u16,
    sequence: u32,
    log_time: u64,
    payload: Vec<u8>,
}

impl<W: Write> McapWriter<W> {
    /// Begin an MCAP recording with the given manifest.
    pub fn new(inner: W, manifest: &RecordingManifest) -> Result<Self> {
        Ok(Self {
            inner,
            manifest: manifest.clone(),
            channel_domains: BTreeMap::new(),
            channel_order: Vec::new(),
            records: Vec::new(),
        })
    }

    /// Buffer one record. Matches `NativeRecordWriter::write_record`.
    pub fn write_record(
        &mut self,
        channel_id: u16,
        sequence: u64,
        timestamp: Timestamp,
        payload: &[u8],
    ) -> Result<()> {
        let sequence =
            u32::try_from(sequence).map_err(|_| RecordError::SequenceTooLarge(sequence))?;
        let nanos = timestamp.as_nanos();
        let log_time = u64::try_from(nanos).map_err(|_| RecordError::TimestampOutOfRange(nanos))?;

        // Remember each channel's domain (first sample wins) so it can be
        // reconstructed on read.
        if let std::collections::btree_map::Entry::Vacant(e) =
            self.channel_domains.entry(channel_id)
        {
            e.insert(timestamp.domain());
            self.channel_order.push(channel_id);
        }

        self.records.push(BufferedRecord {
            channel_id,
            sequence,
            log_time,
            payload: payload.to_vec(),
        });
        Ok(())
    }

    /// Emit the complete MCAP file and return the underlying writer.
    pub fn finish(mut self) -> Result<W> {
        let bytes = self.encode()?;
        self.inner.write_all(&bytes)?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    /// Build the complete MCAP byte image.
    fn encode(&self) -> Result<Vec<u8>> {
        let channels = self.resolve_channels()?;

        let mut out = Vec::new();
        out.extend_from_slice(&MCAP_MAGIC);

        // Header.
        let mut header = Vec::new();
        put_str(&mut header, PROFILE)?;
        put_str(&mut header, LIBRARY)?;
        frame(&mut out, op::HEADER, &header);

        // Embed the manifest losslessly as a single metadata record.
        let manifest_json = serde_json::to_string(&self.manifest).map_err(RecordError::Manifest)?;
        let mut meta = Vec::new();
        put_str(&mut meta, MANIFEST_METADATA)?;
        put_map(&mut meta, &[(MANIFEST_KEY, manifest_json.as_str())])?;
        frame(&mut out, op::METADATA, &meta);
        let metadata_count: u32 = 1;

        // Schema + Channel per channel (data section).
        for ch in &channels {
            self.write_schema_and_channel(&mut out, ch)?;
        }

        // Messages, in recorded order.
        for r in &self.records {
            let mut m = Vec::new();
            put_u16(&mut m, r.channel_id);
            put_u32(&mut m, r.sequence);
            put_u64(&mut m, r.log_time); // log_time
            put_u64(&mut m, r.log_time); // publish_time (same)
            m.extend_from_slice(&r.payload);
            frame(&mut out, op::MESSAGE, &m);
        }

        // DataEnd (data_section_crc32 = 0 = not computed).
        let mut data_end = Vec::new();
        put_u32(&mut data_end, 0);
        frame(&mut out, op::DATA_END, &data_end);

        // Summary section: repeat Schema/Channel, then Statistics.
        let summary_start = out.len() as u64;
        for ch in &channels {
            self.write_schema_and_channel(&mut out, ch)?;
        }
        self.write_statistics(&mut out, &channels, metadata_count);

        // Footer: point at the summary section; no summary-offset section; crc 0.
        let mut footer = Vec::new();
        put_u64(&mut footer, summary_start);
        put_u64(&mut footer, 0); // ofs_summary_offset_section (absent)
        put_u32(&mut footer, 0); // summary_crc32 (not computed)
        frame(&mut out, op::FOOTER, &footer);

        out.extend_from_slice(&MCAP_MAGIC);
        Ok(out)
    }

    /// The channels to emit, each with a stable MCAP schema id, in a
    /// deterministic order (record-bearing channels first, then manifest-only
    /// channels).
    fn resolve_channels(&self) -> Result<Vec<ResolvedChannel>> {
        let mut resolved: Vec<ResolvedChannel> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        // Index manifest channels once (avoids O(n^2) lookups for large
        // recordings).
        let by_id: BTreeMap<u16, &Channel> =
            self.manifest.channels.iter().map(|c| (c.id, c)).collect();

        let mut push = |channel_id: u16,
                        domain: ClockDomain,
                        resolved: &mut Vec<ResolvedChannel>|
         -> Result<()> {
            if !seen.insert(channel_id) {
                return Ok(());
            }
            // MCAP schema ids are `u16` and 0 is reserved ("no schema"), so ids
            // run 1..=u16::MAX. Fail cleanly rather than overflow the counter
            // (which would panic in debug and wrap to the reserved id 0 in
            // release, corrupting the file).
            let schema_id = u16::try_from(resolved.len() + 1)
                .map_err(|_| RecordError::TooManyChannels(resolved.len() + 1))?;
            let (name, schema_identity) = match by_id.get(&channel_id) {
                Some(c) => (c.name.clone(), c.schema_id.clone()),
                None => (format!("channel-{channel_id}"), String::new()),
            };
            resolved.push(ResolvedChannel {
                channel_id,
                schema_id,
                name,
                schema_identity,
                domain,
            });
            Ok(())
        };

        // Record-bearing channels first (domain is the recorded truth).
        for &id in &self.channel_order {
            let domain = self
                .channel_domains
                .get(&id)
                .copied()
                .unwrap_or(ClockDomain::Monotonic);
            push(id, domain, &mut resolved)?;
        }
        // Then manifest channels that carried no records.
        for c in &self.manifest.channels {
            let domain = ClockDomain::parse(&c.clock_domain).unwrap_or(ClockDomain::Monotonic);
            push(c.id, domain, &mut resolved)?;
        }
        Ok(resolved)
    }

    fn write_schema_and_channel(&self, out: &mut Vec<u8>, ch: &ResolvedChannel) -> Result<()> {
        // Schema record: carries the content-addressed schema identity as data.
        let mut schema = Vec::new();
        put_u16(&mut schema, ch.schema_id);
        put_str(&mut schema, &ch.name)?;
        put_str(&mut schema, SCHEMA_ENCODING)?;
        put_bytes(&mut schema, ch.schema_identity.as_bytes())?;
        frame(out, op::SCHEMA, &schema);

        // Channel record.
        let mut channel = Vec::new();
        put_u16(&mut channel, ch.channel_id);
        put_u16(&mut channel, ch.schema_id);
        put_str(&mut channel, &ch.name)?;
        put_str(&mut channel, MESSAGE_ENCODING)?;
        put_map(
            &mut channel,
            out_map_pairs(&ch.domain, &ch.schema_identity).as_slice(),
        )?;
        frame(out, op::CHANNEL, &channel);
        Ok(())
    }

    fn write_statistics(
        &self,
        out: &mut Vec<u8>,
        channels: &[ResolvedChannel],
        metadata_count: u32,
    ) {
        let (start, end) = self.records.iter().fold((u64::MAX, 0u64), |(lo, hi), r| {
            (lo.min(r.log_time), hi.max(r.log_time))
        });
        let start = if self.records.is_empty() { 0 } else { start };

        let mut counts = Vec::new();
        for ch in channels {
            let n = self
                .records
                .iter()
                .filter(|r| r.channel_id == ch.channel_id)
                .count() as u64;
            put_u16(&mut counts, ch.channel_id);
            put_u64(&mut counts, n);
        }

        let mut stats = Vec::new();
        put_u64(&mut stats, self.records.len() as u64); // message_count
        put_u16(&mut stats, channels.len() as u16); // schema_count
        put_u32(&mut stats, channels.len() as u32); // channel_count
        put_u32(&mut stats, 0); // attachment_count
        put_u32(&mut stats, metadata_count); // metadata_count (the embedded manifest)
        put_u32(&mut stats, 0); // chunk_count
        put_u64(&mut stats, start); // message_start_time
        put_u64(&mut stats, end); // message_end_time
        put_u32(&mut stats, counts.len() as u32); // len_channel_message_counts
        stats.extend_from_slice(&counts);
        frame(out, op::STATISTICS, &stats);
    }
}

struct ResolvedChannel {
    channel_id: u16,
    schema_id: u16,
    name: String,
    schema_identity: String,
    domain: ClockDomain,
}

fn out_map_pairs<'a>(domain: &'a ClockDomain, schema_identity: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        (META_CLOCK_DOMAIN, domain.as_str()),
        (META_SCHEMA_IDENTITY, schema_identity),
    ]
}

// ---------------------------------------------------------------------------
// Reader.
// ---------------------------------------------------------------------------

/// A fully parsed MCAP recording held in memory.
#[derive(Debug, Clone)]
pub struct McapRecording {
    manifest: RecordingManifest,
    records: Vec<RawRecord>,
}

impl McapRecording {
    /// Import the historical Neuradix profile under default bounded limits.
    /// Other supported MCAP data belongs in [`crate::McapArchive`]; lossy
    /// projection to this legacy one-timestamp view rejects explicitly.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_reader(bytes, crate::McapImportLimits::default())
    }

    /// Bounded reader with explicit policy and checked legacy projection.
    pub fn from_reader(
        reader: impl std::io::Read,
        limits: crate::McapImportLimits,
    ) -> Result<Self> {
        crate::McapArchive::from_reader(reader, limits)?.try_into_recording()
    }

    pub(crate) fn from_import(manifest: RecordingManifest, records: Vec<RawRecord>) -> Self {
        Self { manifest, records }
    }

    /// The recording manifest.
    pub fn manifest(&self) -> &RecordingManifest {
        &self.manifest
    }

    /// All records, in recorded order.
    pub fn records(&self) -> &[RawRecord] {
        &self.records
    }
}

impl crate::recording::Recording for McapRecording {
    fn manifest(&self) -> &RecordingManifest {
        &self.manifest
    }

    fn records(&self) -> &[RawRecord] {
        &self.records
    }
}

// ---------------------------------------------------------------------------
// Encoding helpers.
// ---------------------------------------------------------------------------

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A `u32`-length-prefixed UTF-8 string.
fn put_str(out: &mut Vec<u8>, s: &str) -> Result<()> {
    put_bytes(out, s.as_bytes())
}

/// A `u32`-length-prefixed byte array.
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) -> Result<()> {
    let len = u32::try_from(b.len()).map_err(|_| RecordError::PayloadTooLarge(b.len()))?;
    put_u32(out, len);
    out.extend_from_slice(b);
    Ok(())
}

/// A `Map<string,string>`: a `u32` byte length then repeated key/value strings.
fn put_map(out: &mut Vec<u8>, pairs: &[(&str, &str)]) -> Result<()> {
    let mut inner = Vec::new();
    for (k, v) in pairs {
        put_str(&mut inner, k)?;
        put_str(&mut inner, v)?;
    }
    let len = u32::try_from(inner.len()).map_err(|_| RecordError::PayloadTooLarge(inner.len()))?;
    put_u32(out, len);
    out.extend_from_slice(&inner);
    Ok(())
}

/// Frame a record: opcode, `u64` content length, content.
fn frame(out: &mut Vec<u8>, opcode: u8, content: &[u8]) {
    out.push(opcode);
    put_u64(out, content.len() as u64);
    out.extend_from_slice(content);
}
