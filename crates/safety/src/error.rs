//! Typed safety-configuration errors.
//!
//! Note: a *rejected command* is not an error — it is a normal [`crate::SafetyDecision`]
//! with a fail-safe output. These errors are only for invalid safety
//! *configuration* (e.g. a constraint whose bounds are inverted).

/// Errors from constructing safety configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SafetyError {
    /// A range constraint had non-finite or inverted bounds.
    #[error("invalid range constraint `{id}`: finite min ({min}) <= max ({max}) required")]
    InvalidRange {
        /// Rule identifier.
        id: &'static str,
        /// Lower bound as written.
        min: String,
        /// Upper bound as written.
        max: String,
    },

    /// A slew-rate constraint had a non-finite or negative rate.
    #[error("invalid slew constraint `{id}`: rate ({rate}) must be finite and non-negative")]
    InvalidSlew {
        /// Rule identifier.
        id: &'static str,
        /// Rate as written.
        rate: String,
    },
    /// An authority envelope had non-finite or inverted bounds.
    #[error("command envelope requires finite min <= max")]
    InvalidEnvelope,
    /// The configured safe output was non-finite or outside a hard range.
    #[error("safe output must be finite and satisfy every hard output range")]
    InvalidSafeOutput,
}
