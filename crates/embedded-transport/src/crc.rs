//! CRC-32 (IEEE 802.3) integrity check.
//!
//! Bitwise and table-free so it is tiny and `no_std` with no lookup table in
//! flash. It is the standard reflected CRC-32 (`poly = 0xEDB88320`,
//! `init = 0xFFFFFFFF`, `xorout = 0xFFFFFFFF`), so its check value over the ASCII
//! string `"123456789"` is `0xCBF43926`, matching every other conforming
//! implementation on the wire.

const POLY: u32 = 0xEDB8_8320;

/// An incremental CRC-32 accumulator (so a frame's header and payload can be fed
/// separately without copying them into one buffer).
#[derive(Debug, Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// A fresh accumulator.
    pub const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    /// Fold `data` into the running CRC.
    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.0;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                // Branch-free reflected division step.
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (POLY & mask);
            }
        }
        self.0 = crc;
    }

    /// The final CRC-32 value.
    pub const fn finish(self) -> u32 {
        !self.0
    }
}

/// The CRC-32 of a single contiguous slice.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.finish()
}
