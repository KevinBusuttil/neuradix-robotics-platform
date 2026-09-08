//! `no_std` framing for constrained MCU links (Embedded Profile WP4, serial-first).
//!
//! This crate turns a noisy byte stream (a UART, later CAN) into a sequence of
//! integrity-checked messages, and back. It is `#![no_std]`, allocation-free and
//! dependency-free (only `core`), so the encoder and the byte-at-a-time decoder
//! run unchanged inside an interrupt handler on the board and in host tests.
//!
//! - [`encode`] frames a payload (sync + seq + len + CRC-32) into a caller
//!   buffer.
//! - [`FrameDecoder`] resynchronises after line noise and yields only
//!   CRC-verified frames, reporting corrupt ones as [`FrameEvent::Corrupt`].
//! - [`SequenceTracker`] classifies each frame's sequence number as in-order,
//!   duplicate, gapped (frames lost) or reordered.
//!
//! Integrity (CRC) and ordering (sequence) live here; **freshness** is enforced
//! upstream by the `neuradix-embedded-core` watchdog — a corrupt or missing
//! frame simply means no fresh command arrived, which drives the node's local
//! safe state. A wired/wireless link is never trusted as a safety channel.
//!
//! # Example — round trip
//!
//! ```
//! use neuradix_embedded_transport::{encode, FrameDecoder, FrameEvent, OVERHEAD};
//!
//! let payload = [0x01, 0x02, 0x03];
//! let mut wire = [0u8; OVERHEAD + 3];
//! let n = encode(7, &payload, &mut wire).unwrap();
//!
//! let mut decoder = FrameDecoder::<64>::new();
//! let mut got = None;
//! for &byte in &wire[..n] {
//!     if let Some(event) = decoder.push(byte) {
//!         got = Some(event);
//!     }
//! }
//! assert_eq!(got, Some(FrameEvent::Frame(neuradix_embedded_transport::Frame { seq: 7, len: 3 })));
//! assert_eq!(decoder.payload(), &payload);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod crc;
pub mod frame;
pub mod sequence;

pub use crc::{Crc32, crc32};
pub use frame::{Frame, FrameDecoder, FrameEvent, OVERHEAD, SYNC, TransportError, encode};
pub use sequence::{SeqStatus, SequenceTracker};
