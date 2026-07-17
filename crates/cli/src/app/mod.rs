//! Application services: the operation logic behind each CLI command.
//!
//! Services take already-parsed inputs and return structured data or a typed
//! [`AppError`]; they never parse command lines and never render output. This
//! keeps the command semantics independent of both the `clap` parser and the
//! terminal presentation layer.

pub mod contract;
pub mod doctor;
pub mod version;

use serde_json::Value;

use crate::exit::ExitCode;

/// A successful command outcome: structured data plus any warnings.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The structured payload for the result envelope's `data` field.
    pub data: Value,
    /// Non-fatal warnings to surface to the user.
    pub warnings: Vec<String>,
}

impl Outcome {
    /// A successful outcome carrying `data` and no warnings.
    pub fn new(data: Value) -> Self {
        Self {
            data,
            warnings: Vec::new(),
        }
    }

    /// A successful outcome carrying `data` and `warnings`.
    pub fn with_warnings(data: Value, warnings: Vec<String>) -> Self {
        Self { data, warnings }
    }
}

/// A failed command outcome carrying an exit code, error messages and any
/// partial structured data.
#[derive(Debug, Clone)]
pub struct AppError {
    /// The process exit code to return.
    pub exit: ExitCode,
    /// Human-readable error messages.
    pub errors: Vec<String>,
    /// Optional structured data (e.g. per-file validation results).
    pub data: Value,
}

impl AppError {
    /// Construct an error with the given exit code and a single message.
    pub fn message(exit: ExitCode, message: impl Into<String>) -> Self {
        Self {
            exit,
            errors: vec![message.into()],
            data: Value::Null,
        }
    }

    /// Attach structured data to an error, returning the modified error.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}
