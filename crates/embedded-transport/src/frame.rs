//! Frame encode/decode for a byte-oriented link (serial-first).
//!
//! A frame is:
//!
//! ```text
//! 0xAA 0x55 | seq:u16 LE | len:u16 LE | payload[len] | crc32:u32 LE
//!            \______________ CRC covers seq, len and payload _______/
//! ```
//!
//! The two-byte sync prefix lets the [`FrameDecoder`] **resynchronise** after
//! line noise, and the trailing CRC-32 rejects any corrupted frame. Both the
//! encoder (into a caller-provided buffer) and the decoder (a byte-at-a-time
//! state machine over a fixed internal buffer) are allocation-free, so they run
//! unchanged in a UART interrupt handler.

use crate::crc::Crc32;

/// The two-byte frame sync prefix.
pub const SYNC: [u8; 2] = [0xAA, 0x55];

/// Per-frame framing overhead in bytes: sync (2) + seq (2) + len (2) + crc (4).
pub const OVERHEAD: usize = 2 + 2 + 2 + 4;

/// Errors from [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// The payload is larger than a `u16` length field allows.
    PayloadTooLarge,
    /// The output buffer is too small for the framed message.
    BufferTooSmall,
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            TransportError::PayloadTooLarge => "payload too large for a frame",
            TransportError::BufferTooSmall => "output buffer too small for the frame",
        })
    }
}

impl core::error::Error for TransportError {}

/// Encode `payload` under sequence number `seq` into `out`, returning the number
/// of bytes written.
pub fn encode(seq: u16, payload: &[u8], out: &mut [u8]) -> Result<usize, TransportError> {
    let len = u16::try_from(payload.len()).map_err(|_| TransportError::PayloadTooLarge)?;
    let total = OVERHEAD + payload.len();
    if out.len() < total {
        return Err(TransportError::BufferTooSmall);
    }

    out[0] = SYNC[0];
    out[1] = SYNC[1];
    out[2..4].copy_from_slice(&seq.to_le_bytes());
    out[4..6].copy_from_slice(&len.to_le_bytes());
    out[6..6 + payload.len()].copy_from_slice(payload);

    // CRC covers seq, len and payload (everything between the sync and the CRC).
    let crc = {
        let mut c = Crc32::new();
        c.update(&out[2..6 + payload.len()]);
        c.finish()
    };
    out[6 + payload.len()..total].copy_from_slice(&crc.to_le_bytes());
    Ok(total)
}

/// The header of a decoded frame (the payload is read via
/// [`FrameDecoder::payload`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The sequence number.
    pub seq: u16,
    /// The payload length.
    pub len: u16,
}

/// The result of feeding a byte to the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEvent {
    /// A complete, CRC-verified frame. Read its payload with
    /// [`FrameDecoder::payload`] before the next `push`.
    Frame(Frame),
    /// A frame was dropped because its CRC failed or it exceeded the decoder's
    /// buffer. The decoder has resynchronised.
    Corrupt,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Sync0,
    Sync1,
    Header(usize),
    Payload,
    Crc(usize),
}

/// A streaming, resync-capable frame decoder over a fixed `N`-byte payload
/// buffer.
#[derive(Debug)]
pub struct FrameDecoder<const N: usize> {
    state: State,
    header: [u8; 4],
    seq: u16,
    len: usize,
    buf: [u8; N],
    idx: usize,
    crc: [u8; 4],
}

impl<const N: usize> Default for FrameDecoder<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FrameDecoder<N> {
    /// A decoder that can hold payloads up to `N` bytes.
    pub const fn new() -> Self {
        Self {
            state: State::Sync0,
            header: [0; 4],
            seq: 0,
            len: 0,
            buf: [0; N],
            idx: 0,
            crc: [0; 4],
        }
    }

    /// The payload of the most recently completed frame (valid immediately after
    /// a [`FrameEvent::Frame`], until the next `push`).
    ///
    /// Clamped to the buffer capacity so it can never panic, even if called
    /// after a dropped (oversized or corrupt) frame.
    pub fn payload(&self) -> &[u8] {
        &self.buf[..self.len.min(N)]
    }

    /// Feed one received byte. Returns an event when a frame completes (verified
    /// or corrupt), otherwise `None`.
    pub fn push(&mut self, byte: u8) -> Option<FrameEvent> {
        match self.state {
            State::Sync0 => {
                if byte == SYNC[0] {
                    self.state = State::Sync1;
                }
                None
            }
            State::Sync1 => {
                if byte == SYNC[1] {
                    self.state = State::Header(0);
                } else if byte == SYNC[0] {
                    // Could be the real start of a sync pair; stay armed.
                    self.state = State::Sync1;
                } else {
                    self.state = State::Sync0;
                }
                None
            }
            State::Header(count) => {
                self.header[count] = byte;
                let count = count + 1;
                if count < 4 {
                    self.state = State::Header(count);
                    return None;
                }
                self.seq = u16::from_le_bytes([self.header[0], self.header[1]]);
                self.len = u16::from_le_bytes([self.header[2], self.header[3]]) as usize;
                if self.len > N {
                    // Cannot buffer this payload: drop and resynchronise. Reset
                    // `len` so `payload()` stays within the buffer.
                    self.len = 0;
                    self.state = State::Sync0;
                    return Some(FrameEvent::Corrupt);
                }
                self.idx = 0;
                self.state = if self.len == 0 {
                    State::Crc(0)
                } else {
                    State::Payload
                };
                None
            }
            State::Payload => {
                self.buf[self.idx] = byte;
                self.idx += 1;
                if self.idx == self.len {
                    self.state = State::Crc(0);
                }
                None
            }
            State::Crc(count) => {
                self.crc[count] = byte;
                let count = count + 1;
                if count < 4 {
                    self.state = State::Crc(count);
                    return None;
                }
                self.state = State::Sync0;
                let mut c = Crc32::new();
                c.update(&self.header);
                c.update(&self.buf[..self.len]);
                if c.finish() == u32::from_le_bytes(self.crc) {
                    Some(FrameEvent::Frame(Frame {
                        seq: self.seq,
                        len: self.len as u16,
                    }))
                } else {
                    Some(FrameEvent::Corrupt)
                }
            }
        }
    }
}
