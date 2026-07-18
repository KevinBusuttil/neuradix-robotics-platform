//! Typed runtime errors.

use crate::lifecycle::LifecycleState;

/// Errors raised by component lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    /// The requested transition is not permitted from the current state.
    #[error("illegal lifecycle transition from `{from}` to `{to}`")]
    IllegalTransition {
        /// The state the component is currently in.
        from: LifecycleState,
        /// The state that was requested.
        to: LifecycleState,
    },
}

/// Errors raised by component implementations and construction.
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    /// A component identity was invalid.
    #[error("invalid component id: {0}")]
    InvalidId(String),

    /// A lifecycle hook failed.
    #[error("component operation failed: {0}")]
    Failed(String),
}
