//! Pure v1 record codec for golden vectors, inspection and crafted test records.
//!
//! Writing bytes to a slot still requires trusted store access. Layout (64 bytes,
//! little-endian):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `"NRXG"` |
//! | 4 | 1 | format version `1` |
//! | 5 | 1 | slot tag: 0 = A, 1 = B |
//! | 6 | 2 | flags, must be 0 |
//! | 8 | 16 | receiver identity |
//! | 24 | 8 | binding key |
//! | 32 | 8 | namespace epoch, nonzero |
//! | 40 | 8 | high-water counter |
//! | 48 | 4 | commit count, nonzero, diagnostic only |
//! | 52 | 8 | reserved, must be 0 |
//! | 60 | 4 | CRC-32/IEEE over bytes 0..60 |
//!
//! Decode order: uniform 0xFF/0x00 is `Blank`; magic or CRC mismatch is `Corrupt`
//! (torn or rotted slots land here); a CRC-valid unknown version, nonzero flags or
//! nonzero reserved bytes is `Unsupported` (fail closed, never reinterpreted); a
//! CRC-valid record with an invalid tag, zero epoch, zero commit count or an
//! unprogrammed receiver identity is `Corrupt`.

use core::num::NonZeroU64;

use super::{
    BindingKey, NamespaceEpoch, RECORD_BYTES, RECORD_FORMAT, RECORD_MAGIC, ReceiverId,
    ReservationKey, Slot,
};

const CRC_POLY: u32 = 0xEDB8_8320;
const CRC_OFFSET: usize = 60;

/// Logical content of one slot record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordFields {
    /// Slot this copy was written for; a mismatch with the slot read is aliasing.
    pub slot: Slot,
    /// Receiver and binding the record belongs to.
    pub key: ReservationKey,
    /// Namespace epoch (high 64 bits of every generation).
    pub epoch: NamespaceEpoch,
    /// Every counter at or below this value may already have been issued.
    pub high_water: u64,
    /// Saturating commit count for wear diagnostics; must be nonzero.
    pub commits: u32,
}

/// Classification of one slot's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotView {
    /// Uniform erased pattern (all 0xFF or all 0x00).
    Blank,
    /// Failed magic/CRC, or a CRC-valid record with invalid field values.
    Corrupt,
    /// CRC-valid record of an unknown format; always fails closed.
    Unsupported,
    /// A well-formed v1 record.
    Valid(RecordFields),
}

/// CRC-32/IEEE: reflected, polynomial 0xEDB88320, init and xorout 0xFFFFFFFF.
/// Check value: `crc32(b"123456789") == 0xCBF43926`. Integrity only, not a MAC.
pub const fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (CRC_POLY & mask);
            bit += 1;
        }
        i += 1;
    }
    !crc
}

/// Rewrite bytes 60..64 with the CRC of bytes 0..60. Pure.
pub fn seal(bytes: &mut [u8; RECORD_BYTES]) {
    let crc = crc32(&bytes[..CRC_OFFSET]);
    bytes[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
}

/// Encode a v1 record, including its CRC.
pub fn encode(fields: &RecordFields) -> [u8; RECORD_BYTES] {
    let mut bytes = [0u8; RECORD_BYTES];
    bytes[0..4].copy_from_slice(&RECORD_MAGIC);
    bytes[4] = RECORD_FORMAT;
    bytes[5] = fields.slot.tag();
    // bytes 6..8 flags = 0
    bytes[8..24].copy_from_slice(fields.key.receiver().as_bytes());
    bytes[24..32].copy_from_slice(&fields.key.binding().get().to_le_bytes());
    bytes[32..40].copy_from_slice(&fields.epoch.get().to_le_bytes());
    bytes[40..48].copy_from_slice(&fields.high_water.to_le_bytes());
    bytes[48..52].copy_from_slice(&fields.commits.to_le_bytes());
    // bytes 52..60 reserved = 0
    seal(&mut bytes);
    bytes
}

/// Classify a slot's bytes in the normative decode order.
pub fn decode(bytes: &[u8; RECORD_BYTES]) -> SlotView {
    if bytes.iter().all(|&b| b == 0xFF) || bytes.iter().all(|&b| b == 0x00) {
        return SlotView::Blank;
    }
    let stored_crc = u32::from_le_bytes(le4(bytes, CRC_OFFSET));
    if bytes[0..4] != RECORD_MAGIC || stored_crc != crc32(&bytes[..CRC_OFFSET]) {
        return SlotView::Corrupt;
    }
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    if bytes[4] != RECORD_FORMAT || flags != 0 || bytes[52..60].iter().any(|&b| b != 0) {
        return SlotView::Unsupported;
    }
    let slot = match bytes[5] {
        0 => Slot::A,
        1 => Slot::B,
        _ => return SlotView::Corrupt,
    };
    let mut receiver = [0u8; 16];
    receiver.copy_from_slice(&bytes[8..24]);
    let Some(receiver) = ReceiverId::new(receiver) else {
        return SlotView::Corrupt;
    };
    let binding = BindingKey::from_u64(u64::from_le_bytes(le8(bytes, 24)));
    let Some(epoch) = NonZeroU64::new(u64::from_le_bytes(le8(bytes, 32))) else {
        return SlotView::Corrupt;
    };
    let high_water = u64::from_le_bytes(le8(bytes, 40));
    let commits = u32::from_le_bytes(le4(bytes, 48));
    if commits == 0 {
        return SlotView::Corrupt;
    }
    SlotView::Valid(RecordFields {
        slot,
        key: ReservationKey::new(receiver, binding),
        epoch: NamespaceEpoch::from_nonzero(epoch),
        high_water,
        commits,
    })
}

fn le4(bytes: &[u8; RECORD_BYTES], at: usize) -> [u8; 4] {
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

fn le8(bytes: &[u8; RECORD_BYTES], at: usize) -> [u8; 8] {
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[at..at + 8]);
    out
}
