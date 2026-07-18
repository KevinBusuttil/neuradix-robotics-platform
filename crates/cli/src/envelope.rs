//! The versioned machine-readable result envelope.
//!
//! Every command produces a [`CommandResult`] independently of how it is
//! rendered (see [`crate::render`]). This keeps operation logic separate from
//! terminal presentation, per the CLI architecture in the implementation plan.
//!
//! Volatile timing fields (`startedAt` / `finishedAt`) are intentionally omitted
//! so that JSON/YAML output is stable and can be asserted in golden tests
//! without normalisation.

use serde::Serialize;

/// The outcome status of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The command succeeded.
    Success,
    /// The command failed.
    Failure,
}

/// The stable, versioned result envelope for a CLI command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// Envelope schema version.
    pub api_version: &'static str,
    /// Always `CommandResult`.
    pub kind: &'static str,
    /// Dotted command name, e.g. `contract.validate`.
    pub command: String,
    /// Success or failure.
    pub status: Status,
    /// Execution context (always `local` in this increment).
    pub context: String,
    /// Command-specific structured payload.
    pub data: serde_json::Value,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
    /// Errors that caused a failure status.
    pub errors: Vec<String>,
}

impl CommandResult {
    /// The current envelope API version.
    pub const API_VERSION: &'static str = "cli.neuradix.io/v1alpha1";

    /// Build a success envelope carrying `data`.
    pub fn success(command: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            api_version: Self::API_VERSION,
            kind: "CommandResult",
            command: command.into(),
            status: Status::Success,
            context: "local".to_owned(),
            data,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Build a failure envelope carrying `errors`.
    pub fn failure(command: impl Into<String>, errors: Vec<String>) -> Self {
        Self {
            api_version: Self::API_VERSION,
            kind: "CommandResult",
            command: command.into(),
            status: Status::Failure,
            context: "local".to_owned(),
            data: serde_json::Value::Null,
            warnings: Vec::new(),
            errors,
        }
    }

    /// Attach warnings, returning the modified envelope.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }
}
