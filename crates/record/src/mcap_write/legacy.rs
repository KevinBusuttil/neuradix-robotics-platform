//! Checked historical-profile streaming facade.
use super::{McapStreamWriter, McapWriteLimits, limits};
use crate::{McapHeader, McapSchema, McapChannel, McapMetadata, McapMessageRef, RecordError, RecordingManifest, Result};
use neuradix_time::{ClockDomain, Timestamp};
use std::{collections::BTreeMap, io::{self, Write}};

/// Bounded historical-profile writer. Definitions follow manifest order;
/// unknown channels/domains and per-record domain changes reject explicitly.
/// Payloads are written immediately. Every write error latches failure.
pub struct McapWriter<W: Write> {
    writer: McapStreamWriter<W>,
    domains: BTreeMap<u16, ClockDomain>,
}
impl<W: Write> McapWriter<W> {
    /// Begin with default inclusive budgets. Initialization may write a prefix.
    pub fn new(inner: W, manifest: &RecordingManifest) -> Result<Self> {
        Self::with_limits(inner, manifest, McapWriteLimits::default())
    }
    /// Begin with validated policy. Manifest JSON is sized before allocating it.
    pub fn with_limits(inner: W, manifest: &RecordingManifest, policy: McapWriteLimits) -> Result<Self> {
        if manifest.format_version != crate::FORMAT_VERSION { return Err(RecordError::UnsupportedVersion(manifest.format_version)); }
        if manifest.channels.len() > u16::MAX as usize { return Err(RecordError::TooManyChannels(manifest.channels.len())); }
        limits::check(manifest.channels.len() as u64, policy.channels() as u64, "channels")?;
        limits::check(manifest.channels.len() as u64, policy.schemas() as u64, "schemas")?;
        let mut counter = Counter { bytes: 0, limit: policy.string_bytes() };
        serde_json::to_writer(&mut counter, manifest).map_err(|_| RecordError::WriteLimit { kind: "manifest JSON bytes", limit: policy.string_bytes() as u64 })?;
        // Metadata body: name prefix/name, map prefix, key prefix/key, value prefix/JSON.
        let metadata_body = limits::add(counter.bytes as u64, 36)?;
        limits::check(metadata_body, policy.record_bytes() as u64, "record bytes")?;
        limits::check(limits::charge(metadata_body,1)?, policy.state_bytes() as u64, "state bytes")?;
        let json = serde_json::to_string(manifest).map_err(RecordError::Manifest)?;
        let mut writer = McapStreamWriter::new(inner, &McapHeader { profile:"neuradix".into(), library:concat!("neuradix-record/",env!("CARGO_PKG_VERSION")).into() }, policy)?;
        writer.metadata(&McapMetadata { name:"neuradix.manifest".into(),entries:BTreeMap::from([("json".into(),json)]) })?;
        let mut domains=BTreeMap::new();
        for (index,c) in manifest.channels.iter().enumerate() {
            let domain=ClockDomain::parse(&c.clock_domain).ok_or(RecordError::InvalidMcapWrite("unknown manifest clock domain"))?;
            if domains.contains_key(&c.id) { return Err(RecordError::InvalidMcapWrite("duplicate manifest channel")); }
            // JSON admission bounded all strings and caller clones before these allocations.
            let schema_id=(index+1) as u16;
            writer.schema(&McapSchema { id:schema_id,name:c.name.clone(),encoding:"neuradix/schema-id".into(),data:c.schema_id.as_bytes().to_vec() })?;
            writer.channel(&McapChannel { id:c.id,schema_id,topic:c.name.clone(),message_encoding:"neuradix".into(),metadata:BTreeMap::from([("clockDomain".into(),c.clock_domain.clone()),("schemaIdentity".into(),c.schema_id.clone())]) })?;
            domains.insert(c.id,domain);
        }
        Ok(Self { writer,domains })
    }
    /// Write one checked legacy record; its single timestamp supplies both MCAP times.
    pub fn write_record(&mut self, channel_id:u16, sequence:u64, timestamp:Timestamp, payload:&[u8]) -> Result<()> {
        if self.domains.get(&channel_id) != Some(&timestamp.domain()) {
            return self.writer.reject(RecordError::InvalidMcapWrite("unknown channel or clock-domain mismatch"));
        }
        let sequence=match u32::try_from(sequence) { Ok(n)=>n, Err(_)=>return self.writer.reject(RecordError::SequenceTooLarge(sequence)) };
        let nanos=timestamp.as_nanos();
        let time=match u64::try_from(nanos) { Ok(n)=>n, Err(_)=>return self.writer.reject(RecordError::TimestampOutOfRange(nanos)) };
        self.writer.message(McapMessageRef { channel_id,sequence,log_time:time,publish_time:time,data:payload })
    }
    /// Flush without finalizing; a flush failure latches failure.
    pub fn flush(&mut self) -> Result<()> { self.writer.flush() }
    /// Report the latched failure state.
    pub fn is_failed(&self) -> bool { self.writer.is_failed() }
    /// Finish the reserved summary/footer and return the flushed sink.
    pub fn finish(self) -> Result<W> { self.writer.finish() }
}
struct Counter { bytes:usize,limit:usize }
impl Write for Counter {
    fn write(&mut self, data:&[u8]) -> io::Result<usize> {
        self.bytes=self.bytes.checked_add(data.len()).filter(|n| *n<=self.limit).ok_or_else(|| io::Error::other("manifest size limit"))?;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
