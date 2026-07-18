//! Typed recording errors.

/// The result type used throughout the record crate.
pub type Result<T> = std::result::Result<T, RecordError>;

/// Errors from writing, reading, decoding or verifying a recording.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// An I/O error while writing or reading a recording.
    #[error("recording i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// The recording manifest could not be (de)serialized.
    #[error("recording manifest error: {0}")]
    Manifest(#[source] serde_json::Error),

    /// The byte stream is not a Neuradix recording (bad magic).
    #[error("not a Neuradix recording (bad magic bytes)")]
    BadMagic,

    /// The recording uses an unsupported format version.
    #[error("unsupported recording format version {0}")]
    UnsupportedVersion(u8),

    /// The recording ended unexpectedly or is corrupt at the given byte offset.
    #[error("recording is truncated or corrupt at byte offset {0}")]
    Truncated(usize),

    /// A record payload exceeds the maximum encodable size.
    #[error("record payload too large: {0} bytes")]
    PayloadTooLarge(usize),

    /// A record referenced an unknown clock-domain code.
    #[error("unknown clock-domain code {0}")]
    UnknownClockDomain(u8),

    /// A codec failed to decode a payload.
    #[error("payload decode error: {0}")]
    Decode(String),
}
