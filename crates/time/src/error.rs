//! Typed time errors.
//!
//! `TimeError` is hand-written (no `thiserror`) so this crate stays dependency-
//! free and `no_std`-compatible; it implements [`core::error::Error`], which is
//! the same trait as `std::error::Error`, so `?` into `Box<dyn Error>` still
//! works on hosted targets.

use crate::domain::ClockDomain;

/// Errors from time arithmetic and clock operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// Two timestamps (or a timestamp and a clock) belong to different domains,
    /// so the operation is not meaningful.
    DomainMismatch {
        /// The domain on the left-hand side of the operation.
        left: ClockDomain,
        /// The domain on the right-hand side of the operation.
        right: ClockDomain,
    },

    /// A time computation overflowed the representable range.
    Overflow,
}

impl core::fmt::Display for TimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TimeError::DomainMismatch { left, right } => {
                write!(f, "clock domain mismatch: `{left}` vs `{right}`")
            }
            TimeError::Overflow => f.write_str("time arithmetic overflow"),
        }
    }
}

impl core::error::Error for TimeError {}
