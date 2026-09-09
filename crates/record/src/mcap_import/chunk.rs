//! Decode exactly one validated LZ4 frame into a pre-sized bounded output.
use std::io::Read;
use super::preflight::Cursor;
use super::malformed;
use crate::{RecordError, Result};

pub(super) fn decode(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    // Validate frame boundaries before library decoding. No dictionaries,
    // skippable/concatenated frames, trailing bytes or oversized blocks.
    let mut c = Cursor::new(data);
    if c.take(4)? != [0x04, 0x22, 0x4d, 0x18] {
        return Err(RecordError::UnsupportedMcap("LZ4 requires one standard frame"));
    }
    let flags = c.take(1)?[0];
    let bd = c.take(1)?[0];
    if flags >> 6 != 1 || flags & 0x03 != 0 || bd & 0x8f != 0 {
        return Err(RecordError::UnsupportedMcap("LZ4 version, reserved bits or dictionary"));
    }
    let block_max = match (bd >> 4) & 7 {
        4 => 64 << 10, 5 => 256 << 10, 6 => 1 << 20, 7 => 4 << 20,
        _ => return Err(malformed("invalid LZ4 block maximum")),
    };
    if flags & 0x08 != 0 && c.u64()? != expected as u64 {
        return Err(malformed("LZ4 content size disagrees with MCAP chunk"));
    }
    c.take(1)?; // Header checksum is checked by the maintained decoder.
    loop {
        let word = c.u32()?;
        if word == 0 { break; }
        let len = word & 0x7fff_ffff;
        if len == 0 || len > block_max { return Err(malformed("invalid LZ4 block length")); }
        c.take(len)?;
        if flags & 0x10 != 0 { c.take(4)?; }
    }
    if flags & 0x04 != 0 { c.take(4)?; }
    if c.remaining() != 0 { return Err(malformed("trailing LZ4 data")); }
    let mut decoder = lz4::Decoder::new(data)?;
    let mut out = vec![0; expected];
    decoder.read_exact(&mut out)?;
    if decoder.read(&mut [0u8; 1])? != 0 {
        return Err(malformed("decoded chunk exceeds advertised size"));
    }
    let (_, finished) = decoder.finish();
    finished?;
    Ok(out)
}
