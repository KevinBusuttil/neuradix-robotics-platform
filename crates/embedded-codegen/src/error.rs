//! Typed code-generation errors.

/// An error generating an embedded projection.
#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    /// A field type is not representable in the fixed-layout embedded wire.
    #[error("field `{field}` has type `{ty}`, which the embedded wire does not support")]
    UnsupportedType {
        /// The offending field name.
        field: String,
        /// The unsupported contract type spelling.
        ty: &'static str,
    },

    /// A usable type name could not be derived from the contract name.
    #[error("cannot derive a type name from contract `{0}`")]
    BadName(String),
}
