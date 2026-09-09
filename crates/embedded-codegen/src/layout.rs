//! Shared canonical scalar layout; identity bytes remain unchanged.
pub use neuradix_contracts::layout::{CODEC_ID, WireField, WireLayout};

pub(crate) fn canonical_fields(
    contract: &neuradix_contracts::Contract,
) -> Vec<&neuradix_contracts::Field> {
    let mut fields: Vec<_> = contract.spec.payload.fields.iter().collect();
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    fields
}
