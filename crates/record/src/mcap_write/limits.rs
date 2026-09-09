//! Validated limits and allocation-free serialization sizing.
use crate::{McapChannel, McapHeader, McapMetadata, McapSchema, RecordError, Result};
use std::collections::BTreeMap;

/// Private validated uncompressed writer policy; limits are inclusive.
/// State bytes are conservative accounting, not a universal process RSS limit.
///
/// ```compile_fail
/// let mut policy = neuradix_record::McapWriteLimits::default();
/// policy.output_bytes = u64::MAX;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McapWriteLimits {
    pub(super) output_bytes: u64,
    pub(super) record_bytes: usize,
    pub(super) message_bytes: usize,
    pub(super) string_bytes: usize,
    pub(super) records: u64,
    pub(super) messages: u64,
    pub(super) schemas: usize,
    pub(super) channels: usize,
    pub(super) metadata: usize,
    pub(super) entries: usize,
    pub(super) state_bytes: usize,
}
macro_rules! budget {
    ($get:ident, $set:ident, $ty:ty, $min:expr, $max:expr, $doc:literal) => {
        #[doc = $doc]
        pub fn $get(self) -> $ty {
            self.$get
        }
        #[doc = concat!("Set ", $doc, " Equality is accepted.")]
        pub fn $set(mut self, value: $ty) -> Result<Self> {
            if !($min..=$max).contains(&value) {
                return Err(RecordError::InvalidWriteLimit(stringify!($get)));
            }
            self.$get = value;
            Ok(self)
        }
    };
}
impl McapWriteLimits {
    budget!(
        output_bytes,
        with_output_bytes,
        u64,
        32,
        1 << 40,
        "file bytes including finalization (32 B..=1 TiB)."
    );
    budget!(
        record_bytes,
        with_record_bytes,
        usize,
        32,
        64 << 20,
        "record body bytes (32 B..=64 MiB)."
    );
    budget!(
        message_bytes,
        with_message_bytes,
        usize,
        1,
        64 << 20,
        "payload bytes (1 B..=64 MiB)."
    );
    budget!(
        string_bytes,
        with_string_bytes,
        usize,
        1,
        1 << 20,
        "UTF-8 bytes per string (1 B..=1 MiB)."
    );
    budget!(
        records,
        with_records,
        u64,
        1,
        20_000_000,
        "records including finalization (1..=20 million)."
    );
    budget!(
        messages,
        with_messages,
        u64,
        1,
        10_000_000,
        "messages (1..=10 million)."
    );
    budget!(
        schemas,
        with_schemas,
        usize,
        1,
        65_535,
        "unique schema IDs (1..=65,535)."
    );
    budget!(
        channels,
        with_channels,
        usize,
        1,
        65_536,
        "unique channel IDs (1..=65,536)."
    );
    budget!(
        metadata,
        with_metadata,
        usize,
        1,
        65_536,
        "unique metadata names (1..=65,536)."
    );
    budget!(
        entries,
        with_entries,
        usize,
        1,
        65_536,
        "map entries (1..=65,536)."
    );
    budget!(
        state_bytes,
        with_state_bytes,
        usize,
        1024,
        256 << 20,
        "accounted definition/finalization storage (1 KiB..=256 MiB)."
    );
}
impl Default for McapWriteLimits {
    fn default() -> Self {
        Self {
            output_bytes: 64 << 20,
            record_bytes: 4 << 20,
            message_bytes: 1 << 20,
            string_bytes: 64 << 10,
            records: 2_000_000,
            messages: 1_000_000,
            schemas: 4096,
            channels: 4096,
            metadata: 4096,
            entries: 4096,
            state_bytes: 4 << 20,
        }
    }
}
pub(super) fn check(value: u64, limit: u64, kind: &'static str) -> Result<()> {
    if value > limit {
        Err(RecordError::WriteLimit { kind, limit })
    } else {
        Ok(())
    }
}
pub(super) fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or(RecordError::InvalidMcapWrite("size/counter overflow"))
}
pub(super) fn charge(body: u64, entries: usize) -> Result<u64> {
    add(
        add(
            body.checked_mul(4)
                .ok_or(RecordError::InvalidMcapWrite("state overflow"))?,
            2048,
        )?,
        (entries as u64)
            .checked_mul(256)
            .ok_or(RecordError::InvalidMcapWrite("map storage overflow"))?,
    )
}
pub(super) fn string(value: &str, limits: McapWriteLimits) -> Result<u64> {
    check(
        value.len() as u64,
        limits.string_bytes as u64,
        "string bytes",
    )?;
    add(4, value.len() as u64)
}
fn map(value: &BTreeMap<String, String>, limits: McapWriteLimits) -> Result<u64> {
    check(value.len() as u64, limits.entries as u64, "map entries")?;
    let mut bytes = 0;
    for (k, v) in value {
        bytes = add(bytes, add(string(k, limits)?, string(v, limits)?)?)?;
    }
    check(bytes, u32::MAX as u64, "encoded map bytes")?;
    add(4, bytes)
}
pub(super) fn header(value: &McapHeader, limits: McapWriteLimits) -> Result<u64> {
    add(
        string(&value.profile, limits)?,
        string(&value.library, limits)?,
    )
}
pub(super) fn schema(value: &McapSchema, limits: McapWriteLimits) -> Result<u64> {
    if value.id == 0 || (value.encoding.is_empty() && !value.data.is_empty()) {
        return Err(RecordError::InvalidMcapWrite("invalid schema ID/encoding"));
    }
    check(
        value.data.len() as u64,
        u32::MAX as u64,
        "encoded schema bytes",
    )?;
    add(
        add(
            6,
            add(
                string(&value.name, limits)?,
                string(&value.encoding, limits)?,
            )?,
        )?,
        value.data.len() as u64,
    )
}
pub(super) fn channel(value: &McapChannel, limits: McapWriteLimits) -> Result<u64> {
    add(
        4,
        add(
            add(
                string(&value.topic, limits)?,
                string(&value.message_encoding, limits)?,
            )?,
            map(&value.metadata, limits)?,
        )?,
    )
}
pub(super) fn metadata(value: &McapMetadata, limits: McapWriteLimits) -> Result<u64> {
    add(string(&value.name, limits)?, map(&value.entries, limits)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn arithmetic_overflow_is_rejected() {
        assert!(super::add(u64::MAX, 1).is_err());
        assert!(super::charge(u64::MAX, 0).is_err());
    }
}
