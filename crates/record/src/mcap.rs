//! Checked historical Neuradix MCAP compatibility interface.
//!
//! [`McapWriter`] streams through [`crate::McapStreamWriter`]. Full schemas and
//! distinct source/log times use that generic interface. Historical buffered
//! recordings remain readable, including the narrowly scoped summary exception
//! documented by the importer. New files use a Statistics-only summary and CRCs.
use crate::error::Result;
pub use crate::mcap_write::legacy::McapWriter;
use crate::model::{RawRecord, RecordingManifest};
/// MCAP leading and trailing magic.
pub const MCAP_MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', b'0', 0x0D, 0x0A];

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
