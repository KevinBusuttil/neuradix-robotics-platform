//! Stable component identity.

use crate::error::ComponentError;

/// A stable, human-meaningful logical identity for a component instance.
///
/// Per requirement COMP-007, components are addressable by a stable logical
/// identity that is independent of process ID. This newtype guarantees the
/// identity is non-empty and free of surrounding whitespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(String);

impl ComponentId {
    /// Create a component id, rejecting empty or whitespace-only names.
    pub fn new(id: impl Into<String>) -> Result<Self, ComponentError> {
        let id = id.into();
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(ComponentError::InvalidId(
                "component id must not be empty".to_owned(),
            ));
        }
        Ok(ComponentId(trimmed.to_owned()))
    }

    /// The identity as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
