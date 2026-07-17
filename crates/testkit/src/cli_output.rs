//! Structured CLI output assertions.
//!
//! Parses the versioned CLI result envelope so tests can assert on its fields
//! without hand-writing JSON traversal.

use serde::Deserialize;
use serde_json::Value;

/// A parsed CLI result envelope (`cli.neuradix.io/v1alpha1` / `CommandResult`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedEnvelope {
    /// Envelope schema version.
    pub api_version: String,
    /// Envelope kind.
    pub kind: String,
    /// Dotted command name.
    pub command: String,
    /// `success` or `failure`.
    pub status: String,
    /// Execution context.
    pub context: String,
    /// Structured payload.
    pub data: Value,
    /// Warnings.
    pub warnings: Vec<String>,
    /// Errors.
    pub errors: Vec<String>,
}

impl ParsedEnvelope {
    /// Parse a JSON envelope string.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Assert the envelope has the expected API version and kind.
    ///
    /// # Panics
    /// Panics if either field is unexpected.
    pub fn assert_well_formed(&self) -> &Self {
        assert_eq!(
            self.api_version, "cli.neuradix.io/v1alpha1",
            "unexpected apiVersion"
        );
        assert_eq!(self.kind, "CommandResult", "unexpected kind");
        self
    }

    /// Assert the command name.
    ///
    /// # Panics
    /// Panics if the command name does not match.
    pub fn assert_command(&self, expected: &str) -> &Self {
        assert_eq!(self.command, expected, "unexpected command");
        self
    }

    /// Assert the status is `success`.
    ///
    /// # Panics
    /// Panics if the status is not `success`.
    pub fn assert_success(&self) -> &Self {
        assert_eq!(
            self.status, "success",
            "expected success, errors: {:?}",
            self.errors
        );
        self
    }

    /// Assert the status is `failure`.
    ///
    /// # Panics
    /// Panics if the status is not `failure`.
    pub fn assert_failure(&self) -> &Self {
        assert_eq!(self.status, "failure", "expected failure");
        self
    }

    /// Borrow a field from `data` by key.
    pub fn data_field(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_asserts_a_success_envelope() {
        let json = r#"{
          "apiVersion":"cli.neuradix.io/v1alpha1","kind":"CommandResult",
          "command":"version","status":"success","context":"local",
          "data":{"name":"neuradix"},"warnings":[],"errors":[]
        }"#;
        let env = ParsedEnvelope::parse(json).unwrap();
        env.assert_well_formed()
            .assert_command("version")
            .assert_success();
        assert_eq!(env.data_field("name").unwrap(), "neuradix");
    }
}
