//! Private, validated import budgets. Equality is accepted; one more rejects.
use crate::{RecordError, Result};

/// Bounded MCAP import policy. Builders reject zero and excessive values.
/// Sizes count bytes, record limits count record bodies (without the 9-byte
/// envelope), and retained/state budgets include conservative object charges.
///
/// ```compile_fail
/// let mut limits = neuradix_record::McapImportLimits::default();
/// limits.input_bytes = u64::MAX;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McapImportLimits {
    pub(super) input_bytes: u64,
    pub(super) record_bytes: usize,
    pub(super) chunk_bytes: usize,
    pub(super) decoded_bytes: u64,
    pub(super) message_bytes: usize,
    pub(super) string_bytes: usize,
    pub(super) records: u64,
    pub(super) messages: u64,
    pub(super) schemas: usize,
    pub(super) channels: usize,
    pub(super) entries: usize,
    pub(super) state_bytes: usize,
    pub(super) retained_bytes: usize,
}
macro_rules! budget {
    ($getter:ident, $setter:ident, $ty:ty, $min:expr, $max:expr, $doc:literal) => {
        #[doc = $doc]
        pub fn $getter(self) -> $ty {
            self.$getter
        }
        #[doc = concat!("Set ", $doc, " Values outside the documented range reject.")]
        pub fn $setter(mut self, value: $ty) -> Result<Self> {
            if !($min..=$max).contains(&value) {
                return Err(RecordError::InvalidImportLimit(stringify!($getter)));
            }
            self.$getter = value;
            Ok(self)
        }
    };
}
impl McapImportLimits {
    budget!(
        input_bytes,
        with_input_bytes,
        u64,
        32,
        1 << 40,
        "input bytes (32..=1 TiB)."
    );
    budget!(
        record_bytes,
        with_record_bytes,
        usize,
        32,
        64 << 20,
        "record body bytes (32..=64 MiB)."
    );
    budget!(
        chunk_bytes,
        with_chunk_bytes,
        usize,
        1,
        64 << 20,
        "decoded bytes per chunk (1..=64 MiB)."
    );
    budget!(
        decoded_bytes,
        with_decoded_bytes,
        u64,
        1,
        1 << 40,
        "cumulative decoded chunk bytes (1..=1 TiB)."
    );
    budget!(
        message_bytes,
        with_message_bytes,
        usize,
        1,
        64 << 20,
        "message payload bytes (1..=64 MiB)."
    );
    budget!(
        string_bytes,
        with_string_bytes,
        usize,
        1,
        1 << 20,
        "UTF-8 string bytes (1..=1 MiB)."
    );
    budget!(
        records,
        with_records,
        u64,
        1,
        20_000_000,
        "record count, including chunks and inner records (1..=20 million)."
    );
    budget!(
        messages,
        with_messages,
        u64,
        1,
        10_000_000,
        "message count (1..=10 million)."
    );
    budget!(
        schemas,
        with_schemas,
        usize,
        1,
        65_535,
        "unique schema count (1..=65,535)."
    );
    budget!(
        channels,
        with_channels,
        usize,
        1,
        65_536,
        "unique channel count (1..=65,536)."
    );
    budget!(
        entries,
        with_entries,
        usize,
        1,
        65_536,
        "entries per map/array (1..=65,536)."
    );
    budget!(
        state_bytes,
        with_state_bytes,
        usize,
        1024,
        256 << 20,
        "accounted persistent definition bytes (1 KiB..=256 MiB)."
    );
    budget!(
        retained_bytes,
        with_retained_bytes,
        usize,
        1024,
        1 << 30,
        "accounted materialized bytes (1 KiB..=1 GiB)."
    );
}
impl Default for McapImportLimits {
    fn default() -> Self {
        Self {
            input_bytes: 64 << 20,
            record_bytes: 4 << 20,
            chunk_bytes: 8 << 20,
            decoded_bytes: 256 << 20,
            message_bytes: 1 << 20,
            string_bytes: 64 << 10,
            records: 2_000_000,
            messages: 1_000_000,
            schemas: 4096,
            channels: 4096,
            entries: 4096,
            state_bytes: 4 << 20,
            retained_bytes: 64 << 20,
        }
    }
}
pub(super) fn check(value: u64, limit: u64, kind: &'static str) -> Result<()> {
    if value > limit {
        Err(RecordError::ImportLimit { kind, limit })
    } else {
        Ok(())
    }
}
pub(super) fn add(value: &mut u64, amount: u64, limit: u64, kind: &'static str) -> Result<()> {
    let next = value
        .checked_add(amount)
        .ok_or(RecordError::ImportLimit { kind, limit })?;
    check(next, limit, kind)?;
    *value = next;
    Ok(())
}
