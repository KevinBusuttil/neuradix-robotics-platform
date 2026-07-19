//! Typed inspection errors.

/// An error from an inspection query.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// The requested channel is not present in the recording.
    #[error("unknown channel id {0}")]
    UnknownChannel(u16),

    /// The requested scalar field was not produced by the decoder.
    #[error("scalar field `{0}` not found")]
    FieldNotFound(String),

    /// A payload could not be decoded into scalar fields.
    #[error("payload decode error: {0}")]
    Decode(String),
}
