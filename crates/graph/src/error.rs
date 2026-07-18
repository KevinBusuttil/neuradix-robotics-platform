//! Typed graph errors (for I/O and parsing; validation problems are reported as
//! [`crate::GraphIssue`]s, not errors).

use std::path::PathBuf;

/// Errors from reading or parsing a deployment manifest.
#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    /// The manifest file could not be read.
    #[error("failed to read deployment `{path}`: {source}")]
    Io {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The manifest was not valid YAML / did not match the expected shape.
    #[error("failed to parse deployment `{path}`: {source}")]
    Parse {
        /// Path that failed to parse.
        path: PathBuf,
        /// Underlying deserialization error.
        #[source]
        source: serde_yaml::Error,
    },
}
