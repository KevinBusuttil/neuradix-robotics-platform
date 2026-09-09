//! Canonical, versioned embedded payload layout, separate from schema identity.

use crate::{Contract, Field, PrimitiveType, schema_identity};
use serde::Serialize;
use sha2::{Digest, Sha256};



/// Version two replaces the legacy, unversioned declaration-order codec.
/// The version covers scalar representations and decoder rejection rules.
pub const CODEC_ID: &str = "neuradix.scalar-le.v2";

/// One field in a wire descriptor, in canonical name order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireField {
    /// Authored field name.
    pub name: String,
    /// Contract primitive spelling.
    pub ty: String,
    /// Byte offset in the payload (no header or implicit padding).
    pub offset: usize,
    /// Fixed encoded width, in octets.
    pub size: usize,
}

/// The complete binding a gateway must associate with a payload channel.
///
/// Never infer this binding from a schema ID alone. The wire ID is the SHA-256
/// of the compact JSON descriptor returned by [`Self::descriptor_bytes`].
/// Compact channel IDs require a separately validated full-identity mapping;
/// this increment does not introduce truncation or a transport handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireLayout {
    /// Versioned scalar codec name.
    pub codec_id: String,
    /// Existing semantic contract identity (unchanged by this codec).
    pub schema_id: String,
    /// Full content-addressed wire identity.
    pub wire_id: String,
    /// Exact encoded payload length.
    pub wire_len: usize,
    /// Canonical ordered field descriptors.
    pub fields: Vec<WireField>,
}

impl WireLayout {
    /// Resolve the single layout used by both generators and golden vectors.
    pub fn for_contract(contract: &Contract) -> Result<Self, LayoutError> {
        let mut offset = 0usize;
        let mut fields = Vec::new();
        for field in canonical_fields(contract) {
            let size = field_size(field.ty, &field.name)?;
            fields.push(WireField {
                name: field.name.clone(),
                ty: field.ty.as_contract_str().to_owned(),
                offset,
                size,
            });
            offset = offset.checked_add(size).ok_or(LayoutError::Overflow)?;
        }
        let mut layout = Self {
            codec_id: CODEC_ID.to_owned(),
            schema_id: schema_identity(contract).to_string(),
            wire_id: String::new(),
            wire_len: offset,
            fields,
        };
        layout.wire_id = format!("sha256:{:x}", Sha256::digest(layout.descriptor_bytes()));
        Ok(layout)
    }

    /// Exact UTF-8 bytes hashed for `wire_id`. JSON keys follow this explicit
    /// struct order; no whitespace, escaping follows `serde_json`, no floats.
    /// The `wire_id` itself is excluded from its descriptor.
    pub fn descriptor_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Descriptor<'a> {
            codec_id: &'a str,
            schema_id: &'a str,
            wire_len: usize,
            fields: &'a [WireField],
        }
        serde_json::to_vec(&Descriptor {
            codec_id: &self.codec_id,
            schema_id: &self.schema_id,
            wire_len: self.wire_len,
            fields: &self.fields,
        })
        .expect("wire descriptor serialization cannot fail")
    }
}

/// Match schema canonicalization without mutating the authored contract.
pub(crate) fn canonical_fields(contract: &Contract) -> Vec<&Field> {
    let mut fields: Vec<_> = contract.spec.payload.fields.iter().collect();
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    fields
}

/// Failure to represent a contract in the scalar v2 layout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Variable-length fields require a different codec.
    #[error("field `{field}` has unsupported scalar type `{ty}`")]
    UnsupportedType {
        /// Offending field name.
        field: String,
        /// Unsupported primitive spelling.
        ty: &'static str,
    },
    /// Payload length does not fit the host address space.
    #[error("scalar layout size overflow")]
    Overflow,
}

/// Exact scalar v2 field size; no target ABI assumptions.
pub fn field_size(ty: PrimitiveType, field: &str) -> Result<usize, LayoutError> {
    Ok(match ty {
        PrimitiveType::Float64 | PrimitiveType::Int64 | PrimitiveType::Uint64 => 8,
        PrimitiveType::Float32 | PrimitiveType::Int32 | PrimitiveType::Uint32 => 4,
        PrimitiveType::Bool => 1,
        PrimitiveType::Str => return Err(LayoutError::UnsupportedType { field: field.to_owned(), ty: "string" }),
    })
}
