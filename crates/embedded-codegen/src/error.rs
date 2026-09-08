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

    /// A supported wire scalar cannot be represented by the selected target ABI.
    #[error(
        "field `{field}` has type `{ty}`, unsupported by C++ target `{target}`; use a supported contract type or an explicit conversion component"
    )]
    UnsupportedTargetType {
        /// Offending field name.
        field: String,
        /// Contract primitive spelling.
        ty: &'static str,
        /// Selected target capability profile.
        target: &'static str,
    },
}
