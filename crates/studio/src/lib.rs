//! Headless inspection and query model for the Neuradix Robotics Platform.
//!
//! Studio inspection is the *read model* a visualization layer — the Studio UI,
//! an XR scene, or a CLI — queries to understand a recording. It is deliberately
//! headless: no rendering, no windowing, no I/O beyond the recording it is
//! handed. Everything here is a pure, deterministic function of a
//! [`neuradix_record::Recording`], so it works identically over a native `.nrec`
//! or an MCAP container, and identical recordings produce identical answers.
//!
//! # What it answers
//!
//! - [`Inspection::timeline`] — per-domain time spans and per-channel statistics
//!   (counts, first/last time, effective rate, payload sizes).
//! - [`Inspection::window`] — the records on a channel within a time range.
//! - [`Inspection::nearest`] — the record on a channel closest to an instant.
//! - [`Inspection::series`] — a plottable scalar series for a chosen field,
//!   using a caller-supplied [`ScalarDecoder`] so the layer stays neutral about
//!   payload encodings.
//!
//! # Example
//!
//! ```
//! use neuradix_record::{NativeRecordWriter, NativeRecording, RecordingManifest, Channel};
//! use neuradix_studio::Inspection;
//! use neuradix_time::{ClockDomain, Timestamp};
//!
//! let manifest = RecordingManifest::builder("doctest")
//!     .channel(Channel {
//!         id: 0,
//!         name: "depth".to_owned(),
//!         schema_id: "sha256:abc".to_owned(),
//!         clock_domain: "simulation".to_owned(),
//!     })
//!     .build();
//! let mut w = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
//! for i in 0..5u64 {
//!     let ts = Timestamp::new(ClockDomain::Simulation, i as i128 * 20_000_000);
//!     w.write_record(0, i, ts, b"payload").unwrap();
//! }
//! let bytes = w.finish().unwrap();
//! let recording = NativeRecording::from_bytes(&bytes).unwrap();
//!
//! let studio = Inspection::new(&recording);
//! let timeline = studio.timeline();
//! assert_eq!(timeline.message_count, 5);
//! let depth = &timeline.channels[0];
//! assert_eq!(depth.count, 5);
//! // 20 ms spacing -> 50 Hz.
//! assert!((depth.rate_hz.unwrap() - 50.0).abs() < 1e-6);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod inspection;
pub mod model;
pub mod series;

pub use error::StudioError;
pub use inspection::Inspection;
pub use model::{ChannelSummary, DomainSpan, Timeline};
pub use series::{ScalarDecoder, ScalarSample, Series, SeriesPoint, SeriesStats};
