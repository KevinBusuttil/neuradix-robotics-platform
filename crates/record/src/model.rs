//! The recording data model: manifest, channels and records.

use neuradix_contracts::SchemaId;
use neuradix_time::{ClockDomain, Timestamp};
use serde::{Deserialize, Serialize};

/// The current native recording format version.
pub const FORMAT_VERSION: u8 = 1;

/// A logical channel within a recording (one contract stream).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Stable channel id, referenced by every record on this channel.
    pub id: u16,
    /// Human-readable channel name (e.g. `navigation/vehicle-depth`).
    pub name: String,
    /// Content-addressed schema identity of the channel's payload contract.
    pub schema_id: String,
    /// The clock domain of the channel's authoritative timestamps.
    pub clock_domain: String,
}

impl Channel {
    /// Construct a channel from typed identity and domain values.
    pub fn new(id: u16, name: impl Into<String>, schema: &SchemaId, domain: ClockDomain) -> Self {
        Self {
            id,
            name: name.into(),
            schema_id: schema.as_str().to_owned(),
            clock_domain: domain.as_str().to_owned(),
        }
    }
}

/// A software component identity captured for provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareId {
    /// Package or component name.
    pub name: String,
    /// Version string.
    pub version: String,
}

impl SoftwareId {
    /// Construct a software identity.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// The recording manifest: everything needed (besides the samples) to interpret
/// and reproduce a recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingManifest {
    /// Native container format version.
    pub format_version: u8,
    /// Identifier of the writer that produced the recording.
    pub writer: String,
    /// Channels present in the recording.
    pub channels: Vec<Channel>,
    /// Software identities captured for provenance.
    pub software: Vec<SoftwareId>,
    /// The random seed used, if the run was seeded.
    pub seed: Option<u64>,
    /// A free-form note (e.g. scenario or mission name).
    pub note: Option<String>,
}

impl RecordingManifest {
    /// Start building a manifest with the given writer identifier.
    pub fn builder(writer: impl Into<String>) -> ManifestBuilder {
        ManifestBuilder {
            manifest: RecordingManifest {
                format_version: FORMAT_VERSION,
                writer: writer.into(),
                channels: Vec::new(),
                software: Vec::new(),
                seed: None,
                note: None,
            },
        }
    }

    /// Look up a channel by id.
    pub fn channel(&self, id: u16) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }
}

/// Builder for a [`RecordingManifest`].
#[derive(Debug, Clone)]
pub struct ManifestBuilder {
    manifest: RecordingManifest,
}

impl ManifestBuilder {
    /// Add a channel.
    pub fn channel(mut self, channel: Channel) -> Self {
        self.manifest.channels.push(channel);
        self
    }

    /// Add a software provenance entry.
    pub fn software(mut self, software: SoftwareId) -> Self {
        self.manifest.software.push(software);
        self
    }

    /// Set the random seed.
    pub fn seed(mut self, seed: u64) -> Self {
        self.manifest.seed = Some(seed);
        self
    }

    /// Set a free-form note.
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.manifest.note = Some(note.into());
        self
    }

    /// Finish building.
    pub fn build(self) -> RecordingManifest {
        self.manifest
    }
}

/// A single recorded sample as raw (encoded) bytes plus metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    /// The channel this record belongs to.
    pub channel_id: u16,
    /// Per-channel monotonically increasing sequence number.
    pub sequence: u64,
    /// The domain-tagged timestamp of the sample.
    pub timestamp: Timestamp,
    /// The encoded payload bytes.
    pub payload: Vec<u8>,
}
