//! Payload codecs.
//!
//! The recording layer is payload-agnostic: it stores and reproduces opaque
//! bytes. A [`RecordCodec`] converts between a typed message and those bytes.
//! Keeping serialization out of the recorder means the record format does not
//! prematurely commit to a wire encoding (Protobuf, etc. come later) and each
//! contract can choose an appropriate, deterministic encoding.

use crate::error::RecordError;

/// Encode and decode a typed message to and from recorded payload bytes.
///
/// Implementations MUST be deterministic: encoding the same message twice must
/// produce identical bytes, and `decode(encode(m)) == m`.
pub trait RecordCodec {
    /// The message type this codec encodes.
    type Message;

    /// Encode a message to bytes.
    fn encode(&self, message: &Self::Message) -> Vec<u8>;

    /// Decode a message from bytes.
    fn decode(&self, bytes: &[u8]) -> Result<Self::Message, RecordError>;
}
