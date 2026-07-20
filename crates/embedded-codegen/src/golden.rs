//! Golden encode/decode vectors: the cross-language conformance anchor.
//!
//! For a contract, a fixed set of deterministic value profiles is encoded with
//! the reference [`ScalarValue::encode`]. Every projection — host Rust, `no_std`
//! Rust and C++ — must reproduce these exact bytes, so the vectors are the wire
//! contract the three implementations agree on. All values are exactly
//! representable (dyadic floats, small integers), so every language encodes them
//! bit-for-bit identically.

use neuradix_contracts::{Contract, PrimitiveType, schema_identity};
use serde::Serialize;

use crate::error::CodegenError;
use crate::wire::{ScalarValue, field_size};

/// One field's value within a golden vector.
#[derive(Debug, Clone, Serialize)]
pub struct GoldenField {
    /// Field name.
    pub name: String,
    /// Contract type spelling.
    pub ty: String,
    /// Human-readable value.
    pub value: String,
}

/// One golden vector: a named value profile and its expected wire bytes.
#[derive(Debug, Clone, Serialize)]
pub struct GoldenVector {
    /// Profile name (e.g. `zeros`, `units`, `ramp`, `edges`).
    pub name: String,
    /// The field values.
    pub fields: Vec<GoldenField>,
    /// The expected wire bytes as lowercase hex.
    pub bytes_hex: String,
}

/// A contract's full golden set.
#[derive(Debug, Clone, Serialize)]
pub struct GoldenSet {
    /// `namespace/name@version`.
    pub contract: String,
    /// Content-addressed schema identity.
    pub schema_id: String,
    /// The fixed wire length in bytes.
    pub wire_len: usize,
    /// The vectors.
    pub vectors: Vec<GoldenVector>,
}

impl GoldenSet {
    /// The concrete scalar values behind each vector, in field order — used by
    /// the code generators to bake conformance harnesses. Recomputed
    /// deterministically from the same profiles.
    pub fn value_rows(
        contract: &Contract,
    ) -> Result<Vec<(String, Vec<ScalarValue>)>, CodegenError> {
        let mut rows = Vec::new();
        for profile in PROFILES {
            let mut values = Vec::new();
            for (i, field) in contract.spec.payload.fields.iter().enumerate() {
                values.push(sample(field.ty, *profile, i)?);
            }
            rows.push((profile.name().to_owned(), values));
        }
        Ok(rows)
    }
}

/// The deterministic value profiles.
#[derive(Debug, Clone, Copy)]
enum Profile {
    Zeros,
    Units,
    Ramp,
    Edges,
}

const PROFILES: &[Profile] = &[
    Profile::Zeros,
    Profile::Units,
    Profile::Ramp,
    Profile::Edges,
];

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Profile::Zeros => "zeros",
            Profile::Units => "units",
            Profile::Ramp => "ramp",
            Profile::Edges => "edges",
        }
    }
}

/// Build the golden set for a contract.
pub fn golden_vectors(contract: &Contract) -> Result<GoldenSet, CodegenError> {
    // Validate all field types up front (also computes the wire length).
    let mut wire_len = 0usize;
    for field in &contract.spec.payload.fields {
        wire_len += field_size(field.ty, &field.name)?;
    }

    let mut vectors = Vec::new();
    for profile in PROFILES {
        let mut bytes = Vec::new();
        let mut fields = Vec::new();
        for (i, field) in contract.spec.payload.fields.iter().enumerate() {
            let value = sample(field.ty, *profile, i)?;
            value.encode(&mut bytes);
            fields.push(GoldenField {
                name: field.name.clone(),
                ty: field.ty.as_contract_str().to_owned(),
                value: value.display(),
            });
        }
        vectors.push(GoldenVector {
            name: profile.name().to_owned(),
            fields,
            bytes_hex: to_hex(&bytes),
        });
    }

    Ok(GoldenSet {
        contract: format!(
            "{}/{}@{}",
            contract.metadata.namespace, contract.metadata.name, contract.metadata.version
        ),
        schema_id: schema_identity(contract).as_str().to_owned(),
        wire_len,
        vectors,
    })
}

/// A deterministic, exactly-representable sample value for a field at index `i`.
fn sample(ty: PrimitiveType, profile: Profile, i: usize) -> Result<ScalarValue, CodegenError> {
    let k = i as i64;
    Ok(match ty {
        PrimitiveType::Float64 => ScalarValue::F64(match profile {
            Profile::Zeros => 0.0,
            Profile::Units => 1.0,
            Profile::Ramp => k as f64 + 0.25,
            Profile::Edges => -9.75,
        }),
        PrimitiveType::Float32 => ScalarValue::F32(match profile {
            Profile::Zeros => 0.0,
            Profile::Units => 1.0,
            Profile::Ramp => k as f32 - 0.5,
            Profile::Edges => -0.125,
        }),
        PrimitiveType::Int32 => ScalarValue::I32(match profile {
            Profile::Zeros => 0,
            Profile::Units => 1,
            Profile::Ramp => (k as i32) * 100 - 50,
            Profile::Edges => -12_345,
        }),
        PrimitiveType::Int64 => ScalarValue::I64(match profile {
            Profile::Zeros => 0,
            Profile::Units => 1,
            Profile::Ramp => k * 1000 - 500,
            Profile::Edges => -9_876_543_210,
        }),
        PrimitiveType::Uint32 => ScalarValue::U32(match profile {
            Profile::Zeros => 0,
            Profile::Units => 1,
            Profile::Ramp => (k as u32) * 7 + 1,
            Profile::Edges => 4_000_000_000,
        }),
        PrimitiveType::Uint64 => ScalarValue::U64(match profile {
            Profile::Zeros => 0,
            Profile::Units => 1,
            Profile::Ramp => (k as u64) * 11 + 3,
            Profile::Edges => 12_000_000_000_000_000_000,
        }),
        PrimitiveType::Bool => ScalarValue::Bool(match profile {
            Profile::Zeros => false,
            Profile::Units => true,
            Profile::Ramp => i.is_multiple_of(2),
            Profile::Edges => false,
        }),
        PrimitiveType::Str => {
            return Err(CodegenError::UnsupportedType {
                field: "<string>".to_owned(),
                ty: "string",
            });
        }
    })
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
