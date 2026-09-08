//! Sequence-number tracking: dropped-frame, duplicate and reorder detection.
//!
//! Framing + CRC guarantee a delivered frame is intact; the sequence number
//! tells the receiver whether frames were *lost* or *reordered* on the way.
//! Sequence numbers wrap at `u16::MAX`, and the tracker compares them with
//! wrapping arithmetic so a wrap is not mistaken for a huge gap.

/// The classification of a received sequence number relative to the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqStatus {
    /// The first frame observed.
    First,
    /// Exactly the next expected sequence number.
    InOrder,
    /// The same sequence number as the previous frame.
    Duplicate,
    /// Ahead of the expected number: the payload of `n` frames was missed.
    Gap(u16),
    /// Behind the last accepted number (an out-of-order / late frame).
    Reordered,
}

/// Tracks the most recent in-order sequence number.
#[derive(Debug, Clone, Copy, Default)]
pub struct SequenceTracker {
    last: Option<u16>,
}

impl SequenceTracker {
    /// A tracker that has seen no frames yet.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// The most recent in-order sequence number, if any.
    pub const fn last(&self) -> Option<u16> {
        self.last
    }

    /// Classify `seq` and, when it advances the stream, record it.
    pub fn observe(&mut self, seq: u16) -> SeqStatus {
        match self.last {
            None => {
                self.last = Some(seq);
                SeqStatus::First
            }
            Some(prev) => {
                if seq == prev {
                    return SeqStatus::Duplicate;
                }
                // Distance ahead of `prev`, modulo the u16 wrap. A value in the
                // lower half of the range is "ahead"; the upper half is "behind".
                let ahead = seq.wrapping_sub(prev);
                if ahead == 1 {
                    self.last = Some(seq);
                    SeqStatus::InOrder
                } else if ahead < 0x8000 {
                    self.last = Some(seq);
                    SeqStatus::Gap(ahead - 1) // number of frames missed in between
                } else {
                    SeqStatus::Reordered
                }
            }
        }
    }
}
