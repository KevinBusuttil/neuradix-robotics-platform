//! Typed error taxonomy for contract handling.
//!
//! Foundation library crates expose typed errors rather than string or
//! `anyhow` errors so that callers (the CLI, the runtime, generators) can map
//! failures to stable behaviour such as CLI exit codes.

use std::path::PathBuf;

/// The result type used throughout the contracts crate.
pub type Result<T> = std::result::Result<T, ContractError>;

/// A single validation problem discovered in an authored contract.
///
/// Validation collects as many problems as possible before returning, so an
/// author can see every issue in one pass rather than fixing them one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Dotted path to the offending location, e.g. `spec.delivery.overflow`.
    pub path: String,
    /// Human-readable description of the problem.
    pub message: String,
}

impl ValidationIssue {
    /// Construct a new issue at `path` with `message`.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Errors that can occur while reading, parsing, validating, hashing or
/// generating from a Neuradix contract.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    /// The contract file could not be read from disk.
    #[error("failed to read contract file `{path}`: {source}")]
    Io {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The document was not syntactically valid YAML, or did not match the
    /// expected contract shape.
    #[error("failed to parse contract `{path}`: {source}")]
    Parse {
        /// Path that failed to parse.
        path: PathBuf,
        /// Underlying deserialization error.
        #[source]
        source: serde_yaml::Error,
    },

    /// The contract parsed but failed one or more validation rules.
    #[error(
        "contract `{name}` is invalid ({} issue(s)): {}",
        issues.len(),
        issues.iter().map(ValidationIssue::to_string).collect::<Vec<_>>().join("; ")
    )]
    Invalid {
        /// Best-effort identifier of the contract (`namespace/name`), or the
        /// file path when the identifier could not be determined.
        name: String,
        /// The full set of validation issues.
        issues: Vec<ValidationIssue>,
    },

    /// The contract used a construct that is understood by the model but is not
    /// yet supported by the requested code generator.
    #[error("code generation unsupported for `{name}`: {reason}")]
    Unsupported {
        /// Contract identifier (`namespace/name`).
        name: String,
        /// Why generation could not proceed.
        reason: String,
    },
}

impl ContractError {
    /// Convenience constructor for a single-issue validation failure.
    pub fn invalid_one(
        name: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        ContractError::Invalid {
            name: name.into(),
            issues: vec![ValidationIssue::new(path, message)],
        }
    }
}
