//! Allocation-free length/shape validation before maintained record parsing.
//! binrw string counts are otherwise allowed to request attacker-chosen storage.
use super::limits::check;
use super::{McapImportLimits, malformed};
use crate::{RecordError, Result};

pub(super) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(size)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| malformed("field extends past record"))?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
    pub fn u32(&mut self) -> Result<usize> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self, limits: McapImportLimits) -> Result<()> {
        let len = self.u32()?;
        check(len as u64, limits.string_bytes as u64, "string bytes")?;
        std::str::from_utf8(self.take(len)?).map_err(|_| malformed("invalid UTF-8"))?;
        Ok(())
    }
    fn map(&mut self, limits: McapImportLimits, width: Option<usize>) -> Result<usize> {
        let len = self.u32()?;
        let mut inner = Cursor::new(self.take(len)?);
        let mut entries = 0;
        while inner.remaining() > 0 {
            entries += 1;
            check(entries as u64, limits.entries as u64, "map/array entries")?;
            if let Some(width) = width {
                inner.take(width)?;
            } else {
                inner.string(limits)?;
                inner.string(limits)?;
            }
        }
        Ok(entries)
    }
}
/// Return a conservative charge for strings, map nodes and definition objects.
pub(super) fn validate(op: u8, data: &[u8], limits: McapImportLimits) -> Result<u64> {
    check(
        data.len() as u64,
        limits.record_bytes as u64,
        "record bytes",
    )?;
    let mut c = Cursor::new(data);
    let mut entries = 0;
    match op {
        0x01 => {
            c.string(limits)?;
            c.string(limits)?;
        }
        0x02 => {
            c.take(20)?;
        }
        0x03 => {
            c.take(2)?;
            c.string(limits)?;
            c.string(limits)?;
            let n = c.u32()?;
            c.take(n)?;
        }
        0x04 => {
            c.take(4)?;
            c.string(limits)?;
            c.string(limits)?;
            entries = c.map(limits, None)?;
        }
        0x05 => {
            c.take(22)?;
            check(
                c.remaining() as u64,
                limits.message_bytes as u64,
                "message bytes",
            )?;
            c.take(c.remaining())?;
        }
        0x06 => {
            c.take(28)?;
            c.string(limits)?;
            let n = c.u64()?;
            check(n, limits.record_bytes as u64, "compressed chunk bytes")?;
            c.take(n as usize)?;
        }
        0x07 => {
            c.take(2)?;
            entries = c.map(limits, Some(16))?;
        }
        0x08 => {
            c.take(32)?;
            entries = c.map(limits, Some(10))?;
            c.take(8)?;
            c.string(limits)?;
            c.take(16)?;
        }
        0x0b => {
            c.take(42)?;
            entries = c.map(limits, Some(10))?;
        }
        0x0c => {
            c.string(limits)?;
            entries = c.map(limits, None)?;
        }
        0x0d => {
            c.take(16)?;
            c.string(limits)?;
        }
        0x0e => {
            c.take(17)?;
        }
        0x0f => {
            c.take(4)?;
        }
        _ => return Err(RecordError::UnsupportedMcapRecord(op)),
    }
    if c.remaining() != 0 {
        return Err(RecordError::UnsupportedMcap(
            "record extension fields are not supported",
        ));
    }
    Ok(data.len() as u64 + 512 + entries as u64 * 128)
}
