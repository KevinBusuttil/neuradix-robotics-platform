//! The byte-stream side of the gateway: an incremental, fixed-buffer frame
//! decoder in front of a verified [`RoutingTable`].
//!
//! # Input handling
//!
//! - **Fragmented input.** Bytes are fed one at a time into the transport's
//!   `FrameDecoder<F>`. A frame split across any number of chunks decodes
//!   exactly as if it arrived whole; the decoder keeps its state between
//!   [`Gateway::push`] calls.
//! - **Several frames per chunk.** Each completed frame is routed before the
//!   next byte is consumed, so one chunk can deliver many frames.
//! - **Incomplete frames.** A partial frame at the end of a chunk is held in
//!   the fixed buffer until more bytes arrive. Nothing is delivered for it. If
//!   the sender never completes it, the next bytes are read as its remainder,
//!   its CRC fails, and the frames that supplied those bytes are lost as well.
//!   [`Gateway::discard_partial`] drops a partial frame explicitly, for example
//!   after a link idle gap. Detecting such gaps is session work (WP-B06).
//! - **Oversized input.** A frame whose declared length exceeds `F` is dropped
//!   as soon as its header is read, without buffering its payload, and the
//!   decoder resynchronises on the next sync pattern. The transport reports
//!   this the same way as a CRC failure ([`Rejection::CorruptFrame`]).
//! - **Line noise** before or between frames is skipped while searching for
//!   the sync pattern.
//!
//! # Resource bounds
//!
//! After construction the gateway holds a fixed `F`-byte frame buffer, the
//! fixed-capacity channel table (`N` entries) and one route per channel. It
//! keeps no queue and no per-frame output: each frame is either handed to its
//! typed handler synchronously or counted as rejected. [`Stats`] counters
//! saturate. Processing a chunk does not allocate.
//!
//! # Sequence numbers
//!
//! Frame sequence numbers are classified with the transport's
//! `SequenceTracker` and counted in [`Stats`], but they do **not** gate
//! delivery: duplicated, reordered and gapped frames that pass every other
//! check are delivered. Duplicate suppression, replay protection, reconnect
//! and reboot handling are deferred to WP-B06 device sessions.

use neuradix_embedded_transport::{
    EnvelopeError, FrameDecoder, FrameEvent, SeqStatus, SequenceTracker,
};

use crate::identity::{Attributed, IdentityClaims, IdentityReport, IdentitySource};
use crate::routing::{Delivered, InitError, Rejection, RoutingTable};

/// What happened to one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// Decoded and handed to a typed handler.
    Delivered {
        /// Frame sequence number.
        seq: u16,
        /// Sequence classification (observational only).
        sequence: SeqStatus,
        /// Resolved channel.
        channel: Delivered,
    },
    /// No handler was called.
    Rejected {
        /// Frame sequence number, when the frame passed its CRC.
        seq: Option<u16>,
        /// Why.
        reason: Rejection,
    },
}

/// Saturating counters since construction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes consumed.
    pub bytes: u64,
    /// Frames that passed their CRC.
    pub frames: u64,
    /// Frames delivered to a typed handler.
    pub delivered: u64,
    /// CRC failures and frames longer than the frame buffer.
    pub corrupt: u64,
    /// Payloads shorter than the envelope header.
    pub truncated: u64,
    /// Payloads without the envelope magic byte.
    pub bad_magic: u64,
    /// Unknown envelope versions.
    pub unsupported_version: u64,
    /// Envelopes tagged with another manifest.
    pub manifest_mismatch: u64,
    /// Unknown or reserved compact IDs.
    pub unknown_channel: u64,
    /// Bodies whose length differs from the bound length.
    pub length_mismatch: u64,
    /// Bodies the generated decoder refused.
    pub decode_rejected: u64,
    /// Frames repeating the previous sequence number.
    pub duplicates: u64,
    /// Frames missed according to sequence gaps.
    pub missed: u64,
    /// Frames older than the last in-order frame.
    pub reordered: u64,
}

impl Stats {
    /// Frames that reached no handler.
    pub fn rejected(&self) -> u64 {
        [
            self.corrupt,
            self.truncated,
            self.bad_magic,
            self.unsupported_version,
            self.manifest_mismatch,
            self.unknown_channel,
            self.length_mismatch,
            self.decode_rejected,
        ]
        .iter()
        .fold(0u64, |a, b| a.saturating_add(*b))
    }

    fn reject(&mut self, reason: Rejection) {
        let counter = match reason {
            Rejection::CorruptFrame => &mut self.corrupt,
            Rejection::Envelope(e) => match e {
                EnvelopeError::Truncated => &mut self.truncated,
                EnvelopeError::BadMagic => &mut self.bad_magic,
                EnvelopeError::UnsupportedVersion(_) => &mut self.unsupported_version,
                EnvelopeError::ManifestMismatch => &mut self.manifest_mismatch,
                EnvelopeError::UnknownChannel(_) => &mut self.unknown_channel,
                EnvelopeError::LengthMismatch { .. } => &mut self.length_mismatch,
                // Only produced when sealing.
                EnvelopeError::BufferTooSmall => &mut self.truncated,
            },
            Rejection::Decode { .. } => &mut self.decode_rejected,
        };
        *counter = counter.saturating_add(1);
    }

    fn sequence(&mut self, status: SeqStatus) {
        match status {
            SeqStatus::Duplicate => self.duplicates = self.duplicates.saturating_add(1),
            SeqStatus::Gap(n) => self.missed = self.missed.saturating_add(u64::from(n)),
            SeqStatus::Reordered => self.reordered = self.reordered.saturating_add(1),
            SeqStatus::First | SeqStatus::InOrder => {}
        }
    }
}

/// Frames delivered and rejected by one [`Gateway::push`] call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkSummary {
    /// Frames delivered to a handler.
    pub delivered: u32,
    /// Frames rejected.
    pub rejected: u32,
}

/// A reference gateway instance: fixed `F`-byte frame buffer, `N`-channel
/// verified routing state, handler type `H`.
#[derive(Debug)]
pub struct Gateway<H, const N: usize, const F: usize> {
    routes: RoutingTable<H, N>,
    frames: FrameDecoder<F>,
    sequence: SequenceTracker,
    stats: Stats,
}

impl<H, const N: usize, const F: usize> Gateway<H, N, F> {
    /// Wrap verified routing state. Refused if the frame buffer cannot hold
    /// the largest bound envelope.
    pub fn new(routes: RoutingTable<H, N>) -> Result<Self, InitError> {
        if routes.max_envelope_len() > F {
            return Err(InitError::FrameCapacity {
                needed: routes.max_envelope_len(),
                capacity: F,
            });
        }
        Ok(Self {
            routes,
            frames: FrameDecoder::new(),
            sequence: SequenceTracker::new(),
            stats: Stats::default(),
        })
    }

    /// Consume a chunk of received bytes, delivering each verified frame to
    /// its typed handler.
    pub fn push(&mut self, chunk: &[u8], handler: &mut H) -> ChunkSummary {
        self.push_observed(chunk, handler, |_| {})
    }

    /// As [`Self::push`], also reporting each frame's outcome to `observe`.
    pub fn push_observed(
        &mut self,
        chunk: &[u8],
        handler: &mut H,
        mut observe: impl FnMut(FrameOutcome),
    ) -> ChunkSummary {
        let mut summary = ChunkSummary::default();
        for &byte in chunk {
            self.stats.bytes = self.stats.bytes.saturating_add(1);
            let outcome = match self.frames.push(byte) {
                None => continue,
                Some(FrameEvent::Corrupt) => FrameOutcome::Rejected {
                    seq: None,
                    reason: Rejection::CorruptFrame,
                },
                Some(FrameEvent::Frame(frame)) => {
                    self.stats.frames = self.stats.frames.saturating_add(1);
                    let sequence = self.sequence.observe(frame.seq);
                    self.stats.sequence(sequence);
                    match self.routes.route(self.frames.payload(), handler) {
                        Ok(channel) => FrameOutcome::Delivered {
                            seq: frame.seq,
                            sequence,
                            channel,
                        },
                        Err(reason) => FrameOutcome::Rejected {
                            seq: Some(frame.seq),
                            reason,
                        },
                    }
                }
            };
            match outcome {
                FrameOutcome::Delivered { .. } => {
                    self.stats.delivered = self.stats.delivered.saturating_add(1);
                    summary.delivered = summary.delivered.saturating_add(1);
                }
                FrameOutcome::Rejected { reason, .. } => {
                    self.stats.reject(reason);
                    summary.rejected = summary.rejected.saturating_add(1);
                }
            }
            observe(outcome);
        }
        summary
    }

    /// Drop any partially received frame and search for the next sync
    /// pattern. Sequence history and counters are kept.
    pub fn discard_partial(&mut self) {
        self.frames = FrameDecoder::new();
    }

    /// Counters since construction.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The verified routing state.
    pub fn routes(&self) -> &RoutingTable<H, N> {
        &self.routes
    }

    /// The verified manifest digest with the supplied identities and their
    /// sources. Nothing here is attested by a connected board.
    pub fn identity(&self, claims: IdentityClaims) -> IdentityReport {
        IdentityReport {
            manifest_digest: self.routes.manifest_digest().to_owned(),
            gateway_build: Attributed::new(
                concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")).to_owned(),
                IdentitySource::GeneratedBuild,
            ),
            claims,
        }
    }

    /// Frame buffer capacity in bytes.
    pub const fn frame_capacity(&self) -> usize {
        F
    }
}
