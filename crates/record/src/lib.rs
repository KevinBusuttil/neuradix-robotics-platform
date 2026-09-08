//! # neuradix-record
//!
//! Deterministic recording and replay. This increment provides a native,
//! self-describing recording container, a payload-agnostic [`RecordCodec`], and
//! a [`replay_digest`] for replay-equivalence checks.
//!
//! The recorder stores opaque payload bytes plus a manifest (channels, schema
//! identities, clock domains and software provenance), so a recording carries
//! enough context to be interpreted and reproduced. External containers such as
//! MCAP are intended to be added later behind the same writer/reader surface.
//!
//! ```
//! use neuradix_record::{NativeRecordWriter, NativeRecording, RecordingManifest, replay_digest};
//! use neuradix_time::{ClockDomain, Timestamp};
//!
//! let manifest = RecordingManifest::builder("neuradix-record/doctest").build();
//! let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
//! writer.write_record(0, 0, Timestamp::new(ClockDomain::Simulation, 0), b"hello").unwrap();
//! let bytes = writer.finish().unwrap();
//!
//! let recording = NativeRecording::from_bytes(&bytes).unwrap();
//! assert_eq!(recording.records().len(), 1);
//! assert!(replay_digest(&recording).starts_with("sha256:"));
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod digest;
pub mod error;
pub mod mcap;
pub mod model;
pub mod native;
pub mod recording;

pub use codec::RecordCodec;
pub use digest::replay_digest;
pub use error::{RecordError, Result};
pub use mcap::{MCAP_MAGIC, McapRecording, McapWriter};
pub use model::{
    Channel, FORMAT_VERSION, ManifestBuilder, RawRecord, RecordingManifest, SoftwareId,
};
pub use native::{MAGIC, NativeRecordWriter, NativeRecording};
pub use recording::Recording;
