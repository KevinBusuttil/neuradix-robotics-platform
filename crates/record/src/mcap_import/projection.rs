//! Explicit compatibility boundary for the historical one-timestamp container.
use super::{McapArchive, malformed};
use crate::{McapRecording, RawRecord, RecordError, RecordingManifest, Result};
use neuradix_time::{ClockDomain, Timestamp};
use std::collections::{BTreeMap, BTreeSet};

impl McapArchive {
    /// Convert only the fully representable historical Neuradix MCAP profile.
    /// Rejects extra metadata, foreign encodings, unequal publishing/logging
    /// timestamps and missing/conflicting clock provenance. The generic archive
    /// remains the appropriate API for other supported MCAP data.
    pub fn try_into_recording(self) -> Result<McapRecording> {
        let unsupported = || {
            RecordError::UnsupportedMcap(
                "legacy Recording cannot preserve this archive; use McapArchive/import_mcap",
            )
        };
        let summary = &self.summary;
        if summary.header.profile != "neuradix"
            || summary.header.library != concat!("neuradix-record/", env!("CARGO_PKG_VERSION"))
            || summary.metadata.len() != 1
            || self.auxiliary.iter().any(|r| r.opcode != 0x0b)
        {
            return Err(unsupported());
        }
        let metadata = summary
            .metadata
            .get("neuradix.manifest")
            .ok_or_else(unsupported)?;
        if metadata.entries.len() != 1 {
            return Err(unsupported());
        }
        let json = metadata.entries.get("json").ok_or_else(unsupported)?;
        // Deserialize the struct first: serde rejects duplicate known fields.
        let manifest: RecordingManifest =
            serde_json::from_str(json).map_err(RecordError::Manifest)?;
        let original: serde_json::Value =
            serde_json::from_str(json).map_err(RecordError::Manifest)?;
        if serde_json::to_value(&manifest).map_err(RecordError::Manifest)? != original {
            return Err(unsupported()); // Includes unknown nested manifest fields.
        }
        if manifest.format_version != crate::FORMAT_VERSION
            || manifest.channels.len() != summary.channels.len()
        {
            return Err(malformed("manifest/channel set mismatch"));
        }
        let mut domains = BTreeMap::new();
        let mut schemas = BTreeSet::new();
        for c in &manifest.channels {
            let domain = ClockDomain::parse(&c.clock_domain).ok_or_else(unsupported)?;
            if domains.insert(c.id, domain).is_some() {
                return Err(malformed("duplicate manifest channel"));
            }
            let channel = summary
                .channels
                .get(&c.id)
                .ok_or_else(|| malformed("manifest references missing channel"))?;
            let schema = summary
                .schemas
                .get(&channel.schema_id)
                .ok_or_else(unsupported)?;
            if channel.topic != c.name
                || channel.message_encoding != "neuradix"
                || channel.metadata.len() != 2
                || channel.metadata.get("clockDomain") != Some(&c.clock_domain)
                || channel.metadata.get("schemaIdentity") != Some(&c.schema_id)
                || schema.name != c.name
                || schema.encoding != "neuradix/schema-id"
                || schema.data != c.schema_id.as_bytes()
            {
                return Err(unsupported());
            }
            schemas.insert(schema.id);
        }
        if schemas.len() != summary.schemas.len() {
            return Err(unsupported());
        }
        let mut records = Vec::with_capacity(self.messages.len());
        for message in self.messages {
            if message.log_time != message.publish_time {
                return Err(unsupported());
            }
            let domain = *domains
                .get(&message.channel_id)
                .ok_or_else(|| malformed("missing manifest channel"))?;
            records.push(RawRecord {
                channel_id: message.channel_id,
                sequence: u64::from(message.sequence),
                timestamp: Timestamp::new(domain, i128::from(message.log_time)),
                payload: message.data,
            });
        }
        Ok(McapRecording::from_import(manifest, records))
    }
}
