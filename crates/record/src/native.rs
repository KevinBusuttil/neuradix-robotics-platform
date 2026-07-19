//! The native, self-describing recording container.
//!
//! ## Layout
//!
//! ```text
//! magic    : "NRXREC"            (6 bytes)
//! version  : u8                  (= FORMAT_VERSION)
//! manifest : u32 length (LE) + UTF-8 JSON manifest
//! records  : repeated
//!            channel_id : u16  (LE)
//!            sequence   : u64  (LE)
//!            domain     : u8   (ClockDomain::code)
//!            nanos      : i128 (16 bytes, LE)
//!            length     : u32  (LE)
//!            payload    : `length` bytes
//! ```
//!
//! The format is fixed-endian and length-prefixed, so a recording round-trips
//! byte-for-byte and can be read back with no ambiguity. MCAP and other external
//! containers are intended to be added later behind the same reader/writer
//! surface without changing recorded component code.

use std::io::Write;

use neuradix_time::{ClockDomain, Timestamp};

use crate::error::{RecordError, Result};
use crate::model::{FORMAT_VERSION, RawRecord, RecordingManifest};

/// Magic bytes identifying a native Neuradix recording.
pub const MAGIC: [u8; 6] = *b"NRXREC";

/// Streaming writer for the native recording container.
pub struct NativeRecordWriter<W: Write> {
    inner: W,
}

impl<W: Write> NativeRecordWriter<W> {
    /// Begin a recording, writing the header and manifest to `inner`.
    pub fn new(mut inner: W, manifest: &RecordingManifest) -> Result<Self> {
        inner.write_all(&MAGIC)?;
        inner.write_all(&[FORMAT_VERSION])?;
        let json = serde_json::to_vec(manifest).map_err(RecordError::Manifest)?;
        let len =
            u32::try_from(json.len()).map_err(|_| RecordError::PayloadTooLarge(json.len()))?;
        inner.write_all(&len.to_le_bytes())?;
        inner.write_all(&json)?;
        Ok(Self { inner })
    }

    /// Append one record.
    pub fn write_record(
        &mut self,
        channel_id: u16,
        sequence: u64,
        timestamp: Timestamp,
        payload: &[u8],
    ) -> Result<()> {
        let len = u32::try_from(payload.len())
            .map_err(|_| RecordError::PayloadTooLarge(payload.len()))?;
        self.inner.write_all(&channel_id.to_le_bytes())?;
        self.inner.write_all(&sequence.to_le_bytes())?;
        self.inner.write_all(&[timestamp.domain().code()])?;
        self.inner.write_all(&timestamp.as_nanos().to_le_bytes())?;
        self.inner.write_all(&len.to_le_bytes())?;
        self.inner.write_all(payload)?;
        Ok(())
    }

    /// Flush and return the underlying writer.
    pub fn finish(mut self) -> Result<W> {
        self.inner.flush()?;
        Ok(self.inner)
    }
}

/// A fully parsed recording held in memory.
#[derive(Debug, Clone)]
pub struct NativeRecording {
    manifest: RecordingManifest,
    records: Vec<RawRecord>,
}

impl NativeRecording {
    /// Parse a recording from its byte representation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);

        if cursor.take(MAGIC.len())? != MAGIC {
            return Err(RecordError::BadMagic);
        }
        let version = cursor.take(1)?[0];
        if version != FORMAT_VERSION {
            return Err(RecordError::UnsupportedVersion(version));
        }

        let manifest_len = cursor.u32()? as usize;
        let manifest_bytes = cursor.take(manifest_len)?;
        let manifest: RecordingManifest =
            serde_json::from_slice(manifest_bytes).map_err(RecordError::Manifest)?;

        let mut records = Vec::new();
        while !cursor.is_empty() {
            let channel_id = cursor.u16()?;
            let sequence = cursor.u64()?;
            let domain_code = cursor.take(1)?[0];
            let domain = ClockDomain::from_code(domain_code)
                .ok_or(RecordError::UnknownClockDomain(domain_code))?;
            let nanos = cursor.i128()?;
            let payload_len = cursor.u32()? as usize;
            let payload = cursor.take(payload_len)?.to_vec();
            records.push(RawRecord {
                channel_id,
                sequence,
                timestamp: Timestamp::new(domain, nanos),
                payload,
            });
        }

        Ok(Self { manifest, records })
    }

    /// The recording manifest.
    pub fn manifest(&self) -> &RecordingManifest {
        &self.manifest
    }

    /// All records, in recorded order.
    pub fn records(&self) -> &[RawRecord] {
        &self.records
    }

    /// The number of records on a given channel.
    pub fn count_for(&self, channel_id: u16) -> usize {
        self.records
            .iter()
            .filter(|r| r.channel_id == channel_id)
            .count()
    }

    /// Iterate the records on a given channel, in recorded order.
    pub fn records_for(&self, channel_id: u16) -> impl Iterator<Item = &RawRecord> {
        self.records
            .iter()
            .filter(move |r| r.channel_id == channel_id)
    }
}

impl crate::recording::Recording for NativeRecording {
    fn manifest(&self) -> &RecordingManifest {
        &self.manifest
    }

    fn records(&self) -> &[RawRecord] {
        &self.records
    }
}

/// A minimal, bounds-checked byte cursor that never panics.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(RecordError::Truncated(self.pos))?;
        if end > self.bytes.len() {
            return Err(RecordError::Truncated(self.pos));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_le_bytes(arr))
    }

    fn i128(&mut self) -> Result<i128> {
        let b = self.take(16)?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(b);
        Ok(i128::from_le_bytes(arr))
    }
}
