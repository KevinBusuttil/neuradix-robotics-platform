//! The backend-neutral recording surface.
//!
//! A recording is a manifest plus an ordered list of raw records, regardless of
//! the container it was read from. The native `.nrec` container and the MCAP
//! container both parse into something implementing [`Recording`], so digest,
//! inspection and replay are written once against this trait rather than against
//! a specific container.

use crate::model::{RawRecord, RecordingManifest};

/// A parsed recording: a manifest and its ordered records.
///
/// Implemented by every container reader (native, MCAP). Consumers that only
/// need to digest, inspect or replay a recording should take `&impl Recording`
/// (or `&dyn Recording`) rather than a concrete container type.
pub trait Recording {
    /// The recording manifest.
    fn manifest(&self) -> &RecordingManifest;

    /// All records, in recorded order.
    fn records(&self) -> &[RawRecord];

    /// The number of records on a given channel.
    fn count_for(&self, channel_id: u16) -> usize {
        self.records()
            .iter()
            .filter(|r| r.channel_id == channel_id)
            .count()
    }

    /// Iterate the records on a given channel, in recorded order.
    ///
    /// Returns a boxed iterator so the trait stays object-safe (usable as
    /// `&dyn Recording`).
    fn records_for<'a>(&'a self, channel_id: u16) -> Box<dyn Iterator<Item = &'a RawRecord> + 'a> {
        Box::new(
            self.records()
                .iter()
                .filter(move |r| r.channel_id == channel_id),
        )
    }
}
