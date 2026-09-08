//! Typed simulation errors.

/// An error constructing or running a simulation.
#[derive(Debug, thiserror::Error)]
pub enum SimError {
    /// A physical or sensor parameter was outside its valid range.
    #[error("invalid simulation parameter `{name}`: {reason}")]
    InvalidParameter {
        /// The offending parameter.
        name: &'static str,
        /// Why it is invalid.
        reason: String,
    },

    /// The simulation step was not a strictly positive duration.
    #[error("simulation step must be a strictly positive duration")]
    NonPositiveStep,

    /// Advancing the simulation clock overflowed the time domain.
    #[error("simulation clock error: {0}")]
    Clock(#[from] neuradix_time::TimeError),
}
