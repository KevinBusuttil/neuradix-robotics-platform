//! Typed safety-configuration errors.
//!
//! Note: a *rejected command* is not an error — it is a normal [`crate::SafetyDecision`]
//! with a fail-safe output. These errors are only for invalid safety
//! *configuration* (e.g. a constraint whose bounds are inverted).

/// Errors from constructing safety configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SafetyError {
    /// A range constraint had `min > max`.
    #[error("invalid range constraint `{id}`: min ({min}) exceeds max ({max})")]
    InvalidRange {
        /// Rule identifier.
        id: &'static str,
        /// Lower bound as written.
        min: String,
        /// Upper bound as written.
        max: String,
    },

    /// A slew-rate constraint had a negative rate.
    #[error("invalid slew constraint `{id}`: rate ({rate}) must be non-negative")]
    InvalidSlew {
        /// Rule identifier.
        id: &'static str,
        /// Rate as written.
        rate: String,
    },
}
