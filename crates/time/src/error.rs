//! Typed time errors.

use crate::domain::ClockDomain;

/// Errors from time arithmetic and clock operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    /// Two timestamps (or a timestamp and a clock) belong to different domains,
    /// so the operation is not meaningful.
    #[error("clock domain mismatch: `{left}` vs `{right}`")]
    DomainMismatch {
        /// The domain on the left-hand side of the operation.
        left: ClockDomain,
        /// The domain on the right-hand side of the operation.
        right: ClockDomain,
    },

    /// A time computation overflowed the representable range.
    #[error("time arithmetic overflow")]
    Overflow,
}
