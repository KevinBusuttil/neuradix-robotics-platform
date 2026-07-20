//! The fixed-layout embedded wire format.
//!
//! A payload is encoded as its scalar fields in declaration order, each a
//! fixed-width little-endian value (IEEE-754 for floats, two's-complement for
//! integers, a single `0`/`1` byte for `bool`). The layout is identical on the
//! host, in `no_std` Rust and in C++, and the [golden vectors](crate::golden)
//! pin it. Variable-length types (strings) are rejected — an embedded contract
//! uses fixed scalars so a frame's size is known at compile time.

use neuradix_contracts::PrimitiveType;

use crate::error::CodegenError;

/// The fixed byte width of a supported scalar type.
pub fn field_size(ty: PrimitiveType, field: &str) -> Result<usize, CodegenError> {
    Ok(match ty {
        PrimitiveType::Float64 | PrimitiveType::Int64 | PrimitiveType::Uint64 => 8,
        PrimitiveType::Float32 | PrimitiveType::Int32 | PrimitiveType::Uint32 => 4,
        PrimitiveType::Bool => 1,
        PrimitiveType::Str => {
            return Err(CodegenError::UnsupportedType {
                field: field.to_owned(),
                ty: "string",
            });
        }
    })
}

/// A concrete scalar value used to build a golden vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarValue {
    /// A 64-bit float.
    F64(f64),
    /// A 32-bit float.
    F32(f32),
    /// A signed 32-bit integer.
    I32(i32),
    /// A signed 64-bit integer.
    I64(i64),
    /// An unsigned 32-bit integer.
    U32(u32),
    /// An unsigned 64-bit integer.
    U64(u64),
    /// A boolean.
    Bool(bool),
}

impl ScalarValue {
    /// Append this value's little-endian wire bytes to `out`. This is the
    /// reference encoder that defines the wire the generated code must match.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            ScalarValue::F64(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::F32(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::I32(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::I64(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::U32(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::U64(x) => out.extend_from_slice(&x.to_le_bytes()),
            ScalarValue::Bool(b) => out.push(*b as u8),
        }
    }

    /// A Rust literal for this value (used inside a typed struct literal, so no
    /// type suffix is needed; bit-exact for the dyadic floats used by golden
    /// vectors).
    pub fn rust_literal(&self) -> String {
        match self {
            ScalarValue::F64(x) => float_literal(*x, false),
            ScalarValue::F32(x) => float_literal(*x as f64, false),
            ScalarValue::I32(x) => format!("{x}"),
            ScalarValue::I64(x) => format!("{x}"),
            ScalarValue::U32(x) => format!("{x}"),
            ScalarValue::U64(x) => format!("{x}"),
            ScalarValue::Bool(b) => format!("{b}"),
        }
    }

    /// A C++ literal for this value (with the suffix its type needs).
    pub fn cpp_literal(&self) -> String {
        match self {
            ScalarValue::F64(x) => float_literal(*x, false),
            ScalarValue::F32(x) => format!("{}f", float_literal(*x as f64, false)),
            ScalarValue::I32(x) => format!("{x}"),
            ScalarValue::I64(x) => format!("{x}LL"),
            ScalarValue::U32(x) => format!("{x}u"),
            ScalarValue::U64(x) => format!("{x}ULL"),
            ScalarValue::Bool(b) => format!("{b}"),
        }
    }

    /// A human-readable value string for the golden-vector record.
    pub fn display(&self) -> String {
        match self {
            ScalarValue::F64(x) => format!("{x}"),
            ScalarValue::F32(x) => format!("{x}"),
            ScalarValue::I32(x) => format!("{x}"),
            ScalarValue::I64(x) => format!("{x}"),
            ScalarValue::U32(x) => format!("{x}"),
            ScalarValue::U64(x) => format!("{x}"),
            ScalarValue::Bool(b) => format!("{b}"),
        }
    }
}

/// Format a float so it always carries a decimal point (so C++/Rust read it as a
/// floating literal, not an integer). Only exactly-representable (dyadic) values
/// are used by golden vectors, so the shortest round-trip form is bit-exact.
fn float_literal(x: f64, _f32: bool) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}
