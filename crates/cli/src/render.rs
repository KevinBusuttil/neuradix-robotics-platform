//! Rendering of [`CommandResult`] envelopes into the selected output format.
//!
//! Rendering is the only layer that knows about presentation; application
//! services never format output themselves.

use serde_json::Value;

use crate::envelope::{CommandResult, Status};

/// The output format selected via `--output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable, aligned text (default).
    #[default]
    Table,
    /// Pretty-printed JSON of the full envelope.
    Json,
    /// YAML of the full envelope.
    Yaml,
}

/// Render a command result to a string in the requested format.
pub fn render(result: &CommandResult, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(result)
            .unwrap_or_else(|e| format!("{{\"renderError\":\"{e}\"}}")),
        OutputFormat::Yaml => {
            serde_yaml::to_string(result).unwrap_or_else(|e| format!("renderError: {e}\n"))
        }
        OutputFormat::Table => render_table(result),
    }
}

fn render_table(result: &CommandResult) -> String {
    let mut out = String::new();
    let status = match result.status {
        Status::Success => "success",
        Status::Failure => "failure",
    };
    out.push_str(&format!("command: {}\n", result.command));
    out.push_str(&format!("status:  {status}\n"));

    if !result.data.is_null() {
        out.push_str("data:\n");
        render_value(&result.data, 1, &mut out);
    }

    if !result.warnings.is_empty() {
        out.push_str("warnings:\n");
        for w in &result.warnings {
            out.push_str(&format!("  - {w}\n"));
        }
    }
    if !result.errors.is_empty() {
        out.push_str("errors:\n");
        for e in &result.errors {
            out.push_str(&format!("  - {e}\n"));
        }
    }
    out
}

/// Recursively render a JSON value as indented human-readable text.
fn render_value(value: &Value, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if is_scalar(v) {
                    out.push_str(&format!("{pad}{k}: {}\n", scalar_to_string(v)));
                } else {
                    out.push_str(&format!("{pad}{k}:\n"));
                    render_value(v, indent + 1, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if is_scalar(item) {
                    out.push_str(&format!("{pad}- {}\n", scalar_to_string(item)));
                } else {
                    out.push_str(&format!("{pad}-\n"));
                    render_value(item, indent + 1, out);
                }
            }
        }
        scalar => out.push_str(&format!("{pad}{}\n", scalar_to_string(scalar))),
    }
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Object(_) | Value::Array(_))
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}
